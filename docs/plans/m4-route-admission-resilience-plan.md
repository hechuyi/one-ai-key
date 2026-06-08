# M4 Route Admission Resilience Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** make route admission resilient to provider/account soft cooldown
without hiding hard local blockers or expanding retry behavior.

**Architecture:** M4 formalizes the existing provider-cooling last-resort
route-planning semantics as a product contract, aligns proxy admission checks
with that contract, and exposes one shared redacted admission summary to
management and CLI explain surfaces. It is not a new retry engine, health-check
system, model catalog, or adaptive router.

**Tech Stack:** Rust, Axum, existing in-memory route state, existing management
projections, clap CLI reports, local mock upstream tests, Docker/Nix x86_64
release smoke.

---

## Review Record

This plan is the result of three review rounds across five angles:

- product boundary and personal-operator UX;
- route admission, channel state, and retry correctness;
- configuration admission and operator command surfaces;
- tests, release smoke, and production-smoke boundary;
- architecture cohesion and module coupling.

The reviewers converged on these decisions:

- M4 is a coherent phase only if it is scoped to route admission resilience,
  not bundled with unrelated diagnostics or deployment work.
- `ProviderCoolingDownLastResort` already exists in the route planner and
  should become a stable contract, not a broad new fallback system.
- Hard blockers remain hard: missing scope/model/config, disabled targets or
  channels, hard channel cooldown, empty credential pools, runtime unavailable,
  unknown channel, and candidate-limit exclusion do not become candidates.
- M4 must not expand M3 retry. Admission last resort is a first upstream
  attempt from the compiled route plan, not a retry and not an adaptive probe.
- No new YAML field is admitted for M4. Future configuration switches require a
  separate admission record.
- CLI must render management projections and must not duplicate route
  admission logic.

## User Contract

When a request has passed the hard local gates for model, client scope, route,
channel, and credential availability, the router should not return local
`503 no_route_candidate` solely because every otherwise-valid route target is in
provider/account soft cooldown. In that narrow case, it may admit one
provider/account soft-cooling target as the last resort from the already
compiled route plan.

When a hard gate fails, the router must still fail closed before touching the
upstream. The operator must be able to see why through redacted management and
CLI explain output.

M4 does not promise that one-ai-key is always more successful than direct
upstream access. It promises that the router does not introduce an avoidable
local admission failure in the soft-cooldown case, and that all remaining
admission failures are classified by stable reason codes.

## Non-Goals

M4 must not add:

- streaming retry or partial-output fallback;
- endpoint-family fallback or Responses-to-Chat conversion;
- live upstream `/v1/models` aggregation;
- active health checks, background probes, or scheduled key scans;
- adaptive routing, historical ledgers, breaker scoring, or usage analytics;
- automatic key repair, route mutation, scope mutation, reload, or discover/apply;
- dashboard/UI work;
- YAML presets, global resilience switches, or hidden config defaults;
- production SSH/systemd deployment in release tests.

## Route Admission Contract

The candidate priority contract is:

```text
available > degraded > provider/account soft-cooling
```

Provider/account soft-cooling is a soft suppression state. If better candidates
exist in the frozen compiled route plan, the soft-cooling target is skipped. If
all otherwise-valid candidates are soft-cooling, one may be selected as a
last-resort first attempt.

Hard blockers are never admitted as last resort:

- missing public route or default-channel route;
- client model scope denial;
- client channel scope denial;
- target disabled;
- channel disabled;
- hard channel cooldown;
- no available credentials;
- runtime unavailable or lock-contention route state;
- unknown channel;
- candidate limit exclusion.

The proxy's secondary route-state gate must validate the frozen selected target
without contradicting the route planner:

- a selected `provider_cooling_down_last_resort` target may be attempted when
  it is the legitimate last resort from the frozen plan;
- if a provider-cooling target has a later available/degraded frozen target, it
  may be skipped to that later target;
- hard states still fail closed or fall through only to another frozen target;
- the proxy must not re-plan the route, read stores, or synthesize a target.

## Configuration Admission

M4 adds no public YAML or registry field.

