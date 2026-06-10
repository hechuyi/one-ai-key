# Operator UX And Client Compatibility Roadmap

**Goal:** improve one-ai-key as an operator-friendly personal/small-team AI key router without changing its core identity: a lightweight OpenAI-compatible data-plane proxy backed by explicit local model routes, bounded runtime state, redacted management projections, and conservative retry behavior.

**Architecture:** operator UX is built from CLI commands and management projections over already-compiled runtime state. The request path must keep using in-memory runtime snapshots and must not query YAML, SQLite, credential stores, client-token stores, live upstream model catalogs, or failure history.

**Tech stack:** Rust, clap, reqwest, serde/serde_json, serde_yaml, Axum management APIs, optional SQLite-backed local stores, local x86_64 Docker/Nix release builds, focused integration and contract tests.

---

## Product Principles

one-ai-key should remain:

- a lightweight local or private-network AI key router;
- OpenAI-compatible at the client boundary;
- explicit about public model ids, upstream model ids, channels, credential sets, and client-token visibility;
- conservative about retry, fallback, reload, and mutation;
- useful to a single operator during incident diagnosis without exposing secrets or untrusted upstream text.

It should not become:

- a hosted multi-tenant API platform;
- a billing, quota, usage, or cost product;
- a live upstream catalog aggregator;
- a background health-check cluster;
- a broad OpenAI protocol conversion gateway;
- a UI/dashboard-first system;
- a replacement for large API platforms such as LiteLLM, New API, or Kong-style gateways.

## Milestone Outcomes

| Milestone | User-visible outcome | Boundary preserved |
| --- | --- | --- |
| M1 | A new operator can generate a minimal local config, validate it offline, and preview public model visibility before starting the service. | No network calls, store writes, probes, or runtime mutation. |
| M2 | An operator can answer what is configured, what is visible, and why a route/key/model failed from read-only CLI commands. | CLI renders management projections; it does not reimplement routing or mutate state. |
| M3 | An operator can import keys, probe one credential, apply explicit probe evidence, and plan local model-route onboarding with dry-run and confirmation gates. | Every write, upstream touch, and runtime mutation is intentional, redacted, and reported through an effect vector. |
| M4 | The management plane can explain reload status/diff, explicitly apply runtime reload, and expose static endpoint compatibility. | Capabilities remain static compiled metadata; request-path protocol shims stay deferred. |

The roadmap is complete when M1-M4 are implemented, documented, locally tested, and release-smoked. Deferred work is recorded to define future boundaries, not to extend this roadmap indefinitely.

## Operator Journeys

The roadmap is acceptable only if it supports these workflows:

- **First run:** generate a minimal local config, inspect it offline, start the server, and see configured public models without upstream catalog calls during `/v1/models`.
- **First-run visibility:** run `check-config` before serving to preview public models for configured client-token references, including clear reasons for empty visibility.
- **Model not visible:** run `models explain` and `route explain` to distinguish client-token scope, missing public route, disabled target, channel state, staged-vs-active drift, endpoint-family mismatch, and no usable credential.
- **Key maintenance:** run `keys stats`, `keys replacement-plan`, `keys import --dry-run`, `keys probe --dry-run`, and explicit `keys probe-apply plan/apply` workflows before writes or upstream calls.
- **Upstream instability or filtering incident:** run `doctor` and `failures tail/explain` to inspect bounded recent evidence, router action, retry decision, and safe next command without raw request/response bodies or matched untrusted text.
- **Config change:** use `check-config` before runtime, `reload status` and `reload diff` after staged changes, and explicit `reload apply --dry-run` or `reload apply --yes` when the operator chooses to mutate active runtime state.

## UX Rubric

Every command and document change should answer:

- What can be done without starting the service, touching upstreams, or mutating state?
- Which runtime object is active, which staged/config object is only planned, and what reload/restart action is supported?
- Why is this client-visible model, route target, credential set, credential, or channel usable, skipped, degraded, or invisible?
- What is the next safe command, and is that command read-only, local-write, upstream-touching, management-mutating, or runtime-mutating?
- Is the result complete, intentionally bounded, or only a recent/windowed projection?
- Did the output avoid secrets, secret-derived identifiers, raw request/response material, absolute sensitive paths, and untrusted text?

