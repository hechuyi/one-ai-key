# Relay Hardening Implementation Plan

> **Execution rule:** implement this plan task-by-task with review between phases. This document is a plan only; it does not authorize remote-server changes, SSH testing, or server-side builds.

**Goal:** harden one-ai-key for real relay usage while preserving its lightweight personal/small-team AI key router boundary.

**Architecture:** request forwarding continues to read only compiled in-memory runtime state. Relay-specific behavior enters through typed config, profile presets, bounded pre-output response inspection, and management-only observability. The plan must not turn `/v1/models` into a live upstream aggregation endpoint, add request-path disk I/O, add request-path storage joins, or fully buffer successful responses.

**Tech Stack:** Rust, Axum, reqwest, serde/serde_yaml, SQLite-backed local stores, local x86_64 NixOS container build harness, focused integration tests.

---

## Review Basis

The original relay critique and the first plan were reviewed by read-only specialist agents and then re-reviewed after the user rejected the first plan as immature. The second review round returned `BLOCK` from all four perspectives:

- State-machine/lifecycle review: Phase 1 lacked exact transition tables, failure-source ownership, reset scope, all-target suppression policy, and response-filter alert lifecycle.
- Hot-path/protocol review: Phase 2 lacked a 2xx scope contract, latency cap, SSE parser contract, guarded classifier evidence path, guard/filter ordering, and header/framing rules for body mutation.
- Operations/security review: management security was too late and too vague; local x86_64 NixOS container build, release artifact identity, repo hygiene, audit schema, allowlist behavior, and unknown-field rejection were not executable gates.
- Test/phase-slicing review: phases were too large, stop nodes were concept-level rather than red/green stop cards, and endpoint/schema assertions were missing.

The plan below adopts those blocking findings. It does not adopt scope-expanding ideas such as live upstream model aggregation, persistent model-discovery state machines, UI work, multi-tenant billing, heavyweight control-plane databases, active probe daemons, hedging, or automatic response-filter-driven channel lifecycle mutation.

## Relation To Models

The relay hardening work is not primarily a model-list project. Model routing matters only as a boundary: clients see public model ids from compiled `model_routes`; route planning maps a public model to bounded channel candidates; `/v1/models` remains a local projection over the authenticated client's compiled scope. Upstream model discovery stays a management-only import aid and is not a request-path dependency.

The reviewed critique mentioned discovered/probed/verified/blocked model states as a possible future trust mechanism. That is parked. It requires a durable catalog model and a product decision about how much model verification one-ai-key should own. It is not needed to fix relay error semantics, 2xx false-success handling, response contamination visibility, retry risk, or management explainability.

## Mature Project Reference Synthesis

The plan uses mature systems as design references, not as feature templates.

Envoy and HAProxy contribute the passive-health lesson: distinguish local transport failure from upstream transaction failure, use bounded ejection/cooldown, make recovery automatic, and make retry pressure observable. one-ai-key translates this to typed channel/credential transitions, TTLs, capped backoff, generation preconditions, and management reset. It does not import active health-check clusters, hedging, or a large circuit-breaker matrix.

Kubernetes and Consul contribute the health-semantics lesson: liveness, serving readiness, and resilience are separate questions. one-ai-key keeps `/health` as process liveness, keeps `/ready` compatible with current serving-readiness behavior, and adds management health projections for serving and resilience so orchestration, operators, and tests do not infer different meanings from one endpoint.

LiteLLM, OpenRouter-style routers, and comparable key/channel routers contribute the client-abstraction lesson: the client configures one base URL, one client token, and public model ids while the router chooses backend targets. one-ai-key keeps that abstraction, but avoids platform features such as teams, billing, price catalogs, and live provider catalog fan-out.

OpenTelemetry contributes the event-contract lesson: events need stable names, bounded fields, redaction rules, and explicit cardinality limits. one-ai-key uses structured event fields such as request id, channel id, credential id/hash, failure source, failure kind, action, and reason code. Events must not include raw keys, client tokens, request bodies, response bodies, matched filter text, absolute key paths, or token-like URL components.

## Non-Negotiable Constraints

