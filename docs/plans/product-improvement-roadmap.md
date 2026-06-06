# Product Self-Explanation And Stability Roadmap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** add the next product-level improvements for personal and small-team operators: endpoint-aware availability explanation, traceable stability behavior, bounded key maintenance, and explicit model onboarding without changing one-ai-key into a hosted platform or protocol conversion gateway.

**Architecture:** the data plane emits bounded facts and keeps using compiled in-memory runtime state. The management and control planes explain, persist, and mutate through redacted APIs. Stability improvements are compiled presets over existing routing and policy profiles, not a new adaptive engine.

**Tech Stack:** Rust, clap, serde/serde_json, Axum management APIs, compiled runtime snapshots, bounded in-memory telemetry, optional local event persistence, SQLite-backed stores where already configured, focused golden and integration tests.

---

## Review Inputs

This roadmap is the result of three review rounds across five independent
perspectives:

- personal/small-team operator UX and daily maintenance;
- data-plane stability, retry/fallback safety, and duplicate-charge risk;
- OpenAI-compatible protocol boundaries for `/v1/models`, `/v1/responses`,
  `/v1/chat/completions`, and endpoint families;
- architecture boundaries, performance budget, and hot-path invariants;
- documentation, golden contracts, test matrix, and release/config lifecycle.

The convergence point is:

- explainability and endpoint-family visibility must come before more
  automation;
- attempt trace and retry eligibility must exist before expanding stability
  behavior;
- key maintenance belongs to the management API boundary, not direct file or
  store editing;
- stability profiles are presets over existing routing/policy mechanisms, not
  a new health engine;
- Responses-to-Chat adaptation remains a separate specification track and is
  not authorized for implementation by this roadmap.

## Product Position

This roadmap continues the same product identity:

- one OpenAI-compatible base URL for clients;
- one client-facing token per client workflow;
- explicit local public model routes;
- upstream keys, relay failures, and lifecycle state hidden behind the router;
- hot path reads compiled runtime state only;
- successful responses stream instead of being fully buffered;
- operator tools are redacted, bounded, and explainable.

This roadmap is not:

- a hosted multi-tenant platform;
- a billing, usage charging, or resale system;
- a full web UI;
- an active health-check cluster;
- a live upstream model catalog aggregator;
- a broad OpenAI protocol compatibility layer;
- a persistent adaptive routing engine.

## Stop Nodes

These stop nodes are hard gates. Do not continue into the next phase when the
current phase requires scope expansion.

| Stop node | Complete when | Blocked, do not continue when |
| --- | --- | --- |
| S0 Terminology | `available`, `eligible`, `blocked`, `unknown`, `deprecated`, `adapter_disabled`, `no_route`, and `no_key` have stable meanings. `available` means routable under the current compiled runtime, not live upstream health. | A feature needs live upstream probing, request-path store reads, or free-form upstream text to define availability. |
| S1 P0 contract | Golden fixtures cover endpoint-aware explain, config lifecycle diagnostics, attempt trace schema, retry eligibility, counters, and redaction. | P0 output is only prose or ad hoc JSON without machine-checkable fixtures. |
| S2 Stability gate | Attempt trace, retry eligibility, failure taxonomy, and bounded ledger behavior are tested before any new stability preset can be enabled. | Conservative retry would be enabled without traceable attempt reasons, duplicate-charge risk, or ledger/counter evidence. |
| S3 Onboarding gate | Endpoint-family-aware explain tests pass before alias/profile/onboard dry-run implementation starts. | Onboarding would present proposed routes as real availability, or infer capability from upstream catalog or model names. |
| S4 Responses gate | The roadmap only produces a compatibility matrix and route opt-in schema for Responses adaptation. | Any worker starts implementing a Responses-to-Chat adapter, capability-triggered adapter, or endpoint fallback in this roadmap. |

P0, P1, and P2 form one minimum stability improvement track. P2 is not a
remote wishlist item: it is delayed only by the trace, eligibility, and ledger
gates needed to make conservative retry explainable and safe.

## Hard Invariants

The proxy request path must not:

- read YAML, SQLite, registry storage, credential storage, or client-token
  storage;
- call upstream `/v1/models` or any live catalog endpoint;
- scan all credentials in a set;
- compute route visibility from historical ledger data;
- complete protocol conversion based on endpoint capability metadata;
- retry after any client-visible output;
- fallback after streaming or partial output has begun;
- transparently retry or fallback any streaming request, including failures
  before upstream headers or body are received;