Rationale: the project already has explicit routing profiles, error
classification, failure scope, route planning, channel state, and telemetry
capacity configuration. M4 clarifies and verifies existing soft-state semantics;
it does not introduce a new operator-selectable routing strategy.

If a future version proposes a switch such as soft-cooldown last-resort
enablement, it must be evaluated separately with:

- exact runtime owner;
- default value;
- conflict behavior;
- compiled projection;
- hot-path proof;
- redaction contract;
- tests for enabled and disabled behavior.

## Architecture Boundary

Prefer the smallest boundary that prevents duplicated logic.

`src/route_plan.rs` remains the owner of candidate ordering, inclusion, and
candidate reasons. It may grow pure helper functions or small DTOs for
admission classification if they remain independent of HTTP, CLI, YAML, SQLite,
credential storage, management routing, and upstream I/O.

Introduce `src/route_admission.rs` only if the shared admission result and
summary would make `route_plan.rs` lose focus. If introduced, it must remain a
thin pure module:

```rust
RouteAdmissionInput -> RouteAdmissionResult
RoutePreview -> AdmissionBlockerSummary
RoutePreviewReason -> hard / soft / last_resort classification
```

It must not become a second route planner.

`src/proxy.rs` consumes the route/admission result and maps local denials to
client errors. It does not own blocker taxonomy, retry policy, or route
replanning.

`src/routing.rs` remains the selected-target failure transition and M3 retry
gate owner. M4 must not change its retry surface except for regression tests
that prove the surface did not expand.

`src/management_routing.rs` serializes a stable redacted
`route_admission_summary`. It may combine candidate statuses with the shared
summary but must not reimplement admission rules.

`src/cli_commands/*` renders management projections. CLI commands may select
safe next-action wording from stable reason codes, but they must not walk
candidates and recompute admission.

## Management Projection Contract

`/management/routing/preview` should expose a stable summary alongside existing
candidate details. The exact field names may be refined during implementation,
but the projection must answer:

- was a target admitted;
- what primary reason explains denial or last-resort admission;
- which hard reason codes appeared;
- which soft suppression reason codes appeared;
- how many candidates were present, included, hard-blocked, and soft-suppressed;
- whether a provider/account soft-cooling target was selected as last resort;
- which safe next action applies.

Representative shape:

```json
{
  "route_admission_summary": {
    "status": "admitted",
    "primary_reason": "provider_cooling_down_last_resort",
    "hard_reasons": [],
    "soft_suppression_reasons": ["provider_cooling_down"],
    "candidate_count": 1,
    "included_count": 1,
    "hard_blocked_count": 0,
    "soft_suppressed_count": 0,
    "last_resort_selected": true,
    "next_action": "none"
  }
}
```

The projection must not include raw keys, tokens, token hashes, request or
response bodies, full URLs, local key paths, upstream free-form error text, or
private deployment values.

## CLI Scope

P0:

- `route explain` renders `route_admission_summary` and candidate reason
  distribution.
- `models explain --endpoint-family ...` consumes the same projection or a
  management-side equivalent to distinguish route admission blockers without
  duplicating route logic.

P1:

- `failures explain` may correlate a recent client-visible
  `no_route_candidate` event with bounded failure evidence and the current route
  admission summary. It must not block the data-plane fix.

Do not add a dashboard, TUI, persistent report store, or analytics view.

## Production Smoke Boundary

M4 may add a parameterized operator-run production smoke harness, but it is not
part of `local-ci` and must not be required for source-only closure.

Allowed shape:

```bash
ONE_AI_KEY_PUBLIC_BASE_URL=... \
ONE_AI_KEY_CLIENT_TOKEN_ENV=... \
ONE_AI_KEY_MANAGEMENT_URL=... \
ONE_AI_KEY_MANAGEMENT_TOKEN_ENV=... \
ONE_AI_KEY_PUBLIC_MODEL=... \
scripts/production-smoke.sh --allow-production
```

The script must:

- default to refusal unless explicitly enabled;
- read token values indirectly through named environment variables;
- avoid real defaults for domains, hosts, models, tokens, or paths;
- print redacted JSON only;
- avoid writing output inside the repository;
- avoid SSH, systemd, NixOS rebuilds, or deployment mutation.

