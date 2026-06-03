# Key Pool Router Architecture

## Goal

Key Pool Router is a personal AI account and API key management gateway. It exposes OpenAI-compatible and named-pool HTTP forwarding endpoints while managing upstream providers, credential pools, credential lifecycle state, routing policy, and upstream-specific failure semantics.

The project deliberately keeps the first implementation backend-only. A future UI should call management APIs; it should not share routing internals or read credential storage directly.

## Layers

## Reference-Derived Boundaries

The routing core intentionally borrows only the stable parts of mature API gateway designs:

- LiteLLM's useful primitive is `client model -> deployment candidates`. A deployment has its own upstream model name, API base, authentication material, route weight, retry/cooldown state, and health. Key Pool Router maps this to `ModelRoute -> RouteTarget[] -> Channel/Pool`, while keeping individual credential lifecycle inside the pool selector instead of treating a key as an anonymous string.
- New API and One API use a practical `channel + ability` model: a channel binds an upstream endpoint, key material, model list, groups, model mapping, priority, weight, and retry behavior; an ability is the materialized `(group, model, channel)` selection index. Key Pool Router keeps the same materialized-selection idea, but splits the large channel object into typed `Provider`, `Account`, `CredentialSet`, `Pool`, `ModelRoute`, `PolicyProfile`, and `RoutingProfile` resources so future management writes do not entangle secrets, protocol adapters, and route policy.
- all-api-hub is a useful reference for control-plane concerns: multi-site account management, credential discovery, health checks, model and price synchronization, and export/sync workflows. It is not a request-path gateway model for this project. Those capabilities belong behind management/import/probe adapters, not inside proxy forwarding.
- OmniRouter and OmniRoute reinforce the same boundary from two directions. The compact OmniRouter design keeps `Provider`, `Model`, `Group`, and weighted group membership as operator-managed resources, with remote model fetching used only as an import aid. OmniRoute's larger resilience design separates connection cooldown, quota cache, per-model lockout, provider breaker, and fallback policy into distinct domain services. Key Pool Router adopts those separations, but keeps request selection on compiled in-memory snapshots instead of querying a database or live catalog on the hot path.

The consequence is a hard boundary: startup or management code may parse YAML, load registry rows, expand provider templates, probe upstreams, synchronize catalogs, calculate prices, and compile route indexes. The request hot path may only read already compiled in-memory snapshots: public model route plans, per-channel pool state, provider adapters, flattened error classifiers, and flattened routing policies. It must not query YAML, registry storage, credential storage, profile provenance, upstream discovery jobs, pricing tables, or external management sites.

Provider templates are therefore a bootstrap/control-plane convenience. A template instance expands into ordinary typed resources before `AppState` is built and then disappears. Adding support for more relay stations should normally add a template or import adapter at this boundary; it should not add relay-specific branches to `proxy.rs`, `pool.rs`, or `routing.rs`.

### Protocol Layer

`src/proxy.rs` owns HTTP protocol adaptation:

- authenticate incoming requests through `auth`;
- extract the requested model for OpenAI-compatible `/v1/*` traffic;
- serve `/v1/models` from the compiled public model-route catalog visible to the authenticated client token;
- apply route-target `upstream_model` rewrites through structured OpenAI JSON body handling;
- map `/pools/{pool}/...` traffic to a named upstream pool for non-OpenAI protocols;
- forward request method, headers, body, query, and streaming responses;
- convert local gateway failures into JSON errors.

This layer should not decide credential state transitions. It observes upstream responses and delegates state changes to `routing`.

OpenAI-compatible request bodies are bounded and buffered because model extraction, public-to-upstream model rewriting, and same-request retry need a replayable JSON body. Generic named-pool request bodies are bounded but streamed to the upstream by default; they are treated as non-replayable pass-through traffic and should not allocate memory proportional to the whole upload. Successful upstream responses are streamed instead of being buffered by the gateway, because successful payloads do not need classifier inspection.

`/v1/models` is a read-only catalog projection over the compiled runtime registry. It does not select upstream credentials, call upstream `/v1/models`, buffer upstream catalog bodies, or apply lifecycle transitions. The response contains only public client-facing model ids from explicit `model_routes` whose enabled targets are visible to the authenticated client token's channel scope and whose public model id is visible to the token's model scope. If no compiled public route is visible, the endpoint returns an OpenAI-compatible empty list. Upstream model ids discovered from providers or used as route-target rewrites remain internal until a management sync explicitly stages them as public routes. The response does not expose channel ids, provider names, credential state, or internal routing reasons.

### Routing Policy Layer

`src/routing.rs` owns the decision made after an upstream error. This mirrors LiteLLM's separation between router strategy, retry policy, and cooldown handling.

Route planning must preserve complete target metadata into forwarding. The proxy consumes `ForwardTarget { channel_id, upstream_model }`, not just channel ids, so one public client model can map to different upstream model names on different relay stations. OpenAI-compatible model rewrites parse and reserialize JSON request bodies; they do not use ad hoc string replacement.

