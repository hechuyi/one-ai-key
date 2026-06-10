# Read-Only Operator Explainability Closure Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** make recent client-visible failures explainable with existing
read-only operator commands, especially local `no_route_candidate` / `503`
admission failures versus selected-upstream `502` / `503` failures.

**Architecture:** this is an explainability closure, not a new control plane.
P0 may add bounded telemetry and management/CLI projections over existing
read-only surfaces. It must not change route selection, retry, credential
selection, config/schema, or runtime mutation behavior.

**Tech Stack:** Rust, Axum, clap CLI, existing management APIs, bounded in-memory
routing telemetry and response-filter event rings, local mock upstreams,
release-artifact smoke tests.

---

## Review Record

Five subagent review angles iterated through architecture, product value,
operations/release, credential UX, and anti-scope veto. The final consensus:

- next implementation should be P0 only: **Read-Only Operator Explainability
  Closure**;
- P0 must include bounded `route_admission_denied` evidence, because current
  failure evidence cannot fully explain local `no_route_candidate` / `503`;
- P0 must distinguish local `503` from selected-upstream `502` / `503`;
- P0 must not add a generic `status`, `diagnose`, history, event, or health
  product;
- P1 **Operator Smoke Checklist** is a separate follow-up plan and release
  boundary, not part of P0 product scope;
- P2 **Single-Credential Keys Enable CLI Closure** is a separate mutating
  maintenance plan and must not be folded into P0.

## Candidate Triage

| Candidate | Decision | Reason |
| --- | --- | --- |
| Configuration and runtime explainability | Keep inside P0 only as existing report consistency. | Operators need config/runtime/reload drift context, but not a second config system. |
| Recent failure evidence | Keep inside P0. | This is the direct answer to frequent client-visible `502` / `503` / `no_route_candidate`. |
| Endpoint-family diagnostics | Keep inside P0 only as diagnostics. | Endpoint family belongs in management/CLI explain, not `/v1/models` or protocol conversion. |
| Operator smoke checklist | Defer to P1. | Useful as an operator/release gate, but not a new P0 product surface. |
| Keys enable / refs filtering | Defer to P2. | Mutating credential maintenance has a different risk boundary. |

## P0 User Contract

After P0, an operator can use existing read-only commands to answer:

```text
Was this recent client-visible failure caused by local route admission,
config/runtime/reload drift, endpoint-family mismatch, credential unavailability,
response filtering, or selected-upstream 502/503?
```

The answer is bounded recent evidence, not a live health verdict. Current
availability remains the responsibility of `models explain`, `route explain`,
`doctor`, and reload projections. `failures tail` and `failures explain` must
continue to state `availability_source=bounded_evidence` and
`current_availability=false`.

## P0 Non-Goals

P0 must not add:

- new command namespaces such as `diagnose`, `status`, `events`, `telemetry`,
  `audit`, `ledger`, `incident`, `timeline`, `bundle`, `support-bundle`, or
  `report all`;
- new failure commands such as `failures history`, `failures export`,
  `failures top`, `failures trend`, `failures stats`, `failures search`,
  `failures query`, `failures since`, `failures follow`, or `failures watch`;
- new management endpoints such as `/management/diagnose`,
  `/management/operator/status`, `/management/telemetry/*`,
  `/management/events/*`, `/management/audit/*`,
  `/management/failures/history`, `/management/failures/export`, or
  `/management/routes/admission-denials`;
- route/provider/channel health products such as `routes health`,
  `routes stats`, `admission stats`, `providers status`, `providers health`,
  `channels health`, or `channels scan`;
- auto-fix or mutation language such as `repair`, `fix`, `auto-fix`,
  `auto-repair`, `sync`, `probe-all`, `scan`, `rotate`, `auto-rotate`,
  `promote`, or `apply-suggested`;
- persistent failure ledger, usage ledger, billing/cost/top views, TUI,
  dashboard, background health checks, continuous probes, model aliases,
  protocol adapters, endpoint fallback, Responses-to-Chat conversion, provider
  catalog aggregation, or live upstream `/v1/models` fan-out.

Existing specific-domain commands such as `reload status` remain allowed.

## Hard Bounds

P0 uses the existing two failure sources only:

```text
routing_telemetry
response_filter_events
```

It must not add a third failure source.

Numeric bounds:

```text
schema_version: 1
failures default --last: 50
failures max --last: 200
failure window source count for P0: 2
combined failure report max returned/window.limit: 400

route_admission_denied.upstream_status: always null
upstream_failure_observed.upstream_status: integer HTTP status or null only

candidate_reason_codes max: 8
recent_failure_hint.reason_codes max: 8
recent_failure_hint.channel_ids max: 8
status/count buckets max: 8

safe_argv max args: 16
safe local label max length: 128 bytes
next_action.summary max length: 240 bytes
endpoint diagnostic label max length: 64 bytes
single projected telemetry event serialized hard cap: 2048 bytes
```