Docs should describe the same workflow as an operator checklist. The stop card
must record `production_smoke_result` as `pass`, `not_run_by_design`, or
`blocked`; private production access must not become a repository release
dependency.

## Implementation Tasks

### Task 1: Characterize Current Admission Behavior

**Files:**

- Test: `src/route_plan.rs`
- Test: `src/main.rs`
- Modify only if needed for test helpers.

- [ ] Add targeted tests proving current candidate ordering:
  `available > degraded > provider-cooling`.
- [ ] Add tests proving all-provider-cooling otherwise-valid routes are
  included as last resort.
- [ ] Add tests proving hard blockers are excluded and produce stable reasons.
- [ ] Add runtime tests for proxy secondary gate behavior:
  provider-cooling last resort is attempted when selected by the route plan,
  but skipped when a better frozen target remains.
- [ ] Run the targeted tests and confirm each filter matches non-zero tests.
- [ ] Commit characterization tests.

### Task 2: Shared Admission Summary

**Files:**

- Modify: `src/route_plan.rs`
- Optional Create: `src/route_admission.rs`
- Modify: `src/main.rs` module declarations only if a new module is justified.

- [ ] Add pure reason classification helpers for hard blockers, soft
  suppression, and last-resort markers.
- [ ] Add `AdmissionBlockerSummary` or equivalent stable DTO.
- [ ] Generate the summary from the same route preview used by `plan_route`.
- [ ] Keep the summary independent of HTTP, CLI, management serialization,
  YAML, SQLite, credential stores, and upstream I/O.
- [ ] Add unit tests for classification, counts, primary reason selection, and
  redaction-safe reason strings.
- [ ] Commit the shared admission summary.

### Task 3: Proxy Admission Alignment

**Files:**

- Modify: `src/proxy.rs`
- Test: `src/main.rs`

- [ ] Replace duplicated local blocker reasoning in proxy with the shared
  admission result or shared summary where feasible.
- [ ] Audit `route_state_unavailable_response_for_attempt` so it does not
  reject a legitimate provider-cooling last-resort selected by the route plan.
- [ ] Preserve frozen fallback behavior when a later target remains.
- [ ] Prove hard blockers cause zero upstream hits when no frozen target can be
  used.
- [ ] Prove M3 attempt budget does not increase.
- [ ] Commit proxy alignment.

### Task 4: Management Projection

**Files:**

- Modify: `src/management_routing.rs`
- Test: `src/main.rs`

- [ ] Add `route_admission_summary` to routing preview responses.
- [ ] Ensure the response uses stable reason codes and bounded counts.
- [ ] Ensure unsafe route labels, token-like values, paths, URLs, and raw
  upstream text are redacted or absent.
- [ ] Add tests for admitted, provider-cooling last-resort, hard-blocked, and
  mixed candidate summaries.
- [ ] Commit management projection changes.

### Task 5: CLI Rendering

**Files:**

- Modify: `src/cli_commands/route.rs`
- Modify: `src/cli_commands/models.rs`
- Optional Modify: `src/cli_commands/failures.rs`
- Test relevant CLI command tests.

- [ ] Render `route_admission_summary` in `route explain` table and JSON
  outputs.
- [ ] Have `models explain` consume management-provided summary or equivalent
  management-side reason codes; do not duplicate route admission logic.
- [ ] Keep `failures explain` P1: add only if the existing bounded event window
  can reference admission failures without broadening scope.
- [ ] Add tests proving CLI output is redacted and does not compute admission
  locally.
- [ ] Commit CLI rendering changes.

### Task 6: Release Smoke And Operator Smoke Boundary

**Files:**

- Modify: `scripts/release-smoke.sh`
- Optional Create: `scripts/production-smoke.sh`
- Modify: `docs/operations.md`
- Modify: `docs/release-build.md`
- Test: `tests/local_release_contract.rs`

- [ ] Add one local mock release-smoke path for provider/account soft-cooling
  last-resort success.
- [ ] Add one local mock negative sanity path for a representative hard blocker
  returning local `no_route_candidate` with zero upstream hits.