Each request carries a frozen route plan from planning into forwarding. The plan includes the request id, the channel registry generation, the public model when one was resolved, and the bounded ordered target set. Forwarding must consume this plan object rather than recomputing candidates or passing a naked vector of channels. Routing telemetry records the plan generation on `route_selected` events, so management views can distinguish which registry snapshot produced a decision when runtime configuration becomes reloadable.

Route planning treats credential availability as a conservative read of the compiled runtime state. If the credential pool lock is contended, planning reports that target as having no proven available credential instead of assuming availability. The route-state query does not advance the selector's cooldown indexes; actual credential selection remains responsible for lazy cooldown promotion when it owns the pool lock. This keeps preview/planning side-effect-free and avoids optimistic routing decisions based on state that could not be observed.

Provider/account failure domains are runtime-only suppression state layered above channel health. A provider-unavailable failure still marks the selected channel cooling down or degraded, but it also opens or degrades the selected account and provider failure domains in memory. While a domain is open because upstream supplied an explicit cooldown such as `Retry-After`, every channel sharing that account or provider is treated as cooling down by route planning and named-channel/default-channel preflight. Rate-limit and quota evidence stay credential-scoped and do not contribute to provider/account suppression. A successful upstream response closes the selected channel's account and provider domains. This is intentionally smaller than OmniRoute's persisted circuit breaker: no database writes, no background half-open worker, and no request-path storage join.

Model routing is represented as explicit `ModelRoute` candidate sets. Public client-facing model names are configured under top-level `model_routes`; each route target points at a channel and may optionally rewrite to a provider-specific upstream model name. The runtime does not keep a separate one-model-to-one-channel map. Client channel scope is applied inside route planning before candidate limits are enforced, so an unauthorized high-priority target cannot hide an authorized lower-priority fallback.

Route strategy is explicit in the model route. `priority` keeps deterministic priority order. `priority_weighted_sticky` keeps the same bounded candidate set but chooses the first attempt from the best priority tier by stable request hash and target weight, then keeps the remaining candidates available for fallback. It does not scan usage counters or all credentials on the hot path.

The route candidate window is configured by `routing.max_route_candidates` and defaults to 16. This bounds per-request fallback work even if a public model has many relay/account targets. Management-only model discovery response buffering is separately bounded by `max_model_catalog_body_bytes`, defaulting to 512 KiB; sync planning may bound explicit discovery batches with `routing.max_model_catalog_channels`, also defaulting to 16. `/management/runtime` exposes these active values under `request_limits` and exposes a lightweight `retry_policy` summary for same-request credential retry usage.

Routing telemetry is a best-effort in-memory ring buffer. `routing.telemetry_buffer_capacity` defaults to 1024 and bounds resident telemetry memory; `/management/runtime` exposes the active capacity as `routing_telemetry_capacity`.

Automatic credential lifecycle persistence is also decoupled from forwarding through a bounded non-blocking queue owned by `AppState`. Proxy-side failure handling may update in-memory credential state and enqueue a durable lifecycle snapshot, but it does not write SQLite on the request path. If the queue is unavailable, full, or closed, the enqueue attempt is dropped, a warning is emitted, and `credential_lifecycle_persistence_dropped` is recorded in routing telemetry with request, channel, credential, lifecycle state, reason class, and drop reason.

Management domain events are persisted to JSONL when `management.event_log_path` is configured, but only a bounded recent window is retained in memory for `/management/events`. `management.event_window_capacity` defaults to 1024 and `/management/runtime` exposes it as `management_event_window_capacity`.

Management credential commands use short mutation-gate critical sections around state validation and final state application. Event append/persistence runs outside the mutation gate and send gate, so slow management storage does not block proxy-side automatic failure transitions or request sends on the same credential set. The command validates the initial state, persists durable lifecycle evidence and appends the management event outside those gates, then reacquires the send/mutation gates and rechecks selector generation and credential state before applying the runtime mutation. If durable lifecycle persistence succeeds but event append or transaction recording fails, the compensation snapshot is persisted through the async credential-store boundary rather than by running blocking store I/O on a Tokio worker.

Current policy:

- `Keep`: keep using the same credential, used for upstream key-switch cooldown errors.
- `Switch`: cool down the failed credential and move future requests to the next available credential.
- `Expire`: mark the credential expired and skip it for future requests.
- `QuotaExhaust`: mark the credential quota-exhausted and skip it for future requests when structured, credential-scoped quota evidence is explicitly mapped to a durable lifecycle action.
- Channel-scoped provider failures can mark a channel `CoolingDown` when the upstream provides cooldown evidence such as `Retry-After`; otherwise they mark it `Degraded`. Automatic channel-health transitions carry the selected channel-health generation from the request snapshot; stale transitions are rejected, successful transitions advance the generation, and manual `Disabled` state is never overwritten by automatic failure evidence. Active channel cooldown is a timed exclusion from explicit model-route planning, default-channel forwarding, and named-pool forwarding until the cooldown deadline elapses. Elapsed channel cooldown is lazily treated as available without requiring restart. Plain degraded state is avoided when a healthy route target exists, but it remains selectable as last resort for explicit model routes and single-target default/named forwarding.
- Same-request retry is controlled by the selected routing profile, not by pool-local booleans. It defaults to false for upstreams that limit key switching. When enabled, `max_same_request_retries` controls the bounded number of alternate credentials that may be tried inside the same replayable request. Retry only happens if the pool proves that it switched to a different available credential.
- Route-target retry is controlled by the selected routing profile and remains a separate directive from credential retry. Channel/provider failures can fall through to the next frozen route candidate when the request is replayable, no streaming response has started, and `route_target_retry.enabled` is true.