- Do not touch the remote gateway server during implementation or testing for this roadmap.
- Build and test locally. Release artifacts must be built in the local x86_64 NixOS container, not on the gateway server or a remote builder.
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
| `response-filter event` | A bounded management-visible event recording rule id, action, channel/request context, and reason code. It is not input to routing, credential lifecycle, or channel health. |

## Locked Invariants

1. Relay hardening is a typed policy/profile feature. It must not add relay-specific free-form branches in `proxy.rs` or lifecycle decisions based on untrusted text.
2. Channel-level relay balance handling is transient channel availability, not durable credential quota exhaustion.
3. Automatic channel transitions never override configured disablement or manual runtime `Disabled` state.
4. All retry and fallback decisions use one attempt-state path with bounded attempts, replayability checks, effective deadline checks, and observable denial reasons.
5. A body-bearing 2xx response may be inspected only in a bounded pre-output window. After any byte is sent to the client, the gateway must not fallback, retry, rewrite status, or mutate routing state from that response body.
6. Guard pass-through reconstructs bytes exactly: the peeked prefix is emitted once, then the remaining upstream stream. The reconstructed stream then enters the response filter exactly once.
7. Response filtering remains a stream-boundary safety feature. Its events and alerts do not mutate credential lifecycle, channel health, routing failure domains, or retry policy in this roadmap.
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
| Response filter health mutation | Reject for this roadmap | First build bounded events and alerts. Lifecycle mutation from filter hits needs a separate design. |
| Conservative retry/risk telemetry | Adopt before 2xx fallback | Guarded fallback can have unknown charge status; denial and risk fields must exist first. |
| Management roles, allowlist, audit | Adopt in Phase 0 | Management writes already have high blast radius; hardening must precede new relay mutation surfaces. |
| Model discovery states | Park | Not required for relay hardening and requires a durable catalog state model. |

## Phase 0: Foundation Gates Before Relay Semantics

**Purpose:** create the security, health, audit, config-validation, local-build, and release gates that every later phase depends on.

**Likely files:** `src/config.rs`, `src/auth.rs`, `src/main.rs`, `src/events.rs`, `src/management_runtime.rs`, `src/management_alerts.rs`, `docs/architecture.md`, `docs/release-build.md`, `README.md`, `scripts/local-ci.sh`, `scripts/build-release-x86_64-linux-docker.sh`, `scripts/build-release-x86_64-linux.sh`.

### 0A. Local Build And Release Harness

- [ ] Add or standardize `scripts/local-ci.sh` as the canonical local verification entrypoint. It must run `cargo fmt -- --check`, `cargo check --locked`, `cargo clippy --locked -- -D warnings`, and `cargo test --locked`.
- [ ] Add or standardize `scripts/build-release-x86_64-linux-docker.sh` as the canonical host-side release-artifact build entrypoint. It must enter the local x86_64 Linux Nix container, produce the binary artifact locally, and write a SHA256 file. `scripts/build-release-x86_64-linux.sh` is the guarded container-side entrypoint, not the host release gate.
- [ ] If the local x86_64 NixOS container is not available, the phase is blocked. Do not SSH to a server, use a remote builder, or declare a phase complete from host-only tests.
- [ ] Secret-backed integration tests must read ignored local config or env vars and must skip with a clear message when absent. They must not hardcode keys.

**Release gate:** before any GitHub release, run `git status -sb --untracked-files=all` and verify that no local config, SQLite database, JSONL log, key file, private agent file, `dist/` output, or release artifact is staged. Release assets must be named by version and target, include SHA256, and be immutable enough for a future NixOS pin by URL plus hash.

### 0B. Management Role And Network Boundary

Use three roles: `readonly`, `operator`, and `admin`. Existing `management.admin_token` remains a bootstrap admin principal for compatibility.

All `/management/*` routes must match exactly one default-deny rule. A test must enumerate registered management route patterns from `src/main.rs` or a single route registry and fail if any route lacks a minimum role. Wildcards or prefix permissions may be used in implementation only if the generated test expands them back to the exact registered method/path list.

