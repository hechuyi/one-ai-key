# Operator Confidence Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development or superpowers:executing-plans to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** ship `v0.2` as an operator-confidence release: existing M1-M4
operator capabilities become reproducible, explainable, redacted, and
release-smoked from the published artifact.

**Architecture:** `v0.2` is a closure release, not a new data-plane feature
release. It validates the existing CLI/management/reporting contracts against a
published artifact and documents the supported operator path. It may perform
one optional `src/main.rs` boundary extraction only after the release-confidence
gate is closed and only if that extraction does not delay the release.
Request-path stability work starts in `v0.3` under a separate conservative
pre-output stability plan.

**Tech Stack:** Rust, Axum, clap, serde/serde_json, local mock upstream tests,
local Docker/Nix Linux x86_64 release build, shell smoke scripts, GitHub release
artifacts.

---

## Review Inputs

This plan assumes the completed M1-M4 operator UX baseline described in
`docs/plans/operator-ux-implementation-plan.md` and the narrower product
roadmap in `docs/plans/product-improvement-roadmap.md`.

The review rounds converged on these concrete invariants:

- keep client model to deployment/channel selection as an explicit compiled
  route index;
- keep channel health, credential lifecycle, and route policy as separate
  runtime domains;
- keep import, probe, discovery, reload, and explanation on the management
  plane;
- keep request forwarding on already compiled in-memory state.

The project must not absorb the platform parts of larger gateways: hosted
multi-tenancy, billing, UI workflows, live provider catalog fan-out, adaptive
background health clusters, broad protocol conversion, or ledger-driven
routing.

This plan also must not preserve support-thread residue. Do not record private
deployment names, real gateway domains, temporary key replacement steps,
incident chronology, chat excerpts, one-off workaround commands, or operator
identity. Convert recurring lessons into stable product rules, smoke checks, or
operations checklist items; otherwise remove them.

## Product Contract

The user-visible `v0.2` promise is:

```text
one-ai-key can be installed from a pinned release artifact, checked with local
mock data, and operated through redacted CLI/management reports that explain
configuration, runtime visibility, key state, reload state, and recent bounded
failure evidence without leaking secrets or touching unsupported automation.
```

The release is complete only when a cold operator can answer these questions
from docs and release-smoke output:

- Is the release artifact runnable without a source checkout?
- Does `check-config` agree with authenticated `/v1/models` for the generated
  local config?
- Do read-only management commands return the unified redacted report envelope?
- Do mutating commands advertise dry-run/confirmation/effect semantics?
- Is a client `/v1` URL rejected when used as a management URL?
- Are release artifacts, build caches, runtime state, configs, keys, databases,
  logs, `AGENTS.md`, and private scripts excluded from Git staging?

## Operator Decision Contract

Every `v0.2` smoke, report test, and stop-card record must preserve this triad:

```text
resource + client-token-ref + endpoint/model context -> can_use / reason_code / next_action
```

`can_use` is not inferred from process liveness. It is established only by
matching offline visibility, authenticated `/v1/models`, model/route
explanation, credential-set state, reload state, and bounded recent failure
evidence.

A report passes the operator-confidence gate only when a personal operator can
answer:

- can this configured client token see and call this public model now;
- if not, whether the blocker is config, client-token scope, model route,
  endpoint family, credential availability, runtime reload drift, management
  auth, or upstream/runtime failure evidence;
- which next command is safe, whether it is read-only, dry-run,
  upstream-touching, or mutating, and whether confirmation is required.

## Version Boundary

`v0.2` closes the operator-confidence surface. It must not be used as a bucket
for adjacent feature requests.

`v0.2` may verify and characterize stability behavior that already exists in the
current codebase, but it must not broaden that behavior. Any gap found while
characterizing upstream jitter becomes a `v0.3_blocker` or post-release debt,
not a `v0.2` production-path patch.

Allowed in `v0.2`:

- release artifact smoke with local mock upstreams;
- fixed representative CLI/management command matrix;
- report-envelope, side-effect, next-action, redaction, and bounded-window
  contract tests;
- offline config diagnostic characterization and, only if needed, local
  organization inside `src/config_diagnostics.rs` without moving
  resolver/compiler ownership or changing accepted config semantics;
- characterization tests for already implemented conservative pre-output
  behavior;
- one optional narrow `src/main.rs` boundary extraction with zero behavior
  change after the release-confidence gate is closed;
- documentation that describes stable product usage, not support-chat history.

Not allowed in `v0.2`:

- new retry or fallback behavior;
- new request-path protocol behavior;
- new management endpoints or CLI commands;
- new YAML fields, registry schema, SQLite schema, or client-token scope
  mutation flows;