### Error Classification Layer

`src/error.rs` maps upstream HTTP status and error body into routing actions. The rules follow LiteLLM's broad idea: authentication failures and rate-limit/server failures affect deployment health, while arbitrary 4xx errors generally should not cause routing churn.

The classifier has three internal stages:

- extract structured upstream evidence such as status, error code, and limit type;
- apply default code/status classification;
- optionally apply explicit adaptation rules that can attach provider-specific cooldown or scope semantics without hardcoding relay-specific text.

Provider-specific behavior should enter through typed adaptation rules, not by parsing free-form upstream messages on the hot path. Rule config distinguishes omitted fields from explicit empty lists: omitted fields inherit defaults, while empty lists disable that rule set. Reusable top-level `policy_profiles` hold structured error rules that pools can reference and locally override; resolution still compiles one effective classifier per channel before runtime forwarding starts. Adaptation rule ids must be unique inside each profile, inside each pool override list, and after profile and pool rules are merged. Matchers must be non-empty. Config resolution rejects unsupported adaptation action combinations, not just unsupported scopes: any rule that changes `kind` or `primary_scope` must provide both fields and the resulting `(FailureKind, FailureScope)` pair must be implemented by the runtime state machine. Reserved or non-state-backed scopes such as `account`, `deployment`, `provider_adapter`, and `client_token` must fail config resolution until corresponding runtime state, management projection, and transitions exist. Management APIs expose the active rule set and rule provenance read-only through a management projection; proxy, routing, and pool code do not inspect policy profile storage or source metadata. Changing rules at runtime belongs in a later configuration persistence layer.

Phase 1A relay semantics are limited to typed classifier inputs. `relay_profile` has three values: `official_openai` keeps the default OpenAI-compatible assumption that bare `401`/`403` means selected-credential authentication failure; `generic_relay` treats bare `401`/`403` as request-only client errors unless structured invalid-key evidence is present; `untrusted_relay` uses the same conservative bare-auth handling as `generic_relay` and keeps same-request retry opt-in for rate-limit cases. Across all three profiles, structured invalid-key evidence remains credential-scoped auth failure, bare `429` remains credential-scoped rate limiting, code-less top-level error envelopes remain request-only client errors, and structured quota evidence is durable credential quota exhaustion only when `balance_scope` is `credential`.

`balance_scope` defaults to `credential`. In Phase 1A, `balance_scope: channel` is accepted by the YAML parser but rejected during semantic config resolution with a Phase 1B reservation error; `account`, `provider`, and `client_token` scopes are rejected because their runtime state machines and management projections do not exist. Error adaptation matchers are restricted to structured status/code/limit-type evidence. Free-form upstream message contains/regex matchers are not supported and must fail config resolution instead of being ignored or evaluated on the request path.

Reusable top-level `routing_profiles` hold routing behavior that is intentionally orthogonal to error classification: key selection strategy, default credential cooldown, same-request credential retry, and route-target retry. Config resolution compiles each profile once into `ResolvedRoutingProfile`; every pool selects either its explicit `routing_profile` or `default_routing_profile`, then receives a flattened `RoutingPolicy`. The proxy hot path reads only the selected channel's flattened policy from `PoolState`; it does not consult the global routing profile catalog. Management APIs expose the resolved profile catalog and profile-to-channel references read-only.

### Credential State Layer

`src/credentials.rs` defines the credential state machine. Credentials are not plain strings in the routing layer.

Current states:

- `Available`
- `CoolingDown { until, reason }`; management snapshots expose a bounded `remaining_seconds` value rather than the internal monotonic deadline.
- `Expired { reason }`
- `Disabled { reason }`; manual management state that removes a credential from selection until an explicit enable command.

Future states should be added here, not embedded in proxy code. Likely additions:

- `BudgetExhausted { reset_at }`

### Pool State Layer

`src/pool.rs` owns credential-set selection state. A channel has one `credential_set_id`, and `KeyPoolConfig.credential_namespace` is derived from that id rather than from the channel id. This means two channels that intentionally share a credential set also share credential identity semantics; later database-backed repositories can replace the file loader without changing proxy/routing code. It exposes a narrow API:

- select the current available credential;
- report switchable failure;
- report expired failure;
- report keep-key failure;
- clear credential cooldown without restoring expired credentials;
- disable and enable credentials without allowing stale cooldown deadlines to re-enable disabled credentials;
- snapshot pool status for management APIs.

The pool keeps a selector-local index of available credential positions and cooldown deadlines. Cooling-down credentials are promoted back to the available index lazily when selection or retry-candidate generation observes an elapsed monotonic deadline. This keeps the request-path critical section bounded by selector operations instead of requiring a full credential scan on every request.