| Minimum role | Routes |
| --- | --- |
| `readonly` | `GET /management/pools`, `GET /management/channels`, `GET /management/client-tokens`, `GET /management/providers`, `GET /management/accounts`, `GET /management/credential-sets`, `GET /management/credential-sets/:id/operations`, `GET /management/credential-sets/:id/credentials`, `GET /management/credential-sets/:id/imports`, `GET /management/credential-sets/:id/imports/:batch_id`, `GET /management/credential-sets/:id/credentials/:credential_id`, `GET /management/credential-sets/:id/credentials/:credential_id/history`, `GET /management/credential-sets/:id/credentials/:credential_id/probes`, `GET /management/model-routes`, `GET /management/policy-profiles`, `GET /management/policy-profiles/:id`, `GET /management/routing-profiles`, `GET /management/routing-profiles/:id`, `GET /management/routing/preview`, `GET /management/runtime`, `GET /management/alerts`, `GET /management/channels/:id`, `GET /management/channels/:id/error-rules`, `GET /management/channels/:id/credentials`, `GET /management/events`, `GET /management/routing-telemetry`, `GET /management/health/serving`, `GET /management/health/resilience`, `GET /management/response-filter-events` after Phase 4. |
| `operator` | `PUT /management/credential-sets/:id/credentials/:credential_id/metadata`, `POST /management/credential-sets/:id/credentials/:credential_id/probe`, `POST /management/credential-sets/:id/credentials/:credential_id/apply-latest-probe`, `POST /management/credential-sets/:id/credentials/apply-latest-probe`, `POST /management/credential-sets/:id/credentials/import`, `POST /management/credential-sets/:id/credentials/:credential_id/expire`, `POST /management/credential-sets/:id/credentials/:credential_id/restore`, `POST /management/credential-sets/:id/credentials/:credential_id/quota-exhaust`, `POST /management/credential-sets/:id/credentials/:credential_id/disable`, `POST /management/credential-sets/:id/credentials/:credential_id/enable`, `POST /management/credential-sets/:id/credentials/:credential_id/reset-cooldown`, `POST /management/model-discovery/sync-plan`, `POST /management/channels/:id/model-discovery`, `POST /management/channels/:id/reset-health`, `POST /management/channels/:id/disable`, `POST /management/channels/:id/enable`, `POST /management/channels/:id/credentials/:credential_id/expire`, `POST /management/channels/:id/credentials/:credential_id/restore`, `POST /management/channels/:id/credentials/:credential_id/quota-exhaust`, `POST /management/channels/:id/credentials/:credential_id/disable`, `POST /management/channels/:id/credentials/:credential_id/enable`, `POST /management/channels/:id/credentials/:credential_id/reset-cooldown`. |
| `admin` | `POST /management/client-tokens`, `PATCH /management/client-tokens/:id`, `POST /management/client-tokens/:id/disable`, `POST /management/client-tokens/:id/enable`, `PUT /management/registry/providers/:id`, `POST /management/registry/providers/:id/disable`, `POST /management/registry/providers/:id/enable`, `PUT /management/registry/accounts/:id`, `POST /management/registry/accounts/:id/disable`, `POST /management/registry/accounts/:id/enable`, `PUT /management/registry/channels/:id`, `POST /management/registry/channels/:id/disable`, `POST /management/registry/channels/:id/enable`, `PUT /management/registry/model-routes/*model`, `PUT /management/registry/policy-profiles/:id`, `PUT /management/registry/routing-profiles/:id`, `POST /management/model-discovery/sync-apply`, `POST /management/runtime/reload`. |

Network boundary choice for this roadmap: implement `management.ip_allowlist` first, not a separate management listener. If the service `listen` address is non-loopback and management is enabled, startup must reject the config unless `management.ip_allowlist` is set. The allowlist uses the peer socket address, not forwarded headers. Loopback listeners allow loopback peers by default.

### 0C. Audit And Config Contracts

- [ ] Every management mutation emits a stable audit record with `created_at_unix_seconds`, actor id, role, action, resource type/id, outcome, request id when present, registry/runtime generation when relevant, and `reason_code`.
- [ ] Persisted/displayed audit records store `reason_code` only. Free-form operator notes, if accepted by request payloads for compatibility, are redacted from management responses and never written to JSONL.
- [ ] If durable audit logging is configured and append fails, the management mutation fails before applying runtime or durable state. Do not apply the mutation and then emit a best-effort audit-failure event.
- [ ] JSONL replay remains compatible with old records that lack `created_at_unix_seconds`.
- [ ] Add `deny_unknown_fields` or explicit unknown-field validation for all new config namespaces introduced by this roadmap: relay profile, balance scope, management principals, IP allowlist, audit settings, guard settings, and response-filter event settings.