- active health checks, background probes, live upstream model aggregation, or
  automatic model exposure;
- Responses-to-Chat conversion, endpoint fallback, default model injection, or
  broad OpenAI protocol emulation;
- persistent failure ledger, support bundle, usage/billing ledger, dashboard,
  or UI;
- response-filter lifecycle expansion beyond already implemented behavior;
- production logic changes in `src/proxy.rs`, `src/routing.rs`, or `src/pool.rs`.
- extraction of `src/config.rs` resolver/compiler boundaries, including new
  `config_resolver`, `config_compiler`, or equivalent semantic compilation
  modules. `v0.2` may characterize existing config behavior but must not move or
  reinterpret resolver semantics.

`v0.3` starts only after `v0.2` is released and tagged, or after `v0.2` is
explicitly abandoned. Its entry scope is `Conservative Pre-Output Stability v1`:
reduce client-visible upstream jitter with at most one extra pre-output attempt
under explicit budget, replayability, streaming, deadline, endpoint-family, and
duplicate-charge gates. `v0.3` must have its own plan, tests, telemetry proof,
and hot-path budget proof.

## Design Discipline

### High Cohesion

Each change must belong to one domain owner:

- release artifact smoke belongs in `scripts/release-smoke.sh`;
- release build contract belongs in `docs/release-build.md`;
- operator usage belongs in `README.md` and `docs/operations.md`;
- report rendering belongs in `src/cli_report.rs`;
- side-effect classification belongs in `src/cli_effects.rs`;
- management HTTP client behavior belongs in `src/operator_client.rs`;
- route registration or app construction belongs in one new module extracted
  from `src/main.rs`;
- request forwarding belongs in `src/proxy.rs`;
- retry/state transition policy belongs in `src/routing.rs`;
- credential selection belongs in `src/pool.rs`.

A task that needs to touch more than one domain must name the interface between
domains before implementation. If the interface is unclear, the task stops and
records a blocker instead of adding cross-module shortcuts.

### Low Coupling

`v0.2` must not introduce new runtime dependencies between CLI, management
projections, config storage, and proxy forwarding. CLI commands may call
management APIs or perform offline config parsing according to their documented
side-effect class. They must not reimplement route planning, read SQLite
directly, infer provider health from local files, or call upstreams unless the
command is already an explicit probe workflow.

The request path may continue reading compiled in-memory runtime state only. It
must not query YAML, registry storage, credential storage, client-token storage,
live upstream catalogs, failure ledgers, or operator reports.

### Config Field Admission Gate

`v0.2` adds no public configuration field. Any future YAML, registry,
CLI-default, environment, or management-stored field must have a checked
admission record before implementation starts. Missing evidence blocks the
task.

| Gate | Required answer |
| --- | --- |
| Owner domain | One of: offline config diagnostics, control-plane registry resolution, management projection, data-plane route policy, credential lifecycle, response filtering, release tooling. Catch-all ownership is rejected. |
| User contract | The field's operator-visible behavior in one sentence, including whether it changes client-visible `/v1/*` behavior. |
| Scope | Exact resource type: route target, channel, credential set, policy profile, routing profile, client-token scope, response-filter rule, or management query. Global fields are rejected unless no narrower owner exists. |
| Resolution phase | One of: CLI flag parse, offline `check-config`, startup registry resolution, staged registry reload, management query only. Request-time parsing is rejected. |
| Compiled projection | Name the concrete compiled runtime field or state slot the request path reads. If none exists, the field cannot affect forwarding. |
| Hot-path access proof | State that the request path performs no YAML read, SQLite/store query, management call, upstream catalog call, filesystem read, dynamic regex compile, or unbounded map insertion for this field. |
| Cardinality/memory bound | Maximum number of values and whether the bound is config-validated before runtime. Unbounded per-model/provider/key/user dimensions are rejected. |
| Failure mode | Stable reason code when the field is malformed, unsupported, deprecated, or conflicts with another field. |
| Management explanation | Which existing management/report command exposes the resolved effect. If none, explain why operator visibility is unnecessary. |
| Redaction class | Trusted local id, operator credential ref, bounded enum, count, or hidden. Secret-derived, URL-like, token-like, upstream free-form, and promotional/injection-like values are rejected. |
| Tests | Named tests for parse/resolve, hot-path negative proof, redaction, docs, and backward compatibility. |
| Docs | README/configuration/technical-design location. Product docs must not present the field as catalog, billing, UI, protocol bridge, or automation-platform behavior. |