- fully buffer successful responses;
- expose endpoint-family support, tools, vision, context windows, pricing,
  upstream ids, native/adapter labels, or profile-derived capabilities through
  client-facing `/v1/models`;
- write raw request bodies, response bodies, upstream keys, client tokens,
  token hashes, complete URLs with token-like components, or untrusted upstream
  text into telemetry, logs, docs, tests, or reports.

Data-plane additions may only do fixed-cost lookups, atomic or sharded
counter increments, and bounded non-blocking event enqueue. Queue overflow must
drop the event and increment a dropped counter; it must not block request send,
credential mutation, or response streaming.

## Milestone Overview

| Milestone | User-visible outcome | Core boundary |
| --- | --- | --- |
| P0 Product self-explanation foundation | An operator can ask whether a client token can use a public model on a specific endpoint family, and see stable reasons without probing upstreams. | Management reads compiled runtime and bounded telemetry only. |
| P1 Operator maintenance and bounded persistence | An operator can maintain keys through official CLI/API workflows and inspect recent failures across restarts. | Mutations go through management APIs; persistence is bounded and redacted. |
| P2 Stability presets | A conservative preset absorbs small pre-output upstream jitter without hiding retry risk. | Preset expands into existing routing/policy profile fields; no adaptive engine. |
| P3 Explicit model onboarding | An operator can generate dry-run alias/profile/route plans with endpoint-family warnings. | Proposed routes stay staged; no automatic catalog trust or exposure. |
| P4 Responses compatibility specification | Future Responses adaptation has a negative contract and explicit opt-in shape. | No adapter implementation in this roadmap. |

## P0: Product Self-Explanation Foundation

**Purpose:** make the product able to explain itself before adding more
automation.

### P0.1 Endpoint-Family Route Visibility Contract

Endpoint family must become a normative route/model visibility dimension, not
only pool capability metadata. A public model can be visible for
`chat_completions` without being visible for `responses`.

Client-facing `/v1/models` remains a family-blind local public model projection.
It reports which public model ids are visible to the authenticated client token;
it does not assert that those ids work on `/v1/responses`, tools, vision,
structured output, adapters, context windows, or any other model capability.
Endpoint-family availability belongs to management explain/status only.

**Files:**
- Modify: `src/config.rs`
- Modify: `src/registry.rs`
- Modify: `src/route_plan.rs`
- Modify: `src/model_catalog.rs`
- Modify: `src/endpoint_capabilities.rs`
- Modify: `src/management_routing.rs`
- Modify: `docs/configuration.md`
- Modify: `docs/technical-design.md`

- [ ] Define endpoint-family visibility in config/runtime terms.
- [ ] Ensure static endpoint capabilities remain diagnostic metadata only.
- [ ] Add tests for a public model visible on chat but blocked on responses.
- [ ] Add tests that capability metadata cannot authorize endpoint fallback.
- [ ] Add golden tests proving `/v1/models` does not expose endpoint-family,
  adapter/native, profile/capability, upstream id, tools, vision, context, or
  pricing metadata.
- [ ] Commit with message `feat: add endpoint-family route visibility`.

### P0.2 Availability Explain Contract

Add or extend a management/CLI explain surface that answers:

```text
client_token_ref + endpoint_family + public_model -> current runtime usability
```

The output must distinguish token visibility, public model mapping,
endpoint-family route eligibility, channel health, credential availability,
adapter disabled, deprecated config, and bounded remediation hints.

**Files:**
- Modify: `src/management_routing.rs`
- Modify: `src/management_runtime.rs`
- Modify: `src/operator_client.rs`
- Modify: `src/cli.rs`
- Modify: `src/cli_commands/models.rs`
- Modify: `src/cli_commands/route.rs`
- Modify: `src/cli_report.rs`
- Modify: `README.md`
- Modify: `docs/technical-design.md`

- [ ] Write golden JSON fixtures for token missing, token disabled, endpoint
  mismatch, model missing, no route, no key, deprecated config, and adapter
  disabled.
- [ ] Implement stable reason codes and bounded remediation hints.
- [ ] Ensure CLI accepts token references, not raw client tokens.
- [ ] Add redaction tests for raw keys, raw tokens, token hashes, paths, and
  raw upstream text.
- [ ] Commit with message `feat: explain endpoint-aware model availability`.

### P0.3 Config Lifecycle Minimum Contract

This is not a heavy migration framework. It is the minimum contract needed for
future product evolution.