### 0D. Health Split

Add management health projections before any new channel suppression behavior:

| Endpoint | Meaning | Status contract | Minimum fields |
| --- | --- | --- | --- |
| `GET /health` | Process liveness only | `200` if process can answer | Existing `ok` body may remain. |
| `GET /ready` | Backward-compatible serving readiness | `200` when at least one request path can serve; `503` when no serving route/credential/channel exists | Existing fields plus no weaker semantics. |
| `GET /management/health/serving` | Current ability to serve client traffic | `200` when service can accept at least one configured route; `503` otherwise | `status`, `serving`, `serving_channels`, `blocking_alerts`, `blocking_reasons[]`. |
| `GET /management/health/resilience` | Spare capacity and degradation | Always `200` if management auth succeeds | `status: ok|degraded|blocked`, `operator_input_alerts`, `credential_sets_without_spare`, `channels_cooling_down`, `response_filter_alerts`. |

**Phase 0 stop card:** write red tests for route role matrix coverage, allowlist fail-closed behavior, audit redaction/timestamp/replay, health endpoint schemas, unknown-field rejection, local CI script, local x86_64 build script, and release artifact hygiene. Complete only when those tests pass and the local build artifact plus SHA256 are produced locally. If the local x86_64 environment cannot run, stop the roadmap as blocked; do not continue into relay semantics.

## Phase 1A: Relay Profiles And Typed Classifier Semantics

**Purpose:** make official provider and relay status semantics explicit without adding new channel suppression state yet.

**Likely files:** `src/config.rs`, `src/error.rs`, `src/upstream_templates.rs`, `src/management_profiles.rs`, `docs/architecture.md`, tests in focused modules or `src/main.rs`.

Add `relay_profile: official_openai | generic_relay | untrusted_relay` at template/config resolution. Existing configs keep current behavior unless they opt into a relay profile. Add `balance_scope`, defaulting to `credential`. In Phase 1A, `balance_scope: channel` may be parsed but must fail config resolution with a clear error until Phase 1B implements its state transition.

Classifier table for Phase 1A:

| Evidence | `official_openai` | `generic_relay` | `untrusted_relay` |
| --- | --- | --- | --- |
| Bare `401` or bare `403` | `AuthInvalid/Credential`, non-retryable, expire selected credential | `ClientError/RequestOnly`, non-retryable, no lifecycle mutation | `ClientError/RequestOnly`, non-retryable, no lifecycle mutation |
| Structured invalid-key code | `AuthInvalid/Credential`, non-retryable, expire selected credential | Same | Same |
| Bare `429` | `RateLimited/Credential`, retryable only through existing gates, credential cooldown | `RateLimited/Credential`, retryable only through existing gates, credential cooldown | `RateLimited/Credential`, no same-request retry unless explicitly enabled, credential cooldown |
| Structured credential quota with `balance_scope: credential` | `QuotaExhausted/Credential`, non-retryable, durable quota-exhaust selected credential | Same | Same |
| Structured channel balance with `balance_scope: channel` | Config rejected until Phase 1B | Config rejected until Phase 1B | Config rejected until Phase 1B |
| Code-less top-level error object | `ClientError/RequestOnly`, non-retryable, no lifecycle mutation | Same | Same |

Unsupported matcher fields such as free-form message contains/regex matching must fail config resolution rather than being ignored. Account/provider/client-token balance scopes remain rejected until a future runtime state machine and management projection exist.

**Phase 1A stop card:** red tests cover profile config resolution, current-config backward compatibility, each table row, unsupported field rejection, unsupported `balance_scope: channel` rejection, management policy projection fields, and absence of request-path free-form message parsing. Complete only when these tests pass under `scripts/local-ci.sh` and the local x86_64 build gate still passes.

## Phase 1B: Channel Balance Suppression State

**Purpose:** enable `balance_scope: channel` as a transient selected-channel availability state.