Fields that only tune operator reports belong to CLI flags or management query
parameters before they belong in YAML. Fields that affect request forwarding
belong in compiled route/policy/profile resources before they reach the proxy
path. Fields that require live probing, persistent learning, or broad protocol
conversion are separate plans.

For future plans, these decisions are already locked unless a new admission
record explicitly overrides them:

- `endpoint_family`, pagination, failure filters, display limits, and
  credential-state filters are management query parameters or CLI flags, not
  YAML fields.
- A `Conservative Pre-Output Stability v1` runtime YAML `preset` is rejected by
  default. Future templates may render explicit `routing_profiles` fields
  instead.
- Existing probe-derived policy fields are compatibility surface only. Do not
  add automatic probe-apply, batch apply, selector promotion, or request-path
  lifecycle mutation from probe evidence.
- Release artifact checks, deployment pin evidence, and stop-card fields are
  process evidence, not runtime configuration.

### Anti-Sprawl Rule

Do not solve a symptom by adding a new catch-all module, catch-all endpoint,
catch-all config struct, or catch-all command. The preferred order is:

1. characterize the existing behavior with a focused test;
2. place the behavior under the existing domain owner;
3. expose only a small typed projection if an operator needs to see it;
4. write a separate plan if the behavior needs a new lifecycle or state machine.

### Anti-Platform Release Gate

Before any `v0.2` task is committed, the staged diff must be checked against
this anti-platform gate. Any match blocks the task unless a separate accepted
plan explicitly authorizes it.

Rejected platform drift:

- a new client-facing `/v1/*` endpoint, endpoint fallback, protocol bridge,
  default model injection, or Responses-to-Chat conversion;
- live upstream `/v1/models` aggregation or automatic exposure of discovered
  upstream models;
- automatic client-token scope mutation, automatic route upsert, or one-shot
  discover/apply/reload/scope workflow;
- background health scanning, continuous key probing, probe-all, auto-fix, or
  adaptive provider/channel breaker;
- usage, billing, budget, price, cost, quota resale, per-client analytics, or
  dashboard semantics;
- UI, hosted multi-tenant concepts, teams, organizations, users, plans, or
  reseller workflows;
- persistent failure ledger or support bundle that stores raw request/response
  material, upstream text, token-derived identifiers, or broad operational
  history;
- generic plugin/provider marketplace, generic route registry, catch-all config
  object, catch-all management endpoint, or catch-all CLI command.

Allowed mature-gateway absorption remains limited to typed routing policy,
explicit compiled route targets, credential lifecycle state, bounded pre-output
retry gates, redacted management projections, and release-smoked operator
workflows.

## Capability Package A: Operator Confidence Release Closure

Purpose: prove the existing M1-M4 operator capabilities are coherent as a
release artifact.

### Required Behaviors

- `scripts/release-smoke.sh` must run the extracted release binary from `dist/`,
  not `cargo run`.
- Smoke uses generated placeholder tokens in a tempdir and a local mock
  upstream only.
- Smoke must verify `check-config` model visibility equals authenticated
  `/v1/models` for the generated local client token.
- Smoke must execute representative read-only management commands and verify
  the unified report envelope contains `status`, `reason_code`, and
  `next_action`.
- Smoke must assert the decision triad for representative states, not only the
  presence of envelope fields.
- Smoke must cover at least one dry-run management mutation report.
- Smoke must reject management commands pointed at a client `/v1` base URL.
- Smoke must not print raw keys, client tokens, management tokens, request
  bodies, response bodies, upstream text, absolute key paths, or token-like
  URLs.

### Fixed Command Matrix

The matrix is fixed for `v0.2`. Do not grow it during implementation without a
plan update.

Local/offline:

- `one-ai-key --help`
- `one-ai-key init local --out config/local.yaml --keys data/relay.keys --dry-run --output json`
- `one-ai-key init local --out config/local.yaml --keys data/relay.keys --yes --output json`
- `one-ai-key --config config/local.yaml check-config --output json`

Client data-plane smoke:

- `GET /health`
- `GET /ready`
- authenticated `GET /v1/models`
- one non-streaming OpenAI-compatible chat completion to the local mock upstream

Management/report smoke:

- `doctor --output json`
- `client-tokens list --output json`
- `models list --client-token-ref local-client --output json`
- `models explain --model gpt-example --client-token-ref local-client --output json`
- `route explain gpt-example --client-token-ref local-client --output json`
- `keys stats --credential-set relay_credentials --output json`
- `failures tail --last 20 --output json`
- `reload status --output json`
- `reload diff --output json`
- `reload apply --dry-run --output json`

### Fixed Decision Scenario Matrix

The smoke fixture must include only local placeholder data and local mock
upstreams. The artifact smoke covers a representative end-to-end path. Contract
tests cover the wider reason-code and redaction matrix.

