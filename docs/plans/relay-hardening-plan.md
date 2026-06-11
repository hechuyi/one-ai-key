# Relay Hardening Implementation Plan

**Goal:** harden one-ai-key for real relay usage while preserving its lightweight personal/small-team AI key router boundary.

**Architecture:** request forwarding continues to read only compiled in-memory runtime state. Relay-specific behavior enters through typed config, profile presets, bounded pre-output response inspection, and management-only observability. The plan must not turn `/v1/models` into a live upstream aggregation endpoint, add request-path disk I/O, add request-path storage joins, or fully buffer successful responses.

**Tech Stack:** Rust, Axum, reqwest, serde/serde_yaml, SQLite-backed local stores, local x86_64 NixOS container build harness, focused integration tests.

---

## Review Basis

The original relay critique and early draft plan were reviewed from four engineering perspectives. The blocking findings were:

- State-machine/lifecycle review: Phase 1 lacked exact transition tables, failure-source ownership, reset scope, all-target suppression policy, and response-filter alert lifecycle.
- Hot-path/protocol review: Phase 2 lacked a 2xx scope contract, latency cap, SSE parser contract, guarded classifier evidence path, guard/filter ordering, and header/framing rules for body mutation.
- Operations/security review: management security was too late and too vague; local x86_64 NixOS container build, release artifact identity, repo hygiene, audit schema, allowlist behavior, and unknown-field rejection were not executable gates.
- Test/phase-slicing review: phases were too large, stop nodes were concept-level rather than red/green acceptance gates, and endpoint/schema assertions were missing.

The plan below adopts those blocking findings. It does not adopt scope-expanding ideas such as live upstream model aggregation, persistent model-discovery state machines, UI work, multi-tenant billing, heavyweight control-plane databases, active probe daemons, hedging, or alert/post-output response-filter-driven channel lifecycle mutation.

## Relation To Models

The relay hardening work is not primarily a model-list project. Model routing matters only as a boundary: clients see public model ids from compiled `model_routes`; route planning maps a public model to bounded channel candidates; `/v1/models` remains a local projection over the authenticated client's compiled scope. Upstream model discovery stays a management-only import aid and is not a request-path dependency.

The reviewed critique mentioned discovered/probed/verified/blocked model states as a possible future trust mechanism. That is parked. It requires a durable catalog model and a product decision about how much model verification one-ai-key should own. It is not needed to fix relay error semantics, 2xx false-success handling, response contamination visibility, retry risk, or management explainability.

## Mature Project Reference Synthesis

The plan uses mature systems as design references, not as feature templates.

Envoy and HAProxy contribute the passive-health lesson: distinguish local transport failure from upstream transaction failure, use bounded ejection/cooldown, make recovery automatic, and make retry pressure observable. one-ai-key translates this to typed channel/credential transitions, TTLs, capped backoff, generation preconditions, and management reset. It does not import active health-check clusters, hedging, or a large circuit-breaker matrix.

Kubernetes and Consul contribute the health-semantics lesson: liveness, serving readiness, and resilience are separate questions. one-ai-key keeps `/health` as process liveness, keeps `/ready` compatible with current serving-readiness behavior, and adds management health projections for serving and resilience so orchestration, operators, and tests do not infer different meanings from one endpoint.

LiteLLM, OpenRouter-style routers, and comparable key/channel routers contribute the client-abstraction lesson: the client configures one base URL, one client token, and public model ids while the router chooses backend targets. one-ai-key keeps that abstraction, but avoids platform features such as teams, billing, price catalogs, and live provider catalog fan-out.

LiteLLM and New API also contribute the retry/fallback reliability lesson: a relay should consume eligible backend, key, and channel failures before writing anything to the client whenever the request can be replayed safely. one-ai-key adopts only the lightweight pre-output form of that principle: same-request credential retry, frozen route-target fallback, typed cooldown, and explainable key/channel selection before returning a client-visible error. It explicitly does not adopt mid-stream continuation fallback, free-text keyword disablement, live model aggregation, complex multi-tenant billing/UI surfaces, or an active probe daemon.

OpenTelemetry contributes the event-contract lesson: events need stable names, bounded fields, redaction rules, and explicit cardinality limits. one-ai-key uses structured event fields such as request id, channel id, credential id/hash, failure source, failure kind, action, and reason code. Events must not include raw keys, client tokens, request bodies, response bodies, matched filter text, absolute key paths, or token-like URL components.

## Non-Negotiable Constraints

- Do not touch deployment servers during implementation or testing for this roadmap.
- Build and test locally. Release artifacts must be built in the local x86_64 NixOS container, not on deployment servers or remote builders.
- Do not commit raw upstream keys, client tokens, management tokens, local config, SQLite databases, JSONL event logs, release artifacts, private agent files, or generated `dist/` output.
- Do not make `/v1/models` call upstream providers. It remains a compiled runtime projection filtered by the authenticated client token.
- Do not add request-path disk I/O, request-path YAML/registry/client-token/credential-store joins, request-path upstream model discovery, or full buffering of successful responses.
- Do not parse free-form upstream text on the request path for lifecycle decisions. Only structured typed evidence may mutate lifecycle or routing state.
- Do not persist untrusted promotional/injection text in docs, tests, commits, config, events, alerts, or generated artifacts. Tests use neutral synthetic markers only.

## Canonical Terms

