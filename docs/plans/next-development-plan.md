# Next Development Plan

**Goal:** move one-ai-key from a working key router into a dependable personal
operator tool by stabilizing runtime semantics, then diagnosis, then model
publication.

**Architecture:** each stage ships one user-visible workflow and pays exactly
one architecture debt. Runtime reliability pays the admission/forwarding debt;
operator diagnosis pays the management projection contract debt; model
publication pays the plan/apply/reload workflow debt. Client compatibility is a
release gate across every stage, not a protocol-conversion product.

**Tech Stack:** Rust, Axum, clap CLI, existing management APIs, local bounded
runtime state, local mock upstreams, Docker/Nix x86_64 release builds.

---

## Review Record

This plan replaces the earlier habit of stretching one local failure into a
phase. Five review angles converged on the following product sequence:

1. **Runtime Reliability Semantics v1**
2. **Operator Diagnosis Loop v1**
3. **Model Publication Workflow v1**
4. **Safe Credential Replacement Workflow v1** after the first three stages
5. **Client Compatibility Contract** as a cross-stage release gate

The ordering is deliberate. Diagnosis is only useful when runtime reason codes
are trustworthy. Model publication is only safe when diagnosis can distinguish
route, scope, reload, credential, and endpoint-family blockers. Credential
replacement is only safe after diagnosis can prove the blocker is actually a
credential problem.

## Product Principles

one-ai-key remains:

- a lightweight personal/small-team AI key router;
- OpenAI-compatible at the client boundary;
- explicit about public model routes and client-token visibility;
- conservative about retry and fallback;
- operator-friendly through redacted CLI and management projections.

It must not become:

- LiteLLM/New API replacement;
- hosted multi-tenant gateway;
- billing, usage, quota, or cost platform;
- live upstream model catalog aggregator;
- background health-check cluster;
- broad protocol conversion gateway;
- UI/dashboard-first product.

## Stage 1: Runtime Reliability Semantics v1

**User value:** the router should not be more fragile than direct upstream use
because of its own local runtime state. When it refuses locally, the refusal
must have a stable hard reason.

**Architecture debt paid:** admission/forwarding boundary. `proxy.rs` must stop
owning route-admission classification. `routing.rs` must remain selected-target
failure transition logic, not admission retry logic.

### Tasks

- **Route admission taxonomy**

  Files: `src/route_plan.rs`, `src/main.rs`.

  Tests: available > degraded > provider/account soft-cooling; all-soft
  otherwise-valid targets become last-resort first attempts; hard channel
  cooldown, no credentials, disabled target/channel, runtime unavailable,
  unknown channel, scope/model/config errors fail closed.

  Non-goals: no YAML fields, no live catalog, no hard blocker downgrade, no M3
  retry expansion.

- **Proxy secondary gate alignment**

  Files: `src/proxy.rs`, `src/main.rs`.

  Tests: provider/account soft-cooling last resort selected by the route planner
  is not rejected again by proxy; frozen fallback still skips a soft-cooling
  target when a later better target exists; hard blockers produce zero upstream
  hits.

  Non-goals: no route replanning after upstream failure; no default-pool
  fallback; no endpoint-family fallback.

- **Relay/error classification hardening**

  Files: `src/error.rs`, `src/routing.rs`, provider/error adaptation tests.

  Tests: invalid key, quota exhausted, relay-balance channel scope, rate limit,
  provider transient, Retry-After, 401/403/429/5xx profile differences. The
  mapping must produce typed `FailureKind`, `FailureScope`, retryability, and
  state mutation intent.

  Non-goals: no free-form upstream message matching as the primary classifier;
  no raw upstream text in telemetry; no broad relay profile framework unless
  already admitted by existing config.

- **Failure-to-state transition summary**

  Files: `src/routing.rs`, `src/failure_state_executor.rs`, management
  projection code if needed.

  Tests: classified failure -> state mutation -> resulting credential/channel
  state. Cover credential expired, credential cooldown, credential quota,
  channel hard cooldown, provider/account soft cooldown, degraded state, no-op
  request errors, and response-filter lifecycle actions.

  Non-goals: the summary is explanatory; it must not become a routing input.

- **M3 retry non-expansion**

  Files: `src/routing.rs`, `src/proxy.rs`, `tests/pre_output_stability_boundary_contract.rs`.

  Tests: last-resort admission is a first attempt, not a retry. M3 remains
  limited to selected-target, pre-output, non-streaming Chat/Responses,
  replayable body, transient failure, one extra attempt.

  Non-goals: no streaming retry, partial-output retry, named-pool retry,
  embeddings retry, `/v1/models` retry, or unknown endpoint retry.