Artifact-smoke states:

| Scenario | Required evidence |
| --- | --- |
| runnable artifact | extracted binary runs `--help`, `init local`, and `check-config` without source checkout |
| usable model | `check-config` preview, `/v1/models`, `models explain`, `route explain`, and one chat completion agree on `gpt-example` |
| reload not applied / no staged diff | `reload status/diff/apply --dry-run` reports staged/active state and does not mutate runtime |
| management URL misuse | `/v1` management URL is rejected with `client_base_url_used_for_management` |
| invalid client token | client auth fails as `invalid router api key` and does not reveal token material |

Contract-test states:

| Scenario | Required evidence |
| --- | --- |
| model not visible | stable reason code distinguishes missing route or client-token scope; next action is read-only or config + `check-config` |
| no usable key | stable reason code distinguishes credential unavailability; next action is `keys stats` or `keys import --dry-run` |
| streaming transient boundary | characterized as no transparent streaming retry; any gap becomes `v0.3_blocker` |
| bounded failure windows | failure output does not exceed documented per-source and combined caps |
| report redaction | reports do not expose raw keys, raw tokens, token hashes, full upstream URLs, absolute key paths, request bodies, or response bodies |

Negative smoke:

- management URL ending in `/v1` fails with the documented
  `client_base_url_used_for_management` reason;
- invalid client token failure stays a client auth problem and does not reveal
  token material;
- any request-path stability gap found by tests becomes a `v0.3_blocker`, not a
  `v0.2` fix.

## GitHub Asset And Deployment Pin Boundary

`v0.2` release closure has three separate gates:

1. **Local release-ready gate:** local CI, local Docker/Nix build, local
   checksum verification, artifact-shape verification, release smoke against
   the extracted artifact, staging denylist, anti-platform gate, and
   `no_prod_touch`.
2. **Published asset gate:** after upload, download the GitHub Release tarball
   and checksum sidecar into a tempdir and verify tag, version, filename,
   checksum, and release notes identity. This gate must not modify an already
   uploaded artifact; failure requires a new artifact/release attempt.
3. **Deployment pin evidence gate:** documentation and optional operator-run
   evidence that a deployment host or deployment repository pins the GitHub
   Release tarball URL and exact checksum.

The `v0.2` implementation plan must not SSH to production hosts, edit server
configuration, restart services, run `systemctl`, rotate tokens, or send traffic
to a real public gateway. Production smoke belongs to `docs/operations.md` as an
operator-run checklist after the operator intentionally updates the host pin.

If deployment evidence is not available during release closure, the stop card
must say `deployment_pin_smoke: not_run_by_design` and link to the operations
checklist. It must not block the local artifact release unless the release claim
says the production deployment has already been updated.

## Stop Nodes

`v0.2_local_release_ready` is complete only when the fixed local evidence set is
present:

- `local_ci`;
- `docker_nix_build`;
- `artifact_sha`;
- `artifact_shape`;
- `release_smoke`;
- `contract_tests`;
- `denylist`;
- `anti_platform_gate`;
- `support_residue_scan`;
- `no_prod_touch`;
- `route_boundary_extraction`, with `deferred` accepted.

`v0.2_published_release_complete` is complete only when
`v0.2_local_release_ready` is complete and the published GitHub assets have
been downloaded and checksum-verified. Deployment evidence is recorded as
`not_run_by_design`, `operator_provided`, or `operator_run`; it does not block
published artifact completion unless the release claim explicitly says the
deployment host was updated.

When a stop node is complete, stop. New findings must be classified as
`blocking_defect`, `v0.3_blocker`, or `post_release_debt`. Do not keep adding
release-smoke cases, docs sections, or review rounds without replacing an
existing matrix item or writing a separate accepted plan.

## Optional Hygiene Package B: Main Boundary Diet

Purpose: stop `src/main.rs` from remaining the default home for every route,
role gate, CLI dispatch, and integration test.

`src/main.rs` is currently too large to be a sustainable boundary. `v0.2` still
must avoid broad refactoring. This package is allowed only after Capability
Package A and release stop-card evidence are complete. It is not required to
answer whether a personal operator can use the release, why a route is
unavailable, or what safe command comes next.

If it risks delaying release smoke, changing route registration behavior, or
touching request-path semantics, defer it to a separate refactor plan after
`v0.2`. The `v0.2` release stop card may record
`route_boundary_extraction: deferred` without blocking release.

Preferred slice:

- create `src/app_routes.rs`;
- move management route specs, management role gate helpers, unregistered
  management route handler, and Axum route registration into that module;