Credential persistence belongs behind the credential store boundary; `pool.rs` remains runtime selection state only. HTTP forwarding should not know whether credentials came from a file, SQLite, or Postgres.

Management writes use a per-channel mutation gate to serialize precondition checks, durable persistence, audit event append, and state application without holding the credential pool lock across JSONL or store I/O. In writable-store mode, lifecycle snapshot persistence must succeed before a credential mutation is accepted; JSONL remains audit/display and is not the lifecycle replay authority. Rejected or idempotent commands do not enter the replay log.

### Configuration Layer

Startup configuration now flows through:

```text
YAML -> YamlRegistryRepository -> RegistryDocument -> ResolvedConfig -> AppState
```

`src/config.rs` keeps `AppConfig` as the YAML compatibility schema and semantic resolver implementation. `src/registry.rs` owns the registry document/repository boundary: `YamlRegistryRepository` reads YAML, applies compatibility defaults, and returns a `RegistryDocument`. `RegistryDocument` is an input-neutral, compatibility-defaulted resource document; `RegistryDocument::resolve*` performs semantic normalization, cross-resource validation, policy-profile merging, credential loading, and construction of `ResolvedConfig`.

Credential secrets remain in `src/credential_repository.rs`, not in `RegistryRepository`. File-backed keys are a bootstrap repository implementation only. Top-level `credential_sets` are the only file-backed credential import entry, and every pool references one of them through `credential_set`. The credential repository boundary loads by `CredentialSetId` plus a typed source spec, so SQLite/Postgres implementations can replace file loading without changing proxy, routing, pool, provider, model-catalog, or other request-path code. Request forwarding must not call `RegistryRepository`, carry `RegistryDocument`, parse YAML, or inspect `AppConfig`.

When `KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE` is set, startup uses the SQLite credential repository. The first load of a credential set imports the configured key file with source path, line, batch, fingerprint, and duplicate-count metadata; later loads are authoritative from SQLite, so the running service no longer depends on the original key file. `AppState` also owns a credential store handle that represents either read-only file-bootstrap mode or a writable store. SQLite is the first local writable adapter for single-node personal deployment; PostgreSQL can be added later behind the same store boundary if multi-process or remote database deployment becomes necessary. Management credential import uses the handle to append new credentials, while request forwarding uses in-memory pool state and does not read or write any credential store on the request hot path.

Client-facing virtual keys follow the same bootstrap-to-store pattern. YAML `client_tokens` are required in read-only bootstrap mode, but become optional when a writable SQLite store is configured. On first writable-store startup, configured YAML client tokens are imported into SQLite; after the SQLite table contains client tokens, it is authoritative for virtual key authentication and scope across restarts, and the YAML bootstrap tokens may be removed. Editing YAML `client_tokens.allowed_model_groups` or `allowed_channels` does not rewrite an existing stored token. `allowed_model_groups` is a legacy field name but now accepts two scope forms: a direct public model id, or a top-level `model_groups` id whose members are public `model_routes` ids. The resolver validates group ids, rejects empty groups, rejects empty members, deduplicates members, and requires every member to reference a known public model route. Runtime authentication keeps the token's raw scope entries, while `AppState.runtime_catalogs` holds compiled in-memory group membership. `/v1/models`, OpenAI request authorization, named-pool replayable model authorization, and routing preview all use that compiled membership and do not query registry storage or upstream catalogs on the request path. Management-created client tokens store only stable SHA-256 token hashes and scope metadata; the plaintext token is accepted at creation time but is not returned by management responses. Runtime authentication reads the in-memory token registry, and management create/enable/disable/scope-update operations persist first, then update that registry, so proxy forwarding does not query SQLite on the request hot path.

When `KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE` is set, startup uses YAML as the bootstrap source for process settings and secrets, then uses SQLite as the persisted registry-resource source. On an empty registry database, the router bootstraps providers, accounts, credential-set descriptors, channels, model routes, policy profiles, routing profiles, and registry defaults from YAML. `model_groups` are registry resources in the YAML resolver, but the first SQLite registry-store slice does not persist them yet; writable-store bootstrap rejects non-empty `model_groups` instead of silently dropping client-visible scope membership. On later starts, persisted non-secret resources are loaded from SQLite and overlaid onto the YAML bootstrap document; listen address, management credentials, client token bootstrap, limits, timeouts, and other process settings still come from YAML or their dedicated stores. Management registry writes, such as provider/account/channel upsert, provider/account/channel enablement, model-route upsert, policy-profile upsert, and routing-profile upsert, update SQLite desired configuration and return a committed registry version. The response reports the staged registry version and whether runtime reload is required. `POST /management/runtime/reload` explicitly rebuilds the compiled runtime channel/provider/account/model-route/model-group snapshot and the management policy/routing catalog projection from the persisted registry, then advances the active registry version; proxy forwarding continues to use only compiled in-memory route, credential, scope, and per-channel policy state and never reads the registry store or global profile catalogs on the request hot path.