**Likely files:** `src/config.rs`, `src/error.rs`, `src/routing.rs`, `src/failure_state_executor.rs`, `src/route_plan.rs`, `src/management_resources.rs`, `src/management_alerts.rs`, tests.

Phase 1B enables `FailureKind::RelayBalanceUnavailable` with `FailureScope::Channel` only for `balance_scope: channel`. It suppresses only the selected channel. It does not suppress account, provider, credential set, or all channels sharing a key file.

Channel transition table:

| Input | Allowed source state | Output | Retry | Recovery/reset |
| --- | --- | --- | --- | --- |
| Structured channel-balance evidence from `UpstreamTransaction` | `Available` or `Degraded` | `CoolingDown { reason_code: relay_balance_unavailable, source, until, suppression_count }` | `RetryRouteTarget` only if replayable, no output, effective deadline remains, route-target retry enabled | TTL expiry lazily makes channel available; next success resets relay suppression count. |
| Structured channel-balance evidence from `GuardedSuccessEnvelope` | `Available` or `Degraded` | Same | Same, plus duplicate-charge risk telemetry | Same. |
| Local transport failure | `Available` | Optional `Degraded { reason_code: local_transport_failure }` only if configured; no quota/auth mutation | Route fallback only through existing gates | Next success clears degraded state. |
| Any automatic transition | `Disabled` or configured disabled | No change | No retry added by the state mutation itself | Manual/configured disablement remains authoritative. |
| Manual `reset-health` | `CoolingDown` or `Degraded` | `Available` and relay suppression counters for that channel cleared | No automatic retry | Does not clear credential expired/quota/disabled state or configured provider/account/channel disablement. |
| Success on selected channel | Any automatic transient state except `Disabled` | `Available` and provider/account success handling preserved | Not applicable | Clears selected-channel relay suppression count. |

Backoff is consecutive per channel, stored in memory, capped, and visible in management. Default cooldown must be small and configurable; explicit upstream cooldown evidence can override it. Generation preconditions match existing channel-health mutation rules: stale automatic transitions are rejected and reported, not applied.

All-target suppression policy: after client-token scope, configured enablement, and route candidate limits are applied, if every route target is excluded by active channel cooldown/suppression, fail closed with `503` and JSON error `code: no_route_candidate`. The response includes redacted reason classes such as `channel_cooling_down` and never includes keys, upstream bodies, or internal secrets. A management alert reports affected public model id, candidate count, suppressed count, and reason codes.

Endpoint/schema acceptance:

| Surface | Required assertion |
| --- | --- |
| `GET /management/channels/:id` | `health.kind`, `health.reason_code`, `health.source`, `health.remaining_seconds`, `health.suppression_count`, `health.generation`. |
| `GET /management/routing/preview` | Per-candidate `included`, `selected`, `reasons[]`, and `health.kind` distinguish `channel_cooling_down` from `channel_degraded`. |
| `POST /management/channels/:id/reset-health` | Clears only transient channel health and relay suppression counters; disabled/configured-disabled states remain excluded. |
| `GET /management/alerts` | All-target suppression alert includes resource kind, public model id when available, channel ids, reason codes, severity. |

**Phase 1B stop card:** red tests cover `balance_scope: channel` config acceptance, account/provider scope rejection, transition table rows, stale generation rejection, TTL expiry, success recovery, manual reset scope, all-target `no_route_candidate`, management schemas, and local build gate. Stop as blocked if implementing account/provider balance scope becomes necessary; do not widen scope inside this phase.

## Phase 2: Retry Boundary And Risk Observability

**Purpose:** make retry/fallback decisions observable before 2xx guarded fallback is allowed.

**Likely files:** `src/routing.rs`, `src/events.rs`, `src/failure_observer.rs`, `src/management_runtime.rs`, `src/management_routing.rs`, docs and tests.

- [ ] Keep same-request credential retry opt-in and disabled by default.
- [ ] Preserve hard gates: non-replayable body, streaming request, partial output, attempt-limit exhaustion, and effective-deadline exhaustion must not retry.
- [ ] All same-request and route-target fallback attempts share one attempt-state path.
- [ ] Add an effective request deadline for retry/fallback if not already present in the selected timeout profile. Fallback must not start if it cannot fit inside the effective deadline.
- [ ] Add duplicate-charge risk fields for fallback after upstream transaction failure or guarded 2xx failure when charge status is unknown.
- [ ] Add bounded retry-pressure counters/events to prevent silent traffic amplification.