| Term | Contract |
| --- | --- |
| `FailureSource::LocalTransport` | The gateway failed to establish, send, receive headers, or receive a required pre-output frame before any upstream HTTP transaction evidence was available. It may affect transient channel availability only through explicitly allowed rules. It must not expire or quota-exhaust credentials. |
| `FailureSource::UpstreamTransaction` | The upstream returned a non-2xx response or a protocol-adapter error before client output began. Existing bounded error-body classification applies. |
| `FailureSource::GuardedSuccessEnvelope` | A body-bearing 2xx upstream response exposed an unambiguous structured error envelope before any byte was sent to the client. The original 2xx status is retained as evidence; lifecycle mutation is allowed only through typed rules. |
| `FailureKind::RelayBalanceUnavailable` | Shared relay balance/account capacity evidence. With `balance_scope: channel`, it creates transient selected-channel cooldown. It is not durable credential quota exhaustion. |
| `balance_scope: credential` | Quota or balance evidence applies only to the selected credential and may use existing durable credential quota semantics when the evidence is structured and stable. |
| `balance_scope: channel` | Relay balance evidence suppresses only the selected channel. It does not suppress all channels sharing an account, provider, or credential set in this roadmap. |
| `guard outcome: pass` | The success guard found no decisive error before its cap/deadline and reconstructs the exact peeked prefix plus remaining stream. |
| `guard outcome: classified` | The success guard found typed structured error evidence before output and sent it through the normal routing failure path. |
| `guard outcome: cap_exhausted` | The guard reached byte or time cap without a decisive envelope and passed through with exact prefix replay. |
| `response-filter event` | A bounded management-visible event recording rule id, action, channel/request context, and reason code. The event and alert projections are observability only. Lifecycle isolation can occur only from the explicit pre-commit rejecting actions before any response bytes are sent to the client. |

## Locked Invariants

1. Relay hardening is a typed policy/profile feature. It must not add relay-specific free-form branches in `proxy.rs` or lifecycle decisions based on untrusted text.
2. Channel-level relay balance handling is transient channel availability, not durable credential quota exhaustion.
3. Automatic channel transitions never override configured disablement or manual runtime `Disabled` state.
4. All retry and fallback decisions use one attempt-state path with bounded attempts, replayability checks, effective deadline checks, and observable denial reasons.
5. A body-bearing 2xx response may be inspected only in a bounded pre-output window. After any byte is sent to the client, the gateway must not fallback, retry, rewrite status, or mutate routing state from that response body.
6. Guard pass-through reconstructs bytes exactly: the peeked prefix is emitted once, then the remaining upstream stream. The reconstructed stream then enters the response filter exactly once.
7. Response filtering remains a stream-boundary safety feature. Its events and alerts do not mutate credential lifecycle, channel health, routing failure domains, or retry policy. Only explicit pre-commit rejecting actions may create typed lifecycle evidence, and they must reuse the normal bounded retry path.
8. `/health` is process liveness; `/ready` remains serving readiness for compatibility; management health endpoints carry the finer serving/resilience semantics.
9. `/v1/models` remains compiled-runtime-only. Model discovery remains management-only and explicit.
10. Management mutation routes are default-deny by role, auditable, redacted, and protected by a peer-address allowlist when management is exposed on a non-loopback listener.

## Decision Matrix

| Proposal | Decision | Reason |
| --- | --- | --- |
| `relay_profile` presets | Adopt | Fits template/config resolution if expanded into typed classifier and routing policy. |
| 401/403/429 semantics by profile | Adopt with table | Bare relay statuses are unsafe without exact kind/scope/retry/mutation outcomes. |
| `balance_scope: credential | channel` | Adopt narrowly | Credential uses existing durable quota path; channel uses transient selected-channel cooldown. Account/provider scopes are rejected in this roadmap. |
| Free-form message matcher on request path | Reject | High false-positive and injection risk; conflicts with structured-evidence boundary. |
| HTTP 2xx success guard | Adopt after retry observability | Detect clear structured errors before output without full buffering or parallel routing. |
| Broad schema validation | Reject for this roadmap | Valid relay streams vary too much; only clear error envelopes and first data-bearing SSE event are inspected. |
| Response filter health mutation | Adopt only for explicit pre-commit rejecting actions | Free-form, alert-driven, automatic disablement, and post-output mutation remain rejected. The accepted slice is bounded to configured actions that produce typed evidence before body commit. |
| Conservative retry/risk telemetry | Adopt before 2xx fallback | Guarded fallback can have unknown charge status; denial and risk fields must exist first. |
| Management roles, allowlist, audit | Adopt in Phase 0 | Management writes already have high blast radius; hardening must precede new relay mutation surfaces. |
| Model discovery states | Park | Not required for relay hardening and requires a durable catalog state model. |

## Phase 0: Foundation Gates Before Relay Semantics

**Purpose:** create the security, health, audit, config-validation, local-build, and release gates that every later phase depends on.

**Likely files:** `src/config.rs`, `src/auth.rs`, `src/main.rs`, `src/events.rs`, `src/management_runtime.rs`, `src/management_alerts.rs`, `docs/architecture.md`, `docs/release-build.md`, `README.md`, `scripts/local-ci.sh`, `scripts/build-release-x86_64-linux-docker.sh`, `scripts/build-release-x86_64-linux.sh`.

### 0A. Local Build And Release Harness

- Add or standardize `scripts/local-ci.sh` as the canonical local verification entrypoint. It must run `cargo fmt -- --check`, `cargo check --locked`, `cargo clippy --locked -- -D warnings`, and `cargo test --locked`.
- Add or standardize `scripts/build-release-x86_64-linux-docker.sh` as the canonical host-side release-artifact build entrypoint. It must enter the local x86_64 Linux Nix container, produce the binary artifact locally, and write a SHA256 file. `scripts/build-release-x86_64-linux.sh` is the guarded container-side entrypoint, not the host release gate.
- If the local x86_64 NixOS container is not available, the phase is blocked. Do not SSH to a server, use a remote builder, or declare a phase complete from host-only tests.
- Secret-backed integration tests must read ignored local config or env vars and must skip with a clear message when absent. They must not hardcode keys.