Providers, accounts, and channels have persistent configured enablement separate from runtime health. A provider defines the upstream protocol family, such as `openai_compatible`; an account binds a provider to an upstream API base and authentication header policy. A pool/channel references an account, a credential set, and a routing profile; it may also reference a policy profile or add channel-local error adaptation rules. A disabled provider, account, or channel makes every affected channel unavailable for default routing, named-pool forwarding, explicit model-route planning, compiled client model catalog projection, and upstream-model-name claiming. Channel configured disablement is stored on `PoolConfig.enabled` and projected into runtime as `PoolState.configured_enabled`; it must not be represented as `ChannelHealth::Disabled`, because runtime health is an operational state machine with independent generation and transition rules. If a pool omits `account`, the resolver still creates synthetic `provider:<provider-kind>` and `account:<channel-id>` resources for bootstrap configs, but new configs should use explicit top-level `providers` and `accounts`.

YAML bootstrap may use `upstreams` as a provider-template shortcut. Template expansion happens before `AppConfig` deserialization and registry resolution. A template instance expands into ordinary provider, account, credential-set, channel, policy-profile, and model-route resources, then disappears; `AppState`, proxy forwarding, route planning, credential selection, and management snapshots consume only the expanded typed resources. This follows the provider/channel/model-map split used by mature gateways while keeping site-specific API base, auth header, policy, and default model knowledge outside the request hot path. Unknown templates are rejected during startup instead of falling back to generic OpenAI-compatible guesses.

Channel ids are normalized and validated during config resolution. `default_pool`, client-token `allowed_channels`, and explicit `model_routes.*.targets[].channel` must reference a known channel before `AppState` is built. Upstream connection parameters are validated at the same boundary: resolved pool/account `api_base` and upstream auth header must be non-empty, while auth prefix is preserved exactly because schemes such as `Bearer ` encode their separator in that prefix. This keeps route planning, management previews, forwarding, and later database-backed registry storage from carrying dangling or unusable string references into runtime.

Every resolved channel carries a stable `config_generation` derived from the effective channel configuration after provider/account inheritance. Request selection snapshots and management channel responses expose this generation. It is separate from selector generation: config generation changes when channel configuration changes; selector generation changes when explicit credential selector mutations change the selector-visible state kind. Time-derived cooldown expiry is evaluated from monotonic deadlines and materialized lazily by the pool selector; it does not append a management event or advance selector generation by itself.

Planned storage migration:

1. Keep YAML for service settings and provider definitions.
2. Use SQLite-backed credential bootstrap and management credential import for local deployment.
3. Persist management-driven credential lifecycle snapshots behind the same credential store boundary. In writable-store mode, those snapshots become startup authority for durable credential state; JSONL remains audit/display and does not also replay credential lifecycle state.
4. Add a registry store boundary for provider, account, channel, model-route, policy-profile, and routing-profile configuration writes. Registry writes should apply typed commands in a transaction, produce a full `RegistryDocument`, and resolve that document successfully before the write is accepted. The first local adapter should be SQLite; the boundary should not be named around SQLite.
5. Promote upstream credentials from secret rows plus selector snapshots into explicit credential resources with import batches, redacted source lineage, structured transition evidence, and optional validation probe summaries. This resource model still belongs behind `CredentialStore`; registry storage should only reference credential-set ids.
6. Add PostgreSQL behind the same repository boundary only if multi-process deployment, concurrent writers, or remote database operation needs it.

### Management API Layer

`src/management.rs` exposes backend JSON status endpoints. It is not a frontend.

Current endpoints:

- `GET /management/pools`
- `GET /management/channels`
- `GET /management/providers`
- `PUT /management/registry/providers/{id}`
- `POST /management/registry/providers/{id}/enable`
- `POST /management/registry/providers/{id}/disable`
- `GET /management/accounts`
- `PUT /management/registry/accounts/{id}`
- `POST /management/registry/accounts/{id}/enable`
- `POST /management/registry/accounts/{id}/disable`
- `PUT /management/registry/channels/{id}`
- `POST /management/registry/channels/{id}/enable`
- `POST /management/registry/channels/{id}/disable`
- `PUT /management/registry/model-routes/{model...}`
- `PUT /management/registry/policy-profiles/{id}`
- `GET /management/credential-sets`
- `GET /management/credential-sets/{id}/imports`
- `GET /management/credential-sets/{id}/imports/{batch_id}`
- `GET /management/credential-sets/{id}/operations`
- `GET /management/credential-sets/{id}/credentials`
- `GET /management/credential-sets/{id}/credentials/{credential_id}`
- `GET /management/credential-sets/{id}/credentials/{credential_id}/history`
- `GET /management/credential-sets/{id}/credentials/{credential_id}/probes`
- `POST /management/credential-sets/{id}/credentials/{credential_id}/probe`
- `POST /management/credential-sets/{id}/credentials/{credential_id}/apply-latest-probe`
- `POST /management/credential-sets/{id}/credentials/import`
- `POST /management/credential-sets/{id}/credentials/{credential_id}/expire`
- `POST /management/credential-sets/{id}/credentials/{credential_id}/restore`
- `POST /management/credential-sets/{id}/credentials/{credential_id}/disable`
- `POST /management/credential-sets/{id}/credentials/{credential_id}/enable`
- `POST /management/credential-sets/{id}/credentials/{credential_id}/reset-cooldown`
- `GET /management/client-tokens`
- `POST /management/client-tokens`
- `PATCH /management/client-tokens/{id}`
- `POST /management/client-tokens/{id}/disable`
- `POST /management/client-tokens/{id}/enable`
- `GET /management/model-routes`
- `GET /management/policy-profiles`
- `GET /management/policy-profiles/{id}`
- `GET /management/routing-profiles`
- `PUT /management/registry/routing-profiles/{id}`
- `GET /management/routing-profiles/{id}`
- `GET /management/routing/preview?model={model}&client_token={token_id_or_name}`
- `GET /management/runtime`
- `POST /management/runtime/reload`
- `GET /management/alerts`
- `GET /management/channels/{id}`
- `POST /management/channels/{id}/model-discovery`
- `POST /management/model-discovery/sync-plan`
- `POST /management/model-discovery/sync-apply`
- `POST /management/channels/{id}/reset-health`
- `GET /management/channels/{id}/error-rules`
- `GET /management/channels/{id}/credentials`
- `POST /management/channels/{id}/credentials/{credential_id}/expire`
- `POST /management/channels/{id}/credentials/{credential_id}/restore`
- `POST /management/channels/{id}/credentials/{credential_id}/disable`
- `POST /management/channels/{id}/credentials/{credential_id}/enable`
- `POST /management/channels/{id}/credentials/{credential_id}/reset-cooldown`
- `GET /management/events`
- `GET /management/routing-telemetry`

