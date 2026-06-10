# Operator Confidence Release Plan

**Goal:** ship `v0.2` as an operator-confidence release: existing M1-M4
operator capabilities become reproducible, explainable, redacted, and
release-smoked from the published artifact.

**Architecture:** `v0.2` is a closure release, not a new data-plane feature
release. It validates the existing CLI, management, reporting, release, and
documentation contracts against a local mock environment and a published-form
artifact. It does not broaden request-path retry, protocol compatibility,
routing, model exposure, or credential lifecycle behavior.

**Tech stack:** Rust, Axum, clap, serde/serde_json, local mock upstream tests,
local Docker/Nix Linux x86_64 release build, shell smoke scripts, and GitHub
release artifacts.

---

## Review Inputs

This plan assumes the completed M1-M4 operator UX baseline described in
`docs/plans/operator-ux-implementation-plan.md` and the product direction in
`docs/plans/product-improvement-roadmap.md`.

The review rounds converged on these invariants:

- client-facing model names map to explicit compiled local model routes;
- channel health, credential lifecycle, and route policy remain separate runtime
  domains;
- import, probe, discovery, reload, and explanation stay on the management
  plane;
- request forwarding reads already compiled in-memory state only;
- support-thread residue is converted into stable product rules or removed.

The release must not absorb platform features from larger gateways: hosted
multi-tenancy, billing, UI workflows, live provider catalog fan-out, adaptive
background health clusters, broad protocol conversion, or ledger-driven routing.

## Product Contract

The user-visible `v0.2` promise is:

```text
one-ai-key can be installed from a pinned release artifact, checked with local
mock data, and operated through redacted CLI/management reports that explain
configuration, runtime visibility, key state, reload state, and recent bounded
failure evidence without leaking secrets or touching unsupported automation.
```

The release is complete only when a cold operator can answer these questions
from public docs and release-smoke output:

- is the release artifact runnable without a source checkout;
- does `check-config` agree with authenticated `/v1/models` for the generated
  local config;
- do read-only management commands return the unified redacted report envelope;
- do mutating commands advertise dry-run, confirmation, and effect semantics;
- is a client `/v1` URL rejected when accidentally used as a management URL;
- are release artifacts, build caches, runtime state, configs, keys, databases,
  logs, `AGENTS.md`, and private scripts excluded from Git staging.

The release is a coherent capability package, not a stream of unrelated small
patches. Internal tasks stay small for reviewability, but the user-facing
milestone is one bounded package with five gates:

1. **Release artifact confidence:** the published-form artifact can be unpacked,
   started, and smoked without a source checkout.
2. **Local configuration and model visibility:** offline `check-config`,
   authenticated `/v1/models`, `models explain`, and `route explain` agree on a
   representative public model and client-token reference.
3. **Operator command safety:** read-only, dry-run, upstream-touching, and
   mutating operator reports expose side-effect class, effect vector,
   confirmation, and structured safe next commands.
4. **Failure evidence boundary:** recent failure evidence is bounded, redacted,
   source-local, and explicitly not historical incident storage or a routing
   input.
5. **Release documentation and deployment boundary:** docs describe stable
   install, operation, release, and deployment-pin contracts without private
   support-thread residue.

## Operator Decision Contract

Every `v0.2` smoke, report test, and release record preserves this triad:

```text
resource + client-token-ref + endpoint/model context -> can_use / reason_code / next_action
```

`can_use` is not inferred from process liveness. It is established by matching
offline visibility, authenticated `/v1/models`, model/route explanation,
credential-set state, reload state, and bounded recent failure evidence.

A report passes the operator-confidence gate only when a personal operator can
answer:

- whether this configured client token can see and call this public model now;
- if not, whether the blocker is config, client-token scope, model route,
  endpoint family, credential availability, runtime reload drift, management
  auth, or upstream/runtime failure evidence;
- which next command is safe, whether it is read-only, dry-run,
  upstream-touching, or mutating, and whether confirmation is required.

## Version Boundary

`v0.2` closes the operator-confidence surface. It may verify and characterize
stability behavior already present in the codebase, but it must not broaden that
behavior. Any upstream-jitter gap found during characterization becomes a
`v0.3_blocker` or post-release debt, not a `v0.2` production-path patch.

Allowed in `v0.2`:

- release artifact smoke with local mock upstreams;
- fixed representative CLI/management command matrix;
- report-envelope, side-effect, next-action, redaction, and bounded-window
  contract tests;
- offline config diagnostic characterization without changing accepted config
  semantics;
- characterization tests for already implemented conservative pre-output
  behavior;
- explicit recording that route/app boundary extraction and config
  resolver/compiler extraction remain architecture debt;
- public documentation for stable product usage.

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
- production logic changes in `src/proxy.rs`, `src/routing.rs`, or `src/pool.rs`;
- config resolver/compiler extraction or semantic reinterpretation.

## Design Discipline

Each change belongs to one domain owner:

