# Product Improvement Roadmap

This document is a product roadmap, not a task queue. It keeps one-ai-key focused
as a lightweight personal/small-team AI key router: clients keep one
OpenAI-compatible base URL and one client token, while routing, upstream keys,
failure handling, and maintenance stay behind the router.

The previous broad P0-P4 plan has been deliberately shrunk. The useful core is:

- explain whether a client can use a public model on a specific endpoint family;
- maintain keys through explicit management workflows;
- keep bounded failure evidence for troubleshooting and retry audit;
- define the smallest conservative pre-output stability contract that can absorb
  isolated upstream jitter without turning routing into an adaptive platform.

Everything else is parked unless a new plan explicitly reopens it.

## Product Narrative

The roadmap follows one operator decision loop:

```text
explain current usability -> maintain with bounded evidence -> cautiously
stabilize eligible pre-output failures
```

M1 answers whether a specific client-token reference can use a specific public
model on a specific endpoint family, and why. M2 makes the next maintenance
action explicit and bounded: read-only, dry-run, upstream-touching, or mutating.
M3 is the only data-plane stability step in this roadmap; it may hide one small
pre-output upstream failure when all retry gates pass, and it must record why it
did or did not retry.

Do not use incident narratives, private deployment traces, temporary key
replacement stories, or one-off support workarounds as product requirements.
Convert recurring lessons into stable routing, reporting, or release rules, then
delete the support context.

## Product Boundary

one-ai-key should remain:

- lightweight and backend-first;
- explicit about local public model routes;
- client-invisible for upstream keys and relay quirks;
- predictable on small deployments;
- conservative about retry and fallback;
- redacted at every management boundary.

It must not become:

- a hosted multi-tenant platform;
- a billing, usage accounting, or resale system;
- a full UI product;
- an active health-check cluster;
- a live upstream model catalog aggregator;
- a broad OpenAI protocol conversion gateway;
- a persistent adaptive routing engine.

## Hard Invariants

The proxy request path must not:

- read YAML, SQLite, registry storage, credential storage, or client-token
  storage;
- call upstream `/v1/models` or any live catalog endpoint;
- scan all credentials in a set;
- compute route decisions from historical ledger data;
- complete protocol conversion based on endpoint capability metadata;
- retry after any client-visible output;
- transparently retry or fallback any streaming request, including pre-header
  failures;
- fallback after partial output;
- fully buffer successful responses;
- record raw request bodies, response bodies, upstream keys, client tokens,
  token hashes, complete URLs with token-like components, or untrusted upstream
  text in telemetry, logs, tests, docs, or reports.

Data-plane additions may only emit bounded facts: fixed-cost counters, bounded
attempt summaries, and non-blocking redacted event records. Failure evidence
does not absorb upstream jitter by itself; it only explains whether M3 retry was
eligible, attempted, denied, or risky. Overflow drops the oldest or current
event according to the concrete buffer/queue contract and increments a
dropped-event counter; it must not block request sending, credential mutation,
or response streaming.

Client-facing `/v1/models` remains a family-blind local public model projection.
It reports visible public model ids for the authenticated client token. It does
not assert Responses support, tools, vision, context window, pricing, adapter
status, endpoint-family support, upstream ids, or provider capability.

Endpoint-family availability belongs to management explain/status only.

## Configuration Surface Rules

Configuration exists to build explicit resources and compiled runtime
projections. It is not a dumping ground for report display preferences,
one-off diagnostics, deployment evidence, or operator-session state.

YAML and registry fields are appropriate only when they change resource
topology, credential lifecycle policy, response-filter policy, or a compiled
routing/profile projection. Management query parameters and CLI flags own
one-time explanation dimensions such as `endpoint_family`, pagination, display
limits, failure source filters, credential-state filters, `--dry-run`, `--yes`,
and local import/probe inputs.

M3 must not add a YAML `preset` field by default. A future
`Conservative Pre-Output Stability v1` plan may name an init-template profile
that renders explicit `routing_profiles` fields. A runtime config `preset`
field is rejected unless a later admission record proves that explicit knobs are
insufficient and defines conflict resolution, compiled projection, redaction,
tests, and hot-path proof.

Existing probe-derived policy configuration is compatibility surface, not a
license to build automated key maintenance. Probe actions remain explicit
management operations; they must not be triggered from request forwarding,
failure evidence, background jobs, or automatic scope mutation.

## Roadmap

### M1: Self-Explanation Contract

**Goal:** make the router answer the operator's core question:

```text
client_token_ref + endpoint_family + public_model -> status / reason_code / next_action
```

This is not live health checking. It explains the current compiled runtime.

Keep:

- endpoint family as a compiled route visibility dimension;
- management-only availability explain;
- family-blind `/v1/models`;
- stable reason codes;
- redacted, bounded next actions;
- minimal config diagnostics for schema version and deprecated fields/templates.