Management responses are redaction boundaries. They may expose stable ids, provider kinds, channel ids, credential counts, health, selector generations, and basename-like credential source ids. They must not expose raw upstream keys, raw client or management tokens, token hashes, absolute key-file paths, URL userinfo, or token-like query parameters.

`PUT /management/registry/providers/{id}` persists a complete desired provider definition through the registry store. The payload uses the same typed `ProviderConfig` schema as bootstrap configuration: provider protocol kind and configured enablement. A successful response includes the committed registry version and reports `runtime_reload_required: true`; `/management/providers` continues to show the active in-memory provider projection until reload or restart.

`PUT /management/registry/model-routes/{model...}` persists a complete desired model-route definition through the registry store. The model path is a catch-all segment so public model ids may contain `/`, matching common provider/model naming conventions. It validates the candidate registry document with the same resolver used at startup, so target channels must exist and policy/routing references must remain coherent. The write is transactional: validation failure returns a conflict and leaves the previous persisted route intact. A successful response includes the committed registry version and reports `runtime_reload_required: true`; `/management/model-routes` continues to show the active in-memory route table until a later explicit reload or process restart applies the persisted registry.

`PUT /management/registry/accounts/{id}` persists a complete desired account definition through the registry store. The payload uses the same typed `AccountConfig` schema as bootstrap configuration: provider id, upstream API base, auth header, auth prefix, and configured enablement. The resolver validates the provider reference and normalizes connection fields before commit acceptance, so accounts cannot point at unknown providers or carry an empty API base into runtime. A successful response includes the committed registry version and reports `runtime_reload_required: true`; `/management/accounts` continues to show the active in-memory account projection until reload or restart.

`PUT /management/registry/channels/{id}` persists a complete desired channel definition through the registry store. The payload uses the same typed `PoolConfig` schema as bootstrap configuration, but new managed configs should use explicit top-level provider/account resources and treat channel-local upstream fields as compatibility defaults. The resolver validates account, credential-set, policy-profile, routing-profile, and local error-rule references before commit acceptance. A successful response includes the committed registry version and reports `runtime_reload_required: true`; `/management/channels` continues to show the active in-memory channel registry until reload or restart.

`PUT /management/registry/policy-profiles/{id}` persists a complete desired policy profile through the registry store. The payload uses the same typed `PolicyProfileConfig` schema as bootstrap configuration, including ordered, named, enableable error adaptation rules and probe-result actions. The resolver compiles those rules before commit acceptance, so unsupported lifecycle action combinations such as account-scoped rate limits are rejected transactionally. Probe-result actions are resolved once per channel alongside the error classifier: by default `invalid` maps to `expire`, `success` maps to `restore`, and transient or ambiguous outcomes remain `noop`; a profile may opt selected outcomes into runtime `cooldown` with a bounded `cooldown_seconds`, or map credential-scoped stable quota evidence to durable `quota_exhaust`. Provider-level and unsupported-model probe outcomes cannot be configured to expire or quota-exhaust individual credentials. A successful response includes the committed registry version and reports `runtime_reload_required: true`; `/management/policy-profiles` continues to show the active in-memory policy catalog until reload or restart.

Policy inspection uses the existing management surfaces. `/management/policy-profiles` and `/management/policy-profiles/{id}` show the resolved policy profile catalog, while `/management/channels/{id}/error-rules` shows the effective per-channel classifier after profile inheritance and channel-local overrides, including the resolved `relay_profile`, `balance_scope`, structured matchers, actions, and provenance. There is no separate Phase 1A management endpoint for relay profiles.

