# Post-v0.2 Optimization Roadmap

This roadmap defines the next optimization sequence after the operator
confidence baseline. It narrows future work to capabilities that improve daily
personal operation without changing one-ai-key into a hosted gateway, analytics
platform, live catalog aggregator, or broad protocol adapter.

The plan is public product design, not an execution transcript. It records the
accepted scope, rejected scope, interface direction, verification gates, and
stop conditions for the next development packages.

## Product Position

one-ai-key remains a lightweight personal or small-team AI key router. Clients
configure one OpenAI-compatible base URL, one client token, and explicit public
model ids. The router hides upstream credentials, channel state, relay quirks,
bounded retry decisions, and maintenance workflows behind compiled local runtime
state and management-only operator reports.

The next optimization work should improve three operator questions:

```text
Can this client call this model on this endpoint family?
If not, is the blocker route admission, credential capacity, runtime drift, or
selected-upstream failure?
What is the next safe maintenance command, and what must remain manual?
```

The request path must stay conservative. Product improvements belong in typed
control-plane projections, offline validation, and release verification unless
a separate plan proves that data-plane behavior is necessary and safe.

## Scope Decisions

The following decisions are accepted for the next sequence.

- Route-admission explanation is a product feature. `no_route_candidate` should
  be explainable as local admission failure with a nested primary blocker, not
  confused with an upstream `503`.
- Credential replacement should be an explicit workflow priority projection,
  not automatic rotation. Existing import, probe, restore, and disable commands
  remain the mutation boundary.
- Response filtering does not need a larger runtime engine. The missing piece
  is offline proof that configured rules match the intended plain-response and
  error-envelope samples without leaking sample bodies.
- Deployment smoke remains an operator-run evidence tool after a deployment has
  already been pinned to a release asset. It must not become a deployer, remote
  repair script, or CI dependency on a live deployment.
- Configuration parsing and runtime assembly need a behavior-preserving
  compiler boundary. That is structural work and should not be bundled into a
  small feature release.

The following proposals are rejected for this sequence.

- live upstream model catalog aggregation;
- automatic key rotation or background key probing;
- persistent failure ledger or routing decisions from historical records;
- broad Responses-to-Chat or endpoint fallback adapter;
- automatic client-token scope repair;
- UI, dashboard, billing, usage accounting, or hosted multi-tenancy;
- request-path reads from YAML, SQLite, registry storage, credential storage,
  client-token storage, failure windows, or live upstream catalogs.

## Version Package: v0.3 Operator Maintenance Evidence

`v0.3` should ship one coherent capability package:

```text
operator evidence contract -> diagnosis alignment -> explicit safe action
```

The package is one operator workflow: gather trusted redacted evidence, align
that evidence across diagnosis surfaces, and expose the next safe action before
any mutation. It may contain two product capabilities:

1. **Unified admission and maintenance projection.** Route, model, failure, and
   key-maintenance reports agree on blocker class, admission primary reason,
   credential-set impact, and safe next action.
2. **Evidence contract validation.** Response-filter rules and deployment smoke
   contracts can be validated locally or by explicit operator-run smoke so the
   reports above are trusted without storing secrets or raw untrusted upstream
   text.

It must not introduce new data-plane retry behavior, new route policy fields,
new registry schema, or automatic mutation.

### Capability 1: Unified Admission And Maintenance Projection

The route planner remains the source of route-admission truth. Management
projections should add stable, redacted fields rather than making CLI commands
derive admission independently.

Accepted interface direction:

- `/management/routing/preview` keeps the existing `admission_summary` and adds
  additive `admission_summary.primary_reason_code`. Existing fields remain
  unchanged.
- `RouteAdmissionDenied` failure projections keep top-level
  `reason_code=no_route_candidate`, while adding nested admission data such as
  `admission.status`, `admission.primary_reason_code`,
  `admission.hard_reason_codes`, and `admission.soft_reason_codes`.
- endpoint-family availability evidence used by `models explain` carries the
  same route-planning admission primary reason as `route explain`.
- `models explain`, `route explain`, and `failures explain` render management
  projections. They must not recompute route admission in CLI code.
- `keys replacement-plan` renders a `replacement_workflow` object produced by a
  management projection or shared domain service. CLI command code renders the
  object; it does not own maintenance business rules.

The `replacement_workflow` object should use operator-maintenance language, not
route-strategy language. `replacement_workflow.operator_maintenance_priority`
is unrelated to `RouteTarget.priority`, route target weight, or route ordering.
Suggested machine fields:

```text
replacement_workflow.operator_maintenance_priority:
  active_capacity_missing
  selected_capacity_low
  fallback_capacity_low
  inventory_only
  route_impact_unknown

replacement_workflow.blocking_reason_code:
  no_available_credentials
  credential_set_exhausted
  route_not_candidate
  route_preview_unavailable
  none
```