**Files:**
- Modify: `src/config.rs`
- Modify: `src/config_diagnostics.rs`
- Modify: `src/operator_templates.rs`
- Modify: `src/upstream_templates.rs`
- Modify: `src/cli_commands/doctor.rs`
- Modify: `docs/configuration.md`
- Modify: `docs/technical-design.md`

- [ ] Define schema version recognition.
- [ ] Define deprecated field/template warning shape.
- [ ] Add dry-run migration/report output that never writes by default.
- [ ] Hard-stop when migration would change route semantics, key selection
  semantics, fallback order, or security boundaries.
- [ ] Add golden tests for missing schema version, deprecated field,
  deprecated template, unknown field policy, idempotent dry-run, and failure
  without writeback.
- [ ] Add release lifecycle tests for old config on a new binary, new template
  on an old release, pinned-release rollback behavior, deprecated compatibility
  windows, and whether golden output may change across patch or minor releases.
- [ ] Commit with message `feat: add config lifecycle diagnostics`.

### P0.4 Attempt Trace, Retry Eligibility, And Minimal Counters

Before changing retry behavior, record why retry was or was not eligible.

**Files:**
- Modify: `src/routing.rs`
- Modify: `src/route_plan.rs`
- Modify: `src/failure_observer.rs`
- Modify: `src/failure_state_executor.rs`
- Modify: `src/events.rs`
- Modify: `src/management_status.rs`
- Modify: `src/proxy.rs`
- Modify: `docs/performance-budget.md`
- Modify: `docs/technical-design.md`

Trace fields must include request id, endpoint family, public model, attempt
index, route target, channel id, credential reference when available,
failure kind, retry directive, replayability, streaming/pre-output state,
commit state, duplicate-charge risk, and elapsed bucket. They must not include
raw body or raw upstream text.

Allowed low-cardinality counter dimensions:

- endpoint family;
- public model;
- channel id;
- status class;
- failure kind;
- retry directive;
- commit state;
- duplicate-charge risk.

Do not add client-token default top dimensions in this phase.

- [ ] Add golden schema tests for attempt trace.
- [ ] Add tests proving retry-ineligible reasons for streaming, partial
  output, non-replayable body, unknown failure, and deadline exhaustion.
- [ ] Add tests that enqueue overflow drops and increments a dropped counter.
- [ ] Add tests that counters do not require disk I/O or high-cardinality
  fields.
- [ ] Add golden schema tests for status/top/counter output, including stable
  field names, sort order, truncation behavior, reset/window semantics, and
  allowed dimension sets.
- [ ] Commit with message `feat: record retry eligibility traces`.

## P1: Operator Maintenance And Bounded Persistence

**Purpose:** give operators official maintenance workflows and durable failure
context without turning observability into billing or active health checking.

### P1.1 Key CLI As Management API Wrapper