- release artifact smoke belongs in `scripts/release-smoke.sh`;
- release build contract belongs in `docs/release-build.md`;
- operator usage belongs in `README.md` and `docs/operations.md`;
- report rendering belongs in `src/cli_report.rs`;
- side-effect classification belongs in `src/cli_effects.rs`;
- management HTTP client behavior belongs in `src/operator_client.rs`;
- route registration or app construction belongs in its own app/router module;
- request forwarding belongs in `src/proxy.rs`;
- retry/state transition policy belongs in `src/routing.rs`;
- credential selection belongs in `src/pool.rs`.

The CLI may call management APIs or perform offline config parsing according to
its documented side-effect class. It must not reimplement route planning, read
SQLite directly, infer provider health from local files, or call upstreams
unless the command is already an explicit probe workflow.

The request path may continue reading compiled in-memory runtime state only. It
must not query YAML, registry storage, credential storage, client-token storage,
live upstream catalogs, failure ledgers, or operator reports.

## Capability Package

### Release Artifact Confidence

The artifact gate proves that the published-form binary can be unpacked and run
without a source checkout. The smoke environment uses generated placeholder
tokens, a tempdir, and a local mock upstream. The smoke script must clean up
background processes and must not depend on repository-local build caches.

### Local Configuration And Visibility

The local config gate proves that `check-config --output json`, authenticated
`/v1/models`, `models explain`, and `route explain` agree on the configured
client token and representative public model. Disagreement is a product defect,
not a documentation issue.

### Operator Command Safety

Reports use the common envelope with stable fields such as `status`,
`reason_code`, `side_effect_class`, `effect_vector`, scope/window metadata, and
structured `next_action`. Next actions are allowlisted argv vectors, not dynamic
shell strings.

### Failure Evidence Boundary

Failure evidence is a bounded management projection. It may explain recent
route, upstream, response-filter, and runtime events using local reason codes.
It must not become a persistent incident ledger, usage analytics layer,
cross-source correlation engine, or routing input.

The default failure window remains intentionally small: enough to explain recent
operator confusion, not enough to become observability infrastructure.

### Release Documentation And Deployment Boundary

Public docs must describe stable workflows:

- install from release artifact;
- generate starter config;
- check config offline;
- run service;
- configure clients;
- inspect `/v1/models`;
- use doctor, models, route, keys, failures, and reload commands;
- build and publish through the local Docker/Nix x86_64 path;
- keep local config, state, and build artifacts out of Git.

Deployment verification is an operator-run boundary. This release plan can
record that a deployment pin smoke is expected, but it must not encode private
hostnames, private domains, one-off token replacement commands, production logs,
or server transcripts.

## GitHub Asset Boundary

A release claim is valid only when tag, Cargo package version, tarball name,
checksum filename, and release notes version match. Uploaded assets are limited
to the expected tarball and checksum sidecar unless a separate packaging plan
adds more artifacts.

Release notes and assets must not contain secrets, private domains, local paths,
temporary keys, raw request/response bodies, support-chat residue, or untrusted
promotional text.

## Stop Nodes

The release has three stop nodes:

| Stop node | Meaning |
| --- | --- |
| `v0.2_local_release_ready` | Local CI, release artifact build, artifact shape check, release smoke, docs, and denylist gates pass. |
| `v0.2_published_release_complete` | GitHub release assets are uploaded and verified by downloading the uploaded tarball and checksum sidecar. |
| `v0.2_deployment_pin_verified` | A separate operator-run deployment pin consumes the release artifact by immutable URL and hash. |

The first stop node is enough to say the source tree is release-ready. The
second is enough to say the GitHub release is published. The third is deployment
evidence and must not be implied by source or GitHub checks alone.

## Architecture Debt

Two pieces of debt remain intentionally outside `v0.2`:

- Route/app boundary extraction should continue shrinking `src/main.rs` and keep
  management route access centralized, but it is not a release blocker.
- Config resolver/compiler extraction should be behavior-preserving and needs a
  separate semantic parity plan before moving startup, reload, `check-config`,
  public model visibility, client-token scope expansion, policy flattening, and
  endpoint capability projection.

Neither debt item authorizes request-path changes, new YAML fields, registry
schema changes, live discovery, protocol behavior changes, or compatibility
defaults.

## `v0.3` Entry Criteria

Start `v0.3` only after `v0.2` is tagged or explicitly abandoned. `v0.3` should
be one stability package, not a catch-all platform upgrade.

Initial `v0.3` scope:

- named capability package: `Conservative Pre-Output Stability v1`;
- no runtime YAML preset by default;
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

The one extra attempt is mutually exclusive: retry a different credential on the
same channel, retry one eligible frozen route target, or retry the same target
once as a last resort. These choices must not chain inside the same original
client request.

`v0.3` must distinguish route admission failures from selected-target pre-output
transient failures. Missing scope, missing model route, disabled targets, empty
credential pools, hard channel cooldown, unsupported endpoint family, and stale
runtime state remain explainable local failures; retry must not manufacture a
route candidate.