- keep existing handler functions in existing management modules;
- expose one small builder function consumed by `src/main.rs`;
- add route/role equivalence tests near the new module.

Fallback slice if route registration is too entangled:

- create `src/server.rs`;
- move app construction and listener startup boundary only;
- keep route registration unchanged;
- add a focused behavior-equivalence test.

Forbidden in this package:

- no extraction of multiple slices in one task;
- no changes to `src/proxy.rs`, `src/routing.rs`, or `src/pool.rs`;
- no new framework, plugin system, generic route registry, or diagnostics
  platform;
- no changes to route paths, HTTP methods, management roles, auth behavior, or
  client-visible error shapes.

## Files And Ownership

Expected `v0.2` write set:

- Create: `docs/plans/operator-confidence-release-plan.md`
- Modify: `scripts/release-smoke.sh`
- Modify: `README.md`
- Modify: `docs/operations.md`
- Modify: `docs/release-build.md`

Optional `v0.2` hygiene write set:

- Create: `src/app_routes.rs` or `src/server.rs`
- Modify: `src/main.rs`
- Modify focused tests colocated with the changed module

Conditional write set:

- Modify: `src/cli_report.rs` only for report-contract tests or mechanical
  test helpers over existing behavior.
- Modify: `src/cli_effects.rs` only for side-effect/effect-vector tests over
  existing behavior.
- Modify: `src/config_diagnostics.rs` only if existing offline diagnostics need
  tests or local helper extraction over existing offline diagnostics. Do not
  create resolver/compiler modules, move semantic resolution out of
  `src/config.rs`, or change accepted YAML, defaults, route resolution, startup
  failure semantics, reload semantics, or runtime state.

Forbidden production write set for `v0.2`:

- `src/proxy.rs`
- `src/routing.rs`
- `src/pool.rs`

Files that must not become catch-all homes:

- `src/main.rs`
- `src/management.rs`
- `src/config.rs`
- `src/state.rs`
- `src/cli_commands/keys.rs`

`src/config.rs` being too broad is a recorded architecture debt, not a `v0.2`
release blocker. Any resolver/compiler extraction requires a separate plan with
semantic equivalence tests, startup/reload/check-config parity, and no
request-path behavior changes.

## Deferred Architecture Debt: Config Resolver/Compiler Boundary

`src/config.rs` remains a known broad ownership area after `v0.2`. The desired
future direction is a separate behavior-preserving resolver/compiler boundary,
but it is not part of this release. A future plan must first lock semantic
parity for startup resolution, offline `check-config`, registry reload/diff,
public model visibility, client-token scope expansion, policy/routing profile
flattening, credential-set references, and endpoint capability projection.

The extraction must not add YAML fields, registry schema, request-path reads,
live discovery, protocol behavior, or compatibility defaults that differ from
the current resolver.

## Task Plan

### Task 0: Preflight And Local Garbage Guard

**Files:**

- Read: `.gitignore`
- Read: `.dockerignore`
- Read: `AGENTS.md`
- Read: `docs/release-build.md`

- [ ] **Step 1: Confirm clean worktree**

Run:

```bash
git status -sb --untracked-files=all
```

Expected: no unstaged or untracked implementation artifacts.

- [ ] **Step 2: Confirm no repository-local build caches**

Run:

```bash
du -sh target key-pool-router dist 2>/dev/null || true
find . -maxdepth 4 \( -name target -o -name key-pool-router -o -name __pycache__ -o -name dist \) -print
```

Expected: no `target/`, no `key-pool-router/`, no `__pycache__`, no `dist/`
before implementation begins. Release build may recreate `dist/`, but it must
not be staged.

- [ ] **Step 3: Confirm Git object garbage is not masking local bloat**

Run:

```bash
git count-objects -vH
```

Expected: `garbage: 0`. If garbage exists, run `git gc --prune=now` before
continuing.

- [ ] **Step 4: Commit nothing**

This task is a guardrail only.

### Task 1: Release Smoke Matrix Closure

**Files:**

- Modify: `scripts/release-smoke.sh`

- [ ] **Step 1: Inspect current smoke coverage**

Run:

```bash
sed -n '1,280p' scripts/release-smoke.sh
```

Expected: identify current artifact extraction, generated temp config, mock
upstream, service startup, data-plane smoke, management smoke, and negative
management-url check.

- [ ] **Step 2: Add only missing fixed matrix entries**

Update `scripts/release-smoke.sh` so it covers the artifact-smoke matrix above.
Keep local placeholder tokens and local mock upstreams only. Do not grow smoke
with every negative branch; put combinatorial reason-code, redaction, failure
window, endpoint-family, and credential-state cases in contract tests.