Key maintenance commands must not directly edit active config files, key files,
SQLite databases, or runtime memory. Except for bootstrap-only dry-run helpers,
all mutations go through management APIs and inherit management auth, roles,
redaction, audit, and generation preconditions.

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/cli_commands/keys.rs`
- Modify: `src/operator_client.rs`
- Modify: `src/cli_effects.rs`
- Modify: `src/cli_report.rs`
- Modify: `src/management_credentials.rs`
- Modify: `docs/configuration.md`
- Modify: `README.md`

Target workflows:

- `keys stats`;
- `keys import` / `keys append`;
- `keys promote` where selector semantics support it;
- `keys retire` / `expire` / `restore`;
- `keys probe` and `keys probe-apply` with explicit confirmation.

- [ ] Add effect-class tests for every mutating command.
- [ ] Add tests proving raw key material is never printed.
- [ ] Add tests that CLI rejects client `/v1` base URLs for management
  operations.
- [ ] Add tests for generation/precondition failures.
- [ ] Commit with message `feat(cli): add key maintenance workflows`.

### P1.2 Persistent Bounded Failure Ledger

The ledger records redacted event projections across restarts. It is not a
billing ledger and not a breaker-state source.

**Files:**
- Modify: `src/events.rs`
- Modify: `src/management_events.rs`
- Modify: `src/management_status.rs`
- Modify: `src/cli_commands/failures.rs`
- Modify: `src/operator_client.rs`
- Modify: `docs/technical-design.md`
- Modify: `docs/performance-budget.md`

- [ ] Define versioned ledger schema and retention policy.
- [ ] Persist bounded redacted failure/attempt events through existing event
  boundaries.
- [ ] Ensure writer failure is observable but cannot block request handling.
- [ ] Add restart-read tests, corruption/partial-record tests, capacity tests,
  and redaction tests.
- [ ] Commit with message `feat: persist bounded failure ledger`.

### P1.3 Status And Top Views

Status/top views should help a local operator answer what is unstable without
becoming billing, auditing, or prompt logging.

**Files:**
- Modify: `src/management_status.rs`
- Modify: `src/cli_commands/doctor.rs`
- Modify: `src/cli_report.rs`
- Modify: `src/operator_client.rs`
- Modify: `README.md`

- [ ] Add local summary views over allowed counter dimensions.
- [ ] Keep all top-N outputs bounded and low-cardinality.
- [ ] Define stable JSON schema, field names, sort order, truncation behavior,
  and reset/window semantics for status/top output.
- [ ] Add tests rejecting raw body, prompt, completion, full headers, raw
  token, raw key, and complete upstream URL components.
- [ ] Commit with message `feat: add bounded local status summaries`.

## P2: Stability Presets

**Purpose:** reduce client-visible failure from small upstream jitter while
preserving traceability, streaming safety, and duplicate-charge visibility.

P2 must not start until P0 trace/eligibility and P1 ledger behavior pass their
stop nodes.

### P2.1 Conservative Stability Preset

The preset is a compile-time control-plane expansion into existing routing and
policy fields. It is not a new state machine.

**Files:**
- Modify: `src/config.rs`
- Modify: `src/routing.rs`
- Modify: `src/route_plan.rs`
- Modify: `src/upstream_templates.rs`
- Modify: `src/config_diagnostics.rs`
- Modify: `docs/configuration.md`
- Modify: `docs/performance-budget.md`
- Modify: `docs/technical-design.md`

Default conservative behavior:

- maximum one additional pre-output attempt;
- total wall-clock retry budget <= 1500 ms unless explicitly configured lower;
  the budget includes backoff, candidate selection, and the additional attempt
  until upstream headers or terminal failure;
- no retry may start unless the remaining effective client/request deadline can
  contain the retry budget;
- only replayable request bodies;
- only known endpoint families marked replayable;
- no retry after client-visible output;
- no transparent retry or fallback for streaming requests, including pre-header
  failures;
- no partial-output fallback;
- no default cross-provider fallback;
- 429 is not retryable by default;
- 502/503/504, connect failure, TLS/connect timeout, connection reset before
  headers, and header timeout may be eligible when all gates pass;
- duplicate-charge risk is always recorded when it cannot be proven absent.

- [ ] Add tests for connect failure, header timeout, 502/503/504 pre-output
  retry, and no retry for unknown failures.
- [ ] Add tests for no retry on streaming requests, pre-header streaming
  failures, partial body, non-replayable endpoint, 401/403/404, model missing,
  permission error, quota exhausted, context length, and schema/parameter
  errors.
- [ ] Add tests proving no retry starts when the remaining effective deadline
  cannot contain the wall-clock retry budget.
- [ ] Add tests for no default cross-provider fallback.
- [ ] Add tests proving attempt traces include eligibility and final outcome.
- [ ] Commit with message `feat: add conservative stability preset`.

### P2.2 Stability Explain Integration

Operators must be able to see why a request was or was not eligible for retry.

**Files:**
- Modify: `src/management_routing.rs`
- Modify: `src/management_status.rs`
- Modify: `src/cli_commands/failures.rs`
- Modify: `src/cli_commands/doctor.rs`
- Modify: `README.md`

- [ ] Show stability preset name, retry gates, and last denial reasons in
  management-only views.
- [ ] Add fixtures for "eligible but no candidate", "ineligible because
  streaming", "ineligible because deadline", and "retry attempted with
  duplicate-charge risk unknown".
- [ ] Commit with message `feat: explain stability retry decisions`.

## P3: Explicit Model Onboarding

**Purpose:** make local public model configuration easier without trusting
upstream catalogs or implying capabilities from model names.

### P3.1 Alias/Profile Dry-Run Contract

Aliases are public ids. Profiles are operator templates. Neither implies tool,
vision, Responses, context-window, pricing, or provider truth.

**Files:**
- Modify: `src/cli_commands/models_onboard.rs`
- Modify: `src/model_catalog.rs`
- Modify: `src/config_diagnostics.rs`
- Modify: `src/operator_templates.rs`
- Modify: `docs/configuration.md`
- Modify: `README.md`

- [ ] Add dry-run output for alias/profile/route diffs.
- [ ] Mark proposed routes as proposed, not available.
- [ ] Warn on official-looking aliases whose endpoint-family visibility or
  capability metadata conflicts with the proposed route.
- [ ] Add tests that alias does not auto-expand endpoint family.
- [ ] Add tests that dry-run does not change runtime or `/v1/models`.
- [ ] Commit with message `feat(cli): add explicit model onboarding plans`.

### P3.2 Endpoint-Aware Onboarding Warnings

**Files:**
- Modify: `src/cli_commands/models_onboard.rs`
- Modify: `src/cli_report.rs`
- Modify: `docs/configuration.md`

- [ ] Warn when Responses family is requested but only chat native route exists.
- [ ] Warn when adapter-only future route is proposed.
- [ ] Warn when multi-target endpoint-family candidates differ.
- [ ] Commit with message `feat(cli): warn on endpoint-family onboarding gaps`.

## P4: Responses Compatibility Specification Track

**Purpose:** define the future adapter contract without implementing it here.
Any route opt-in schema in this milestone is reserved and future-only: the
current release must not accept it, enable it, or present it as a working
configuration example.

**Files:**
- Create: `docs/responses-compatibility.md`
- Modify: `docs/technical-design.md`
- Modify: `docs/configuration.md`

The specification must state:

- endpoint capabilities never trigger adapters;
- future adapters require explicit route target opt-in;
- native Responses and adapter-backed Responses are distinct in management
  explain;
- unsupported fields hard-fail deterministically;
- no silent field dropping;
- no missing-model defaulting;
- no automatic endpoint fallback;
- no streaming implementation unless valid Responses SSE semantics are
  generated;
- no state, tools, multimodal input, structured output, or `previous_response_id`
  support unless a future plan proves exact behavior.

- [ ] Write a support/reject matrix for Responses fields.
- [ ] Define future route opt-in schema without enabling it.
- [ ] Add docs stating the schema is reserved/future-only and current releases
  must reject or ignore it as specified rather than enable an adapter.
- [ ] Define deterministic error code shape such as
  `unsupported_response_adapter_feature`.
- [ ] Add docs that P4 does not implement the adapter.
- [ ] Commit with message `docs: define responses compatibility boundary`.

## Parallel Execution Matrix

The plan can use up to five subagents after the main controller freezes shared
interfaces for the current milestone.

| Milestone | Parallel lanes | Shared files guarded by main controller |
| --- | --- | --- |
| P0 | Endpoint-family visibility, config diagnostics, and trace schema can be explored in parallel after naming shared DTOs. | `src/config.rs`, `src/cli.rs`, `src/cli_report.rs`, `src/operator_client.rs`, this plan. |
| P1 | Key CLI, ledger persistence, and status summaries can run separately after redaction envelope is frozen. | `src/cli.rs`, `src/operator_client.rs`, `src/events.rs`, `src/cli_report.rs`. |
| P2 | Preset expansion and explain rendering should be serialized unless routing profile DTOs are already committed. | `src/routing.rs`, `src/route_plan.rs`, `src/config.rs`, `src/management_routing.rs`. |
| P3 | Onboarding warnings and docs can run in parallel after endpoint-family dry-run schema is frozen. | `src/cli_commands/models_onboard.rs`, `src/cli_report.rs`. |
| P4 | Documentation-only review lanes can run in parallel. | `docs/responses-compatibility.md`, `docs/technical-design.md`. |

Subagents must report their write set. If two lanes need the same guarded file,
the main controller serializes the edits or creates a small integration commit.

## Verification Gate

Before closing any milestone:

- run non-zero filtered test discovery for every filtered Cargo command;
- run the narrow tests named by the task;
- run the milestone integration tests;
- run `cargo test --locked` unless the task explicitly documents why a narrower
  suite is the release gate;
- run `git diff --check`;
- run `git status -sb --untracked-files=all`;
- inspect `git diff --cached --name-only`;
- confirm no staged `dist/`, `target/`, `key-pool-router/`, `config/`, `data/`,
  SQLite, logs, keys, raw fixtures, or `AGENTS.md`;
- update docs only with product usage and stable contracts, not support-chat
  transcripts.

## Roadmap Completion Criteria

This roadmap is complete when P0-P4 are either implemented or intentionally
parked exactly as described:

- P0-P3 code, tests, docs, and commits are complete;
- P4 produces a specification only, not runtime code;
- all stop nodes pass;
- hot-path invariants remain true;
- local release/build discipline remains unchanged;
- no Responses adapter, active health cluster, live model aggregation, billing,
  UI, persistent adaptive routing, or request-path storage dependency has been
  introduced.

If implementation requires any forbidden capability, stop this roadmap and
create a separate plan with its own invariants, hot-path proof, UX contract, and
release gate.