Stable telemetry schema additions:

| Surface | Fields |
| --- | --- |
| `/management/routing-telemetry` event `retry_decision` | `request_id`, `public_model`, `channel_id`, `credential_id_hash`, `attempt`, `failure_source`, `failure_kind`, `failure_scope`, `directive: retry_credential|retry_route_target|return_error`, `denial_reason`, `duplicate_charge_risk: none|unknown|known_no_charge`, `effective_deadline_remaining_ms`. |
| `/management/runtime` | Retry profile summary, `retry_pressure_capacity`, recent retry/fallback counters. |
| `/management/routing/preview` | Read-only policy summary: route-target retry enabled, same-request credential retry enabled, max retries, candidate limit. |

Allowed `denial_reason` values: `failure_not_retryable`, `body_not_replayable`, `streaming_not_retryable`, `partial_output_started`, `attempt_limit_reached`, `policy_disabled`, `no_frozen_candidate`, `effective_deadline_exhausted`, `route_target_retry_disabled`, `no_route_candidate`.

**Phase 2 stop card:** red tests cover each denial reason, duplicate-charge risk for guarded and non-2xx fallback classes, effective-deadline denial, bounded retry-pressure capacity, schemas above, and existing same-request retry behavior. Complete only when local CI and x86_64 build pass.

## Phase 3: HTTP 2xx Success Guard

**Purpose:** detect obvious relay error envelopes in body-bearing 2xx upstream responses before any client output, then reuse the normal failure and retry path from Phase 2.

**Likely files:** new `src/success_guard.rs`, `src/proxy.rs`, `src/upstream_response.rs`, `src/error.rs`, `src/provider.rs`, docs and tests.

Scope: guard body-bearing `status.is_success()` upstream responses before `stream_response`. Skip no-body success statuses. Retain original status in telemetry and classifier evidence.

Guard budget:

- `max_peek_bytes`: 8192 bytes.
- `max_peek_events`: first complete data-bearing SSE event only.
- `max_peek_duration`: 200 ms default, configurable only within a small documented range.
- No accumulation across fallback attempts. Each attempt owns and drops its bounded peek buffer.

`PeekedUpstreamResponse` contract:

- Contains original status, sanitized headers, content type, endpoint kind, peeked prefix bytes, remaining stream, guard outcome, and optional structured evidence.
- On `pass` or `cap_exhausted`, emits the exact prefix once followed by the exact remaining stream.
- On `classified`, no bytes have been emitted; the response is converted into `ClassifiedFailure` using `FailureSource::GuardedSuccessEnvelope` and the normal transition/retry path.
- It must not produce stale body headers. If a downstream filter may mutate the body, strip `Content-Length` and `Content-Encoding` before returning the response.

Detection rules:

- JSON content: inspect only unambiguous top-level structured error envelopes: an `error` object, or a top-level typed `code` plus `message`. Code-less error objects classify as request-only unless a typed relay-profile rule maps them.
- SSE content: parse only the first complete data-bearing event. Ignore comment-only and empty events. Concatenate multiple `data:` lines per SSE rules. Treat `[DONE]` as pass-through. A structured error object in that first data-bearing event may classify.
- The SSE scanner must handle LF and CRLF line endings, leading spaces after `:`, `event:` fields, comments, blank events, and split chunks. Pass-through replays original bytes exactly; parsing normalization must not rewrite the stream.
- Other content types pass through on cap/deadline exhaustion unless a clear JSON object is fully available within the cap. Do not add broad schema mismatch validation.

Pipeline order: upstream response -> 2xx guard -> reconstructed pass-through stream -> response filter -> client. The reconstructed prefix must be filtered exactly once and must not bypass the response filter.

Guard telemetry outcomes: `pass`, `classified`, `cap_exhausted`, `deadline_exhausted`, `parse_unsupported`. Classified fallback emits the Phase 2 retry decision event with `failure_source: guarded_success_envelope`.