- [ ] **Step 3: Shell-check by execution parser**

Run:

```bash
bash -n scripts/release-smoke.sh
```

Expected: pass.

- [ ] **Step 4: Verify release-smoke does not depend on repository-local build
output except `dist/`**

Run:

```bash
rg -n "cargo run|target/|key-pool-router|real upstream|sk-|Bearer [A-Za-z0-9._-]+" scripts/release-smoke.sh
```

Expected: no match except harmless comments if introduced deliberately.

- [ ] **Step 5: Commit**

```bash
git status -sb --untracked-files=all
git add scripts/release-smoke.sh
git diff --cached --name-only
git commit -m "test: close release smoke matrix"
```

### Task 2: Operator Report Contract Closure

**Files:**

- Modify test modules near `src/cli_report.rs`, `src/cli_effects.rs`, and
  existing command modules as needed.

- [ ] **Step 1: List existing report/effect tests**

Run:

```bash
cargo test --locked cli_report -- --list
cargo test --locked cli_effects -- --list
```

Expected: non-zero relevant tests, or a clear local note that a broader named
filter is needed.

- [ ] **Step 2: Add contract tests for existing report behavior**

Cover:

- every representative command report has `status`, `reason_code`, and
  `next_action`;
- `safe_argv` is structured and redacted;
- dry-run, read-only, local-write, upstream-touching, and management-mutating
  reports keep their current side-effect classes;
- bounded failure output does not exceed documented caps: routing telemetry and
  response-filter events default to 1024 in-memory events, each rejects public
  configuration outside 1 to 4096, `failures` defaults to 50 records per source,
  caps each source at 200, and returns at most 400 combined records.

- [ ] **Step 3: Run narrow tests and prove filters are non-empty**

Run the relevant `cargo test --locked <filter> -- --list` command first, then
run the same filter normally.

- [ ] **Step 4: Commit**

```bash
git status -sb --untracked-files=all
git diff --name-only
# Stage only files changed by this task, normally a narrow subset of:
# src/cli_report.rs src/cli_effects.rs src/cli_commands/*.rs
git add <actual-report-contract-files>
git diff --cached --name-only
git commit -m "test: lock operator report contracts"
```

Do not use `git add src/cli_commands` unless every file listed by
`git diff --name-only` belongs to this task.

### Task 3: Config Diagnostic Closure

**Files:**

- Modify: `src/config_diagnostics.rs` only if required.
- Modify tests near existing config diagnostics.

- [ ] **Step 1: Characterize current offline diagnostics**

Run:

```bash
rg -n "check-config|diagnostic|deprecated|visibility" src tests docs
```

Expected: identify existing diagnostic owner and tests.

- [ ] **Step 2: Add tests for current offline guarantees**

Cover:

- no network calls;
- no SQLite/store mutation;
- visible public model preview for configured client-token references;
- empty visibility reports a stable reason;
- deprecated fields/templates warn without rewriting config.

- [ ] **Step 3: Avoid resolver semantic changes**

If a test requires changing accepted YAML, defaults, route resolution, startup
failure semantics, or runtime state, stop and record a blocker.

- [ ] **Step 4: Commit**

```bash
git status -sb --untracked-files=all
git diff --name-only
# Stage only files changed by this task.
git add <actual-config-diagnostic-files>
git diff --cached --name-only
git commit -m "test: close config diagnostic contracts"
```

Only stage files actually changed.

### Task 4: Existing Stability Boundary Characterization

**Files:**

- Modify integration tests or existing test-only modules only.
- Do not modify `src/proxy.rs`, `src/routing.rs`, or `src/pool.rs`, including
  inline test additions, unless a separate maintainer decision explicitly
  reclassifies the task. If the only practical test location is inside those
  files, stop and record a `v0.3_testability_blocker`.

- [ ] **Step 1: Locate existing pre-output stability tests**

Run:

```bash
rg -n "retry_same_target|guarded_success|streaming_not_retryable|partial_output|duplicate_charge" src tests
```

Expected: identify current coverage.

- [ ] **Step 2: Add characterization tests only**

Cover:

- non-streaming replayable pre-output transient behavior that already exists;
- streaming retry denial;
- partial-output retry denial;
- guarded 2xx body behavior if already implemented;
- duplicate-charge risk evidence if already emitted.

- [ ] **Step 3: Treat every request-path behavior gap as a `v0.3` input**

If characterization proves that upstream jitter is still too client-visible,
record a `v0.3` blocker with the failing scenario, expected conservative gate,
and test name. Do not change production request-path code, retry policy,
credential selection, route planning, streaming behavior, or response buffering
in `v0.2`.