**Release gate:** before any GitHub release, run `git status -sb --untracked-files=all` and verify that no local config, SQLite database, JSONL log, key file, private agent file, `dist/` output, or release artifact is staged. Release assets must be named by version and target, include SHA256, and be immutable enough for a future NixOS pin by URL plus hash.

### 0B. Management Role And Network Boundary

Use three roles: `readonly`, `operator`, and `admin`. Existing `management.admin_token` remains a bootstrap admin principal for compatibility.

All `/management/*` routes must match exactly one default-deny rule. A test must enumerate registered management route patterns from `src/main.rs` or a single route registry and fail if any route lacks a minimum role. Wildcards or prefix permissions may be used in implementation only if the generated test expands them back to the exact registered method/path list.

| Minimum role | Routes |
| --- | --- |
| `readonly` | `GET /management/pools`, `GET /management/channels`, `GET /management/client-tokens`, `GET /management/providers`, `GET /management/accounts`, `GET /management/credential-sets`, `GET /management/credential-sets/:id/operations`, `GET /management/credential-sets/:id/credentials`, `GET /management/credential-sets/:id/imports`, `GET /management/credential-sets/:id/imports/:batch_id`, `GET /management/credential-sets/:id/credentials/:credential_id`, `GET /management/credential-sets/:id/credentials/:credential_id/history`, `GET /management/credential-sets/:id/credentials/:credential_id/probes`, `GET /management/model-routes`, `GET /management/policy-profiles`, `GET /management/policy-profiles/:id`, `GET /management/routing-profiles`, `GET /management/routing-profiles/:id`, `GET /management/routing/preview`, `GET /management/runtime`, `GET /management/alerts`, `GET /management/channels/:id`, `GET /management/channels/:id/error-rules`, `GET /management/channels/:id/credentials`, `GET /management/events`, `GET /management/routing-telemetry`, `GET /management/health/serving`, `GET /management/health/resilience`, `GET /management/response-filter-events` after Phase 4. |
| `operator` | `PUT /management/credential-sets/:id/credentials/:credential_id/metadata`, `POST /management/credential-sets/:id/credentials/:credential_id/probe`, `POST /management/credential-sets/:id/credentials/:credential_id/apply-latest-probe`, `POST /management/credential-sets/:id/credentials/import`, `POST /management/credential-sets/:id/credentials/:credential_id/expire`, `POST /management/credential-sets/:id/credentials/:credential_id/restore`, `POST /management/credential-sets/:id/credentials/:credential_id/quota-exhaust`, `POST /management/credential-sets/:id/credentials/:credential_id/disable`, `POST /management/credential-sets/:id/credentials/:credential_id/enable`, `POST /management/credential-sets/:id/credentials/:credential_id/reset-cooldown`, `POST /management/model-discovery/sync-plan`, `POST /management/channels/:id/model-discovery`, `POST /management/channels/:id/reset-health`, `POST /management/channels/:id/disable`, `POST /management/channels/:id/enable`, `POST /management/channels/:id/credentials/:credential_id/expire`, `POST /management/channels/:id/credentials/:credential_id/restore`, `POST /management/channels/:id/credentials/:credential_id/quota-exhaust`, `POST /management/channels/:id/credentials/:credential_id/disable`, `POST /management/channels/:id/credentials/:credential_id/enable`, `POST /management/channels/:id/credentials/:credential_id/reset-cooldown`. |
| `admin` | `POST /management/client-tokens`, `PATCH /management/client-tokens/:id`, `POST /management/client-tokens/:id/disable`, `POST /management/client-tokens/:id/enable`, `PUT /management/registry/providers/:id`, `POST /management/registry/providers/:id/disable`, `POST /management/registry/providers/:id/enable`, `PUT /management/registry/accounts/:id`, `POST /management/registry/accounts/:id/disable`, `POST /management/registry/accounts/:id/enable`, `PUT /management/registry/channels/:id`, `POST /management/registry/channels/:id/disable`, `POST /management/registry/channels/:id/enable`, `PUT /management/registry/model-routes/*model`, `PUT /management/registry/policy-profiles/:id`, `PUT /management/registry/routing-profiles/:id`, `POST /management/model-discovery/sync-apply`, `POST /management/runtime/reload`. |

Network boundary choice for this roadmap: implement `management.ip_allowlist` first, not a separate management listener. If the service `listen` address is non-loopback and management is enabled, startup must reject the config unless `management.ip_allowlist` is set. The allowlist uses the peer socket address, not forwarded headers. Loopback listeners allow loopback peers by default.

### 0C. Audit And Config Contracts

- Every management mutation emits a stable audit record with `created_at_unix_seconds`, actor id, role, action, resource type/id, outcome, request id when present, registry/runtime generation when relevant, and `reason_code`.
- Persisted/displayed audit records store `reason_code` only. Free-form operator notes, if accepted by request payloads for compatibility, are redacted from management responses and never written to JSONL.
- If durable audit logging is configured and append fails, the management mutation fails before applying runtime or durable state. Do not apply the mutation and then emit a best-effort audit-failure event.
- JSONL replay remains compatible with old records that lack `created_at_unix_seconds`.
- Add `deny_unknown_fields` or explicit unknown-field validation for all new config namespaces introduced by this roadmap: relay profile, balance scope, management principals, IP allowlist, audit settings, guard settings, and response-filter event settings.

### 0D. Health Split

Add management health projections before any new channel suppression behavior:

| Endpoint | Meaning | Status contract | Minimum fields |
| --- | --- | --- | --- |
| `GET /health` | Process liveness only | `200` if process can answer | Existing `ok` body may remain. |
| `GET /ready` | Backward-compatible serving readiness | `200` when at least one request path can serve; `503` when no serving route/credential/channel exists | Existing fields plus no weaker semantics. |
| `GET /management/health/serving` | Current ability to serve client traffic | `200` when service can accept at least one configured route; `503` otherwise | `status`, `serving`, `serving_channels`, `blocking_alerts`, `blocking_reasons[]`. |
| `GET /management/health/resilience` | Spare capacity and degradation | Always `200` if management auth succeeds | `status: ok|degraded|blocked`, `operator_input_alerts`, `credential_sets_without_spare`, `channels_cooling_down`, `response_filter_alerts`. |