**Phase 3 stop card:** red tests cover JSON error classification, code-less request-only behavior, body-bearing non-200 2xx responses, no-body status skip, first SSE error split across chunks, CRLF/comment/multiple-data-line SSE parsing, `[DONE]` pass-through, byte-exact prefix replay, no stale content length when filtered, cap and deadline pass-through, no fallback for streaming/non-replayable/partial-output paths, duplicate-charge-risk telemetry, and no false rejection of valid OpenAI Responses-style SSE. Complete only when local CI and x86_64 build pass.

## Phase 4: Response Filter Events And Protocol Framing

**Purpose:** make contaminated successful responses visible without mutating routing health, and make response-filter body mutation protocol-correct.

**Likely files:** `src/response_filter.rs`, `src/upstream_response.rs`, `src/events.rs`, `src/management_alerts.rs`, `src/management_runtime.rs`, `docs/response-filter.md`, tests.

Event ownership:

- `response_filter.rs` owns rule evaluation and returns rule id/action/reason code without raw matched text.
- `upstream_response.rs` owns stream framing and body/header mutation behavior.
- The forwarding context attaches request id, channel id, public model, and route target context before enqueueing the event.
- `AppState` owns a bounded in-memory `response_filter_events` ring, default capacity 1024.
- Management projections expose events and alerts. These events are not routing telemetry and are not replay input for credential/channel lifecycle.

Event schema at `GET /management/response-filter-events`:

`event_id`, `created_at_unix_seconds`, `request_id`, `channel_id`, `public_model`, `rule_id`, `action: redact|reject`, `content_kind: json|sse|bytes|unknown`, `reason_code: rule_matched|required_rule_missing`, `outcome: redacted|rejected`, `body_committed: true|false`.

Alert aggregation at `GET /management/alerts`: if a channel has at least three response-filter reject/redact events with the same rule id inside `response_filter.alert_window_seconds` (default 900), expose a `response_filter_contamination` alert with channel id, rule id, action counts, window seconds, and severity. Alerts decay by time window; no lifecycle mutation occurs.

Framing/header contract:

- Any body-mutating path strips `Content-Length` and `Content-Encoding`. Redaction preserves the original content type; rejection sets a local JSON or SSE content type that matches the emitted payload.
- SSE redaction emits valid SSE frames and preserves event boundaries.
- SSE rejection emits a valid local SSE error event. Non-SSE rejection emits a local JSON error payload with sanitized body headers.
- Non-UTF-8 bytes pass through unchanged and do not create matched-text events.
- Response-filter events never store matched text, raw chunks, request bodies, response bodies, upstream keys, client tokens, or absolute paths.

**Phase 4 stop card:** red tests cover event schema/redaction, ring capacity, alert decay window, SSE redaction/rejection framing, non-SSE rejection headers, content-length stripping on mutation, guard-prefix filtering exactly once, no lifecycle/channel mutation from filter events, no routing telemetry writes from filter hits, and local build gates.

## Phase 5: Explanation, Documentation, And Final Boundary Hardening

**Purpose:** complete operator explanation surfaces and documentation after the core relay hardening mechanisms are stable.

**Likely files:** `src/management_runtime.rs`, `src/management_routing.rs`, `src/management_alerts.rs`, `README.md`, `docs/architecture.md`, `docs/performance-budget.md`, tests.

- [ ] Add or stabilize `GET /management/explain/runtime` with runtime generation, registry source, credential source, client-token source, staged-vs-runtime state, reload requirement, last reload time, and last reload error.
- [ ] Add or stabilize model-route explanation using existing `/management/routing/preview` rather than adding live model discovery to `/v1/models`.
- [ ] Ensure `GET /management/health/serving` and `GET /management/health/resilience` include relay suppression, retry pressure, credential-set spare capacity, and response-filter alert summaries.
- [ ] Update README with a clear suitable/not-suitable boundary.
- [ ] Update performance docs with composed guard/filter byte and latency budgets.
- [ ] Update response-filter docs to state the event boundary and the explicit non-input to routing/channel lifecycle.

Explain endpoint schemas must be redacted: no raw tokens, token hashes, upstream keys, raw request/response bodies, matched text, absolute key paths, URL userinfo, or token-like query parameters.