- [ ] **Step 4: Commit**

```bash
git status -sb --untracked-files=all
git diff --name-only
# Stage only focused characterization test files changed by this task.
git add <actual-stability-characterization-files>
git diff --cached --name-only
git commit -m "test: characterize pre-output stability boundary"
```

Do not stage production request-path files for this task.

### Task 5: Optional Main Boundary Diet

**Files:**

- Create: `src/app_routes.rs` preferred, or `src/server.rs` fallback.
- Modify: `src/main.rs`

- [ ] **Step 0: Confirm release confidence is already closed**

Run:

```bash
scripts/local-ci.sh
scripts/build-release-x86_64-linux-docker.sh
scripts/release-smoke.sh
```

Expected: pass. If any command fails, skip this optional hygiene package and
return to Capability Package A.

- [ ] **Step 1: Choose exactly one extraction slice**

Preferred:

```text
management route specs + role gate helpers + route registration -> src/app_routes.rs
```

Fallback:

```text
server app construction/listener boundary -> src/server.rs
```

Record the chosen slice in the commit message.

- [ ] **Step 2: Write equivalence tests before moving behavior**

Tests must prove route path/method/role equivalence and management URL behavior
where practical.

- [ ] **Step 3: Move the minimum code**

Do not rename routes, handlers, roles, auth behavior, request extractors, or
error shapes.

- [ ] **Step 4: Run narrow and full checks**

Run:

```bash
cargo fmt -- --check
cargo test --locked app_routes -- --list
cargo test --locked app_routes
cargo test --locked
```

If the chosen module is `src/server.rs`, use the matching non-empty filter.

- [ ] **Step 5: Commit**

```bash
git status -sb --untracked-files=all
git diff --name-only
# Preferred slice:
git add src/main.rs src/app_routes.rs
# Fallback slice, only if src/server.rs was chosen instead:
# git add src/main.rs src/server.rs
git diff --cached --name-only
git commit -m "refactor: extract app route boundary"
```

Do not stage both `src/app_routes.rs` and `src/server.rs`.

### Task 6: Documentation Closure

**Files:**

- Modify: `README.md`
- Modify: `docs/operations.md`
- Modify: `docs/release-build.md`

- [ ] **Step 1: Cold-read README as a personal operator**

Verify first screen states the product identity: lightweight OpenAI-compatible
key router with explicit local model routes and redacted operator tooling.

- [ ] **Step 2: Remove support-thread material**

Docs must not narrate private deployments, temporary keys, one-off incidents,
chat history, real domains, server names, or workaround transcripts.
This plan file follows the same rule.

- [ ] **Step 3: Document stable workflows only**

Cover:

- install from release artifact;
- generate starter config;
- check config offline;
- run service;
- configure clients;
- inspect `/v1/models`;
- use doctor/models/route/keys/failures/reload commands;
- build/publish release through Docker/Nix x86_64 path;
- keep local config/state/build artifacts out of Git.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/operations.md docs/release-build.md
git diff --cached --name-only
git commit -m "docs: close operator confidence release docs"
```

### Task 7: Full Local Gate And Release Stop Card

**Files:**

- Read-only verification unless docs need correction.

- [ ] **Step 1: Run local CI**

Run:

```bash
scripts/local-ci.sh
```

Expected: pass.

- [ ] **Step 2: Prove release-contract tests are non-empty and pass**

Run:

```bash
cargo test --locked --test local_release_contract -- --list \
  | tee /tmp/one-ai-key-local-release-contract.list
awk '/: test$/{n++} END{exit(n > 0 ? 0 : 1)}' \
  /tmp/one-ai-key-local-release-contract.list