Do not add:

- live upstream probes;
- upstream catalog fan-out;
- endpoint fallback;
- automatic protocol conversion;
- a config migration framework;
- full release-lifecycle migration matrices;
- capability metadata in `/v1/models`.

Acceptance gates:

- explain can distinguish token missing/disabled, model missing, endpoint
  family mismatch, unsupported endpoint family, no route, no usable key, and
  deprecated config;
- all outputs are redacted and machine-testable;
- `/v1/models` remains a simple local public-model list;
- deprecated config diagnostics report risk but do not rewrite config or change
  route semantics.

### M2: Bounded Maintenance And Failure Evidence

**Goal:** make daily maintenance less ad hoc without creating a control-plane
platform.

Key maintenance is a thin management API wrapper. It must not directly edit
active config files, key files, SQLite stores, or runtime memory.

Keep:

- `keys list` / `keys stats`;
- `keys import` or `keys append` with dry-run and explicit confirmation;
- a simple disable/retire action for bad keys when the existing management API
  supports it;
- manual `keys probe` only as an explicit upstream-touching operator action;
- bounded recent failure/attempt evidence for troubleshooting and retry audit;
- a small status summary for recent failures and dropped telemetry.

Park:

- `probe-apply`;
- selector promotion workflows;
- broad restore/promote lifecycle command sets;
- usage ledger, billing-style top views, audit ledger, and cost tracking;
- persistent failure ledger as a required prerequisite.

If an implementation already exposes a probe-apply management path, this
roadmap treats it as existing compatibility surface only. M2 does not expand it,
promote it as a daily-maintenance dependency, add new action enums, add batch
auto-apply, or let probe evidence mutate credentials without an explicit
management action.

Failure evidence is not a routing input and must not be used to absorb
upstream jitter. The minimum failure evidence comes only from two in-memory
rings: routing telemetry controlled by `routing.telemetry_buffer_capacity` and
response-filter events controlled by `response_filter.event_window_capacity`.
Both default to 1024 events and reject public configuration outside 1 to 4096.
Management API reads are paged separately: event endpoints default to a 100-item
page and clamp a single page to 1000 items. CLI failure summaries are narrower
for one-screen operation: `one-ai-key failures` defaults to 50 records per
source and caps each source at 200, so a combined routing + response-filter
report can return at most 400 records. Overflow, dropped appends, and
runtime-reload shrink only increment the relevant dropped counter; they must
not block proxy requests or management reads. This roadmap does not introduce a
persistent failure ledger.

Failure evidence should be minimal: request id, time bucket, endpoint family,
public model, channel/credential reference when safe, failure kind, retry
decision, commit state, streaming state, duplicate-charge risk, and dropped
event count.

Management failure-evidence output is a field whitelist, not raw telemetry
serialization. Displayable string fields are local identifiers only: trimmed,
128 bytes or shorter, ASCII `[A-Za-z0-9._:-]`, and rejected if they contain
token-like, URL-like, or promotional/injection-like fragments. Unsafe values are
rendered as `null`; raw credential ids are exposed only as short irreversible
hashes. Upstream bodies, matched filter text, complete URLs, key paths, token
hashes, request payloads, and upstream free-form `error.message` are never
failure evidence. `max_error_body_bytes` is a classification input cap, not a
recording allowance.

Acceptance gates:

- key CLI mutations go through management API auth, redaction, and existing
  mutation boundaries;
- no command prints raw keys, raw tokens, token hashes, absolute key paths, raw
  request/response bodies, or full upstream URLs with token-like components;
- failure evidence is bounded and redacted, with routing telemetry and
  response-filter event windows remaining 1024 in-memory events by default and
  capped at 4096 events when explicitly configured;
- status output is one-screen operational context, not an analytics product.

Provider/account transient cooldown is a soft route state, not an automatic
local admission failure. Normal and degraded candidates win first. If every
otherwise valid route target is provider-cooling, the router may use a
provider-cooling target as the last resort instead of returning local
`no_route_candidate` immediately. Within a frozen fallback chain,
provider-cooling targets are skipped while later non-cooling targets remain.
Hard channel cooldown from relay balance, response-filter rejection, disabled
channels, runtime lock contention, and empty credential pools remain admission
blockers.

### M3: Conservative Pre-Output Stability

**Goal:** reduce client-visible failures from small upstream jitter without
making retry behavior opaque.

This is the only stability behavior in this roadmap. It is a small capability
package over explicit routing/profile mechanics, not a new stability engine and
not a runtime YAML preset.

Allowed conservative retry:

- at most one extra upstream attempt for the original client request;
- total wall-clock retry budget <= 1500 ms;
- budget includes backoff, candidate selection, and the extra attempt until
  upstream headers or terminal failure;