**Phase 5 stop card:** red tests cover explain runtime schema, staged-vs-runtime reporting, routing preview skip reasons for relay-suppressed channels, health serving/resilience truth table, README boundary, performance-budget updates, response-filter docs consistency, and final local build/release hygiene. Complete only when the full roadmap verification gate passes.

## Deferred Boundary: Model Discovery

This roadmap does not implement persistent `discovered`, `probed`, `verified`, or `blocked` model states. The existing management-only discovery and explicit sync flow may continue to exist, but client traffic and `/v1/models` must not depend on live upstream discovery.

Reopen this boundary only if a new user goal explicitly asks for durable model trust state. Reopening requires a separate plan for catalog persistence, operator verification semantics, client-token scope interaction, and rollback. It must not be smuggled into relay error hardening.

## Parked Items

| Item | Why parked | Revisit trigger | Visibility |
| --- | --- | --- | --- |
| Free-form upstream message matcher | Unsafe for hot-path lifecycle decisions and injection-prone | A separate management-only probe summarizer design | Operator-visible follow-up |
| Automatic response-filter-driven channel cooldown/disable | Needs false-positive policy and lifecycle state design | Response-filter events show stable signal and user authorizes lifecycle mutation | Operator-visible follow-up |
| Account/provider balance scope | Suppression domain is broader than current roadmap | User explicitly needs shared-account suppression beyond selected channel | Architecture follow-up |
| Persistent model verification states | Requires durable catalog and product policy | New explicit model trust goal | Product/architecture follow-up |
| UI, billing, price sync, PostgreSQL, plugin ecosystems | Outside lightweight router boundary | New product goal | Out of scope |

## Production Bug Backlog

| Bug | Observed behavior | Required behavior | Planned owner phase |
| --- | --- | --- | --- |
| Contaminated 2xx completion does not isolate the selected credential or fallback to a clean credential | A real relay completion returned HTTP 200 with a normal-looking chat completion envelope but contaminated assistant content. The selected credential stayed available, so importing a replacement key did not automatically move traffic away from the contaminated credential; manual credential expiration was required. | A configured high-confidence response-filter rejection must occur before body commit when possible, record redacted evidence, mark the selected credential or channel according to an explicit operator policy, and retry only through the unified Phase 2 retry gates. If no clean candidate exists, return a local sanitized error instead of forwarding contaminated content. | Post-Phase 4 follow-up, after response-filter events and framing are implemented. |

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
| Phase 4 | Response-filter events, alerts, framing, and no-lifecycle-mutation tests pass | Filter hits need automatic channel mutation to satisfy acceptance. |
| Phase 5 | Explain, docs, health truth table, and final release hygiene pass | New product surfaces such as UI, billing, or persistent model trust are required. |

The whole roadmap stops after Phase 5. Any work on automatic filter-to-health mutation, account/provider balance suppression, persistent model discovery states, or remote deployment requires a new explicit goal.

## Convergence Record

Root issue ledger for this revision:

| Root | Resolution |
| --- | --- |
| Phase slicing too broad | Split foundation, classifier, channel state, retry observability, 2xx guard, filter events, and explain/docs into separate stop nodes. |
| State-machine lifecycle underspecified | Added failure source terms, classifier table, channel transition table, reset scope, and all-target fail-closed policy. |
| Hot-path protocol contract underspecified | Added 2xx scope, byte/time/event caps, SSE scanner rules, synthetic guarded evidence, prefix replay, and guard/filter ordering. |
| Management and release gates too late/vague | Moved roles, allowlist, audit, unknown-field validation, local x86_64 build, and release hygiene into Phase 0. |
| Tests not red/green enough | Added phase stop cards with endpoint/schema assertions and blocked conditions. |
| Model discovery scope drift | Reframed model discovery as deferred boundary only. |

Propagation audit: the locked invariants are restated in the decision matrix, phase tasks, stop cards, deferred boundary, and parked-item lifecycle. Cold scan result: no remaining internally solvable blocker is intentionally left open; parked items have revisit triggers and visibility. Current status: internally converged as a plan document after second-round subagent review, with implementation still not started.