cargo test --locked --test local_release_contract
```

Expected: the test target exists, lists at least one test, and passes. If a
future task uses any other filtered Cargo command, it must use the same non-zero
`-- --list` proof first.

- [ ] **Step 3: Build release through the supported path**

Run:

```bash
scripts/build-release-x86_64-linux-docker.sh
```

Expected: `dist/one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz` and
matching `.sha256`.

- [ ] **Step 4: Verify release artifact shape**

Run:

```bash
PACKAGE_METADATA="$(cargo metadata --locked --no-deps --format-version 1)"
PACKAGE_NAME="$(jq -r '.workspace_members[0] as $root | .packages[] | select(.id == $root) | .name' <<<"$PACKAGE_METADATA")"
PACKAGE_VERSION="$(jq -r '.workspace_members[0] as $root | .packages[] | select(.id == $root) | .version' <<<"$PACKAGE_METADATA")"
TARGET=x86_64-unknown-linux-gnu
ARCHIVE="dist/${PACKAGE_NAME}-${PACKAGE_VERSION}-${TARGET}.tar.gz"
test -f "$ARCHIVE"
test -f "${ARCHIVE}.sha256"
awk -v name="$(basename "$ARCHIVE")" '$2 == name {ok=1} $2 ~ /\// {bad=1} END{exit(ok && !bad ? 0 : 1)}' "${ARCHIVE}.sha256"
(cd dist && shasum -a 256 -c "$(basename "${ARCHIVE}.sha256")")
tar -tzf "$ARCHIVE" | tee /tmp/one-ai-key-archive.list
printf '%s\n' "$PACKAGE_NAME" | diff -u - /tmp/one-ai-key-archive.list
```

Expected: checksum sidecar contains only the archive basename, checksum passes,
and the tarball contains only the release binary.

- [ ] **Step 5: Run release smoke**

Run:

```bash
scripts/release-smoke.sh
```

Expected: pass.

- [ ] **Step 6: Verify GitHub release asset identity after upload**

After the local release-ready gate passes and the release is uploaded, download
the uploaded tarball and `.sha256` into a tempdir and verify the checksum from
the uploaded sidecar. Confirm tag, Cargo version, artifact filename, checksum
filename, and release notes version match.

Expected: uploaded assets are exactly the release tarball and checksum sidecar
unless a separate packaging task explicitly added more artifacts.

- [ ] **Step 7: Record deployment pin boundary**

Do not touch production servers in this task. Record one of:

- `deployment_pin_smoke: not_run_by_design` with a link to
  `docs/operations.md`;
- `deployment_pin_evidence: verified_from_operator_supplied_readonly_record`
  when the operator provides a redacted host pin/checksum record;
- `production_smoke: operator_run` only if a separate explicit operator action
  ran it outside this implementation plan.

The stop card must not imply production has been updated unless the evidence is
present.

- [ ] **Step 8: Verify staging denylist and anti-platform gate**

Run:

```bash
git status -sb --untracked-files=all
git diff --check
git diff --cached --name-only
```

Expected: no staged `dist/`, `target/`, `key-pool-router/`, `config/`, `data/`,
SQLite, logs, key files, private scripts, or `AGENTS.md`.

Also verify the staged diff adds no client-facing protocol compatibility, live
catalog aggregation, billing/usage analytics, UI/multi-tenancy, background
probing, adaptive routing, or automatic discover/apply/reload/scope behavior.

- [ ] **Step 9: Record `v0.2` stop card**

The release stop card must state only fixed fields:

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
- `release_smoke_result`;
- `artifact_sha`;
- `published_asset_verification`;
- `redaction_and_denylist_result`;
- `anti_platform_gate_result`;
- `support_residue_scan_result`;
- `deployment_boundary_result`;
- `no_prod_touch`;
- `route_boundary_extraction`;
- `known_blockers`;
- `v0.3_blockers`;
- `next_version_candidates`.

- [ ] **Step 10: Commit stop-card docs if needed**

```bash
git status -sb --untracked-files=all
git diff --name-only docs
# Stage only the stop-card document(s) changed by this task.
git add <actual-stop-card-docs>
git diff --cached --name-only
git commit -m "docs: record v0.2 release stop card"
```

Do not use `git add docs`.

## `v0.3` Entry Criteria

Start `v0.3` only after `v0.2` is tagged or explicitly abandoned. `v0.3` should
be one stability package, not a catch-all platform upgrade.

Initial `v0.3` scope:

- named capability package: `Conservative Pre-Output Stability v1`;
- no runtime YAML preset by default; init templates may render explicit
  `routing_profiles` fields;
- at most one extra upstream attempt per original client request;
- total retry wall-clock budget <= 1500 ms;
- non-streaming `chat_completions` and non-streaming `responses` only in the
  initial endpoint-family allowlist;
- replayable bodies only;
- retry before upstream response bytes reach the client only;
- no retry after partial output;
- no default 429 retry;
- no default cross-provider fallback;
- stable denial reason codes;
- duplicate-charge risk telemetry;
- management-only bounded retry evidence.

The one extra attempt is mutually exclusive: retry a different credential on
the same channel, retry one eligible frozen route target, or retry the same
target once as a last resort. These choices must not chain inside the same
original client request.

`v0.3` must not add:

- background health scanning;
- persistent adaptive routing;
- usage/billing analytics;
- live model catalog aggregation;
- protocol conversion;
- broad endpoint fallback;
- response body buffering beyond existing pre-output and streaming-filter
  budgets.

The `v0.3` plan must include a hot-path proof against
`docs/performance-budget.md` before implementation starts.