**Phase 0 acceptance gate:** write red tests for route role matrix coverage, allowlist fail-closed behavior, audit redaction/timestamp/replay, health endpoint schemas, unknown-field rejection, local CI script, local x86_64 build script, and release artifact hygiene. Complete only when those tests pass and the local build artifact plus SHA256 are produced locally. If the local x86_64 environment cannot run, stop the roadmap as blocked; do not continue into relay semantics.

## Phase 1A: Relay Profiles And Typed Classifier Semantics

**Purpose:** make official provider and relay status semantics explicit without adding new channel suppression state yet.

**Likely files:** `src/config.rs`, `src/error.rs`, `src/upstream_templates.rs`, `src/management_profiles.rs`, `docs/architecture.md`, tests in focused modules or `src/main.rs`.

Add `relay_profile: official_openai | generic_relay | untrusted_relay` at template/config resolution. Existing configs keep current behavior unless they opt into a relay profile. Add `balance_scope`, defaulting to `credential`. Earlier Phase 1A slices rejected `balance_scope: channel` until the Phase 1B state transition existed; the current public configuration contract supports `balance_scope: channel` as selected-channel transient suppression and continues to reject account, provider, credential-set, and client-token balance scopes.

Classifier table for Phase 1A:

| Evidence | `official_openai` | `generic_relay` | `untrusted_relay` |
| --- | --- | --- | --- |
| Bare `401` or bare `403` | `AuthInvalid/Credential`, non-retryable, expire selected credential | `ClientError/RequestOnly`, non-retryable, no lifecycle mutation | `ClientError/RequestOnly`, non-retryable, no lifecycle mutation |
| Structured invalid-key code | `AuthInvalid/Credential`, non-retryable, expire selected credential | Same | Same |
| Bare `429` | `RateLimited/Credential`, retryable only through existing gates, credential cooldown | `RateLimited/Credential`, retryable only through existing gates, credential cooldown | `RateLimited/Credential`, no same-request retry unless explicitly enabled, credential cooldown |
| Structured credential quota with `balance_scope: credential` | `QuotaExhausted/Credential`, non-retryable, durable quota-exhaust selected credential | Same | Same |
| Structured channel balance with `balance_scope: channel` | `RelayBalanceUnavailable/Channel`, suppress selected channel transiently | Same | Same |
| Code-less top-level error object | `ClientError/RequestOnly`, non-retryable, no lifecycle mutation | Same | Same |

Unsupported matcher fields such as free-form message contains/regex matching must fail config resolution rather than being ignored. Account/provider/client-token balance scopes remain rejected until a future runtime state machine and management projection exist.

**Phase 1A acceptance gate:** red tests cover profile config resolution, current-config backward compatibility, each table row, unsupported field rejection, management policy projection fields, and absence of request-path free-form message parsing. The historical rejection of `balance_scope: channel` applied only before Phase 1B; current acceptance belongs to the Phase 1B selected-channel suppression contract. Complete only when these tests pass under `scripts/local-ci.sh` and the local x86_64 build gate still passes.

**Status note:** Phase 1A documentation and acceptance scope is limited to the typed classifier semantics above. The current management acceptance surface is the existing `/management/channels/:id/error-rules` effective-rule projection plus `/management/policy-profiles` and `/management/policy-profiles/:id` profile projections. `/v1/models` remains a compiled runtime projection over explicit `model_routes`; Phase 1A does not add live upstream model aggregation, channel balance suppression, free-form message matchers, or new management endpoints.

## Phase 1B: Channel Balance Suppression State

**Purpose:** enable `balance_scope: channel` as a transient selected-channel availability state.

**Likely files:** `src/config.rs`, `src/error.rs`, `src/routing.rs`, `src/failure_state_executor.rs`, `src/route_plan.rs`, `src/management_resources.rs`, `src/management_alerts.rs`, tests.

Phase 1B enables `FailureKind::RelayBalanceUnavailable` with `FailureScope::Channel` only for `balance_scope: channel`. It suppresses only the channel selected for the failed attempt. It does not suppress account, provider, credential set, or all channels sharing a key file, and it does not durably quota-exhaust the selected credential.

Out of scope for Phase 1B: Phase 2 retry telemetry and retry-pressure counters, Phase 3 guarded-success 2xx classification, and Phase 4 response-filter event lifecycle mutation. Phase 1B may preserve existing retry/fallback gates, existing routing telemetry, and existing response-filter behavior, but acceptance must not depend on adding those later mechanisms.

Phase 1B recognizes structured channel-balance evidence only when it comes from
`FailureSource::UpstreamTransaction`. Guarded-success 2xx classification and its
retry-risk telemetry remain Phase 3/Phase 2 work and are intentionally absent
from this table.

| Input | Allowed source state | Output | Retry | Recovery/reset |
| --- | --- | --- | --- | --- |
| Structured channel-balance evidence from `UpstreamTransaction` | `Available` or `Degraded` | `CoolingDown { reason_code: relay_balance_unavailable, source, until, suppression_count }` | `RetryRouteTarget` only if replayable, no output, effective deadline remains, route-target retry enabled | TTL expiry lazily makes channel available; next success resets relay suppression count. |
| Local transport failure | `Available` | Optional `Degraded { reason_code: local_transport_failure }` only if configured; no quota/auth mutation | Route fallback only through existing gates | Next success clears degraded state. |
| Any automatic transition | `Disabled` or configured disabled | No change | No retry added by the state mutation itself | Manual/configured disablement remains authoritative; cooldown expiry, success recovery, and suppression reset do not auto-enable disabled/configured-disabled channels. |
| Manual `reset-health` | `CoolingDown` or `Degraded` | `Available` and relay suppression counters for that channel cleared | No automatic retry | Does not clear credential expired/quota/disabled state or configured provider/account/channel disablement. |
| Success on selected channel | Any automatic transient state except `Disabled` | `Available` and provider/account success handling preserved | Not applicable | Clears selected-channel relay suppression count. |