- [ ] Do not expand release smoke into the full admission matrix.
- [ ] If adding `scripts/production-smoke.sh`, require explicit
  `--allow-production`, parameterize all values, and reject missing env/token
  references without printing secrets.
- [ ] Document production smoke as an operator-run deployment check, not a CI or
  source release gate.
- [ ] Update local release contract tests for the new script boundaries and
  denylist expectations.
- [ ] Commit release and operator smoke changes.

### Task 7: Documentation And Stop Card

**Files:**

- Modify: `README.md`
- Modify: `docs/operations.md`
- Modify: `docs/architecture.md`
- Modify: `docs/technical-design.md`
- Create: `docs/plans/m4-route-admission-resilience-stop-card.md`

- [ ] Document `no_route_candidate` in user terms: hard blockers versus
  provider/account soft cooldown last resort.
- [ ] Document that M4 does not expand retry, streaming behavior, endpoint
  fallback, or model discovery.
- [ ] Record the configuration admission decision: no new YAML in M4.
- [ ] Create the stop card with the fields below.
- [ ] Commit documentation.

## Test Matrix

Required targeted tests:

- route admission taxonomy table;
- hard blockers excluded with stable reasons;
- hard blocker zero-upstream runtime behavior;
- available/degraded/provider-cooling ordering;
- all-soft provider/account cooling last-resort admission;
- frozen fallback skips provider-cooling when a better later target exists;
- proxy secondary gate regression;
- M3 retry non-expansion for streaming, partial output, embeddings,
  `/v1/models`, named pool, unknown endpoint, and deadline exhaustion;
- management projection schema and redaction;
- CLI rendering redaction and no local admission recomputation;
- release smoke extracted-artifact soft last-resort and one hard blocker sanity.

Verification commands must include non-zero filtered test evidence. A filtered
test command that reports `0 tests` is not evidence.

## Release Gate

Before closing M4:

- run targeted tests and record non-zero match counts;
- run `scripts/local-ci.sh`;
- run `git diff --check`;
- run `scripts/check-staged-denylist.sh`;
- build with `scripts/build-release-x86_64-linux-docker.sh`;
- run `scripts/release-smoke.sh` against the extracted artifact;
- verify release archive checksum after upload if publishing;
- ensure staged files exclude `dist/`, `target/`, `key-pool-router/`, `config/`,
  `data/`, SQLite, logs, keys, raw production fixtures, private scripts, and
  `AGENTS.md`.

## Stop Card Fields

The M4 stop card must include:

- `plan_id`;
- `release_or_scope_name`;
- `closed_capability`;
- `implemented_scope`;
- `deferred_scope`;
- `parked_or_rejected_items`;
- `route_admission_taxonomy_result`;
- `soft_cooling_last_resort_result`;
- `hard_blocker_zero_upstream_result`;
- `proxy_secondary_gate_result`;
- `m3_retry_non_expansion_result`;
- `management_projection_result`;
- `cli_rendering_result`;
- `operator_contract_evidence`;
- `test_evidence`;
- `non_empty_filtered_test_evidence`;
- `local_ci_result`;
- `release_artifact_result`;
- `release_smoke_result`;
- `production_smoke_result`;
- `redaction_and_denylist_result`;
- `anti_platform_gate_result`;
- `support_residue_scan_result`;
- `deployment_boundary_result`;
- `known_blockers`;
- `next_version_candidates`.

`production_smoke_result` may be `pass`, `not_run_by_design`, or `blocked`.

## Stop Or Block Conditions

Stop and re-plan if implementation requires any of:

- changing `src/routing.rs` to route admission failures through M3 retry;
- request-path reads from YAML, SQLite, registry storage, credential storage,
  management APIs, failure history, or live upstream catalogs;
- live probes, scheduled health scans, or persistent adaptive state;
- broad YAML switches or presets;
- hard blockers admitted as last resort;
- CLI commands reimplementing admission classification;
- production-specific domains, keys, hostnames, deployment paths, or incident
  transcripts entering the repository;
- release smoke depending on private upstreams or real keys.