Unsafe values render as `null` or a redacted enum. Reports must never contain
raw tokens, upstream keys, token hashes, credential fingerprints, key prefixes
or suffixes, raw request or response bodies, upstream error text, headers,
complete URLs with token-like components, absolute secret paths, private
deployment URLs, SSH targets, or NixOS host details.

## Module Boundaries

P0 may touch:

- `src/events.rs` for a closed `RoutingTelemetry::RouteAdmissionDenied` schema;
- `src/proxy.rs` only at final admission-denial return points, after the router
  has already decided to return a local error;
- `src/management_runtime.rs` for redacted projection and failure event
  shaping;
- `src/cli_commands/failures.rs` for bounded aggregate rendering inside
  existing `failures tail` and `failures explain`;
- existing tests in `src/main.rs`, `src/events.rs`, `src/management_runtime.rs`,
  `src/cli_commands/failures.rs`, and `tests/local_release_contract.rs`;
- docs and stop-card files under `docs/`.

P0 must not touch behavior in:

- `src/route_plan.rs` route decision semantics;
- `src/routing.rs` retry behavior or state transitions;
- `src/pool.rs` credential selection;
- YAML config schema, registry schema, SQLite schema, or credential store
  schema;
- client-facing `/v1/models` semantics.

If an implementation needs to change any forbidden surface, stop and write a new
plan.

## P0 Tasks

### Task 1: Closed Route Admission Denial Event

**Files:**

- Modify: `src/events.rs`
- Test: `src/events.rs`

- [ ] Add a closed `RoutingTelemetry::RouteAdmissionDenied` event.
- [ ] Include only whitelisted fields: request id, registry generation,
      endpoint family, public model, client-token reference, route kind, stable
      reason code, blocking domain, local client-visible status, bounded
      admission counts, and optional runtime generation/version drift fields.
- [ ] Force `upstream_status` to `null` for this event kind.
- [ ] Keep the projected serialized event within the 2048 byte hard cap.
- [ ] Add tests proving unsafe labels are redacted or omitted.

### Task 2: Best-Effort Admission Denial Recording

**Files:**

- Modify: `src/proxy.rs`
- Test: `src/main.rs`

- [ ] Record `RouteAdmissionDenied` only after admission has already failed and
      the proxy is about to return a local `no_route_candidate` / `503` style
      response.
- [ ] Use the existing bounded routing telemetry ring.
- [ ] Do not block the request on telemetry lock contention or overflow.
- [ ] Prove route planning, retry, pool selection, and state transition behavior
      do not read this telemetry.
- [ ] Add a local test where route admission fails, upstream hit count remains
      zero, and bounded telemetry contains the denial evidence.

### Task 3: Failure Projection And Status Distinction

**Files:**

- Modify: `src/management_runtime.rs`
- Test: `src/management_runtime.rs`

- [ ] Project `route_admission_denied` into management failure evidence without
      mixing it into retry pressure.
- [ ] Preserve exact upstream HTTP status for upstream failures as
      `upstream_status`, integer or `null` only.
- [ ] Distinguish local `client_visible_status=503` with
      `upstream_status=null` from selected-upstream `upstream_status=502` or
      `upstream_status=503`.
- [ ] Keep `failure_events` field-whitelisted and redacted.
- [ ] Do not expose upstream bodies, error messages, URLs, headers, token-like
      strings, credential ids, or fingerprints.

### Task 4: Existing Failure CLI Aggregate

**Files:**

- Modify: `src/cli_commands/failures.rs`
- Test: `src/cli_commands/failures.rs`

- [ ] Extend existing `failures tail` and `failures explain` reports with a
      small bounded aggregate block.
- [ ] Do not add a new `failures summary`, `status`, `diagnose`, `events`, or
      telemetry command.
- [ ] Aggregate only within the current bounded window by local/upstream status,
      reason code, stage, public model, channel reference, retry directive, and
      retry denial reason.
- [ ] Keep each bucket list capped at 8.
- [ ] Preserve existing window contract: default 50, max 200 per source,
      combined max 400, `availability_source=bounded_evidence`, and
      `current_availability=false`.
- [ ] Ensure diagnostic next actions remain read-only and never suggest
      `keys import`, `keys probe`, `keys enable`, `reload apply`, model apply, or
      `curl`.

### Task 5: Explain Report Consistency

**Files:**

- Modify only if needed: `src/management_routing.rs`,
  `src/cli_commands/models.rs`, `src/cli_commands/route.rs`,
  `src/cli_commands/doctor.rs`
- Test: targeted command/projection tests and `tests/local_release_contract.rs`