Backoff is consecutive per channel, stored in memory, capped, and visible in management. Default cooldown must be small and configurable; explicit upstream cooldown evidence can override it. Generation preconditions match existing channel-health mutation rules: stale automatic transitions are rejected and reported, not applied.

All-target suppression policy: after client-token scope, configured enablement, and route candidate limits are applied, if every route target is excluded by active selected-channel cooldown/suppression, fail closed with `503` and JSON error `code: no_route_candidate`. The response includes redacted reason classes such as `channel_cooling_down` and never includes keys, upstream bodies, or internal secrets. A management alert reports affected public model id, candidate count, suppressed count, and reason codes.

Endpoint/schema acceptance:

| Surface | Required assertion |
| --- | --- |
| `GET /management/channels/:id` | `health.kind`, `health.reason_code`, `health.source`, `health.remaining_seconds`, `health.suppression_count`, `health.generation`. |
| `GET /management/routing/preview` | Per-candidate `included`, `selected`, `reasons[]`, and `health.kind` distinguish `channel_cooling_down` from `channel_degraded`. |
| `POST /management/channels/:id/reset-health` | Clears only transient channel health and relay suppression counters; disabled/configured-disabled states remain excluded. |
| `GET /management/alerts` | All-target suppression alert includes resource kind, public model id when available, channel ids, reason codes, severity. |

**Phase 1B acceptance gate:** red tests cover `balance_scope: channel` config acceptance, account/provider/credential-set scope rejection, transition table rows, no durable credential quota-exhaust from channel balance evidence, stale generation rejection, TTL expiry, success recovery, manual reset scope, disabled/configured-disabled exclusion, all-target `no_route_candidate`, management schemas, and local build gate. Stop as blocked if implementing account/provider/credential-set balance scope becomes necessary; do not widen scope inside this phase.

**Phase 1B documentation acceptance:** README and architecture docs must describe `balance_scope: channel` as selected-channel transient suppression, state that account/provider/credential-set suppression remains rejected, state that channel balance does not durably quota-exhaust credentials, state that disabled/configured-disabled channels are not auto-restored, state that all-target cooldown fails closed as `no_route_candidate`, and identify the management surfaces for channel health, routing preview, and alerts. They must also explicitly say that Phase 1B does not implement Phase 2 retry telemetry, Phase 3 2xx guard behavior, or Phase 4 response-filter lifecycle mutation.

## Phase 2: Retry Boundary And Risk Observability

**Purpose:** make retry/fallback decisions observable before 2xx guarded fallback is allowed.

**Likely files:** `src/routing.rs`, `src/events.rs`, `src/failure_observer.rs`, `src/management_runtime.rs`, `src/management_routing.rs`, docs and tests.

- Keep same-request credential retry opt-in and disabled by default.
- Preserve hard gates: non-replayable body, streaming request, partial output, attempt-limit exhaustion, and effective-deadline exhaustion must not retry.
- All same-request and route-target fallback attempts share one attempt-state path.
- Add an effective request deadline for retry/fallback if not already present in the selected timeout profile. Fallback must not start if it cannot fit inside the effective deadline.
- Add duplicate-charge risk fields for fallback after upstream transaction failure or guarded 2xx failure when charge status is unknown.
- Add bounded retry-pressure counters/events to prevent silent traffic amplification.
- Keep HTTP 2xx success-guard classification, response-filter lifecycle actions, and live `/v1/models` aggregation out of Phase 2.

Stable telemetry schema additions:

| Surface | Fields |
| --- | --- |
| `/management/routing-telemetry` event `retry_decision` | `request_id`, `public_model`, `channel_id`, `credential_id_hash`, `attempt`, `failure_source`, `failure_kind`, `failure_scope`, `directive: retry_credential|retry_route_target|retry_same_target|return_error`, `denial_reason`, `duplicate_charge_risk: none|unknown|known_no_charge`, `effective_deadline_remaining_ms`. |
| `/management/runtime` | Retry profile summary, `retry_pressure_capacity`, bounded recent retry/fallback counters split by directive, denial reason, and duplicate-charge risk. |
| `/management/routing/preview` | Read-only policy summary: route-target retry enabled, same-request credential retry enabled, max retries, candidate limit. |

Allowed `denial_reason` values: `failure_not_retryable`, `body_not_replayable`, `streaming_not_retryable`, `partial_output_started`, `attempt_limit_reached`, `policy_disabled`, `no_frozen_candidate`, `effective_deadline_exhausted`, `route_target_retry_disabled`, `no_route_candidate`. The field is present when the directive is `return_error`; retry directives leave it unset or `null`.

`duplicate_charge_risk` is a conservative retry-observability field, not a settlement or billing assertion. Use `none` when the gateway has no evidence of a completed upstream transaction, `known_no_charge` only when typed evidence proves the failed attempt could not have charged, and `unknown` whenever an upstream transaction or guarded-success envelope may have reached provider-side accounting before the gateway decided to retry. Phase 3 now uses the `guarded_success_envelope` source for classified 2xx guard failures and reuses the Phase 2 retry gates; it does not add a guard-specific retry branch.