`PUT /management/registry/routing-profiles/{id}` persists a complete desired routing profile through the registry store. The payload uses the same typed `RoutingProfileConfig` schema as bootstrap configuration: key selection strategy, default credential cooldown, same-request credential retry budget, and route-target retry. The resolver validates policy coherence before commit acceptance, for example rejecting disabled same-request retry with a nonzero retry budget. A successful response includes the committed registry version and reports `runtime_reload_required: true`; `/management/routing-profiles` continues to show the active in-memory routing policy catalog until reload or restart.

`/management/model-routes` includes each target channel's runtime health so an operator can distinguish static route configuration from temporary channel cooldown or degradation without inspecting routing internals.

`/management/routing/preview` is a read-only explanation endpoint for the same route-planning boundary used by proxy requests. It accepts a client-visible model name and an optional client token id or name, applies the token's model/channel scope, explicit model routes or default-channel fallback, channel health, route strategy, and the configured candidate limit, then returns the selected target and per-target inclusion or exclusion reasons. It distinguishes `channel_cooling_down` from `channel_degraded`: active cooldown is a timed exclusion, while degraded is a last-resort health warning. It does not call upstream providers, write events or telemetry, or call credential selection APIs that mutate the pool's sticky current index. Its credential data is limited to selector generation, credential-set id, and aggregate lifecycle counts.

`POST /management/channels/{id}/model-discovery` is a management-only, read-only probe for one OpenAI-compatible runtime channel. It calls the channel's upstream `/v1/models` endpoint with one currently available credential, returns sorted unique model ids plus redacted provider/account/channel/credential metadata, and classifies non-2xx upstream evidence without applying lifecycle transitions or writing registry state. Some relay stations do not implement a standard OpenAI model catalog and may return a successful non-catalog payload; that is reported as a structured `catalog_unsupported_or_malformed` provider-adapter error with an empty model list, not as inferred model data. The endpoint is intentionally separate from route sync: adding discovered models to `model_routes` remains an explicit registry write so client requests continue to use compiled in-memory routes and never perform upstream discovery on the hot path.

`POST /management/model-discovery/sync-plan` is the corresponding read-only planning step for one or more channels. The request supplies `channel_ids`; the service runs the same channel discovery boundary, compares discovered model ids with the active in-memory `model_routes` snapshot, and returns deterministic actions: `would_create_route` when no public route exists, `would_add_target` when a route exists but does not target that channel, and `unchanged` when the channel is already a route target. The endpoint does not write the registry store, reload runtime, update virtual-key scope, mutate credential lifecycle, or infer models from client traffic.

`POST /management/model-discovery/sync-apply` is the explicit write step for discovery results. It requires a writable registry store, repeats discovery through the same redacted management boundary, compares against the staged registry document rather than the active runtime snapshot, and persists route changes as one validated registry batch. New discovered models become `priority` routes targeting the discovering channel; existing routes keep their strategy and targets, with the discovering channel appended only if it is missing. The endpoint never hot-reloads runtime and never changes client-token scope, so client traffic sees the new routes only after `POST /management/runtime/reload` or process restart applies the staged registry.

`/management/providers` aggregates channel health counts per provider and counts unique accounts, not channels. `/management/accounts` aggregates all channels that reference the same account and exposes `channel_ids`, credential-set ids, channel-health counts, and credential counts. Account responses distinguish the account's own configured `enabled` state from `provider_enabled` and `effective_enabled`, so a disabled provider is not misreported as a disabled account. Credential lifecycle counts in `/management/providers`, `/management/accounts`, and `/management/runtime` are deduplicated by credential set within that resource scope; channel health remains channel-counted because each channel can have distinct routing health even when it shares credentials. `/management/credential-sets` aggregates all channels that reference the same credential set and exposes redacted import metadata plus credential lifecycle counts once per set, including separate expired and quota-exhausted counts, so shared credential pools are visible without leaking raw keys or absolute source paths. `/management/credential-sets/{id}/operations` projects the operator-facing state of one credential set from the same runtime snapshot: `healthy`, `degraded`, `transition_required`, or `exhausted`, plus serving mode, whether requests can still be accepted, whether operator input is required, the required action, and bounded alert records. `/management/alerts` aggregates those resource-level alerts across credential sets, preserves severity counts and resource ids, and stays read-only so a CLI, UI, or later webhook sink can react when key rotation is required while the proxy request path remains storage-free. `/management/credential-sets/{id}/imports` lists persisted import batch records from the credential store with pagination, source kind, redacted source reference, and aggregate counts; `/management/credential-sets/{id}/imports/{batch_id}` returns one such persisted import batch. Neither endpoint exposes raw keys or absolute key-file paths. `/management/credential-sets/{id}/credentials` lists credentials directly by credential set id with the same pagination and state filters as the channel-scoped compatibility endpoint, including `state=quota_exhausted`. `/management/credential-sets/{id}/credentials/{credential_id}` combines the current runtime credential state with the durable credential resource projection, including stable position, source line, batch id, redacted source reference, persistent fingerprint, and the latest redacted probe summary when one exists, without returning the raw secret or absolute source path. `/management/credential-sets/{id}/credentials/{credential_id}/history` lists durable lifecycle transition records from the credential store with pagination, normalized state and source fields, and the same redaction boundary. `/management/credential-sets/{id}/credentials/{credential_id}/probe` runs a management-only, single-credential probe through the channel/account/provider auth boundary and persists a normalized result class without automatically expiring, disabling, restoring, or quota-exhausting the credential. Provider adapters own the probe target: by default OpenAI-compatible channels use model retrieve against `/v1/models/{model}`; when the request specifies `kind: chat_completion`, they instead issue a bounded non-streaming chat completion probe, classify any 2xx response as `success` for default availability probing, and only require an exact assistant-message match when the request explicitly supplies `expected_output`. Provider kinds without a structured probe, such as `generic_http`, persist `unsupported_model` evidence without calling the upstream. `/management/credential-sets/{id}/credentials/{credential_id}/probes` lists persisted probe results with pagination. `/management/credential-sets/{id}/credentials/{credential_id}/apply-latest-probe` is an explicit management action that applies the latest redacted probe evidence to one credential according to the channel's resolved policy profile. Durable outcomes still use the same persisted lifecycle command path as manual mutations: `expire`, `quota_exhaust`, and `restore` record probe-derived evidence and use distinct audit event kinds. Runtime `cooldown` is deliberately transient selector state and is not written as a durable lifecycle snapshot. Probe responses store and return structured evidence such as outcome, classifier id, adaptation rule id, upstream status, upstream code, upstream limit type, channel id, provider id, account id, latency, and timestamp; they do not store raw upstream response bodies or free-form upstream text. `/management/credential-sets/{id}/credentials/import` appends new credentials to the writable credential store and running in-memory pool, ignores duplicate fingerprints, advances selector generation only when new credentials become available, and returns redacted credential statuses. Set-scoped credential mutation endpoints resolve the credential set to a stable canonical channel and then use the same persisted management command path as channel-scoped mutations, so shared set state, event replay or store snapshot restore, selector preconditions, and redaction stay consistent. Credential sets keep their configured id in management output.