Command behavior should be explicit rather than clever. A planning or diagnostic command must not silently probe, reload, import, scope, apply, or retry on behalf of the operator.

## Public Documentation Contract

Public docs should describe stable usage, configuration, deployment boundaries, and operator workflows. They must not contain support transcripts, private deployment facts, one-off workaround commands, real tokens, real domains, raw upstream responses, or internal execution records.

Required public documentation surface:

- `README.md`: product fit, install/release path, quick start, minimal config, client configuration, operations overview, verification, release build.
- `docs/configuration.md`: field-level config semantics, upstream templates, routing/profile fields, response filter, endpoint capabilities, explicit defaults.
- `docs/technical-design.md`: request-path invariants, retry/fallback, model catalog, credential lifecycle, management/persistence boundaries.
- `docs/architecture.md`: control-plane and management endpoint matrix, runtime reload/diff semantics, route preview/model explanation projections, deferred boundaries.
- `docs/operations.md`: safe operator workflows, diagnosis commands, reload workflows, key maintenance workflows, and release/deployment boundaries.

## Non-Negotiable Constraints

- Do not make `/v1/models` call upstream providers. It remains a compiled-runtime projection filtered by authenticated client token.
- Do not add request-path disk I/O, request-path YAML/registry/client-token/credential-store joins, request-path upstream model discovery, or full buffering of successful responses.
- Do not retry or fallback after response bytes have been sent to the client.
- Do not add a background health-check cluster, continuous key scanner, live provider catalog, billing ledger, multi-tenant UI, or broad protocol conversion surface.
- Do not expose raw upstream keys, client tokens, management tokens, token hashes, secret-derived identifiers, raw request/response bodies, matched response-filter text, token-like URL components, or absolute sensitive paths in CLI output, management output, tests, docs, logs, errors, or generated artifacts.
- Do not persist or reproduce untrusted promotional/injection text in code, docs, configs, tests, events, reports, or generated files.

## Redaction Classes

Diagnostics should remain useful without leaking sensitive or untrusted material:

- **Secret:** upstream keys, client tokens, management tokens, HMACs, hashes, fingerprints, key prefixes/suffixes, bearer values, API-key headers.
- **Sensitive local context:** absolute private paths, deployment hostnames, private domains, runtime database paths, local scripts, support-thread identifiers.
- **Untrusted upstream text:** upstream error bodies, response-filter matches, provider messages, relay advertisements, model/channel names from untrusted sources when not already normalized.
- **Safe identifiers:** configured public model ids, configured channel ids, configured credential-set ids, endpoint-family names, reason codes, effect classes, bounded counts, synthetic fixture ids.

Reports should prefer stable reason codes, counts, and safe local ids over prose copied from upstream or local private state.

## M1: Offline Configuration And First-Run Confidence

M1 adds offline commands that help an operator understand local config before the service starts.

Core capabilities:

- `init local` produces a minimal runnable config and local key-file layout.
- `check-config` validates config, reports compiled resource counts, route visibility, and client-token model visibility without touching network or runtime state.
- `/v1/models` remains local compiled model projection and does not poll upstream catalogs.

Acceptance:

- offline commands do not read management runtime, call upstreams, or write persistent stores unless explicitly requested;
- outputs use the shared redacted report envelope;
- docs include quick-start and first-run verification.

Non-goals:

- live provider discovery;
- automatic model exposure;
- background probing;
- default model aliases/profiles.

## M2: Read-Only Diagnosis Loop

M2 adds read-only diagnostics for route, model, client-token, and recent-failure questions.

Core capabilities:

- `models explain` is the canonical first diagnosis command for client/model/endpoint-family visibility.
- `route explain` renders management route preview and admission summary without recomputing route logic in the CLI.
- `failures tail/explain` reads bounded recent evidence and distinguishes current availability from historical events.
- `doctor` defaults to lightweight runtime/service health and only includes bounded deeper context when explicitly requested.
- safe next actions are structured as `safe_argv` arrays rather than dynamic shell strings.