The workflow must include one deterministic next action. Diagnosis next actions
are read-only or local-preview by default. Mutating key operations remain
explicit commands that require their existing dry-run and confirmation gates.

Verification gates:

- local admission `503` and selected-upstream `503` remain distinguishable:
  local admission reports `client_visible_status=local_503`,
  `upstream_status=null`, and top-level `reason_code=no_route_candidate`;
  selected-upstream failure reports `client_visible_status=upstream_5xx`,
  `upstream_status=503`, and no nested admission evidence.
- `models explain`, `route explain`, and `failures explain` agree on admission
  primary reason for the same model, client-token reference, and endpoint
  family.
- local admission `503` performs zero upstream calls, creates no retry
  directive, does not increment retry-pressure counters, and does not recompute
  candidates after the route-planning boundary.
- `keys replacement-plan` is still read-only and does not POST, probe, import,
  restore, disable, reload, or write local stores.
- replacement workflow priority has matrix tests for missing set, empty set, no
  available credential, partial loss, selected route candidate, fallback route
  candidate, not-a-candidate, route unavailable, and no model context.
- output does not contain raw keys, raw tokens, token hashes, fingerprints,
  absolute key paths, raw request or response bodies, complete upstream URLs, or
  upstream free-form text.

Stop conditions:

- stop if CLI logic starts duplicating route planning, credential lifecycle
  classification, or endpoint-family visibility;
- stop if replacement workflow requires automatic key import, automatic probe
  apply, background scan, route mutation, reload apply, or direct SQLite edits;
- stop if any new field is needed by request forwarding before a management
  projection and redaction contract exist.

### Capability 2: Offline Evidence Validation

The filtering and smoke surfaces should prove their contracts without widening
runtime behavior.

#### Response-Filter Sample Validation

Runtime response filtering already has separate paths for plain success content,
pre-output error envelopes, and committed-output observations. The next
improvement is an offline checker that proves configured rules match intended
samples.

Accepted interface direction:

- sample bodies must be supplied through an explicit local sample input to
  `check-config` or a focused `response-filters check` command, and must not be
  stored in runtime YAML or router configuration;
- support two sample kinds: `plain` and `error_json`;
- report only sample id, expected outcome, actual outcome, matched rule ids, and
  reason code;
- never echo sample body, matched text, upstream payload, URL, key path, token,
  or private file path;
- reuse the compiled response-filter policy so sample validation cannot drift
  from runtime matching semantics.

The initial sample format can be small and local-only:

```text
id
content_kind: plain | error_json
body
expect_outcome: redacted | rejected | unchanged
expect_rule_ids
```

The sample file is test input, not router state. It must not affect request
forwarding, reload, management state, or response-filter event windows.

Verification gates:

- invalid sample schema fails with a stable reason code;
- expected match missing fails without printing sample body;
- `plain` and `error_json` samples use the same matching paths as runtime;
- required-rule absence does not falsely reject error-envelope samples;
- at least one redaction and one rejection case prove non-zero rule matches in
  targeted tests.

Stop conditions:

- stop if this becomes a built-in rule list, moderation system, routing input,
  lifecycle policy engine, or request-path file read;
- stop if error reporting requires storing or displaying raw sample content.

#### Operator-Run Deployment Smoke Contract

The existing local release smoke remains the release artifact gate. Deployment
smoke is separate: it runs only after an operator intentionally points it at a
deployed instance.

Accepted interface direction:

- keep one explicit script entry with an allow flag;
- require base URL, management URL, client-token environment variable name,
  management-token environment variable name, and public model id through
  explicit environment inputs;
- output stable JSON statuses and reason codes;
- store only redacted status, public model id, check names, release identity
  when provided, and checksum identity when provided;
- add a local mock positive test for the deployment-smoke script so its success
  contract is verified without calling a live deployment.

Verification gates:

- missing allow flag refuses with structured JSON;
- repository-local output directories and repository-local temporary directories
  are rejected;
- local mock success covers exactly the checked surface: `/health`,
  `/management/health/serving`, `/management/health/resilience`, `/v1/models`,
  and one non-streaming `/v1/chat/completions` request;
- the mock contract verifies expected methods, presence of Authorization without
  logging token values, public model visibility, completion message shape,
  summary `status=ok`, and the stable check names/count;
- successful and failed runs scan stdout, summary JSON, and per-check artifacts
  for token values, full URLs, raw request bodies, raw response bodies,
  provider payloads, and private paths;
- the script does not SSH, rebuild NixOS, publish releases, copy files to a
  host, or repair configuration.

Stop conditions:

- stop if deployment smoke becomes CI against a live service;
- stop if it performs deployment, remote mutation, release publication, or host
  repair;
- stop if it needs private defaults in the repository.

## Version Package: v0.4 Control-Plane Compiler Boundary