- [ ] Keep endpoint-family evidence diagnostic-only.
- [ ] Keep `/v1/models` family-blind and local-catalog only.
- [ ] Ensure `models explain`, `route explain`, `failures tail/explain`, and
      `doctor` use consistent reason codes, blocking domains, reload drift
      labels, and read-only next actions.
- [ ] Do not introduce config/runtime "explain all" commands.
- [ ] Do not recompute routing logic in CLI modules.

### Task 6: P0 Release Smoke Additions

**Files:**

- Modify: `scripts/release-smoke.sh`
- Test: `tests/local_release_contract.rs`

- [ ] Use the release artifact binary extracted from `dist/`; do not use
      `cargo run` or a source checkout binary.
- [ ] Add one local mock case for route admission denial with zero upstream hits.
- [ ] Add one local mock case for selected-upstream `503` with
      `upstream_status == 503`.
- [ ] Assert `route_admission_denied.upstream_status == null`.
- [ ] Assert `window.kind`, `limit`, `returned`, `truncated`,
      `availability_source`, and `current_availability`.
- [ ] Capture reports through an explicit enumerated list only; no glob capture.
- [ ] Run the leak scanner over every captured report.
- [ ] Verify all next actions are read-only.

### Task 7: Documentation And Stop Card

**Files:**

- Modify: `README.md` only if first-use behavior changes
- Modify: `docs/operations.md`
- Modify: `docs/technical-design.md`
- Add: `docs/plans/read-only-operator-explainability-closure-stop-card.md`

- [ ] Document that bounded recent failure evidence explains recent events, not
      current availability.
- [ ] Document that `keys restore` does not enable manually disabled
      credentials.
- [ ] Document that disabled credentials remain out of rotation until an
      explicit future `keys enable` workflow exists.
- [ ] Record explicit deferred scope: operator smoke checklist and
      single-credential keys enable.
- [ ] Close the stop card with implemented scope, deferred scope, rejected
      items, test evidence, release-smoke evidence, redaction evidence, and
      known blockers.

## P0 Acceptance Gates

- Local `no_route_candidate` / `503` produces bounded redacted admission-denied
  evidence and zero upstream hits.
- Selected-upstream `502` / `503` is distinguishable from local `503`.
- `route_admission_denied` uses `upstream_status=null`; upstream failures use
  integer `upstream_status` or `null`.
- Failure evidence remains bounded to the two existing in-memory sources.
- `models explain` and `route explain` remain the current-availability sources.
- `failures tail/explain` continue to identify themselves as bounded evidence,
  not current availability.
- No request-path decision reads telemetry or failure windows.
- No routing, retry, pool, config/schema, registry/schema, SQLite/schema, or
  `/v1/models` behavior changes.
- No persistent ledger, background probe, adaptive routing, automatic repair,
  generic status/diagnose/event/history command, or mutation next action is
  added.
- `scripts/local-ci.sh`, `cargo test --locked --test local_release_contract`,
  `scripts/release-smoke.sh`, `git diff --check`, and
  `scripts/check-staged-denylist.sh` pass before commit.
- If publishing a release, run
  `scripts/build-release-x86_64-linux-docker.sh` and release-smoke against the
  extracted artifact.

## P1: Operator Smoke Checklist

P1 is a separate plan. It may document or implement an operator-run checklist
for a real deployment, but it must not be part of CI or local release smoke
against private infrastructure.

P1 boundaries:

- no private default URL, token, model, SSH host, NixOS path, or systemd unit;
- tokens only through environment variable names or explicit secret references;
- output is redacted JSON with `pass`, `blocked`, or `not_run_by_design`;
- no SSH, `systemctl`, Nix rebuild, remote mutation, key rotation, or repository
  writes;
- production execution records stay private and never enter public fixtures,
  commits, release notes, or stop cards.

## P2: Single-Credential Keys Enable CLI Closure

P2 is a separate mutating maintenance plan.

Allowed future scope:

```text
one-ai-key keys enable --credential-set <id> --credential-ref <ref> \
  --reason <reason> --dry-run

one-ai-key keys enable --credential-set <id> --credential-ref <ref> \
  --reason <reason> --yes
```

P2 must use the management mutation allowlist, side-effect classification,
redacted report envelope, explicit `credential_ref`, and dry-run/confirmation
model already used by credential lifecycle commands.

P2 must not add bulk enable, auto enable, probe-triggered enable, inferred
targets from stats, background scans, automatic repair, selector promotion,
`append-first`, `prefer`, or any raw key positional argument.

## Global Stop Conditions

Stop and re-plan immediately if implementation requires:

- a third failure source;
- persistent failure storage;
- time-range queries, cross-window inference, export bundles, streaming
  telemetry, or support packages;
- route decisions based on telemetry or failure evidence;
- new config/schema/store fields;
- protocol conversion or endpoint fallback;
- background probe or health loop;
- mutating key/model/reload next actions from diagnosis;
- private deployment material as test, doc, release, or commit evidence.