- **Pre-output guard and response-filter stability**

  Files: `src/proxy.rs`, response guard/filter modules, `src/main.rs` tests.

  Tests: 2xx error envelope before client output is classified; precommit filter
  rejection only mutates lifecycle when action says so; body already committed
  never retries or changes route; filter events remain bounded and redacted.

  Non-goals: no full buffering of successful responses; no content-safety
  product; no filter-driven routing engine.

- **Runtime projection exported for diagnosis**

  Files: management routing/failure projection modules.

  Tests: stable reason codes and summaries exist for admission, hard/soft
  blockers, selected target, failure transition, retry allowed/denied, and
  credential/channel state. CLI must be able to render them without recomputing
  domain logic.

### Stage 1 Stop Criteria

- Hard blockers fail closed with zero upstream hits.
- Provider/account soft-cooling last resort succeeds as a first attempt when it
  is the only otherwise-valid route.
- M3 retry behavior is unchanged outside its existing gates.
- Relay/error classification maps typed evidence to stable failure scope and
  state mutation.
- Response filter and pre-output guard remain bounded, redacted, and
  precommit-only for lifecycle mutation.
- Request hot path still reads only compiled in-memory state.
- No YAML, live catalog, background probe, adaptive routing, or persistent
  ledger is added.
- `scripts/local-ci.sh`, targeted non-zero tests, `git diff --check`, and
  staged denylist pass.

## Stage 2: Operator Diagnosis Loop v1

**User value:** an operator can answer “can this client token call this model on
this endpoint family, and if not, what is the next safe command?” without
guessing between several commands.

**Architecture debt paid:** management projection contract. Management produces
typed redacted summaries; CLI renders them. CLI must not duplicate route,
credential, endpoint, reload, or retry logic.

### Tasks

- **Canonical diagnosis entry**

  Files: `src/cli_commands/models.rs`, `src/management_routing.rs`,
  `tests/local_release_contract.rs`.

  Tests: `models explain --client-token-ref <ref> --model <model>
  --endpoint-family <family> --json` returns `can_use`, `reason_code`,
  `blocking_domain`, `next_action`, `endpoint_family`, `model`,
  `client_token_ref`, and bounded evidence.

  Non-goals: no new top-level diagnose command unless the existing model entry
  cannot remain coherent; no auto-fix.

- **Management diagnosis projection**

  Files: `src/management_routing.rs`, relevant response structs.

  Tests: token missing, token unknown, token disabled, model missing, endpoint
  family unsupported/mismatch, no route, no usable key, reload drift, recent
  failure hint. Existing consumers must remain compatible.

  Non-goals: no mutation endpoints, no YAML fields.

- **Route/models/failures projection alignment**

  Files: `src/cli_commands/route.rs`, `src/cli_commands/models.rs`,
  `src/cli_commands/failures.rs`.

  Tests: route explain and models explain agree on blocker domain; failures
  explain is bounded evidence, not a replacement for current availability; no
  command recomputes admission locally.

  Non-goals: no full incident ledger, no raw response text, no production
  transcript support.

- **Safe next-action contract**

  Files: CLI command modules, shared report/next-action helpers if already
  present.

  Tests: every reason code maps to one safe command or `none`. Diagnosis next
  actions are read-only: models explain, route explain, failures explain/tail,
  doctor, reload status, reload diff. No key import, probe, reload apply, or
  upstream curl appears as an automatic suggestion.

- **Doctor and reload role boundaries**

  Files: `src/cli_commands/doctor.rs`, `src/cli_commands/reload.rs`,
  `docs/operations.md`.

  Tests: doctor remains lightweight runtime/service context by default; opt-in
  route/model context remains bounded; reload status/diff can be suggested,
  reload apply cannot be suggested by diagnosis.

- **Documentation and release smoke**

  Files: `README.md`, `docs/operations.md`, `docs/release-build.md`,
  `scripts/release-smoke.sh`.

  Tests: local release smoke exercises canonical diagnosis commands against
  local mock config and confirms redacted output.

### Stage 2 Stop Criteria

- One documented first command answers client/model/endpoint availability.
- CLI output includes reason code, blocker domain, evidence source, and safe
  next action.
- CLI renders management projections and does not duplicate domain algorithms.
- Failures evidence remains bounded and optional.
- All output redacts keys, tokens, token hashes, raw request/response bodies,
  full URLs, private paths, and upstream free-form text.