`v0.4` should be a refactor-only boundary release unless a separate feature
plan is accepted. Its goal is to make startup, offline config diagnostics,
registry mutation validation, and reload use one behavior-preserving compiler
path while keeping request forwarding on compiled in-memory state.

Accepted module boundaries:

- `ConfigSource`: local YAML read, existing template expansion, unknown
  top-level checks, and `RegistryDocument` construction. It does not resolve
  credentials or read stores.
- `RegistryOverlay`: combines bootstrap control fields with staged registry
  resources using the existing overlay semantics.
- `ConfigCompiler`: takes a `RegistryDocument`, credential repository, and
  credential-store path input, and returns the existing resolved configuration
  projection. The first step is to wrap the current resolver behind one entry
  point, not to change defaults.
- `RuntimeAssembler`: takes resolved configuration and constructs or replaces
  runtime snapshots. Store opening, lifecycle snapshot restore, event replay,
  and runtime health state remain assembler side effects, not compiler logic.
- `Diagnostics`: consumes compiler projections for `check-config` and model
  visibility preview without writing stores, calling upstreams, or duplicating
  route logic.

Verification gates:

- YAML/template parity fixtures prove the new source boundary produces the same
  registry document as the existing loader.
- compiler parity compares redacted structural projections for ids, route
  ordering, policy merge, routing defaults, model groups, response-filter
  resolved fields, endpoint-family visibility, and existing per-channel
  `config_generation`. Runtime registry generation, active/staged registry
  versions, and reload status belong to runtime assembly and reload parity.
- registry overlay parity preserves bootstrap control fields and current staged
  resource replacement semantics.
- `check-config` remains offline: no SQLite writes, no upstream calls, no token
  or key leakage, no absolute key path output.
- `check-config` model visibility preview parity covers disabled client tokens,
  model-group scope, allowed-channel scope, disabled targets, disabled channels,
  missing routes, and normal visible routes against compiled catalog semantics.
- reload keeps staged-version preconditions and updates active runtime only on
  explicit apply.
- runtime assembly parity covers startup versus reload side effects: event
  replay, lifecycle snapshot restore, response-filter event capacity changes,
  routing telemetry capacity changes, management allowlist refresh, active
  registry version update, and no implicit active-runtime reload after registry
  writes.
- hot-path guard tests prove proxy, model catalog, and route planning do not
  import or call YAML parsing, config-file or filesystem I/O, SQLite,
  registry storage, credential storage, client-token storage, env-selected local
  stores, failure windows, or live upstream catalog code.
- public behavior parity proves reason codes, report envelopes, redaction
  boundaries, CLI JSON/table field names, reload status/diff/apply semantics,
  and `check-config` exit codes remain unchanged unless a separate feature plan
  accepts the change.

Stop conditions:

- stop if extraction requires new YAML semantics, registry schema changes, or
  broader store persistence;
- stop if resolved configuration parity cannot be proven with redacted
  structural comparison;
- stop if request forwarding gains any dependency on config files, stores,
  failure windows, or live upstream catalogs;
- stop if `check-config` starts touching SQLite, writing files, or probing
  upstreams.

## Later Candidates

These items are plausible but not part of `v0.3` or `v0.4`.

- `init` templates that generate a runnable local config, placeholder key file,
  and client setup hints.
- provider/relay templates that reduce boilerplate while still requiring
  explicit public model routes.
- model aliases or usage profiles that map local names such as `fast` or
  `coding` to explicit public routes.
- endpoint capability declarations for management diagnosis only.
- a narrow protocol adapter only after a separate replayability, field-mapping,
  and compatibility plan proves it is worth the coupling.
- local usage counters limited to request count, error count, selected public
  model, selected channel, latency bucket, and upstream `usage` fields when
  already returned.

Each candidate needs its own admission record before implementation. The record
must prove user value, negative scope, ownership boundary, configuration
surface, redaction contract, cardinality bounds, hot-path impact, and release
gate.

## Release Gates

Every package in this roadmap must pass:

- targeted tests with non-zero match evidence for the intended paths;
- the repository local CI script or a justified narrower gate for a
  documentation-only change;
- staged denylist and public-plan hygiene checks;
- `git diff --check`;
- verification that staged files exclude build caches, release artifacts, local
  config, mutable data, keys, SQLite databases, logs, and private scripts;
- documentation updates in README, operations, configuration, architecture, or
  release-build docs when behavior changes.

Publishing a release additionally requires the established local Docker/Nix
x86_64 build path, artifact shape verification, release artifact smoke, checksum
sidecar, release notes that describe shipped scope and deferred scope, and a
clean source tree.

## Roadmap Closure

This roadmap is complete when `v0.3` ships the operator maintenance evidence
package and `v0.4` either lands the behavior-preserving compiler boundary or is
explicitly abandoned in favor of a smaller accepted refactor. New ideas must be
classified as blocking defect, next-version candidate, or rejected scope. The
work should stop at the named release gates rather than expanding into
open-ended investigation.