- retry may start only if the remaining effective client/request deadline can
  contain that budget;
- request body must be replayable;
- endpoint family must be known and retry-eligible;
- failure must be explicit pre-output transient evidence such as connect
  failure, TLS/connect timeout, header timeout, connection reset before
  headers, or 502/503/504 before body;
- duplicate-charge risk must be recorded when it cannot be proven absent.

The one extra attempt is mutually exclusive. A request may choose only one of:

- retry with another credential on the same selected channel when same-request
  credential retry is explicitly enabled and the replacement credential is
  proven different;
- retry the next eligible frozen route target from the original compiled route
  plan when route-target retry is explicitly enabled;
- retry the same target once as a last resort for selected transient
  channel/provider failure when no frozen target remains and no cooldown
  evidence is present.

These choices do not chain. A failed credential retry must not then fall through
to a route-target retry inside the same original client request.

Endpoint-family eligibility is an allowlist. Initial M3 eligibility covers
non-streaming `chat_completions` and non-streaming `responses` only when the
request body is replayable, the public model has been resolved, and the route
plan is frozen. `/v1/models` is local projection and is never retry-eligible.
Streaming Chat/Responses, Embeddings, named-pool requests, and unknown endpoint
families are not eligible unless a separate plan proves replayability,
idempotency/charge risk, parser behavior, and tests.

Default prohibitions:

- no streaming retry or fallback;
- no partial-output fallback;
- no retry after client-visible output;
- no default cross-provider fallback;
- no retry for unknown failures;
- no retry for 401/403/404, model missing, permission errors, quota exhausted,
  context length, schema errors, or request parameter errors;
- 429 is not retryable by default;
- no active health scanning, sliding windows, adaptive routing, persistent
  breakers, or ledger-driven route decisions.

Acceptance gates:

- every retry decision has a recorded eligibility reason;
- every denial has a stable reason code;
- retry evidence is visible in management-only failure/explain output;
- the hot path still satisfies the hard invariants above.

## Parked Items

The following are outside this roadmap and must not be implemented by extending
this document:

- Responses-to-Chat adapter;
- future Responses route opt-in schema;
- adapter-backed Responses examples;
- endpoint capability triggered protocol conversion;
- automatic endpoint fallback;
- YAML retry presets or global retry switches that bypass explicit
  `routing_profiles`;
- model alias/profile onboarding planner;
- automatic model exposure from upstream catalog;
- persistent model trust states;
- full config migration framework;
- status/top dashboards;
- billing, usage accounting, cost tracking, or per-client analytics;
- active background health probing;
- provider-wide adaptive breakers.

If any parked item becomes necessary, stop this roadmap and write a separate
plan with its own hot-path proof, user contract, and release gate.

## Completion Criteria

This roadmap is complete when M1-M3 are implemented, documented, tested, and
committed. Parked items are not part of completion.

Before closing the roadmap:

- filtered tests must prove they matched non-zero intended tests;
- `cargo test --locked` or a justified narrower release gate must pass;
- `git diff --check` must pass;
- staged files must exclude `dist/`, `target/`, `key-pool-router/`, `config/`,
  `data/`, SQLite, logs, keys, raw fixtures, and `AGENTS.md`;
- docs must describe stable product behavior, not support transcripts or future
  feature promises.

## Plan Closure

A milestone is closed by evidence, not by continued searching. Its stop card
must record:

- `plan_id`;
- `release_or_scope_name`;
- `closed_capability`;
- `implemented_scope`;
- `deferred_scope`;
- `parked_or_rejected_items`;
- `operator_contract_evidence`;
- `test_evidence`;
- `non_empty_filtered_test_evidence`;
- `local_ci_result`;
- `release_artifact_result`;
- `redaction_and_denylist_result`;
- `anti_platform_gate_result`;
- `support_residue_scan_result`;
- `deployment_boundary_result`;
- `known_blockers`;
- `next_version_candidates`.

Once the fixed evidence fields are present and valid, stop. New concerns must
be classified as a blocking defect, a next-version blocker, or post-release
debt. Do not keep expanding smoke tests, docs, or review loops without replacing
an existing matrix item or opening a separate accepted plan.

## Next Version Entry

A next version starts only after the current stop card is closed or the current
plan is explicitly abandoned. It must define one named capability package, not a
catch-all upgrade. Its entry record must include:

- `version_name`;
- `single_capability_package`;
- `entry_precondition`;
- `problem_statement_without_incident_narrative`;
- `user_visible_contract`;
- `non_goals`;
- `mature_gateway_principles_absorbed`;
- `explicit_rejections`;
- `new_command_api_config_admission_records`;
- `hot_path_proof`;
- `state_and_cardinality_bounds`;
- `redaction_contract`;
- `test_plan`;
- `release_gate`;
- `stop_or_block_conditions`.