The effective-deadline gate is evaluated before every fallback attempt. It uses the selected timeout profile's effective request deadline, including any existing per-request timeout budget, and denies retry as `effective_deadline_exhausted` when the next attempt cannot fit. Attempt limits and candidate limits remain independent gates; passing one does not bypass the deadline gate.

Retry-pressure observability is bounded. Implementations may use an in-memory ring, windowed counters, or equivalent capped structure, but `/management/runtime` must expose the configured capacity and recent counts without unbounded cardinality from model ids, upstream text, credentials, request ids, or raw provider payloads.

**Phase 2 acceptance gate:** red tests cover each denial reason, duplicate-charge risk for guarded and non-2xx fallback classes, effective-deadline denial, bounded retry-pressure capacity, schemas above, and existing same-request retry behavior. Tests must also assert that Phase 2 does not implement HTTP 2xx success-guard classification, response-filter lifecycle actions, or live `/v1/models` aggregation. Complete only when local CI and x86_64 build pass.

## Phase 3: HTTP 2xx Success Guard

**Purpose:** detect obvious relay error envelopes in body-bearing 2xx upstream responses before any client output, then reuse the normal failure and retry path from Phase 2.

**Implemented vertical slice files:** `src/success_guard.rs`, `src/proxy.rs`, `src/main.rs`, docs and tests. `src/upstream_response.rs` supplies the stream reconstruction helpers used by the guard pass-through path.

Scope: guard body-bearing `status.is_success()` upstream responses before `stream_response`. Skip no-body success statuses. Retain original status in telemetry and classifier evidence.

Guard budget:

- `max_peek_bytes`: 8192 bytes.
- `max_peek_events`: first complete data-bearing SSE event only.
- `max_peek_duration`: 200 ms.
- No accumulation across fallback attempts. Each attempt owns and drops its bounded peek buffer.

Guard pass-through contract:

- Keeps original status, response headers, content type, endpoint kind, guard outcome, bounded prefix, remaining stream, and optional structured evidence in process only.
- On `pass`, `cap_exhausted`, `deadline_exhausted`, or `parse_unsupported`, emits the exact prefix once followed by the exact remaining stream.
- On `classified`, no bytes have been emitted; the response is converted into `ClassifiedFailure` using `FailureSource::GuardedSuccessEnvelope` and the normal transition/retry path.
- It must not produce stale body headers. If a downstream filter may mutate the body, strip `Content-Length`. Preserve non-identity `Content-Encoding` unless the gateway decodes and re-encodes the body; this phase does not add that transform.
- Public guard evidence records only status/content kind/endpoint/lengths/error shape; it must not hold or serialize peeked bytes, SSE data, raw body text, matched text, credentials, or tokens.

Detection rules:

- JSON content: inspect only unambiguous top-level structured error envelopes: an `error` object, or a top-level typed `code` plus `message`. Code-less error objects classify as request-only unless a typed relay-profile rule maps them.
- SSE content: parse only the first complete data-bearing event. Ignore comment-only and empty events. Concatenate multiple `data:` lines per SSE rules. Treat `[DONE]` as pass-through. A structured error object in that first data-bearing event may classify.
- The SSE scanner must handle LF and CRLF line endings, leading spaces after `:`, `event:` fields, comments, blank events, and split chunks. Pass-through replays original bytes exactly; parsing normalization must not rewrite the stream.
- Other content types do not run free-text or opportunistic JSON-object sniffing; they pass through as `parse_unsupported` unless the guard exits by cap/deadline.

Pipeline order: upstream response -> 2xx guard -> reconstructed pass-through stream -> response filter -> client. The reconstructed prefix must be filtered exactly once and must not bypass the response filter.

Guard outcomes: `pass`, `classified`, `cap_exhausted`, `deadline_exhausted`, `parse_unsupported`. Classified fallback emits the Phase 2 retry decision event with `failure_source: guarded_success_envelope` and `duplicate_charge_risk: unknown` when a retry proceeds.

**Phase 3 acceptance gate:** red tests cover JSON error classification, code-less request-only behavior, body-bearing non-200 2xx responses, no-body status skip, first SSE error split across chunks, CRLF/comment/multiple-data-line SSE parsing, `[DONE]` pass-through, byte-exact prefix replay, no stale content length when filtered, cap and deadline pass-through, no fallback for streaming/non-replayable/partial-output paths, duplicate-charge-risk telemetry, and no false rejection of valid OpenAI Responses-style SSE. Complete only when local CI and x86_64 build pass.

## Phase 3B: Pre-Output Retry/Fallback Reliability Tightening

**Purpose:** tighten same-request retry and route-target fallback so one small
eligible pre-output backend miss can be absorbed before client-visible output,
while preserving one-ai-key's lightweight, no-surprise, high-performance
boundary.

This stop node may be implemented before or after Phase 4. It refines the retry/fallback reliability contract but must not retroactively block a completed Phase 3. It stops immediately if acceptance requires live model aggregation, request-path storage joins, full buffering of successful responses, or post-output transparent fallback.

Scope:

- For non-streaming requests with replayable bodies, before returning an
  upstream-derived client error, allow at most one extra upstream attempt within
  the unified attempt-state gates.
- Same-request credential retry must exclude credentials already failed in the current request and must not switch back to the same key.
- Route-target retry uses frozen candidates from the original route planning result. It skips targets that are already cooling down, disabled, or without an eligible credential, and its inclusion/skip reasons are explainable in telemetry and routing preview.
- Single-target transient provider failures get one same-target pre-output retry when no frozen route target remains, no cooldown evidence is present, and the normal replayability, streaming, partial-output, and deadline gates allow another attempt.
- Same-request credential retry, frozen route-target retry, and same-target
  retry are mutually exclusive consumers of the one-extra-attempt budget and
  must not chain inside the same original client request.