- No auto-fix, reload apply, key import, probe, UI, TUI, or background worker is
  added.

## Stage 3: Model Publication Workflow v1

**User value:** an operator can turn an explicitly known upstream model and
channel into a public model visible to a client, through a predictable
plan/apply/reload/verify workflow.

**Architecture debt paid:** plan/apply/reload workflow. Read-only planning,
explicit staged write, explicit reload diff/apply, and read-only verification
become a reusable control-plane pattern.

### Tasks

- **Publication plan projection**

  Files: `src/cli_commands/models_onboard.rs`, `src/operator_client.rs`,
  `docs/operations.md`.

  Tests: plan output includes public model, upstream model, channel id,
  credential set, current conflicts, endpoint capability summary, client
  visibility status, and reload requirement. Plan does not mutate anything.

  Non-goals: no live discovery, upstream probe, registry write, YAML patch, or
  client-scope mutation.

- **Model route staged apply**

  Files: `src/management_registry.rs`, management route registration,
  `src/operator_client.rs`.

  Tests: explicit confirmed apply upserts staged model route and returns staged
  registry version, active runtime generation, reload required, validation
  errors, and redacted audit event. Missing writable store fails closed.

  Non-goals: no active runtime mutation, no YAML write, no credential store
  touch, no automatic provider/channel creation.

- **CLI publication apply**

  Files: `src/cli.rs`, `src/cli_commands/models_onboard.rs` or a focused
  models publication module, `src/cli_effects.rs`.

  Tests: dry-run does not POST; non-tty apply requires `--yes`; apply output
  reports management write, staged version, reload required, and next reload
  command.

  Non-goals: no automatic reload, no `/v1/models` call inside apply, no
  credential probe.

- **Reload diff integration**

  Files: `src/cli_commands/reload.rs`,
  `src/cli_commands/runtime_reload_projection.rs`, registry diff projection.

  Tests: staged route shows typed `model_route` diff; reload apply requires
  expected staged registry version; generation mismatch fails before mutation.

- **Visibility verification**

  Files: `src/cli_commands/models.rs`, `src/management_routing.rs`,
  `src/operator_client.rs`.

  Tests: before reload, models explain reports staged/active drift; after
  reload, models explain and `/v1/models` agree on visibility for the client;
  client scope mismatch is explained, not automatically fixed.

- **Publication smoke and docs**

  Files: `scripts/release-smoke.sh` or a focused local smoke helper,
  `docs/operations.md`, `docs/configuration.md`, `README.md`.

  Tests: local mock plan/apply/reload/verify path in temp writable store.
  Production publication smoke remains operator-run only.

### Stage 3 Stop Criteria

- Plan is read-only and complete enough to review impact.
- Apply writes only staged registry and requires explicit confirmation.
- Reload is explicit and preconditioned by expected staged version.
- Verify proves client visibility and one minimal request against local mock.
- No live upstream catalog aggregation, automatic scope mutation, automatic
  reload, alias/profile layer, default model, or Responses-to-Chat adapter is
  added.

## Stage 4: Safe Credential Replacement Workflow v1

**User value:** replace bad or exhausted upstream credentials without leaking
secrets, guessing the wrong blocker, or mutating more than intended.

### Tasks

- **Credential capacity and route impact plan**

  Files: `src/cli.rs`, `src/cli_commands/keys.rs`, `src/cli_effects.rs`,
  `scripts/release-smoke.sh`.

  Tests: `keys replacement-plan` reads credential-set management projections,
  optional bounded credential refs, and optional route preview context. Route
  preview failures degrade to unavailable route impact; credential-set reads
  remain required. JSON and default table output show whether the requested
  credential set is selected, a fallback candidate, not a candidate, or unknown.

  Non-goals: no probe, import, lifecycle mutation, reload, route mutation,
  background health scan, or live catalog call.

- **Import remains the only replacement key ingress**

  Files: `src/cli_commands/keys.rs`, `docs/configuration.md`,
  `docs/operations.md`, `scripts/release-smoke.sh`.

  Tests: `keys import --dry-run` reads a local source file for redacted counts
  only; confirmed `keys import --yes` requires a writable credential store and
  reports management-write/runtime-mutation effect without exposing source
  secrets. Release smoke covers both paths with placeholder local files.

  Non-goals: no raw key positional CLI argument, no paste-key command, no
  automatic import suggestion from diagnosis.

