# M4 Runtime Resilience Implementation Plan

**Goal:** make the runtime less fragile than direct upstream use by closing the
local route-admission gap, auditing hard/soft state transitions, and giving
operators bounded evidence for every local `no_route_candidate`.

**Architecture:** M4 is a runtime resilience package with four coupled slices:
route admission semantics, state-transition hard/soft classification, shared
redacted admission evidence, and release/operator smoke. It formalizes the
existing provider-cooling last-resort route-planning semantics, aligns proxy
admission checks with that contract, audits state mutations that create hard
versus soft route states, and exposes one shared admission summary to management
and CLI explain surfaces. It is not a new retry engine, health-check system,
model catalog, or adaptive router.

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

- M4 is a coherent phase only if it is scoped to runtime resilience around
  admission, state classification, bounded evidence, and smoke verification,
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

## Capability Package

M4 is intentionally larger than a single `no_route_candidate` bugfix, but it
still has one product thesis:

```text
If the upstream would be reachable through an otherwise-valid configured route,
one-ai-key should not fail locally just because its own soft runtime state is too
conservative; if it does fail locally, the operator must see the exact hard
reason.
```

The phase closes four surfaces together:

1. **Admission behavior:** provider/account soft cooldown can be selected as a
   last-resort first attempt when every otherwise-valid candidate is soft
   suppressed.
2. **State-transition classification:** the failures that create hard channel
   cooldown, provider/account soft cooldown, degraded state, credential
   cooldown, quota exhaustion, and no-op transitions are audited and locked by
   tests.
3. **Operator evidence:** `route explain`, `models explain`, and bounded failure
   evidence share one admission taxonomy instead of each inventing its own
   explanation.
4. **Release/operator proof:** local release smoke proves the extracted artifact
   handles one soft last-resort path and one hard fail-closed path; production
   smoke is an opt-in operator harness, not a CI dependency.

This is enough for a stage release because it changes runtime behavior, locks
state semantics, adds explainability, and verifies the published artifact
without expanding the project into a platform.

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

## State Transition Contract

M4 must audit the state mutations that feed route admission. The admission layer
can only be correct if hard and soft route states are assigned consistently.

Hard route states:

- manual/config channel disabled;
- response-filter channel rejection that explicitly cools or disables a channel;
- relay-balance or account-balance exhaustion when configured as channel scope;
- no available credentials after credential lifecycle filtering;
- runtime unavailable or lock contention.

Soft route states:

- provider/account unavailable from transient upstream evidence;
- provider/account retry-after suppression when a later or last-resort route may
  still be valid;
- degraded provider/account state after transport or pre-output provider
  instability that did not prove channel balance exhaustion.

Credential-scoped states:

- credential cooldown;
- credential expired/auth failure;
- credential quota exhausted when balance scope is credential.

Request-scoped states:

- schema errors;
- endpoint-family mismatch;
- model/scope/config errors;
- upstream 4xx that does not prove credential or channel lifecycle failure.

M4 must not add automatic repair. It must ensure existing transitions are
classified, tested, and projected consistently. If tests reveal a relay error is
being mapped to an overly hard state, the fix belongs in the classifier or
state-transition mapping, not in a broad fallback rule.

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

## Failure Evidence Contract

M4 may add or refine bounded admission evidence, but it must remain an
explanation surface and never become a routing input.

Allowed fields are stable and small:

- request id;
- endpoint family;
- public model label after existing safe-label redaction;
- route kind;
- registry generation;
- admission status;
- primary reason;
- hard reason codes;
- soft suppression reason codes;
- selected last-resort marker when present;
- candidate count and included count;
- client-visible status for local denial.

Do not record raw request bodies, response bodies, upstream messages, key
material, token hashes, complete URLs, file paths, or private deployment values.
Overflow behavior remains bounded in-memory telemetry with dropped counters.

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

## Delivery Requirements

M4 closes only when the code and docs establish these product-level contracts:

- route planning classifies candidates through one shared admission taxonomy;
- candidate priority remains `available > degraded > provider/account soft-cooling`;
- all-soft provider/account cooling can be selected only as a last-resort first
  attempt from the frozen compiled route plan;
- hard blockers remain fail-closed and produce stable redacted reasons without
  touching upstreams;
- proxy secondary admission uses the same route-plan semantics rather than a
  second local taxonomy;
- state-transition tests distinguish hard channel state, soft provider/account
  state, credential-scoped state, and request-scoped no-op failures;
- management routing preview exposes a bounded `route_admission_summary`;
- `route explain`, `models explain`, and bounded failure evidence render backend
  projections without recomputing route admission locally;
- local release smoke covers one soft last-resort success path and one hard
  local `no_route_candidate` path;
- public docs explain `no_route_candidate`, hard/soft state, credential state,
  and M4 non-goals without private deployment evidence or internal execution
  records.

## Test Matrix

Required targeted tests:

- route admission taxonomy table;
- state-transition hard/soft taxonomy table;
- hard blockers excluded with stable reasons;
- hard blocker zero-upstream runtime behavior;
- available/degraded/provider-cooling ordering;
- all-soft provider/account cooling last-resort admission;
- frozen fallback skips provider-cooling when a better later target exists;
- proxy secondary gate regression;
- M3 retry non-expansion for streaming, partial output, embeddings,
  `/v1/models`, named pool, unknown endpoint, and deadline exhaustion;
- management projection schema and redaction;
- bounded admission/failure evidence schema and redaction;
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

## Release Record

The M4 release record should summarize the shipped route-admission capability,
deferred scope, parked or rejected items, verification categories, known
blockers, and next-version candidates. It should avoid private deployment
state, raw command logs, artifact hashes used only for a past upload, local
absolute paths, secret material, and internal execution checklists.

Production smoke remains an operator-run deployment check, not source-release
evidence. A public release record may state that boundary, but it must not imply
production was updated unless separately documented with redacted operator
evidence.

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