- Streaming, non-replayable, and partial-output paths never fallback or retry. They return stable denial reasons instead of attempting continuation.
- `Retry-After` and typed channel-failure evidence may influence transient cooldown. They never override manual disablement or configured disablement.
- Retry/fallback decisions do not parse free-form upstream text and do not record raw response bodies, request bodies, upstream keys, client tokens, or token-like values.

Acceptance tests:

- Bad selected key retries to a good credential in the same request without reusing the failed credential.
- Credential candidate exhaustion falls through to the next eligible frozen
  route target only when the one-extra-attempt budget has not already been
  consumed.
- Candidate exhaustion without an eligible route target returns stable `no_frozen_candidate`, `attempt_limit_reached`, or `no_route_candidate` denial/error codes as appropriate.
- Primary target `502`, `503`, and `504` pre-output failures fallback to the
  next eligible frozen target when replayability, deadline, endpoint-family, and
  policy gates allow it. Default `429` and guarded-`2xx` failures do not consume
  the conservative retry budget unless a later plan explicitly changes that
  contract.
- Single-route `5xx` provider failures retry the same target once before returning a client-visible error; `Retry-After` cooldown evidence, guarded-success 2xx failures, streaming, non-replayable, and partial-output paths do not use same-target retry.
- `/v1/models`, embeddings, named-pool requests, and unknown endpoint families
  do not retry and do not create a second upstream hit.
- Streaming, non-replayable, and partial-output paths produce no fallback and expose the stable denial reason.
- Telemetry records retry directive, denial reason, and duplicate-charge-risk classification without raw body/key/token material.
- Retry pressure remains bounded under repeated failures.
- Routing preview explains skipped frozen candidates without leaking secrets or raw upstream payloads.

**Phase 3B acceptance gate:** complete only when the tests above pass under the normal local CI/build gates. Stop as blocked if the implementation needs mid-stream continuation fallback, free-text keyword disablement, live model aggregation, complex multi-tenant billing/UI behavior, active probe daemons, request-path storage joins, full successful-response buffering, or post-output transparent fallback.

## Phase 4: Response Filter Events And Protocol Framing

**Purpose:** make contaminated successful responses visible without event/alert-driven routing-health mutation, add explicit pre-commit lifecycle actions, and make response-filter body mutation protocol-correct.

**Likely files:** `src/response_filter.rs`, `src/upstream_response.rs`, `src/events.rs`, `src/management_alerts.rs`, `src/management_runtime.rs`, `docs/response-filter.md`, tests.

Event ownership:

- `response_filter.rs` owns rule evaluation and returns rule id/action/reason code without raw matched text.
- `upstream_response.rs` owns stream framing and body/header mutation behavior.
- The forwarding context attaches request id, channel id, public model, and route target context before enqueueing the event.
- `AppState` owns a bounded in-memory `response_filter_events` ring, default capacity 1024.
- Management projections expose events and alerts. These projections are not routing telemetry and are not replay input for credential/channel lifecycle.

Event schema at `GET /management/response-filter-events`:

`event_id`, `created_at_unix_seconds`, `request_id`, `channel_id`, `public_model`, `rule_id`, `action: redact|reject|reject_and_expire_credential|reject_and_cooldown_channel`, `content_kind: json|sse|bytes|unknown`, `reason_code: rule_matched|required_rule_missing`, `outcome: redacted|rejected`, `body_committed: true|false`.

Alert aggregation at `GET /management/alerts`: if a channel has at least three response-filter reject/redact events with the same rule id inside `response_filter.alert_window_seconds` (default 900), expose a `response_filter_contamination` alert with channel id, rule id, action counts, window seconds, and severity. Alerts decay by time window and never mutate lifecycle state.

Framing/header contract:

- Any body-mutating path strips `Content-Length`. Non-identity `Content-Encoding` is preserved unless the body is decoded and re-encoded; redaction preserves the original content type only for bodies that are safe to inspect as text. Rejection sets a local JSON or SSE content type that matches the emitted payload.
- SSE redaction emits valid SSE frames and preserves event boundaries.
- SSE rejection emits a valid local SSE error event. Non-SSE rejection emits a local JSON error payload with sanitized body headers.
- Non-UTF-8 bytes pass through unchanged and do not create matched-text events.
- Response-filter events never store matched text, raw chunks, request bodies, response bodies, upstream keys, client tokens, or absolute paths.

**Phase 4 acceptance gate:** red tests cover event schema/redaction, ring capacity, alert decay window, SSE redaction/rejection framing, non-SSE rejection headers, content-length stripping on mutation, guard-prefix filtering exactly once, no lifecycle/channel mutation from filter events or alerts, explicit pre-commit lifecycle actions through the unified retry gates, no routing telemetry writes from ordinary filter hits, and local build gates.

## Phase 5: Explanation, Documentation, And Final Boundary Hardening

**Purpose:** complete operator explanation surfaces and documentation after the core relay hardening mechanisms are stable.

**Likely files:** `src/management_runtime.rs`, `src/management_routing.rs`, `src/management_alerts.rs`, `README.md`, `docs/architecture.md`, `docs/performance-budget.md`, tests.

- Add or stabilize `GET /management/explain/runtime` with runtime generation, registry source, credential source, client-token source, staged-vs-runtime state, reload requirement, last reload time, and last reload error.
- Add or stabilize model-route explanation using existing `/management/routing/preview` rather than adding live model discovery to `/v1/models`.
- Ensure `GET /management/health/serving` and `GET /management/health/resilience` include relay suppression, retry pressure, credential-set spare capacity, and response-filter alert summaries.
- Update README with a clear suitable/not-suitable boundary.
- Update performance docs with composed guard/filter byte and latency budgets.
- Update response-filter docs to state the event/alert boundary and the explicit pre-commit lifecycle-action exception.