- **Single-credential probe and probe-apply workflow**

  Files: `src/cli_commands/keys.rs`, `scripts/release-smoke.sh`.

  Tests: `keys probe --dry-run` is read-only, confirmed `keys probe --yes`
  touches one explicit `credential_ref` with a provider-facing upstream model
  and persists redacted probe evidence, `keys probe-apply plan` exposes a
  bounded `probe_result_ref`, and `keys probe-apply apply --dry-run` verifies
  the precondition without mutation.

  Non-goals: no batch probe apply, no background probing, no automatic
  lifecycle mutation from diagnosis.

- **Explicit disable and restore lifecycle repair**

  Files: `src/cli.rs`, `src/cli_commands/keys.rs`, `src/cli_effects.rs`,
  `src/operator_client.rs`, `scripts/release-smoke.sh`.

  Tests: `keys disable` and `keys restore` accept only non-secret
  `credential_ref` values. Dry-run is offline readonly; confirmed operations
  require `--yes`, call only set-scoped management mutation endpoints, and
  redact raw keys, fingerprints, private paths, raw bodies, and token-like
  values. Path-like operator reasons are not echoed into next-action argv.

  Non-goals: no bulk restore, no inferred target from stats, no direct internal
  credential id use.

- **Post-action verification and documentation**

  Files: `README.md`, `docs/operations.md`, `docs/configuration.md`,
  `scripts/release-smoke.sh`, public operations docs, and release notes.

  Tests: release smoke verifies stats, replacement-plan, import,
  probe/probe-apply dry-run and confirmed evidence paths, disable/restore,
  route/model explain, `/v1/models`, and one mock completion without private
  deployment state. Documentation states that keys come from configured files,
  environment/startup secret sources, local source files, writable credential
  stores, or `keys import --source`, not raw CLI positional arguments.

Rejected:

- auto rotation;
- batch probe apply;
- background key scanning;
- usage/cost/billing;
- balance dashboard;
- raw key output;
- automatic model/route mutation as part of key maintenance.

### Stage 4 Stop Criteria

- Credential replacement has a documented read-only first step:
  `keys stats` plus `keys replacement-plan`.
- Replacement key ingress is `keys import --source <path>` only; raw upstream
  keys are not positional CLI arguments.
- Single-credential workflows use non-secret `credential_ref` values and never
  ask operators for internal ids or fingerprints.
- Dry-run commands do not write management state, call upstreams, or mutate
  active runtime except where explicitly classified as local preview.
- Confirmed import, disable, restore, and probe-apply require `--yes` or
  interactive confirmation and disclose management-write/runtime-mutation
  effects.
- Confirmed probe is single-credential and upstream-touching; it is not a
  background health check.
- Release smoke covers the replacement workflow with local placeholders and
  leak scanning.
- Request hot path still reads only compiled in-memory state.

## Cross-Stage Client Compatibility Contract

Every stage must preserve this compatibility matrix:

- `/v1/models` is a local compiled public model projection filtered by client
  token scope. It does not call upstream catalogs or expose capability metadata.
- `/v1/chat/completions` works for a local mock public model and preserves
  existing retry boundaries.
- `/v1/responses` works as native pass-through/rewrite where configured; no
  Responses-to-Chat bridge.
- endpoint-family explain is diagnostic metadata, not protocol conversion.
- invalid client token and management/client URL misuse remain stable and
  redacted.
- production client smoke is optional operator-run harness/checklist; it is not
  `local-ci`, not release-smoke, and contains no private defaults.

## Global Release Discipline

Every stage must include:

- targeted tests with non-zero match evidence;
- `scripts/local-ci.sh`;
- `git diff --check`;
- `scripts/check-staged-denylist.sh`;
- local Docker/Nix x86_64 release build when publishing;
- extracted-artifact release smoke;
- release record with implemented scope, deferred scope, explicit rejections,
  local-ci result, release-smoke result, redaction result, compatibility result,
  and known blockers.

Every stage must exclude:

- `dist/`, `target/`, `key-pool-router/`;
- `config/`, `data/`, SQLite, logs, keys, raw fixtures;
- private scripts, private hostnames, real API keys, deployment transcripts;
- `AGENTS.md`.

## Plan Stop Conditions

Stop and re-plan if a stage requires:

- request path reads from YAML, SQLite, registry store, credential store,
  client-token store, failure history, or live upstream catalog;
- broad protocol conversion;
- background probes or adaptive routing;
- persistent failure ledger;
- new global YAML presets;
- automatic key/model/scope mutation outside an explicit management write;
- UI/dashboard work;
- private deployment state as release evidence.