Set-scoped lifecycle command endpoints include `expire`, `quota-exhaust`, `restore`, `disable`, `enable`, and `reset-cooldown`. `quota-exhaust` is the manual counterpart to policy-applied probe quota evidence: it records a durable `quota_exhausted` lifecycle snapshot and a `credential_quota_exhausted` audit event, while `restore` clears that durable state through the same command path used for expired credentials.

Channel-scoped credential mutation endpoints expose the same lifecycle command vocabulary for compatibility with older management clients. They delegate to the same command executor and durable credential-store path as set-scoped mutations, so `quota-exhaust` has identical state, event, redaction, and restore semantics regardless of which management path is used.

Future endpoints should stay resource-oriented, for example:

- `POST /management/pools/{pool}/credentials/import`
- `PATCH /management/credentials/{id}`

Future registry-write endpoints must distinguish configured enablement from transient runtime health. A configured channel disable is a persistent registry property that survives restart and should be validated with the full registry document. A runtime health disable is an operational state transition used to remove a currently running channel from selection. These must not share an ambiguous command path once persistent channel CRUD exists.

## Frontend Boundary

The frontend should be a separate app or separate crate/package. It should use only management APIs and never call routing internals. The backend remains usable headlessly from CLI, curl, or other clients.

## Current Verification

The current backend has unit tests for:

- sticky selection until failure;
- switchable failures cooling down the failed key and advancing the pool only when another key is available;
- key-switch cooldown preserving the current credential;
- invalid credentials transitioning to expired;
- conservative no-same-request-retry behavior;
- opt-in same-request retry behavior.
- duplicate public model mappings resolving into multi-target route candidates;
- explicit empty error rules disabling inherited defaults.
- configured adaptation rules attaching provider-specific cooldown semantics from structured upstream evidence.
- provider/account management status endpoints and management redaction boundaries;
- manual channel health reset for transient channel cooldown/degraded state;
- basename-like credential source ids instead of absolute key-file paths;
- URL userinfo and token-like query suppression in management-visible upstream bases;
- route fallback gates for streaming and non-replayable requests;
- explicit model routes with disabled targets not falling back to default channels.
- configured-disabled channels being excluded from `/v1/models` compiled catalog projection.
- provider-disabled accounts retaining distinct configured and effective enabled status in management output.
- management credential commands capturing state and selector-generation preconditions, with idempotent success or conflict on stale apply.
- explicit production and management credential state mutations advancing selector generation when they change selector-visible state kind.
- manual cooldown reset clearing only `CoolingDown` credentials while rejecting `Expired` credentials.
- manual disable/enable state, including set-scoped management APIs, disabled-state filtering, selector generation movement, and replay of persisted disabled/enabled events.
- Phase 1A relay profile parsing and classifier semantics for `official_openai`, `generic_relay`, and `untrusted_relay`.
- `balance_scope` defaulting to `credential`, `channel` being parser-accepted but config-resolution rejected until Phase 1B, and account/provider/client-token scopes being rejected.
- unsupported free-form error message matcher fields being rejected, with request-path lifecycle decisions limited to structured status/code/limit-type evidence.