Explain endpoint schemas must be redacted: no raw tokens, token hashes, upstream keys, raw request/response bodies, matched text, absolute key paths, URL userinfo, or token-like query parameters.

**Phase 5 acceptance gate:** red tests cover explain runtime schema, staged-vs-runtime reporting, routing preview skip reasons for relay-suppressed channels, health serving/resilience truth table, README boundary, performance-budget updates, response-filter docs consistency, and final local build/release hygiene. Complete only when the full roadmap verification gate passes.

## Deferred Boundary: Model Discovery

This roadmap does not implement persistent `discovered`, `probed`, `verified`, or `blocked` model states. The existing management-only discovery and explicit sync flow may continue to exist, but client traffic and `/v1/models` must not depend on live upstream discovery.

Reopen this boundary only if a new user goal explicitly asks for durable model trust state. Reopening requires a separate plan for catalog persistence, operator verification semantics, client-token scope interaction, and rollback. It must not be smuggled into relay error hardening.

## Parked Items

| Item | Why parked | Revisit trigger | Visibility |
| --- | --- | --- | --- |
| Free-form upstream message matcher | Unsafe for hot-path lifecycle decisions and injection-prone | A separate management-only probe summarizer design | Operator-visible follow-up |
| Automatic response-filter-driven channel disable | Needs durable false-positive review and manual recovery design | User explicitly needs auto-disable beyond bounded pre-commit cooldown | Operator-visible follow-up |
| Account/provider balance scope | Suppression domain is broader than current roadmap | User explicitly needs shared-account suppression beyond selected channel | Architecture follow-up |
| Persistent model verification states | Requires durable catalog and product policy | New explicit model trust goal | Product/architecture follow-up |
| UI, billing, price sync, PostgreSQL, plugin ecosystems | Outside lightweight router boundary | New product goal | Out of scope |

## Production Bug Backlog

No open production bugs remain inside this roadmap. The contaminated 2xx
completion item was closed by the post-Phase-4 follow-up: guarded success
prefixes are checked by response-filter rules before body commit, explicit
`reject_and_expire_credential` and `reject_and_cooldown_channel` actions create
redacted lifecycle evidence, and retry proceeds only through the unified Phase 2
gates. If no clean candidate exists, the proxy returns a local sanitized error
instead of forwarding contaminated content.

## Global Verification Commands

Every phase must run:

```bash
scripts/local-ci.sh
scripts/build-release-x86_64-linux-docker.sh
```

The Docker wrapper is the host-side release gate. It enters the local x86_64
Linux Nix container and calls `scripts/build-release-x86_64-linux.sh` there.
Do not run the inner script on the macOS host, and do not replace this gate
with a server build.

Until Phase 0 creates those scripts, the equivalent manual commands are:

```bash
cargo fmt -- --check
cargo check --locked
cargo clippy --locked -- -D warnings
cargo test --locked
```

Manual equivalents are not sufficient after Phase 0. From Phase 1 onward, the canonical scripts are the stop gate.

## Roadmap Stop Nodes

This roadmap has hard stopping points. Do not continue into the next phase when the current phase fails because it needs a scope expansion.

| Stop node | Complete when | Blocked, do not continue when |
| --- | --- | --- |
| Phase 0 | Local CI/build/release harness, management role matrix, allowlist, audit, health split, and unknown-field tests pass | Local x86_64 build cannot run; management route coverage is incomplete; release hygiene cannot exclude artifacts/secrets. |
| Phase 1A | Relay profiles and typed classifier table pass red/green tests | Free-form matcher or unsupported scope becomes necessary. |
| Phase 1B | Channel balance suppression transition table, reset scope, all-target fail-closed behavior, and schemas pass | Account/provider/credential-set suppression becomes necessary. |
| Phase 2 | Retry denial, duplicate-charge risk, retry pressure, and effective deadline schemas pass | Guarded fallback needs behavior not representable by the unified retry path. |
| Phase 3 | 2xx guard passes byte/time/SSE/prefix/retry tests without full buffering | Guard requires broad schema validation, full buffering, or fallback after output. |
| Phase 3B | Pre-output credential retry, frozen route-target fallback, or same-target retry consumes one mutually exclusive extra attempt for replayable non-streaming requests before client errors | Reliability acceptance requires chained retries, more than one extra attempt, mid-stream continuation fallback, free-text keyword disablement, live model aggregation, active probe daemons, request-path storage joins, full successful-response buffering, or post-output transparent fallback. |
| Phase 4 | Response-filter events, alerts, framing, and explicit pre-commit lifecycle action tests pass | Filter hits need post-output mutation, automatic disablement, or full successful-response buffering to satisfy acceptance. |
| Phase 5 | Explain, docs, health truth table, and final release hygiene pass | New product surfaces such as UI, billing, or persistent model trust are required. |

The whole roadmap stops after Phase 5. Any work on post-output filter-to-health mutation, automatic response-filter-driven disablement, account/provider balance suppression, persistent model discovery states, or remote deployment requires a new explicit goal.

## Design Rationale

Root issue ledger for this revision:

| Root | Resolution |
| --- | --- |
| Phase slicing too broad | Split foundation, classifier, channel state, retry observability, 2xx guard, filter events, and explain/docs into separate stop nodes. |
| State-machine lifecycle underspecified | Added failure source terms, classifier table, channel transition table, reset scope, and all-target fail-closed policy. |
| Hot-path protocol contract underspecified | Added 2xx scope, byte/time/event caps, SSE scanner rules, synthetic guarded evidence, prefix replay, and guard/filter ordering. |
| Management and release gates too late/vague | Moved roles, allowlist, audit, unknown-field validation, local x86_64 build, and release hygiene into Phase 0. |
| Tests not red/green enough | Added phase acceptance gates with endpoint/schema assertions and blocked conditions. |
| Model discovery scope drift | Reframed model discovery as deferred boundary only. |