Acceptance:

- CLI renders management projections instead of duplicating routing, reload, credential, or endpoint-family logic;
- evidence is bounded and redacted;
- diagnosis commands never suggest mutating operations as automatic next steps.

Non-goals:

- auto-fix;
- active health checks;
- incident ledger;
- raw response transcript support.

## M3: Safe Key And Model Maintenance

M3 adds explicit maintenance workflows for replacement credentials and local model publication.

Core capabilities:

- `keys stats` and `keys replacement-plan` summarize credential capacity and route impact.
- `keys import --dry-run` previews local source-file import counts without revealing secrets.
- confirmed `keys import` is the only replacement-key ingress and requires a writable credential store.
- `keys probe --dry-run` is read-only; confirmed `keys probe` touches one explicit non-secret `credential_ref`.
- `keys probe-apply plan/apply` applies explicit bounded probe evidence.
- `models onboard-plan` produces a read-only model-route publication plan; confirmed apply writes staged registry only.

Acceptance:

- every mutating or upstream-touching command reports its effect vector and requires `--yes` or interactive confirmation;
- raw keys are never accepted as positional CLI arguments and never echoed;
- model publication requires explicit reload before active runtime changes;
- release smoke covers local placeholder workflows.

Non-goals:

- auto rotation;
- batch probe apply;
- inferred credential selection from stats;
- automatic client-scope mutation;
- automatic reload.

## M4: Reload And Static Compatibility Diagnostics

M4 completes the explicit plan/apply/reload/verify control-plane pattern and static endpoint compatibility surface.

Core capabilities:

- `reload status` explains active/staged generations and last reload result.
- `reload diff` reports bounded typed changes without mutating runtime.
- `reload apply --dry-run` verifies preconditions without mutating runtime.
- `reload apply --yes` requires expected staged generation and mutates active runtime only after precondition checks.
- static endpoint capability metadata appears in management projections and CLI diagnostics.
- default-model compatibility remains explicitly parked rather than silently rewriting client requests.

Acceptance:

- reload apply is explicit, preconditioned, redacted, and auditable;
- endpoint compatibility is diagnostic metadata, not request-path protocol conversion;
- docs explain staged vs active runtime behavior.

Non-goals:

- Responses-to-Chat adapter;
- endpoint fallback;
- dynamic provider capability probing;
- usage/cost counters unless separately accepted.

## Deferred Or Rejected Work

Deferred work may be planned later, but is outside M1-M4:

- Responses-to-Chat adapter for clients that only support `/v1/responses`;
- model aliases and profiles;
- default model per endpoint;
- manual probe enhancements beyond explicit single-credential workflows;
- bounded local usage counters.

Rejected for this roadmap:

- hosted multi-tenant platform semantics;
- billing or cost accounting;
- live upstream catalog aggregation as a source of production truth;
- automatic key/model/scope mutation from diagnosis;
- background health-check loops;
- UI/dashboard work.

## Release Gate

Each milestone or release candidate must pass:

- targeted non-zero tests for the changed behavior;
- `scripts/local-ci.sh` with `CARGO_TARGET_DIR` outside the repository;
- `git diff --check`;
- `scripts/check-staged-denylist.sh`;
- release artifact build through the documented local Docker/Nix x86_64 path when publishing;
- extracted-artifact release smoke;
- redaction review for docs, CLI output, management output, test fixtures, and release artifacts.

Release artifacts and commits must exclude:

- `dist/`, `target/`, `key-pool-router/`;
- `config/`, `data/`, SQLite, logs, keys, raw fixtures;
- private scripts, private hostnames, private domains, real API keys, deployment transcripts;
- `AGENTS.md` and internal execution records.

## Stop Conditions

Stop and re-plan if a proposed change requires:

- request-path reads from YAML, SQLite, registry store, credential store, client-token store, failure history, or live upstream catalog;
- broad protocol conversion;
- background probes or adaptive routing;
- persistent failure ledger;
- new global YAML presets;
- automatic key/model/scope mutation outside explicit management write commands;
- UI/dashboard work;
- private deployment state as release evidence.
