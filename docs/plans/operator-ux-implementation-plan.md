# Operator UX And Client Compatibility Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** add operator-facing configuration, explanation, maintenance, and client-compatibility diagnostics while preserving one-ai-key as a lightweight personal/small-team AI key router.

**Architecture:** the first milestones are CLI and management-plane composition over existing runtime projections. Later milestones add narrowly scoped backend capabilities only when they can be expressed as compiled config, bounded runtime state, or explicit management projections. The proxy request path must keep using compiled in-memory runtime state and must not query YAML, SQLite, upstream model catalogs, or background health probes.

**Tech Stack:** Rust, clap, reqwest, serde/serde_json, serde_yaml, Axum management APIs, SQLite-backed local stores, local x86_64 Docker/Nix release build, focused integration and contract tests.

---

## Review Inputs

This plan is the converged result of multiple read-only review rounds across five perspectives:

- operator experience and first-run usability;
- architecture boundaries and hot-path invariants;
- implementation cost, file boundaries, and commit slicing;
- client-compatibility diagnostics for Codex-like Responses clients, Continue, OpenWebUI, and Cursor;
- test matrix, release risk, dry-run/confirmation, and redaction gates.

The main correction from those reviews is structural: read-only explanation, probing/write workflows, and protocol/request-path changes must be separate milestones. A command name such as `doctor`, `models explain`, or `keys stats` must not hide upstream probes, store writes, runtime reloads, scope mutations, or live model aggregation.

The latest convergence review adds these hard corrections:

- this roadmap diagnoses client compatibility but does not implement broad protocol emulation;
- plan checkbox state is owned by the main controller and is committed separately from implementation commits;
- non-secret `credential_ref` support is a blocking gate before individual credential probe/apply workflows;
- reload must have a status/diff/apply operator path, because a diff without an apply/restart answer is not a complete UX;
- `doctor` defaults to compiled runtime health only and must not become an implicit alert/probe aggregation platform;
- all operator reports use a unified redacted envelope with structured `safe_argv`, not dynamic shell command strings;
- release closure includes local artifact smoke, representative operator-command smoke, GitHub release asset checks, docs cold-read, and ignore/denylist verification;
- the active execution baseline is post-M2 stop-card, with M3 as the next implementation gate.

## Product Principles

This roadmap improves one-ai-key as a personal/small-team router, not as a hosted platform. Every milestone should reinforce these product traits:

- clients configure one OpenAI-compatible base URL, one client-facing token, and explicit public model ids;
- upstream keys, relay quirks, fallback, and lifecycle state stay behind the router;
- model exposure is explicit and local, not live-aggregated from upstream catalogs;
- operators get explanations and safe maintenance workflows before they get automation;
- successful responses stream through the data plane without full buffering;
- management commands are redacted, bounded, and predictable enough to use during an incident;
- features that require platform semantics, billing semantics, persistent model trust, or broad protocol emulation are parked until a separate plan justifies them.

## Milestone User Outcomes

| Milestone | User-visible outcome | Core boundary preserved |
| --- | --- | --- |
| M1 | A new user can generate a minimal local config, validate it offline, and preview which public models should be visible to configured client tokens before starting the service. | No network, store writes, probes, or runtime state. |
| M2 | An operator can answer “what is configured, what is visible, why did this route/key/model fail?” from read-only CLI commands. | CLI wraps existing management projections; it does not reimplement routing or mutate state. |
| M3 | An operator can import keys, probe one credential, apply probe evidence, and prepare explicit local model-route onboarding plans with dry-run and confirmation steps. | Every write, upstream touch, and runtime mutation is intentional, redacted, and reported through an effect vector. |
| M4 | The management plane can explain reload status/diff, explicitly apply runtime reload, and expose static endpoint compatibility. | Capabilities remain static compiled metadata; request-path endpoint/default-model compatibility shims remain deferred unless a separate protocol plan accepts them. |
| Deferred gates | Responses support remains parked, and counters are either explicitly deferred or approved through a performance decision record after M1-M4. | No request-path protocol adapter or counter work is part of M1-M4 completion. If later approved, counters must be bounded, low-cardinality, non-billing, and non-buffering. |

## Roadmap Completion Criteria

This plan is complete when M1-M4 are implemented, documented, locally tested, committed at the listed checkpoints, and pass the release gate. The deferred counter decision is a conditional post-completion enhancement: it may be implemented only after M1-M4 if the maintainer records acceptance of the request-path counter budget. The Responses adapter gate is explicitly outside this plan.

Do not keep extending this roadmap after M1-M4 by pulling in parked, rejected, or merely adjacent features. If implementation reveals a need for full Responses support, active health scanning, live model aggregation, billing, a UI, or broader protocol compatibility, stop this roadmap and create a separate plan with its own invariants, hot-path proof, UX contract, and release gate. A worker must not reopen this plan merely because a parked item would be useful.

Each milestone has its own stop card. A milestone is not closed by code existence alone; it is closed only when its tests, redaction checks, UX report contract, docs updates, and commit checkpoint are complete.

## Plan Governance

Use this document as an execution contract, not as a feature wishlist. Work proceeds in milestone order unless a later task is explicitly split out by a new accepted plan. A task may be skipped only when its stop card remains satisfied without it or when the task text itself declares the work deferred.

Execution rules:

- implementation workers and subagents must not edit this plan file unless explicitly assigned the plan-status task;
- the main controller owns checkbox state and updates this file only after the corresponding code, tests, docs, staged-denylist review, and implementation commit checkpoint are complete;
- checkbox updates are committed as separate plan-status commits, for example `chore(plan): mark M2.2 complete`; do not mix plan-status-only edits into feature commits from subagents;
- do not batch unrelated milestones into one commit, even when the code change is small;
- do not use `git add .`; each commit must stage only the named implementation files, test files, and documentation files needed for that task;
- before every commit, inspect `git diff --cached --name-only` and run the staged denylist review in the release gate;
- do not implement parked gates, rejected items, or future proof points while working on M1-M4;
- when a task discovers that an existing endpoint is missing or too expensive, stop at the named blocker instead of hiding a page scan, store scan, or duplicated route planner in the CLI;
- before each milestone close, run a cold UX read from the perspective of a personal operator who knows the management URL, client OpenAI-compatible Base URL, and management-token reference but does not know storage internals.

The stop node for the current roadmap is therefore unambiguous: M1-M4 plus the release gate. Deferred gates are recorded so future work has a shape, not so current workers keep extending the roadmap.

### Task Execution Template

Before editing production code for any unchecked task, the worker must make the task locally executable:

- name the exact test file and test function(s) that will fail first;
- for every filtered Cargo test command, first prove the filter matches at least one intended test with `cargo test --locked <filter> -- --list | rg '<expected_test_name_or_prefix>'`, or use a named test target that cannot silently run zero relevant tests;
- run the narrow failing command and record the expected failure reason in the task notes or commit message;
- implement the smallest production change that satisfies those tests without broadening the task scope;
- run the same narrow test command again and then the task's listed verification command;
- record the non-zero test-discovery evidence for filtered commands in the task completion notes;
- run `git diff --cached --name-only` and a staged denylist review before committing; never stage local configs, keys, databases, logs, build artifacts, raw request/response fixtures, or untrusted text;
- send or record completion evidence for the main controller; the worker does not update this plan checkbox unless explicitly assigned the plan-status commit.

If the task text only lists coverage bullets, the worker must first translate them into concrete test names before implementation. This keeps the plan executable without adding hundreds of brittle test names to the roadmap itself.

### Parallel Execution Matrix

The plan is allowed to use up to five subagents, but only after the main controller freezes the shared CLI interfaces for the milestone. A subagent may own a command module, tests, and docs for that slice; it must not edit `src/cli.rs`, `src/cli_report.rs`, `src/operator_client.rs`, or this plan unless that write set is explicitly assigned.

| Phase | Main-controller work before parallelism | Parallel lanes allowed | Shared files guarded by main controller |
| --- | --- | --- | --- |
| M1 | Already reconciled and committed. Future workers verify rather than reopen M1 unless a regression test fails. | None. | `src/cli.rs`, `src/main.rs`, `src/cli_report.rs`, `src/cli_effects.rs`, this plan. |
| M2 | Closed after commits `242ecbc9` (`fix(cli): reconcile M2 report contract`) and `c1523f4d` (`fix(cli): harden keys safe argv validation`). Future workers verify rather than reopen M2 unless a regression test fails. | No further M2 feature lanes are open. | `src/cli.rs`, `src/cli_commands/mod.rs`, `src/cli_report.rs`, `src/operator_client.rs`, route registration, this plan. |
| M3 | Commit mutating-command confirmation envelope and verify `credential_ref` resolver gate. | Serialize `keys import`, `keys probe`, and `keys probe-apply` unless the main controller first commits a module split such as `keys_import.rs`, `keys_probe.rs`, and `keys_probe_apply.rs` plus shared report/effect helpers. `models onboard-plan` may run separately after its effect class is frozen. | `src/cli.rs`, `src/operator_client.rs`, `src/cli_report.rs`, `src/cli_commands/keys.rs` or split key modules, this plan. |
| M4 | Commit reload/capability shared DTO shape before command display work. | Backend-only reload and capability schema work can run in parallel when write sets are disjoint. CLI display updates to `models.rs`, `route.rs`, and `cli_report.rs` are serialized unless a shared explanation DTO/render extension point is committed first. | `src/cli.rs`, `src/cli_report.rs`, `src/operator_client.rs`, `src/cli_commands/models.rs`, `src/cli_commands/route.rs`, request-path files, this plan. |

Subagents must state their write set in their final report. If two lanes need the same shared file, the main controller either serializes those edits or creates a small integration commit after reviewing both outputs.

### Rollback And Abort Protocol

When a task fails after code has been committed, use a normal `git revert <commit>` for that task rather than rewriting history. When a task hits a named blocker before commit, leave its checkbox unchecked and add a blocker note only if the blocker changes the execution contract. Do not mark a task complete because a workaround exists outside the supported CLI path.

For M3 and M4 mutating commands, each command report must state whether the operation is reversible. If there is no automatic rollback, the report must name the supported recovery command or explicitly say that recovery requires a manual config/store correction plus `check-config` and `reload status`. Tests must cover the irreversible/reporting case for at least `keys import` and `reload apply`.

## Current Baseline And Preflight

As of the latest plan review, the active baseline is:

- M1 is complete and marked in commit `89cd0fc7` (`chore(plan): mark M1 foundation complete`);
- M2.1-M2.4b are complete and marked through commit `81c03a57` (`chore(plan): mark M2.4b complete`);
- M2.5 `failures tail/explain` and M2.6 `doctor` are complete and marked through commit `eee0fb1c` (`feat(cli): add read-only doctor command`);
- M3 and M4 have not started for this roadmap;
- the M2.5 and M2.6 draft files have been integrated and committed; the next active gate is the M2 stop card.

A fresh worker must first run:

```bash
git status -sb --untracked-files=all
git log --oneline -5
```

Known current files:

| File | Baseline treatment |
| --- | --- |
| `src/cli.rs` | Tracked CLI skeleton and M2 commands exist through M2.6. Preserve command wiring while closing the M2 stop card. |
| `src/main.rs` | Server/CLI dispatch wiring exists. Preserve legacy server invocation and existing management route registration. |
| `src/cli_effects.rs` | Tracked classifier exists through M2.6. Preserve M1/M2 side-effect classifications during report-contract reconciliation. |
| `src/cli_report.rs` | Tracked report helpers exist. Reconcile completed commands against the unified report envelope before M2 closes. |
| `src/operator_client.rs` | Tracked operator client exists. Preserve the M2 read-only allowlist and `/v1` management-URL misuse rejection. |
| `src/cli_commands/mod.rs` | Tracked module registry includes M2.5/M2.6 modules. Do not remove integrated module declarations. |
| `src/cli_commands/failures.rs` | M2.5 is integrated and committed. Preserve bounded windows, unified envelope, structured `safe_argv`, and redaction. |
| `src/cli_commands/doctor.rs` | M2.6 is integrated and committed. Preserve default runtime-only projections and opt-in bounded store projections. |
| `docs/plans/operator-ux-implementation-plan.md` | Treat this file as the active execution contract. |

If the preflight finds additional modified or untracked files, the worker must inspect whether they are part of the current task before editing. Never delete, overwrite, or revert unrelated local changes. If a plan step says “Create” but the file already exists, treat the step as “Continue/modify existing” and preserve the established local pattern.

The immediate execution order after M2 stop-card closure is:

1. start M3 only from the post-M2 report contract and side-effect envelope;
2. verify the non-secret `credential_ref` resolver gate before individual credential workflows;
3. serialize `keys import`, `keys probe`, and `keys probe-apply` unless the main controller first commits a disjoint module split;
4. keep plan-status updates separate from feature commits.

## Operator Journey Acceptance

The roadmap should improve concrete personal-operator workflows, not abstract management surface area. The implementation is acceptable only if these journeys are covered by commands, docs, and tests:

- first run: generate a minimal local config, inspect it offline, start the existing server mode, and see the configured public models without contacting upstream catalogs during `/v1/models`;
- first-run visibility: use `check-config` before serving to preview which public models each configured client-token reference should see, including a clear reason when the preview is empty;
- model not visible: use `models explain` and `route explain` to see whether the issue is client-token scope, missing public route, disabled target, channel state, or staged-vs-active drift. M1-M4 diagnose and guide supported config/reload paths; they do not promise automatic scope mutation or model-route upsert unless a later task explicitly implements that command;
- key maintenance: use `keys stats`, `keys import --dry-run`, `keys probe --dry-run`, and explicit `keys probe-apply plan` / `keys probe-apply apply` workflows to understand replacement impact before any write or upstream call;
- upstream instability or filtering incident: use `doctor` and `failures tail/explain` to see the bounded recent evidence, router action, and next safe command without exposing raw request/response bodies or matched untrusted text;
- config change: use `check-config` before runtime, `reload status` / `reload diff` after staged changes, and explicit `reload apply --dry-run` / `reload apply --yes` when the operator chooses to mutate active runtime state.

These journeys must remain simple enough that a personal user can operate the router without learning storage internals, provider account topology, or Rust module names.

## UX Decision Rubric

Every command and document change must be judged by the same operator questions:

- What can I do now without starting the service, touching upstreams, or mutating state?
- If the service is running, which compiled runtime object is active, which staged/config object is only planned, and what exact reload/restart action is supported?
- Why is this client-visible model, route target, credential set, or channel usable, skipped, degraded, or invisible?
- What is the next safe command, and is that command read-only, local-write, upstream-touching, or management-mutating?
- Is the result complete, intentionally bounded, or only a recent/windowed projection?
- Did the output avoid secrets, secret-derived identifiers, raw request/response material, local absolute paths, and untrusted text?

Prefer boring, explicit operator language over clever command behavior. A command that silently probes, reloads, scopes, imports, applies, or retries on behalf of the operator fails this rubric even if the final result is useful. Planning commands must say when they are planning-only and must not use names that imply live discovery or automatic model exposure.

## Documentation Contract

README and docs updates are product documentation, not a transcript of project history or one-off support answers. They should describe stable usage, configuration, deployment boundaries, and operator workflows.

Required public-doc structure after M1-M4:

- README: product fit, install/release path, quick start, first-run transcript, minimal config, client configuration, operations overview, verification, release build.
- `docs/configuration.md`: field-level config semantics, upstream templates, routing/profile fields, response filter, endpoint capabilities, and explicit defaults.
- `docs/technical-design.md`: stable request-path invariants, retry/fallback, model catalog, credential lifecycle, management/persistence boundaries, and the M4 no-new-request-path-behavior boundary.
- `docs/architecture.md`: control-plane and management endpoint matrix, runtime reload/diff semantics, route preview/model explanation projections, and deferred boundaries.

Public docs must not:

- narrate historical chat decisions, user complaints, temporary keys, real domains, private server names, or incident-specific workaround commands;
- over-index on key replacement, probe, or health check questions as if they were the product's main purpose;
- imply that `/v1/models` aggregates upstream catalogs, that `doctor` performs active health checks, or that discovery automatically exposes models to clients;
- include raw tokens, key prefixes/suffixes, token hashes, local absolute sensitive paths, raw request/response bodies, or matched untrusted text.

The docs should make the primary product identity clear in the first screen: a lightweight OpenAI-compatible key router with explicit local model routes and redacted operator tooling.

## Non-Negotiable Constraints

- Do not touch deployment servers during implementation or testing for this roadmap.
- Product commands may support remote `--management-url` for users, but roadmap tests must use local loopback or in-process test servers only.
- Do not make `/v1/models` call upstream providers. It remains a compiled-runtime projection filtered by the authenticated client token.
- Do not add request-path disk I/O, request-path YAML/registry/client-token/credential-store joins, request-path upstream model discovery, or full buffering of successful responses.
- Do not add a background health-check cluster, continuous key scanner, live provider catalog, billing ledger, multi-tenant UI, or LiteLLM/New API replacement surface.
- Do not retry or fallback after response bytes have been sent to the client.
- Do not expose raw upstream keys, client tokens, management tokens, token hashes, secret-derived identifiers, raw request/response bodies, matched response-filter text, token-like URL components, or absolute sensitive paths in CLI output, management output, tests, docs, logs, errors, or generated artifacts. Secret-derived identifiers include key prefixes/suffixes, hashes, fingerprints, HMACs, or any stable value derived from credential material.
- Do not persist or reproduce untrusted promotional/injection text in code, docs, configs, tests, events, reports, or generated files. Untrusted text includes provider and management error bodies, event text, model/channel/provider names, response-filter snippets, fixture inputs, generated reports, and encoded or mixed-script variants after normalization.

## Display And Redaction Classes

Diagnostics must stay useful without leaking secrets or untrusted text. Use these display classes consistently:

| Class | May display? | Examples | Rules |
| --- | --- | --- | --- |
| Trusted local config id | Yes, after validation and escaping. | public model ids, channel ids, credential-set ids, routing profile ids from local config or compiled runtime. | These are operator-authored identifiers. They may appear in table/JSON output if they are not token-like URLs and do not contain untrusted promotional/injection content after normalization. |
| Operator credential reference | Yes, after M2.4a creates it. | `credential_ref` values derived from store position/import metadata or another non-secret stable resource id. | This is the only credential reference CLI commands may print or accept. It must not be derived from raw key material. |
| Internal credential identity | No in CLI output. | current backend `credential_id` values, fingerprints, key-derived hashes. | These may exist internally or in legacy management APIs, but M1-M4 CLI wrappers must drop them from output and must not require users to type them. If a backend call still needs an internal id, resolution from `credential_ref` must happen server-side or inside a non-printing client boundary. |
| Store lineage summary | Limited. | import batch id, redacted source ref, source line, counts. | Never show absolute source paths, raw source lines, raw keys, fingerprints, or duplicate secret-derived values. |
| Upstream/free-form text | No. | upstream model names discovered from provider catalogs, upstream error messages, response-filter matched text, provider returned bodies. | Classify, count, or map to local resource references. Do not quote or preserve the raw text in reports, fixtures, docs, or generated artifacts. |
| Secret material | No. | upstream keys, client tokens, management tokens, token hashes, DSNs, token-like URL components. | Never print, store in golden fixtures, or derive stable public identifiers from this material. |

The current codebase still contains management projections with credential ids and fingerprints derived from credential material. M1-M4 CLI wrappers must not surface those fields. Individual credential workflows such as `keys probe` and `keys probe-apply` require a non-secret `credential_ref`; if M2.4a cannot provide one safely, those individual workflows must be blocked rather than exposing secret-derived ids.

## Canonical Invariants

1. M1/M2 read-only commands are strictly non-mutating: no SQLite writes, no store creation/migration/bootstrap import, no lifecycle persistence worker, no upstream calls, no discovery, no probe, no reload, and no runtime mutation.
2. Offline config checking is an offline dry-run resolver and inspector. It may parse YAML, expand `upstreams`, validate references, and inspect local file existence/counts. It must not construct production `AppState` if that path creates stores, imports bootstrap state, starts workers, or writes audit/event data.
3. Mutating or upstream-touching commands must provide a dry-run plan mode. Real execution requires an explicit resource id and `--yes` or an interactive confirmation; non-TTY execution without `--yes` exits `3` with no side effects.
4. `doctor` is a read-only aggregator over compiled runtime, serving, and resilience projections by default. It is not an active health check, alert platform, auto-fix, probe, discovery, reload, or mutation command. Store-backed alerts or event windows are opt-in only and must be labeled bounded/windowed.
5. `models explain` explains compiled `model_routes`, client-token scope, route targets, and local `/v1/models` projection. Endpoint capability fields appear only after M4; before M4, the command must report `capability_status: unavailable_until_m4` rather than guessing.
6. Model aliases, if added as authoring sugar, disappear after config expansion and become ordinary explicit `model_routes`. There is no runtime alias catalog.
7. Endpoint capabilities are static compiled projections from explicit config and a small template default set. They are not learned by background probing and must not become a provider/model catalog.
8. `responses_to_chat_completions` is not authorized by this plan. A future protocol plan must prove per-channel/per-target explicit selection, pre-credential validation, valid Responses output shape, no full successful-response buffering, and no cross-protocol retry after upstream failure or client-visible output.
9. Runtime counters, if separately approved after M1-M4, are bounded, low-cardinality, redacted, and non-billing. They must not require buffering successful responses.
10. Default model injection and endpoint protocol bridging are not authorized by M1-M4. Clients should send explicit public model ids; any future exception must be disabled by default, scoped to explicit public route ids, and covered by a separate request-path protocol plan.
11. `models onboard-plan` is a local planning command. It does not discover upstream catalogs, stage registry changes, reload runtime, mutate client-token scope, or make a model visible to clients.
12. Runtime reload is allowed only as an explicit `management_mutation` command after `reload status` / `reload diff` and confirmation. Read-only reload commands must not call the production reload handler.

## CLI Contract

### Top-Level Shape

Existing invocation must remain compatible:

```bash
one-ai-key --config config/local.yaml
```

The new explicit server form is allowed:

```bash
one-ai-key serve --config config/local.yaml
```

New operator subcommands:

```text
one-ai-key init local ...
one-ai-key check-config ...
one-ai-key doctor ...
one-ai-key route explain ...
one-ai-key models list|explain ...
one-ai-key client-tokens list ...
one-ai-key keys list|stats|import|probe|probe-apply ...
one-ai-key failures tail|explain ...
one-ai-key models onboard-plan ...
one-ai-key reload status|diff|apply ...
```

### Common Flags

```text
--config <path>                         Local YAML config path.
--management-url <url>                  Running one-ai-key management API origin/base for operator CLI commands. It may share an origin with the client `/v1` base, but it uses `/management/*` endpoints and a management token. Values that include a client `/v1` path are rejected with `reason_code: client_base_url_used_for_management`.
--base-url <url>                        Deprecated compatibility alias for `--management-url`; help must direct users to `--management-url`. If kept for compatibility, it should be hidden from examples and must use the same `/v1` misuse rejection.
--management-token-env <ENV>            Environment variable containing the management token.
--management-token-stdin                Read management token from stdin without echoing it.
--output table|json                     Default: table for humans, json for scripts.
--timeout-seconds <n>                   Default: small bounded timeout.
--limit <n>                             Default: bounded page/window size.
--client-token-ref <id-or-name>         Client token id/name reference. Never pass or echo a raw client token.
--dry-run                               Show plan without writing or probing where applicable.
--yes                                   Execute a mutating or upstream-touching action without interactive confirmation.
--force                                 Allow local file overwrite where that command supports it.
```

Redaction is unconditional. The CLI must not accept `--no-redact`, `--redact=false`, or any equivalent flag that prints secrets.

Execution mode contract:

| Side-effect class | No flags in TTY | No flags in non-TTY | `--dry-run` | `--yes` |
| --- | --- | --- | --- | --- |
| `offline_readonly` | Execute read-only action. | Execute read-only action. | Execute validation/preview if supported. | Rejected unless documented for compatibility. |
| `runtime_readonly` | Execute read-only management calls. | Execute read-only management calls. | Execute read-only preview if supported. | Rejected unless documented for compatibility. |
| `local_write` | Prompt before write. | Exit `3`, no write. | Print local write plan, no write. | Execute write after validation. |
| `upstream_touching` | Prompt before upstream call. | Exit `3`, no upstream call. | Print the bounded probe or explicitly named upstream-touching plan, no upstream call. | Execute the bounded upstream call. |
| `management_mutation` | Prompt before mutation. | Exit `3`, no mutation. | Print mutation plan, no mutation. | Execute the bounded mutation. |

Additional confirmation rules:

- non-TTY execution of a `local_write`, `upstream_touching`, or `management_mutation` command without `--yes` exits `3` and performs no write/probe/upstream request;
- TTY execution may prompt; declining the prompt exits without side effects;
- `--dry-run` and `--yes` are mutually exclusive unless a command explicitly documents a harmless validation-only interpretation;
- `--force` only affects local file overwrite behavior and never bypasses management confirmation or upstream-touching confirmation.
- every concrete invocation/action variant must resolve through a central effect dispatch table before execution. A subcommand may have multiple variants, such as `init local --dry-run` versus `init local --yes`, but each parsed invocation has exactly one primary side-effect class and one explicit `effect_vector`. Variants with multiple risks must union all effect-vector bits and warnings; tests must prove read-only variants can call only read-only method/path allowlists while write/probe variants cannot execute without confirmation.

### Exit Codes

```text
0  success
1  configuration, validation, or operator action error
2  environment, local IO, network, auth, timeout, or malformed management response error
3  command misuse, unsupported flag combination, or missing required confirmation
```

Every non-zero exit should include a stable `reason_code` in JSON output and a short redacted human message in table output.

### Side-Effect Classes

| Class | Examples | Default behavior |
| --- | --- | --- |
| `offline_readonly` | `check-config`, `init local --dry-run` | Never opens network or writes stores. |
| `runtime_readonly` | `doctor`, `route explain`, `models explain`, `client-tokens list`, `keys stats`, `failures tail`, `reload status`, `reload diff` | Uses only GET/read-only management endpoints unless explicitly documented. |
| `local_write` | `init local` | Refuses overwrite unless `--force`; never writes real secrets. |
| `upstream_touching` | confirmed `keys probe` | Dry-run or confirmation required; warns that upstream quota/rate limit may be affected. If the backend persists probe evidence, the effect vector also sets `writes_management_store`. |
| `management_mutation` | `keys import`, `keys probe-apply apply`, `reload apply`, deferred sync-apply/scope actions | Dry-run/plan first; explicit `--yes` or confirmation required. The effect vector must disclose whether the command also mutates active runtime state. If this plan does not define the command task, the command remains deferred. |

Side-effect classes are assigned to concrete action variants, not only to the top-level subcommand name. For example, `init local --dry-run` is `offline_readonly`, while confirmed `init local` is `local_write`. `models onboard-plan --dry-run` is a local planning/read-only variant in M1-M4; confirmed upstream discovery, sync-plan, sync-apply, and client-token scope mutation are deferred to separate plans.

Effect vector contract:

| Bit | Meaning |
| --- | --- |
| `reads_local_files` | Reads local config or source files. |
| `reads_management_runtime` | Calls bounded runtime/read-only management projections. |
| `reads_management_store` | Calls bounded/indexed store-backed read-only management projections. |
| `writes_local_files` | Writes local files. |
| `writes_management_store` | Persists management data, audit events, lifecycle snapshots, imported credentials, or probe evidence. |
| `calls_upstream` | Causes the service or CLI to contact an upstream provider/relay. |
| `mutates_runtime` | Changes active in-memory routing, credential, channel, or registry state. |

Dry-run variants must set all write/upstream/runtime-mutation bits to false. Composite actions are allowed only when the report names every active bit; for example, confirmed `keys probe` is upstream-touching and, with the current management backend, also writes bounded probe evidence to the management store. If confirmed `keys import` appends to both the durable store and the running in-memory pool, it must set `writes_management_store` and `mutates_runtime`; hiding that immediate route eligibility behind a generic “import” label fails this plan.

## Command UX Quality Bar

Every implemented command must satisfy these operator-experience rules before its milestone can close:

- `--help` names the side-effect class and states whether the command can write local files, call upstreams, or mutate management state;
- table output is useful without reading JSON and includes status, reason, affected resource, and next action;
- JSON output is stable enough for scripts and includes `status`, `reason_code`, and command-specific structured fields;
- errors distinguish configuration mistakes, auth failures, network failures, permission failures, and missing confirmation;
- read-only commands never suggest a mutating action without naming the exact follow-up command and its dry-run form;
- mutating commands show a redacted plan before execution and never accept raw upstream keys or raw client tokens as positional arguments;
- commands with pagination or event windows state whether the result is complete, bounded, or only the latest window;
- command output must not require the operator to understand internal Rust module names or storage internals before taking the next safe action.

## File Map

Planned new or modified files:

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Keep legacy server invocation, call CLI dispatcher, and keep server route registration. Avoid adding command business logic. |
| `src/cli.rs` | clap structs/enums, common flags, dispatch to focused modules. |
| `src/cli_effects.rs` | Side-effect class and effect-vector classification for parsed command variants, confirmation gates, and method/path allowlists. |
| `src/config_diagnostics.rs` | Offline config diagnostics, dry-run registry/template validation, local key-file and store inspection without writes. |
| `src/cli_report.rs` | Redacted report models, table/json rendering, exit-code/reason-code mapping. |
| `src/operator_client.rs` | CLI-side management API client: base URL normalization, bearer token injection, timeout, JSON decode, pagination, error mapping. |
| `src/cli_commands/mod.rs` | Declares focused CLI command modules and shared command helpers. |
| `src/cli_commands/route.rs` | `route explain` wrapper over backend route preview. |
| `src/cli_commands/models.rs` | `models list` and `models explain` wrappers over compiled runtime/model-route projections. |
| `src/cli_commands/client_tokens.rs` | `client-tokens list` wrapper over bounded token reference projections. |
| `src/cli_commands/keys.rs` | `keys list`, `keys stats`, `keys import`, `keys probe`, and `keys probe-apply` workflows. |
| `src/cli_commands/failures.rs` | `failures tail` and `failures explain` wrappers over bounded telemetry/filter event projections. |
| `src/cli_commands/doctor.rs` | `doctor` bounded aggregator over runtime and explicitly classified store-backed projections. |
| `src/cli_commands/models_onboard.rs` | M1-M4 planning-only `runtime_readonly` model route onboarding report; no upstream discovery, sync-plan execution, reload, scope mutation, or visibility change. |
| `src/cli_commands/reload.rs` | `reload status`, `reload diff`, and explicit `reload apply` wrappers over management runtime projections. |
| `src/operator_templates.rs` | `init local` and static provider/relay authoring templates. |
| `src/management_credential_refs.rs` | Non-secret credential reference projection and server-side resolver. Do not expose key-derived internal ids through the CLI. |
| `src/management_runtime_diff.rs` | Backend redacted staged-vs-active runtime diff endpoint for later CLI wrapping. |
| `src/endpoint_capabilities.rs` | Static endpoint capability model and projections. |
| `src/responses_adapter.rs` | Deferred; do not create or modify during M1-M4. Requires a separate protocol plan before implementation. |
| `src/runtime_counters.rs` | Deferred; do not create or modify during M1-M4. Requires D2 decision record before implementation. |
| `docs/plans/operator-ux-implementation-plan.md` | This plan. |
| `README.md`, `docs/configuration.md`, `docs/technical-design.md`, `docs/architecture.md` | Update only when a milestone changes public usage or runtime contract. |

`config.rs`, `registry.rs`, `provider.rs`, `proxy.rs`, `route_plan.rs`, and `management_*` modules should be touched only where the milestone explicitly needs backend or request-path semantics.

## Client Compatibility Diagnostic Matrix

This matrix informs diagnostics and later protocol work. It is not a promise of full client support or protocol emulation.

| Client class | Diagnostic boundary | Diagnostic focus |
| --- | --- | --- |
| OpenWebUI-like | `GET /v1/models`, streaming/non-streaming `/v1/chat/completions`, optional `/v1/embeddings` | Empty model list, client scope mismatch, missing public `model_routes`, embedding route/capability mismatch. Responses is not an OpenWebUI-like target surface in this plan. |
| Continue-like | Chat Completions and optional embeddings in M1-M4. Responses use is outside this roadmap unless the client can be configured to avoid it. | Explain whether a public model is suitable for chat/edit/apply/embedding, and whether client-side Responses usage should be disabled. |
| Codex-like Responses clients | Unsupported in M1-M4 except for native upstream pass-through that already exists and is explicitly routed. | Diagnostics may explain that Responses adapter work is parked. Do not claim tool/reasoning/stream-event support without a separate protocol plan and fixtures. |
| Cursor-like | Standard Chat Completions with explicit public model and normal parameter passthrough | Do not expand scope for Cursor-specific agent features or automatic provider-model discovery. |

External client behavior changes quickly. Re-check this matrix when implementing a client-facing milestone; never use it to justify live provider discovery or broad protocol emulation.

## Report Contract

Human table output and JSON output must preserve the same semantic fields. Each report should include enough information to decide the next operator action without exposing secrets.

All command JSON reports must use a common top-level envelope:

| Field | Requirement |
| --- | --- |
| `status` | Stable local status such as `ok`, `degraded`, `blocked`, `partial`, or `error`. |
| `reason` | Short redacted local explanation. It must not quote upstream, management, event, or response-filter text. |
| `reason_code` | Stable machine-readable code. |
| `side_effect_class` | Concrete invocation side-effect class from the central classifier. |
| `effect_vector` | All effect bits for the parsed invocation, including read-only commands. Dry-runs set write/upstream/runtime mutation bits to false. |
| `scope` | Optional structured scope for resource, model, client-token reference, credential set, channel, or endpoint. Scope values must be validated local ids or redacted placeholders. |
| `window` | Required for paged/event/windowed reports. Fields: `kind`, `source`, `limit`, `returned`, `truncated`, optional `cursor`, and optional `bounded_reason`. |
| `next_action` | Structured next action as defined below. |
| `data` | Command-specific payload after redaction and typed projection. |

Table output may be condensed, but it must preserve the same semantics: status, reason code, side-effect class, resource/window completeness when applicable, and next action must be visible without reading JSON.

| Command | Required report fields |
| --- | --- |
| `check-config` | `status`, human `reason`, stable `reason_code`, resource counts, warnings, config path basename or safe relative display path, ignored/deferred store checks, offline `model_visibility_preview`, structured `next_action`. |
| `doctor` | `status`, human `reason`, stable `reason_code`, runtime generation, serving/resilience status from current runtime projections, optional bounded alert/event status when explicitly requested, reload-required flag, structured `next_action`. |
| `route explain` | public model, client-token reference, selected target, candidates, inclusion/exclusion reasons, retry/fallback eligibility, lifecycle/serving projection from current runtime state, structured `next_action`. |
| `models list/explain` | visible public models, client-token reference, route targets, scope reason, `/v1/models` consistency status, capability status if available, structured `next_action`. |
| `client-tokens list` | bounded client-token references by id/name, visible scope summary, active/disabled status, no raw token, token hash, or secret-derived identifier, structured `next_action`. |
| `keys list/stats` | credential set id, lifecycle counts, conditional `latest_probe_summary` only when an existing bounded/indexed projection is available, `probe_summary_status` otherwise, available credential count, no quota/usage/price/balance inference, structured `next_action`. |
| `keys import/probe/probe-apply` | side-effect class, effect vector, dry-run/planned/executed status, target ids or non-secret `credential_ref`, redacted counts/outcomes, audit/result id when available, structured `next_action`. |
| `failures tail/explain` | bounded window metadata, request id when present, model/channel/directive filters, failure stage, failure class, router action, retry eligibility, retry blocked reason, client-visible status, final outcome when present, structured `next_action`. |
| `reload status/diff/apply` | active/staged generation, resource-level redacted diff where available, reload-required flag, last reload status, dry-run/executed status for apply, structured `next_action`. |

`next_action` is a structured object, not only an enum. It should include `summary`, optional `template_id`, optional `safe_argv`, `side_effect_class`, and `requires_confirmation`. If no safe follow-up exists, it must say so explicitly.

`safe_argv` is an array of command arguments, not a shell string. It may be rendered for humans only after allowlisted argv validation and display escaping. Dynamic arguments such as model ids, channel ids, credential refs, request ids, paths, or token refs must pass token-like, URL-like, shell-metacharacter, control-character, and promotional/injection-text rejection before they appear in `safe_argv`. A rendered command is display-only; scripts should consume `safe_argv`.

### Next Action Matrix

Command tests must assert representative next actions, not only redaction. At minimum:

| Situation / reason code | Required next action |
| --- | --- |
| `ok` / healthy | `next_action.summary` states no action required; no mutating command is suggested. |
| `model_not_in_client_scope` | From `doctor`, `failures`, or `route explain`, suggest `models explain ... --client-token-ref ...`. From `models explain` itself, do not loop; state that M1-M4 has no client-token scope mutation CLI and suggest the supported config/staged-registry edit path plus `check-config` / `reload status`, or mark the repair path `deferred_by_m1_m4`. |
| `model_route_missing` | Suggest editing config plus `check-config`, or `models onboard-plan --dry-run` only as a local planning aid; do not imply onboarding exposes the model. |
| `route_target_disabled` | Suggest `route explain` or `reload status`; do not auto-enable channels. |
| `channel_cooling_down` / `channel_degraded` | Suggest `failures tail --last <n>` or `route explain`; do not probe unless the report can name a specific credential and dry-run command. |
| `credential_set_transition_required` | Suggest `keys import ... --dry-run` and `keys stats`; confirmation required before import. |
| `credential_probe_available` | Suggest `keys probe-apply plan --credential-set ... --credential-ref ...`; applying requires confirmation and a probe result reference/precondition. |
| `management_token_missing` | Suggest setting `--management-token-env` or `--management-token-stdin`; do not print any token value. |
| `reload_required` | Suggest `reload status`, `reload diff`, and after M4.1c `reload apply --dry-run`; before M4.1c, state `reload_apply_unavailable_until_m4_1c` rather than implying the diff applies changes. |
| `reload_diff_unavailable_without_staged_projection` | State that no staged projection is available for diffing. Suggest `reload status` and the supported config/restart or staged-registry preparation path; do not suggest `reload apply` unless M4.1c is implemented and a staged generation is present. |
| `capability_status: unavailable_until_m4` | State that endpoint capability diagnostics are unavailable until M4; do not guess provider support. |

### Failure Explanation Taxonomy

`failures explain` and any report that summarizes a failed request must use this fixed schema instead of free-form log text:

| Field | Required values / notes |
| --- | --- |
| `stage` | `client_auth`, `model_visibility`, `route_planning`, `credential_selection`, `upstream_transport`, `upstream_response_guard`, `response_filter`, `post_output`, `management_projection`, or `unknown`. |
| `public_model` | Local public model id when known; omit or set `unknown` rather than echoing untrusted upstream text. |
| `client_token_ref` | Non-secret token reference when known; never raw token or token hash. |
| `selected_target` | Redacted route target id/channel id when known. |
| `failure_class` | Stable local classifier such as `invalid_client_token`, `model_not_visible`, `no_route_candidate`, `credential_unavailable`, `upstream_5xx`, `upstream_timeout`, `response_filter_rejected`, `stream_committed_failure`, or `unknown`. |
| `router_action` | `returned_local_error`, `retried_before_output`, `fell_back_before_output`, `marked_credential`, `marked_channel`, `recorded_event_only`, or `none`. |
| `retry_eligibility` | `eligible_before_output`, `blocked_streaming`, `blocked_bytes_sent`, `blocked_policy`, `blocked_duplicate_charge_risk`, or `not_applicable`. |
| `retry_blocked_reason` | Required when `retry_eligibility` starts with `blocked_`. |
| `client_visible_status` | Local status/code category returned to the client, never raw upstream body text. |

M2.5 tests must cover at least scope miss, no route, credential cooldown/exhausted, upstream 5xx, response filter rejection, and post-output failure. A failure report that only wraps a raw event list without these fields does not satisfy the UX contract.

Per-command tests must cover at least one healthy path and one degraded/error path with a complete `next_action` object and table/JSON semantic parity.

Minimal table skeleton:

```text
Status: degraded
Reason: replacement credentials are needed before this key pool has fallback coverage
Reason code: credential_set_transition_required
Side effect: runtime_readonly
Resource: credential_set:relay_credentials
Visible scope: public_model:gpt-example
Next action: preview replacement credential import before any write; JSON includes structured safe_argv
```

Minimal JSON skeleton:

```json
{
  "status": "degraded",
  "reason": "replacement credentials are needed before this key pool has fallback coverage",
  "reason_code": "credential_set_transition_required",
  "side_effect_class": "runtime_readonly",
  "effect_vector": {
    "reads_local_files": false,
    "reads_management_runtime": true,
    "reads_management_store": true,
    "writes_local_files": false,
    "writes_management_store": false,
    "calls_upstream": false,
    "mutates_runtime": false
  },
  "scope": {"kind": "credential_set", "id": "relay_credentials"},
  "window": null,
  "next_action": {
      "summary": "preview replacement credential import",
      "template_id": "keys_import_dry_run",
      "safe_argv": [
        "one-ai-key",
        "keys",
        "import",
        "--credential-set",
        "relay_credentials",
        "--from-file",
        "<ignored-local-file>",
        "--dry-run"
      ],
      "side_effect_class": "management_mutation",
      "requires_confirmation": true
    },
  "data": {
    "available_credentials": 0,
    "visible_scope": ["public_model:gpt-example"]
  }
}
```

## Operator Examples

These examples are acceptance examples, not secret fixtures. Expected output must be redacted.

```bash
one-ai-key init local --out config/local.yaml --keys data/relay.keys --dry-run
one-ai-key init local --out config/local.yaml --keys data/relay.keys --yes

one-ai-key check-config --config config/local.yaml --output table
one-ai-key check-config --config config/local.yaml --output json

one-ai-key doctor \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN

one-ai-key route explain \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --model gpt-example \
  --client-token-ref local-client

one-ai-key models explain \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --model gpt-example \
  --client-token-ref local-client

one-ai-key client-tokens list \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN

one-ai-key keys stats \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials

one-ai-key keys import \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials \
  --from-file data/new.keys \
  --dry-run

one-ai-key keys import \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials \
  --from-file data/new.keys \
  --yes

one-ai-key keys probe \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials \
  --credential-ref <credential-ref> \
  --dry-run

one-ai-key keys probe \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials \
  --credential-ref <credential-ref> \
  --yes

one-ai-key keys probe-apply plan \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials \
  --credential-ref <credential-ref>

one-ai-key keys probe-apply apply \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials \
  --credential-ref <credential-ref> \
  --probe-result-ref <probe-result-ref> \
  --dry-run

one-ai-key keys probe-apply apply \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials \
  --credential-ref <credential-ref> \
  --probe-result-ref <probe-result-ref> \
  --yes

one-ai-key failures tail \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --last 20 \
  --channel relay

one-ai-key failures explain <request-id> \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --last 50

one-ai-key models onboard-plan \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --channel relay \
  --dry-run

one-ai-key reload status \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN

one-ai-key reload diff \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --output json

one-ai-key reload apply \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --dry-run

one-ai-key reload apply \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --yes
```

Negative examples that must be tested:

```bash
one-ai-key doctor --management-url http://127.0.0.1:4101
# exits 2 with reason_code: management_token_missing

one-ai-key keys import \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --credential-set relay_credentials \
  --from-file data/new.keys
# non-TTY exits 3; TTY prompts. No write occurs unless confirmed.
```

`config/local.yaml`, `data/*.keys`, `.env*`, SQLite files, logs, release artifacts, and generated test snapshots in these examples are local-only artifacts. They must be ignored, created in tempdirs during tests, or represented as placeholders. They must not be committed as fixtures or golden outputs.

### First-Run Transcript And Smoke Requirement

README and M1 docs must include a concise offline first-run transcript that covers:

```bash
one-ai-key init local --out config/local.yaml --keys data/relay.keys --dry-run
one-ai-key init local --out config/local.yaml --keys data/relay.keys --yes
one-ai-key check-config --config config/local.yaml
```

After M2, README must add a served first-run transcript that covers:

```bash
one-ai-key serve --config config/local.yaml
one-ai-key doctor --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN
one-ai-key client-tokens list --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN
one-ai-key models list --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --client-token-ref local-client
curl http://127.0.0.1:4101/v1/models -H 'Authorization: Bearer <client-token>'
curl http://127.0.0.1:4101/v1/chat/completions \
  -H 'Authorization: Bearer <client-token>' \
  -H 'Content-Type: application/json' \
  -d '{"model":"gpt-example","messages":[{"role":"user","content":"ok"}]}'
```

The transcript must explain where the management token reference comes from, how it differs from the client-facing API token, and how the management URL differs from the client OpenAI-compatible Base URL, without printing either token value. The client-side smoke commands must use the client token placeholder and explicit public model id, never the management token. Release-gate smoke tests must run the built binary in a tempdir with a local mock upstream, then call `/v1/models` and one model-bearing request using placeholders and local test servers rather than real upstream keys.

The wording must be precise: the management API URL may share the same origin and port as the client OpenAI-compatible Base URL, but the client base normally includes `/v1` while the management CLI calls `/management/*` and authenticates with the management token.

End-to-end first-run acceptance must compare one source of truth across the loop: for the same tempdir config generated by `init local`, the same configured `client-token-ref`, and the same public route set, `check-config --output json` `model_visibility_preview[].visible_models` must equal the model ids returned by authenticated `GET /v1/models` after the service starts. This comparison must use placeholders/local mock upstreams and must not call upstream `/v1/models`.

---

## Milestone M1: CLI Skeleton, Init Template, And Offline Config Check

**Purpose:** create the CLI foundation and first-run local configuration loop without touching management APIs, upstream providers, or persistent runtime stores.

**Files:**
- Create/modify: `src/cli.rs`
- Create: `src/cli_effects.rs`
- Create/modify: `src/config_diagnostics.rs`
- Create/modify: `src/cli_report.rs`
- Create/modify: `src/operator_templates.rs`
- Modify: `src/main.rs`
- Test: focused unit tests in the new modules plus CLI integration-style tests where practical
- Docs: `README.md`, `docs/configuration.md` only for stable public command syntax

### Task M1.0: Add Side-Effect And Effect-Vector Classification

- [x] **Step 1: Write failing tests in `src/cli_effects.rs`**

  Add tests named:
  - `classifies_check_config_as_offline_readonly`
  - `classifies_init_local_dry_run_as_offline_readonly`
  - `classifies_init_local_yes_as_local_write`
  - `rejects_dry_run_yes_combination`
  - `non_tty_write_without_yes_requires_confirmation`

  Initial expected failure: `SideEffectClass` / `EffectVector` does not exist or command variants are unclassified.

- [x] **Step 2: Implement `src/cli_effects.rs`**

  Define `SideEffectClass`, `EffectVector`, confirmation outcome, and command-variant classification helpers. Keep output formatting in `src/cli_report.rs`, not in this module.

- [x] **Step 3: Wire M1 commands through the classifier**

  `check-config`, `init local --dry-run`, and confirmed `init local` must go through the central classifier before execution.

- [x] **Step 4: Run verification**

  ```bash
  cargo test --locked cli_effects
  cargo check --locked
  ```

- [x] **Step 5: Commit**

  ```bash
  git add src/cli.rs src/cli_effects.rs src/cli_report.rs
  git commit -m "feat(cli): add side-effect classification"
  ```

### Task M1.1: Preserve Server Invocation And Add CLI Dispatch

Status: completed in commit `3cf37457` (`refactor(cli): add operator command skeleton`). Future workers should verify rather than recommit this task.

- [x] **Step 1: Write failing tests for legacy invocation compatibility**

  Cover:
  - `one-ai-key --config config/local.yaml` still selects server mode.
  - `one-ai-key serve --config config/local.yaml` selects server mode.
  - `one-ai-key --help` and subcommand help do not print secrets from env.

- [x] **Step 2: Implement `src/cli.rs` skeleton**

  `src/cli.rs` owns clap command definitions, common flags, and dispatch. `src/main.rs` should delegate and enter the existing server startup path only for legacy or `serve` mode.

- [x] **Step 3: Run verification**

  ```bash
  cargo test --locked cli
  cargo check --locked
  ```

  Expected: CLI tests pass; existing server compilation remains unchanged.

- [x] **Step 4: Commit**

  ```bash
  git add src/main.rs src/cli.rs
  git commit -m "refactor(cli): add operator command skeleton"
  ```

### Task M1.2: Add `init local`

Status: completed in commit `b5ef46df` (`feat(cli): add local init template`). Future workers should verify rather than recommit this task.

- [x] **Step 1: Write failing tests for local template generation**

  Cover:
  - `--dry-run` prints planned relative paths and resource ids only.
  - Real execution creates YAML and empty/placeholder key file when `--yes` is supplied.
  - Existing files are not overwritten unless `--force` is supplied.
  - Generated files contain placeholders, not real keys or tokens.
  - generated local config/key paths are ignored by repo rules or created in tempdirs during tests.
  - generated files, `.env*`, SQLite/WAL/SHM, logs, and release artifacts are never staged by template tests.

- [x] **Step 2: Implement `src/operator_templates.rs`**

  The first template is `init local`. Keep provider/relay template expansion small and static. Do not add a remote template market or live provider catalog.

- [x] **Step 3: Run verification**

  ```bash
  cargo test --locked operator_templates
  ```

- [x] **Step 4: Commit**

  ```bash
  git add src/cli.rs src/operator_templates.rs
  git commit -m "feat(cli): add local init template"
  ```

### Task M1.3: Add Offline `check-config`

- [x] **Step 1: Write failing tests for offline non-mutating behavior**

  Cover:
  - valid config returns exit `0`;
  - unknown fields and dangling references return exit `1` with stable reason codes;
  - missing files, unreadable files, malformed YAML, or environment errors return exit `2`;
  - no network listener is bound;
  - no HTTP request is made;
  - no SQLite database, SQLite WAL/SHM sidecar, JSONL log, or runtime artifact is created;
  - no SQLite migration or schema initialization runs;
  - no bootstrap client token or credential import occurs;
  - offline `model_visibility_preview` reports configured client-token references and visible public model ids;
  - empty model visibility returns a stable reason such as `client_scope_empty`, `model_route_missing`, or `target_channel_disabled`;
  - output does not include raw key material, client tokens, management tokens, token hashes, secret-derived identifiers, or absolute sensitive paths.

- [x] **Step 2: Implement `src/config_diagnostics.rs`**

  Use YAML parsing, upstream shortcut expansion, and registry/config validation where safe. If existing production resolution opens or writes stores, add a separate read-only inspection path instead of calling that code.

- [x] **Step 3: Implement redacted reports in `src/cli_report.rs`**

  Support `--output table|json`. JSON should include stable `status`, `reason_code`, `warnings[]`, summarized resource counts, and `model_visibility_preview[]`. Table output must show at least one line answering whether the generated/default client-token reference would see public models.

- [x] **Step 4: Run verification**

  ```bash
  cargo test --locked config_diagnostics
  cargo test --locked cli_report
  scripts/local-ci.sh
  ```

- [x] **Step 5: Commit**

  ```bash
  git add src/cli.rs src/config_diagnostics.rs src/cli_report.rs README.md docs/configuration.md
  git commit -m "feat(cli): add offline check-config diagnostics"
  ```

**M1 stop card checklist:**

- legacy server invocation is compatible;
- side-effect/effect-vector classification exists for M1 commands;
- `init local` is dry-run/confirm safe;
- `check-config` is offline, non-mutating, redacted, reason-coded, and includes model visibility preview;
- M1 command reports use the common envelope or are listed in the later M2 report-contract reconciliation gate;
- offline first-run docs are updated and pass the docs gate;
- `cargo test --locked <filter> -- --list` evidence proves filtered M1 commands matched intended tests;
- `scripts/local-ci.sh`, staged file review, staged denylist review, and ignore-sync gate pass.

**M1 blockers:** if a truly read-only config/store inspection path cannot be implemented without changing production bootstrap semantics, stop and split that store-inspection work into its own plan.

---

## Milestone M2: Read-Only Management CLI

**Purpose:** expose existing management projections as redacted operator explanations without creating new runtime state or new management mutations.

**Files:**
- Create: `src/operator_client.rs`
- Create: `src/cli_commands/route.rs`
- Create: `src/cli_commands/models.rs`
- Create: `src/cli_commands/client_tokens.rs`
- Create: `src/cli_commands/keys.rs`
- Create: `src/cli_commands/failures.rs`
- Create: `src/cli_commands/doctor.rs`
- Create: `src/management_credential_refs.rs`
- Modify: `src/cli.rs`
- Modify: `src/cli_commands/mod.rs`
- Modify: `src/main.rs`
- Modify: `src/cli_report.rs`
- Modify: `src/operator_client.rs` only when adding missing read-only endpoint enum entries or central sanitization tests
- Test: mock/in-process management API tests and output redaction tests
- Docs: README operations examples after commands stabilize

### Task M2.1: Add CLI-Side Operator Client

- [x] **Step 1: Write failing tests for HTTP boundary**

  Cover:
  - base URL normalization;
  - `--management-url` is the primary flag and `--base-url` is only a deprecated compatibility alias;
  - help text says management URL may share origin with the client `/v1` base but uses `/management/*` endpoints and a management token;
  - `--management-url` or deprecated `--base-url` values containing a client `/v1` path are rejected with `reason_code: client_base_url_used_for_management`;
  - management bearer token injection from env/stdin;
  - no token in stdout/stderr/errors;
  - timeout and non-JSON error mapping;
  - 401/403/404 reason-code mapping;
  - malicious JSON error messages, HTML/non-JSON bodies, encoded promotional text, and control-character payloads are mapped to local redacted reason codes and never copied into reports, logs, docs, or fixtures;
  - method/path allowlist for read-only commands.

- [x] **Step 2: Implement `src/operator_client.rs`**

  This module owns HTTP transport only. It should not format business reports and should not know command-specific semantics beyond safe method helpers.

- [x] **Step 3: Run verification**

  ```bash
  cargo test --locked operator_client
  cargo test --locked cli_report
  ```

- [x] **Step 4: Commit**

  ```bash
  git add src/cli.rs src/operator_client.rs src/cli_report.rs
  git commit -m "feat(cli): add operator API client"
  ```

### Task M2.2: Add `route explain`

- [x] **Step 1: Write failing tests**

  Assert it calls only `GET /management/routing/preview` with the requested model and client-token reference, then prints selected target, candidates, inclusion/exclusion reasons, `reload_diff_status: unavailable_until_m4`, and lifecycle/serving projection from current runtime state without secrets. It must not trigger active health checks, probes, discovery, or credential scans.

- [x] **Step 2: Implement command**

  Do not reimplement route planning in the CLI.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked route_explain
  git add src/cli.rs src/cli_commands/route.rs src/cli_report.rs
  git commit -m "feat(cli): add route explain command"
  ```

### Task M2.3: Add `client-tokens list`, `models list`, And `models explain`

- [x] **Step 1: Write failing tests**

  Cover:
  - `client-tokens list` shows only bounded token references by id/name and scope summary;
  - client-token output never includes raw client token, token hash, bearer token, or secret-derived identifier;
  - list from compiled runtime/model-route projections;
  - explain public model visibility for a client-token reference, not a raw token;
  - explain target channel/upstream model mapping;
  - management visibility explanation is consistent with authenticated client `GET /v1/models` projection;
  - for a config produced by `init local`, `check-config --output json` model visibility preview for the same client-token reference matches authenticated `GET /v1/models` after serving with a local mock upstream;
  - endpoint capability output is `unavailable_until_m4` before capability matrix is implemented;
  - reload/staged-vs-active output is `reload_diff_status: unavailable_until_m4` before reload diff is implemented;
  - do not call upstream `/v1/models`;
  - bounded preview calls, no all-model all-token matrix by default.

- [x] **Step 2: Implement command**

  Use `/management/client-tokens`, `/management/model-routes`, and single-model routing preview when a model is specified. The CLI must not infer token refs from raw token material and must not enumerate all model-token combinations by default. Before M4, staged-vs-active and endpoint capability fields must be explicit unavailable statuses rather than guessed values.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked models_explain
  git add src/cli.rs src/cli_commands/client_tokens.rs src/cli_commands/models.rs src/cli_report.rs
  git commit -m "feat(cli): add model and client-token explanation commands"
  ```

### Task M2.4a: Add Non-Secret Credential Reference Projection And Resolver

- [x] **Step 1: Write failing backend tests**

  Add tests named:
  - `credential_ref_projection_is_not_key_derived`
  - `credential_ref_is_stable_across_restart`
  - `credential_ref_resolves_server_side_without_cli_internal_id`
  - `credential_ref_rejects_unknown_or_cross_set_reference`
  - `credential_ref_projection_omits_fingerprint_hash_prefix_suffix`

  Initial expected failure: management projections expose only internal `credential_id` / fingerprint style fields and individual credential routes cannot resolve a non-secret operator reference.

- [x] **Step 2: Implement `src/management_credential_refs.rs`**

  Define a non-secret operator reference contract and server-side resolver. The reference must come from store/import/resource metadata or another non-secret stable resource id, not raw key material, hash, HMAC, prefix, suffix, fingerprint, or current key-derived `credential_id`. If no safe reference source exists in the current store, stop M2.4a and mark M3.2/M3.3 blocked; do not fall back to printing internal ids.

- [x] **Step 3: Integrate projections**

  Read-only credential-set and credential-resource management projections may include `credential_ref`, but CLI wrappers must still drop internal ids and fingerprints. Existing internal-id management routes may remain for backward compatibility, but M1-M4 CLI must not require or display them.

- [x] **Step 4: Verify and commit**

  ```bash
  cargo test --locked credential_ref
  git add src/management_credential_refs.rs src/management_credentials.rs docs/architecture.md
  git commit -m "feat(management): add credential references"
  ```

### Task M2.4b: Add Read-Only `keys list` And `keys stats`

- [x] **Step 1: Write failing tests**

  Cover:
  - credential-set summary from management projections;
  - lifecycle counts from bounded management projections;
  - bounded non-secret `credential_ref` values from M2.4a are shown for follow-up commands;
  - CLI output drops backend `credential_id`, fingerprint, raw hash, prefix/suffix, HMAC, and other secret-derived fields even if the management projection returns them;
  - latest probe summaries only when an existing bounded/indexed summary projection is available;
  - when no bounded summary projection exists, output `probe_summary_status: unavailable_without_summary_projection`;
  - no raw keys, raw source paths, token hashes, fingerprints, HMACs, prefixes/suffixes, or other secret-derived display identifiers;
  - stats must prefer summary endpoints and must not page-scan or CLI-aggregate an unbounded credential set by default.

- [x] **Step 2: Implement command**

  Use `/management/credential-sets`, `/management/credential-sets/:id/operations`, and bounded `/credentials` pages only when explicitly requested. If M2.4a did not pass, stop here and do not implement `keys probe` / `keys probe-apply`. If a richer latest-probe aggregate is required, stop and add a separate backend read-only aggregate endpoint with indexed/low-cardinality/bounded tests, no migration-on-read, no upstream calls, and no writes. Do not compute that aggregate in the CLI by scanning pages.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked keys_stats
  git add src/cli.rs src/cli_commands/keys.rs src/cli_report.rs
  git commit -m "feat(cli): add read-only key pool commands"
  ```

### Task M2.5: Add `failures tail` And `failures explain`

- [x] **Step 1: Write failing tests**

  Cover:
  - bounded `--last` default;
  - filtering by request id, model, channel, or directive;
  - ring-buffer semantics are clear and do not imply full audit history;
  - no raw bodies, matched text, keys, or tokens;
  - fixed taxonomy fields `stage`, `failure_class`, `router_action`, `retry_eligibility`, `retry_blocked_reason`, and `client_visible_status`;
  - scope miss, no route, credential cooldown/exhausted, upstream 5xx, response filter rejection, and post-output failure use stable reason codes;
  - `failures explain` includes next actions to `models explain`, `route explain`, or `keys stats` only when the event has enough bounded local resource context;
  - during M2 it must not suggest executable `keys probe` commands because M3 has not implemented that workflow yet. If an event has enough credential context, report `keys_probe_unavailable_until_m3` or point to `keys stats`.

- [x] **Step 2: Implement command**

  Wrap `/management/routing-telemetry` and `/management/response-filter-events`.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked failures_cli
  git diff --cached --name-only
  git add src/cli.rs src/main.rs src/cli_commands/mod.rs src/cli_commands/failures.rs src/cli_effects.rs src/cli_report.rs src/operator_client.rs
  git diff --cached --name-only
  git commit -m "feat(cli): add failure event commands"
  ```

### Task M2.6: Add Read-Only `doctor`

Dependency: M2.5 must be implemented and committed first. `doctor` may suggest `failures tail --last <n>` only after that command exists in the released CLI surface.

- [x] **Step 1: Write failing tests**

  Default doctor calls only:
  - `GET /management/explain/runtime`;
  - `GET /management/runtime`;
  - `GET /management/health/serving`;
  - `GET /management/health/resilience`.

  `/management/alerts` is opt-in through `--include-alerts`. It is allowed only if tests prove no migration-on-read, no writes, no upstream calls, bounded/indexed reads, redaction, and graceful degradation when that projection is unavailable. Events, model routes, and routing preview are opt-in through explicit flags such as `--include-events`, `--include-routes`, or `--model`.

  Also cover malicious management error bodies and partial fan-out failures: upstream/management JSON messages, HTML, encoded promotional text, and control characters must be discarded and mapped to local reason codes.

- [x] **Step 2: Implement command**

  Handle partial management failures without printing tokens. Do not probe, discover, reload, mutate, page-scan credential sets, call alerts/events by default, or enumerate all model-token combinations. If doctor does not include event evidence, it must return an explicit `next_action` to `failures tail --last <n>` instead of implying it has diagnosed the incident. If repeated fan-out becomes expensive, stop and add a separate bounded composite management endpoint rather than hiding broader enumeration in the CLI.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked doctor_cli
  scripts/local-ci.sh
  git diff --cached --name-only
  git add src/cli.rs src/main.rs src/cli_commands/mod.rs src/cli_commands/doctor.rs src/cli_effects.rs src/cli_report.rs src/operator_client.rs README.md
  git diff --cached --name-only
  git commit -m "feat(cli): add read-only doctor command"
  ```

**M2 stop card checklist:**

Status: completed after commits `242ecbc9` (`fix(cli): reconcile M2 report contract`) and `c1523f4d` (`fix(cli): harden keys safe argv validation`), plus the follow-up plan-status verification. Evidence included targeted filtered tests, non-zero `cargo test --locked <filter> -- --list` discovery for M2 filters, `cargo test --locked`, `scripts/local-ci.sh`, README cold-read, staged file review, staged denylist review, and ignore-sync review.

- all read-only commands have method/path tests, auth/error mapping tests, redaction tests, bounded pagination/window defaults, and no write/upstream/reload/runtime-mutation effect bits;
- completed M1/M2 commands are reconciled against the unified report envelope: `init local`, `check-config`, `route explain`, `models list`, `models explain`, `client-tokens list`, `keys list`, `keys stats`, `failures tail`, `failures explain`, and `doctor`;
- JSON/table parity tests cover `status`, `reason_code`, `side_effect_class`, `effect_vector`, `scope` or `window` when applicable, and structured `next_action.safe_argv`;
- `credential_ref` projection/resolver gate passes or explicitly blocks dependent M3 tasks;
- `--management-url` / deprecated `--base-url` misuse with a client `/v1` path is rejected with `client_base_url_used_for_management`;
- README operations examples remain under operations/troubleshooting and do not shift the product first screen toward key replacement, probe, or health-check narratives;
- `cargo test --locked <filter> -- --list` evidence proves each filtered M2 test command matched intended tests;
- `scripts/local-ci.sh`, docs cold-read, staged file review, staged denylist review, and ignore-sync gate pass.

**M2 blockers:** if a command needs a new backend endpoint to avoid duplicating server logic, stop and split that endpoint into a separate task rather than reimplementing runtime semantics in the CLI.

---

## Milestone M3: Explicit Operator Workflows

**Purpose:** add daily maintenance commands that can write local files, touch upstreams, or mutate management state, with dry-run, confirmation, audit, and redaction boundaries.

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/operator_client.rs`
- Modify: `src/cli_report.rs`
- Modify: `src/cli_commands/keys.rs`
- Create: `src/cli_commands/models_onboard.rs`
- Docs: README and configuration examples for stable workflows

### Task M3.1: Add `keys import`

- [x] **Step 1: Write failing tests for CLI-local import preview**

  Cover:
  - `--dry-run` parses source and reports line count, duplicate count when knowable, target credential set, and store mode;
  - dry-run is labeled `local_preview` when it cannot know durable-store duplicates without calling management;
  - dry-run states whether confirmed import would become immediately route-eligible under the current backend;
  - confirmed import effect vector includes `writes_management_store` and, if current backend applies imported credentials to the running pool, `mutates_runtime`;
  - no raw key is printed, including malformed/failing lines;
  - real import requires `--yes` or confirmation;
  - readonly stores fail before source secrets are sent to management;
  - report states whether the operation has automatic rollback; if not, it names the supported recovery path.

- [x] **Step 2: Implement command**

  The first implementation uses CLI-local dry-run. Do not add a backend preview endpoint in the same commit. Current management import semantics append credentials to the writable store and may append to the running in-memory pool when confirmed; the CLI must disclose that confirmed imports can become route-eligible immediately and must report that as `mutates_runtime`. Candidate/staged credential import is deferred to a separate lifecycle plan because it changes credential-state semantics.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked keys_import
  git add src/cli.rs src/cli_commands/keys.rs src/operator_client.rs src/cli_report.rs
  git commit -m "feat(cli): add key import workflow"
  ```

### Task M3.2: Add `keys probe`

Dependency: M2.4a must be complete. If non-secret `credential_ref` cannot be implemented safely, this task remains unchecked and blocked; do not substitute internal credential ids.

- [x] **Step 1: Write failing tests**

  Cover:
  - probe is classified as upstream-touching;
  - confirmed probe effect vector sets both `calls_upstream` and `writes_management_store` when the backend persists probe evidence;
  - probe requires one explicit non-secret `credential_ref`;
  - default probe plan covers exactly one credential and never probes all credentials by default;
  - `--dry-run` prints the channel, `credential_ref`, endpoint/probe mode, and quota/rate-limit warning without sending an upstream request;
  - no upstream request occurs without `--yes` or confirmation;
  - non-TTY execution without `--yes` exits `3`;
  - output shows `credential_ref`, outcome, status/code/limit type, channel id, and timestamp only;
  - no key-derived fingerprint, hash, prefix/suffix, HMAC, or stable secret-derived identifier is displayed;
  - request/response bodies are never displayed.

- [x] **Step 2: Implement command**

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked keys_probe
  git add src/cli.rs src/cli_commands/keys.rs src/operator_client.rs src/cli_report.rs
  git commit -m "feat(cli): add credential probe workflow"
  ```

### Task M3.3: Add Explicit `keys probe-apply`

Dependency: M2.4a must be complete. If non-secret `credential_ref` cannot be implemented safely, this task remains unchecked and blocked; do not substitute internal credential ids.

**Files:**
- Backend plan endpoint: `src/main.rs`, `src/management.rs`, `src/management_credentials.rs`, `src/management_errors.rs` if needed, `docs/architecture.md`.
- CLI wrapper: `src/cli.rs`, `src/cli_commands/keys.rs`, `src/operator_client.rs`, `src/cli_report.rs`.

- [x] **Step 1: Write failing backend plan tests**

  Cover:
  - read-only apply plan support;
  - exact endpoint or equivalent route accepts a non-secret `credential_ref` and resolves it server-side to the internal credential identity with minimum role `readonly`;
  - optional bounded bulk-plan endpoint may exist only if it requires a specific probe filter and limit;
  - bulk apply requires a specific filter and limit;
  - no broad `all` or `unprobed` apply by default;
  - no lifecycle mutation, audit mutation record, or runtime state change occurs while building the plan;
  - plan output includes the `probe_result_id` or equivalent non-secret probe record reference that the mutating apply must precondition on;
  - plan output is redacted and never includes probe raw request/response bodies.

- [x] **Step 2: Add or expose backend read-only apply plan**

  The mutating CLI path must not be implemented until a read-only apply plan is available. If current APIs already expose a safe plan, expose that as a documented management projection. Otherwise add the exact backend read-only plan endpoint above with management route matrix coverage, redaction tests, and no lifecycle mutation. Dry-run must never call the mutating `POST .../apply-latest-probe` endpoint.

- [x] **Step 3: Verify and commit backend plan**

  ```bash
  cargo test --locked keys_probe_apply_plan
  git add src/main.rs src/management_credentials.rs docs/architecture.md
  git commit -m "feat(management): add probe apply plan"
  ```

- [x] **Step 4: Add CLI dry-run wrapper**

  `keys probe-apply plan --credential-set <id> --credential-ref <ref>` and `keys probe-apply apply --dry-run ...` must call only the read-only plan projection and must not call the mutating endpoint.

- [x] **Step 5: Verify and commit dry-run CLI**

  ```bash
  cargo test --locked keys_probe_apply_dry_run
  git add src/cli.rs src/cli_commands/keys.rs src/operator_client.rs src/cli_report.rs
  git commit -m "feat(cli): add probe apply dry run"
  ```

- [x] **Step 6: Add mutating apply CLI**

  Real apply requires a specific non-secret `credential_ref`, the planned `--probe-result-ref <probe-result-ref>` precondition, and `--yes` or interactive confirmation. Summarize management response and audit/result ids without secrets. If the latest probe changed after the plan, the apply path must fail closed and ask the operator to rerun the plan.

- [x] **Step 7: Verify and commit mutating CLI**

  ```bash
  cargo test --locked keys_probe_apply
  git add src/cli.rs src/cli_commands/keys.rs src/operator_client.rs src/cli_report.rs
  git commit -m "feat(cli): add probe apply workflow"
  ```

### Task M3.4: Add Local Model Route Onboarding Plan

Effect class: `runtime_readonly` management-plane planning. The command may read bounded compiled runtime/model-route projections to explain current exposure, but it remains planning-only: no upstream discovery, no model-discovery, no sync-plan/sync-apply, no reload, no client-token scope mutation, and no local config write.

- [x] **Step 1: Write failing tests**

  Cover:
  - `models onboard-plan --dry-run` is planning-only `runtime_readonly` and does not call upstream discovery, model-discovery, sync-plan, sync-apply, reload, client-token scope mutation, or local file writes;
  - it calls only allowlisted read-only management projections, such as compiled model routes or channel projections when available;
  - if the needed read-only projection is unavailable, it reports `onboard_plan_projection_unavailable` rather than probing, discovering, or mutating;
  - no `--discover`, `--sync-plan`, `--sync-apply`, reload apply, or client-token scope update variant is implemented in M1-M4;
  - output differentiates configured/exposed public models from merely possible upstream onboarding ideas;
  - output states that the result is only an onboarding plan and does not make any model visible to clients;
  - output includes `planning_only_no_visibility_change`;
  - every later action is printed as deferred unless this plan defines a concrete dry-run command;
  - no live `/v1/models` aggregation or request-path dependency is introduced.

- [x] **Step 2: Implement local dry-run report**

  `models onboard-plan --dry-run` reports the channel id, configured credential-set id, currently exposed public routes when available from read-only management APIs, and which explicit local config/model-route changes would be required before a model could become client-visible. It must not call `model-discovery` or `sync-plan`.

  This command prepares an onboarding plan. It does not by itself discover an upstream catalog entry or make a model visible to clients. The report must say which public route, staged registry change, reload, or client-token scope action is still required and whether that action is outside M1-M4.

- [x] **Step 3: Reject confirmed live-discovery/sync-plan variants**

  If a user passes `--discover`, `--sync-plan`, `--sync-apply`, or equivalent confirmed variants during M1-M4, return exit `3` with `reason_code: deferred_to_separate_plan` and no upstream or management mutation.

- [x] **Step 4: Verify and commit**

  ```bash
  cargo test --locked models_onboard
  scripts/local-ci.sh
  git add src/cli.rs src/cli_commands/models_onboard.rs src/operator_client.rs src/cli_report.rs README.md docs/configuration.md
  git commit -m "feat(cli): add model route onboarding plan"
  ```

**M3 stop card checklist:**

- every write, runtime-mutating, or upstream-touching command has dry-run/plan behavior, explicit confirmation, effect-vector reporting, redaction tests, local mock/in-process management tests, and local CI passes;
- key workflows are serialized or implemented through committed split modules; no parallel lane may edit the same `keys` command module without a main-controller integration commit;
- `keys import`, `keys probe`, and `keys probe-apply` reports include reversibility or explicit manual recovery instructions;
- `models onboard-plan` is `runtime_readonly` and planning-only, with no upstream discovery, no reload, no scope mutation, and no visibility change;
- README/configuration examples stay in operations/troubleshooting sections and do not make key replacement, probe, or health check the product's main narrative;
- `cargo test --locked <filter> -- --list` evidence proves each filtered M3 test command matched intended tests;
- `scripts/local-ci.sh`, docs cold-read, staged file review, staged denylist review, and ignore-sync gate pass.

**M3 blockers:** if any workflow needs to couple discovery apply, runtime reload, and client-token scope mutation into one command to be useful, stop and redesign; that coupling violates the current management boundary.

---

## Milestone M4: Safe Backend Additions And Reload Closure

**Purpose:** add backend projections, explicit reload UX, and static compatibility metadata that improve explanation and preflight without changing protocol semantics prematurely.

### Task M4.1a: Add `reload status`

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/cli_commands/reload.rs`
- Modify: `src/operator_client.rs`
- Modify: `src/cli_report.rs`
- Docs: `README.md`, `docs/architecture.md`

- [x] **Step 1: Write failing tests**

  Add tests named:
  - `reload_status_calls_runtime_projection_only`
  - `reload_status_reports_active_and_staged_generation`
  - `reload_status_reports_last_reload_result_without_raw_errors`
  - `reload_status_suggests_only_available_reload_commands_without_mutating`

  Initial expected failure: no CLI command summarizes reload state as an operator status report.

- [x] **Step 2: Implement CLI wrapper**

  `one-ai-key reload status` wraps existing read-only runtime/explain projections such as `/management/runtime` and `/management/explain/runtime`. It must not call `POST /management/runtime/reload`, must not compute staged registry semantics itself, and must not inspect YAML or stores in the CLI. Its `next_action` must be phase-aware: before `reload diff` or `reload apply` is implemented, it reports the command as unavailable rather than suggesting it as executable; when no staged projection exists, it suggests only `reload status` and the supported config/restart or staged-registry preparation path.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked reload_status_cli
  git add src/cli.rs src/cli_commands/reload.rs src/operator_client.rs src/cli_report.rs README.md docs/architecture.md
  git commit -m "feat(cli): add reload status command"
  ```

### Task M4.1b0: Add Reload Diff Availability Contract

**Files:**
- Create/modify: `src/management_runtime_diff.rs`
- Modify: `src/main.rs`
- Modify: `src/management_runtime.rs`
- Modify: `src/cli.rs`
- Test: management endpoint and CLI tests

- [x] **Step 1: Write failing tests**

  Add tests named:
  - `runtime_reload_diff_reports_unavailable_without_staged_projection`
  - `runtime_reload_diff_status_is_readonly`
  - `reload_diff_cli_reports_unavailable_without_suggesting_apply`

  Initial expected failure: no stable read-only diff availability contract exists.

  Cover:
  - read-only endpoint `GET /management/runtime/reload-diff` with minimum role `readonly`;
  - route registration and `MANAGEMENT_ROUTE_SPECS` coverage for the new endpoint;
  - when no staged/validated projection exists, it returns `status: blocked` or `status: unavailable`, `reason_code: unavailable_without_staged_projection`, active generation metadata when available, and a safe next action that does not suggest `reload apply`;
  - no raw YAML diff, keys, DSNs, token-like URLs, absolute paths, or client tokens;
  - calling reload diff status does not reload runtime, write audit events, update reload timestamps, create staged-registry state, or inspect YAML/stores in the CLI;
  - no upstream discovery, production reload path, audit write, timestamp update, store migration, or active runtime rebuild occurs.

- [x] **Step 2: Implement availability endpoint and CLI wrapper**

  This closes the UX gap when staged diff data is unavailable. It must not create a new persistent staged-registry subsystem. The CLI wraps the backend response and must not recompute registry semantics.

- [x] **Step 3: Verify and commit availability contract**

  ```bash
  cargo test --locked runtime_reload_diff_reports_unavailable
  cargo test --locked reload_diff_cli
  git diff --cached --name-only
  git add src/main.rs src/management_runtime.rs src/management_runtime_diff.rs src/cli.rs src/cli_commands/reload.rs src/operator_client.rs src/cli_report.rs docs/architecture.md README.md
  git diff --cached --name-only
  git commit -m "feat(management): add reload diff availability contract"
  ```

### Task M4.1b1: Add Redacted Reload Diff When A Staged Projection Exists

Dependency: M4.1b0 must be complete. This task proceeds only if an existing staged/validated runtime projection or document is already available. It must not create a new staged-registry subsystem merely to satisfy diff output. If no such projection exists, leave M4.1b1 unchecked and close M4.1b0 as the supported `unavailable_without_staged_projection` UX.

**Files:**
- Modify: `src/management_runtime_diff.rs`
- Modify: `src/management_runtime.rs`
- Modify: `src/cli_commands/reload.rs`
- Modify: `src/cli_commands/models.rs`
- Modify: `src/cli_commands/route.rs`
- Modify: `src/cli_report.rs`
- Test: management endpoint and CLI tests

- [x] **Step 1: Write failing tests**

  Add tests named:
  - `runtime_reload_diff_is_readonly`
  - `runtime_reload_diff_is_bounded_and_redacted`
  - `runtime_reload_diff_does_not_invoke_reload`
  - `reload_diff_cli_wraps_backend_projection_only`

  Cover:
  - staged-vs-active typed diff over providers, accounts, credential sets, channels, model routes, policy profiles, and routing profiles;
  - no raw YAML diff, keys, DSNs, token-like URLs, absolute paths, or client tokens;
  - CLI wraps the backend diff instead of recomputing registry semantics;
  - calling reload diff does not reload runtime, write audit events, update reload timestamps, or change staged/active registry state;
  - diff reads only active compiled runtime snapshot and an already staged/validated registry projection or document;
  - if the diff exceeds the configured budget, it returns a redacted summary and stable `reason_code` rather than expanding unbounded details.

- [x] **Step 2: Implement typed backend diff**

  Add redacted DTOs and endpoint tests for the typed diff. The endpoint must use a bounded structural diff between current runtime projection and an already staged projection; it must not invoke reload code to obtain the comparison state.

- [x] **Step 3: Verify and commit backend diff**

  ```bash
  cargo test --locked runtime_diff
  git add src/main.rs src/management_runtime.rs src/management_runtime_diff.rs docs/architecture.md
  git commit -m "feat(management): add redacted reload diff"
  ```

- [x] **Step 4: Add CLI wrapper**

  Wrap `GET /management/runtime/reload-diff` in `one-ai-key reload diff`. The CLI must not compute diff semantics itself. Also update `models explain` and `route explain` to replace `reload_diff_status: unavailable_until_m4` with active/staged generation, reload-required status, and a `reload diff` next action when relevant.

- [x] **Step 5: Verify and commit CLI wrapper**

  ```bash
  cargo test --locked reload_diff_cli
  git add src/cli.rs src/cli_commands/reload.rs src/cli_commands/models.rs src/cli_commands/route.rs src/operator_client.rs src/cli_report.rs README.md
  git commit -m "feat(cli): add reload diff command"
  ```

  Completed in `6acfd186 feat(cli): add reload diff command`.

### Task M4.1c: Add Explicit `reload apply`

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/cli_commands/reload.rs`
- Modify: `src/operator_client.rs`
- Modify: `src/cli_report.rs`
- Docs: `README.md`, `docs/architecture.md`

- [x] **Step 1: Write failing tests**

  Add tests named:
  - `reload_apply_dry_run_does_not_post_reload`
  - `reload_apply_requires_confirmation_in_non_tty`
  - `reload_apply_yes_posts_runtime_reload_once_when_precondition_supported`
  - `reload_apply_fails_closed_without_generation_precondition`
  - `reload_apply_fails_closed_when_generation_changed_after_plan`
  - `reload_apply_reports_mutates_runtime_effect_vector`
  - `reload_apply_reports_last_reload_failure_redacted`

  Initial expected failure: there is no explicit operator CLI wrapper for runtime reload apply.

- [x] **Step 2: Implement dry-run and confirmation**

  `reload apply --dry-run` summarizes what would be applied using `reload status` and `reload diff` data when available, then exits without POSTing. The dry-run report must include the active/staged generation or a plan id.

  Real execution requires `--yes` or interactive confirmation and an enforceable expected-generation or plan precondition. If the current backend reload endpoint lacks that precondition, the mutating CLI path must fail closed with `reason_code: reload_apply_precondition_unavailable` and must not POST reload. A worker may add a backend precondition endpoint or request parameter only as a separate backend subtask with tests proving generation mismatch fails before runtime mutation.

  If the staged generation changes after the plan, the command must fail closed and ask the operator to rerun `reload status` / `reload diff`. It must report `side_effect_class: management_mutation` and `effect_vector.mutates_runtime: true`.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked reload_apply_cli
  git add src/cli.rs src/cli_commands/reload.rs src/operator_client.rs src/cli_report.rs README.md docs/architecture.md
  git commit -m "feat(cli): add explicit reload apply"
  ```

  Completed in `ec32173e feat(cli): add explicit reload apply`.

### Task M4.2a: Add Static Endpoint Capability Schema

**Files:**
- Create: `src/endpoint_capabilities.rs`
- Modify: `src/config.rs`
- Modify: `src/provider.rs`
- Modify: `src/upstream_templates.rs`
- Docs: `docs/configuration.md`, `docs/technical-design.md`

- [x] **Step 1: Write failing tests**

  Add tests named:
  - `endpoint_capabilities_parse_static_config`
  - `endpoint_capabilities_expand_template_defaults`
  - `endpoint_capabilities_do_not_create_provider_catalog`
  - `endpoint_capabilities_do_not_affect_request_path`

  Cover static capabilities for endpoint kinds only: chat completions, responses, embeddings, local models projection, and diagnostic labels. The schema must be a small local config/template model, not provider/model catalog data.

- [x] **Step 2: Implement static capability config model**

  Add config structs, template expansion, provider defaults, and resolver validation. Do not add request-path behavior, dynamic health detection, model-level provider catalogs, or live probing.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked endpoint_capabilities
  git add src/endpoint_capabilities.rs src/config.rs src/provider.rs src/upstream_templates.rs docs/configuration.md docs/technical-design.md
  git commit -m "feat(config): add static endpoint capabilities"
  ```

Status: completed in commit `7e041c42` (`feat(config): add static endpoint capabilities`). Evidence included red/green tests for SQLite registry capability persistence and reload diff capability-only changes, `cargo test --locked endpoint_capabilities`, `cargo test --locked sqlite_registry_store`, `cargo test --locked runtime_reload_diff`, `scripts/local-ci.sh` with 949 unit tests and 7 local release contract tests, staged denylist review, and subagent specification/code-quality re-review.

### Task M4.2b: Add Capability Diagnostics To `check-config`

**Files:**
- Modify: `src/config_diagnostics.rs`
- Modify: `src/cli_report.rs`
- Docs: `docs/configuration.md`

- [x] **Step 1: Write failing tests**

  Add tests named:
  - `check_config_warns_endpoint_capability_mismatch`
  - `check_config_capability_warning_is_offline`
  - `check_config_capability_warning_does_not_probe_upstream`

- [x] **Step 2: Implement diagnostics**

  Add offline warnings for obvious endpoint mismatch, such as an embeddings-only public model routed through a chat-only capability. Do not reject or mutate data-plane requests in this commit.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked endpoint_capability_diagnostics
  cargo test --locked config_diagnostics
  git add src/config_diagnostics.rs src/cli_report.rs docs/configuration.md
  git commit -m "feat(cli): warn on endpoint capability mismatches"
  ```

Status: completed in commit `feb3b83d` (`feat(cli): warn on endpoint capability mismatches`). Evidence included red/green `endpoint_capability_diagnostics` tests, `cargo test --locked config_diagnostics`, `scripts/local-ci.sh` with 952 unit tests and 7 local release contract tests, staged denylist review, and multi-agent design review. The warning path remains offline-only and does not probe upstreams, open SQLite stores, mutate runtime, or reject data-plane requests.

### Task M4.2c: Expose Capability Management Projections

**Files:**
- Modify: `src/endpoint_capabilities.rs`
- Modify: `src/management_resources.rs`
- Modify: `src/management_routing.rs`
- Modify: `src/state.rs`
- Modify: `src/main.rs` if route specs require updates
- Docs: `docs/architecture.md`

- [x] **Step 1: Write failing tests**

  Add tests named:
  - `management_model_routes_include_static_capabilities`
  - `management_routing_preview_includes_static_capabilities`
  - `management_channel_capabilities_are_redacted_static_metadata`
  - `management_capability_projection_does_not_probe_or_reload`

- [x] **Step 2: Implement projections**

  Expose capabilities through `/management/model-routes`, `/management/channels/:id`, and `/management/routing/preview` where relevant. Do not add request-time proxy rejection, endpoint routing changes, or live probing.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked endpoint_capability_projection
  git add src/management_routing.rs src/main.rs docs/architecture.md
  git commit -m "feat(management): expose endpoint capabilities"
  ```

Status: completed in commit `84ac49d0` (`feat(management): expose endpoint capabilities`). Evidence included red/green `endpoint_capability_projection` tests, `management_model_routes`, `management_routing_preview`, and `management_channels` regression tests, `management_routing_responses_live_outside_management_service` after tightening source-boundary assertions, `scripts/local-ci.sh` with 956 unit tests and 7 local release contract tests, staged denylist review, and subagent review. The implementation exposes only static compiled endpoint capability metadata, omits capability fields when channel and runtime catalog generations diverge, and does not probe upstreams, reload runtime, write events, alter routing, or affect `/v1` request handling.

### Task M4.2d: Show Capabilities In CLI Explanations

**Files:**
- Modify: `src/cli_commands/models.rs`
- Modify: `src/cli_commands/route.rs`
- Modify: `src/cli_report.rs`
- Docs: `README.md`, `docs/configuration.md`

- [x] **Step 1: Write failing tests**

  Add tests named:
  - `models_explain_shows_static_endpoint_capabilities`
  - `route_explain_shows_static_endpoint_capabilities`
  - `capability_cli_does_not_probe_upstream`
  - `capability_cli_preserves_table_json_semantics`

- [x] **Step 2: Update CLI display**

  Update `models explain` and `route explain` so endpoint capability output changes from `unavailable_until_m4` to actual static capability fields. The CLI still must not probe upstreams, reject data-plane requests, or imply Responses compatibility beyond static diagnostics.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked endpoint_capability_cli
  git add src/cli_commands/models.rs src/cli_commands/route.rs src/cli_report.rs README.md docs/configuration.md
  git commit -m "feat(cli): show endpoint capabilities"
  ```

Status: completed in commit `e4548f79` (`feat(cli): show endpoint capabilities`). Evidence included red/green `endpoint_capability_cli` tests for `models explain`, `route explain`, JSON/table semantics, untrusted capability-shape sanitization, and no extra upstream/channel probing; `cargo test --locked models_explain`; `cargo test --locked route_explain`; `scripts/local-ci.sh` with 961 unit tests and 7 local release contract tests; and staged denylist review.

### Task M4.3: Park Default Model Injection

**Files:**
- Modify: `README.md`
- Modify: `docs/configuration.md`
- Modify: `docs/technical-design.md`
- Modify: `src/main.rs`

- [x] **Step 1: Write documentation/diagnostic tests**

  Add tests or checks named:
  - `default_model_injection_remains_deferred`
  - `responses_defaulting_remains_deferred`
  - `docs_do_not_claim_default_model_injection`

  Cover:
  - clients are expected to send explicit public model ids in M1-M4;
  - no-model requests continue to fail locally unless existing endpoint semantics already permit them;
  - no proxy change injects a configured default model;
  - Responses defaulting and protocol bridging remain outside this plan;
  - docs do not imply default model injection is available.

- [x] **Step 2: Document the deferred policy**

  Record that any future default-model exception requires a separate protocol/request-path plan proving disabled-by-default behavior, explicit public route ids, client-token scope enforcement, no YAML/store/upstream reads on the hot path, no Responses authorization, and no successful-response buffering.

- [x] **Step 3: Verify and commit**

  ```bash
  cargo test --locked config_diagnostics
  rg -n "default_model_for_endpoint|default model injection|responses_to_chat" README.md docs
  git add docs/configuration.md docs/technical-design.md
  git commit -m "docs: park default model injection"
  ```

  The `rg` check should show only explicit deferred/parked-policy mentions. Any product-usage claim that presents default model injection or Responses-to-Chat as available in M1-M4 fails this task.

Status: completed in commit `38377bc5` (`docs: park default model injection`). Evidence included non-zero filtered discovery for `default_model_injection`, `responses_defaulting`, and `config_diagnostics`; `cargo test --locked default_model_injection -- --nocapture`; `cargo test --locked responses_defaulting -- --nocapture`; `cargo test --locked config_diagnostics -- --nocapture`; exact `rg -n 'default_model_for_endpoint|default model injection|responses_to_chat' README.md docs` review showing only deferred/parked-policy mentions; `git diff --check`; staged file review; and staged denylist review. The change keeps clients responsible for explicit public model ids, parks default-model injection and Responses protocol bridging outside M1-M4, and removes an ambiguous README phrase that could be misread as a default-model product claim.

**M4 stop card checklist:**

- reload status/diff/apply and endpoint capability features have negative tests proving no live `/v1/models`, no dynamic probing, no request-path protocol/default-model shim, no scope bypass, no secret output, and no hidden reload mutation from read-only commands;
- reload diff closes in one of two explicit states: typed redacted diff from an existing staged projection, or `unavailable_without_staged_projection` with no `reload apply` suggestion;
- reload apply either uses an enforceable expected-generation/plan precondition, or the mutating path is unavailable and fails closed without POSTing reload;
- capability fields remain static compiled metadata and do not affect request-path routing or provider/model catalogs;
- CLI display updates to `models explain` and `route explain` are serialized after reload/capability DTO shapes are committed;
- README/configuration/technical docs keep the product first screen centered on lightweight OpenAI-compatible key routing, not reload/probe/health troubleshooting;
- `cargo test --locked <filter> -- --list` evidence proves each filtered M4 test command matched intended tests;
- `scripts/local-ci.sh`, docs cold-read, staged file review, staged denylist review, and ignore-sync gate pass.

---

## Deferred Gates After M1-M4

**Purpose:** keep Responses-to-Chat out of this implementation plan and park runtime counters behind a separate performance decision. Start the counter gate only after M1-M4 are complete and the maintainer records that request-path counter instrumentation is worth the added hot-path state.

### Parked Gate D1: Responses-To-Chat Adapter

**Files:**
- Deferred to a separate protocol implementation plan.

This plan does not authorize implementing `responses_to_chat_completions`. A later plan must first prove that the adapter can preserve the successful-response streaming contract, avoid full buffering, reject unsupported fields before credential selection, and return a valid Responses API subset without cross-protocol retry.

Required future proof points:

- adapter disabled on chat-only target returns local unsupported endpoint and upstream hit count is zero;
- native Responses target uses only `/v1/responses`;
- chat bridge target uses only `/v1/chat/completions`;
- upstream 4xx/5xx/timeout does not trigger cross-protocol retry;
- unsupported fields fail locally before credential selection, before upstream send, and without lifecycle mutation;
- successful response conversion has a bounded streaming or otherwise explicitly budgeted implementation that does not violate the no-full-buffering contract;
- `stream:true` is either rejected before credential selection or covered by a separate streaming event mapping proof.

### Deferred Decision D2: Bounded Runtime Counters

This item is not required for the M1-M4 release. Until an explicit decision record exists, no worker may modify `proxy.rs`, `state.rs`, `model_catalog.rs`, `/v1` route registration, or other request-path code for counters.

Required decision record before any future implementation plan:

- accepted hot-path budget and rationale;
- frozen dimensions and maximum slot count;
- allowed update points;
- abort criteria if benchmarks or tests show measurable request-path regression;
- statement that counters remain non-billing telemetry.

Required future proof points:

- fixed low-cardinality dimensions and bounded memory;
- no per-request dynamic insertion into unbounded maps;
- model dimension limited to precompiled public route ids or `unmapped`/`unknown` buckets;
- atomics or precompiled index slots instead of hot-path string-keyed maps;
- no raw request/response data, token, key, prompt, completion, or matched filter text;
- no successful-response buffering;
- no per-token, per-stream-chunk, per-response-fragment, per-user, or per-raw-key dimensions;
- updates only at bounded lifecycle points such as request start, route selection, retry/fallback decision, terminal outcome, and filter/guard decision;
- no billing/cost/price table semantics;
- no CLI wrapper until backend counter tests and management projection tests pass in a separate plan.

**Deferred gate close condition:** Responses-to-Chat remains parked until a separate protocol plan exists. Runtime counters are deferred and not a blocker for an M1-M4 release. If D2 lacks a maintainer decision record, the deferred section legally closes as “Responses parked; counters deferred.” If D2 proceeds, it requires the decision record, local CI, request-path negative tests, and benchmark or budget evidence.

---

## Explicitly Rejected Or Deferred

- Live upstream `/v1/models` aggregation on client requests.
- Runtime model alias directory independent of `model_routes`.
- Background active health-check cluster or continuous key scanner.
- Auto-fix behavior hidden inside `doctor`.
- Automatic scope mutation from model discovery or local model-route onboarding.
- One-shot discover/apply/reload/scope update workflow.
- Persistent billing ledger, price catalog, cost accounting, or quota resale logic.
- Full Web UI or multi-tenant platform surface.
- Request-path storage joins, YAML reads, DB reads/writes, or upstream discovery.
- Full Responses API implementation without a separate protocol plan.
- Cross-protocol retry after upstream failure.
- Fallback or retry after stream/body bytes are committed to the client.

## Verification And Release Gate

Each milestone must run the targeted tests listed in its tasks. Before pushing a completed milestone:

```bash
scripts/local-ci.sh
git status -sb --untracked-files=all
```

An M1-M4 release may close while D2 is deferred, provided all completed milestone stop cards pass, no open blocker remains, and the release notes explicitly state that Responses-to-Chat and runtime counters are not part of that release unless separately implemented.

Task-level `cargo test` commands must use one filter at a time or a named test target. Do not write `cargo test filter_a filter_b`; Cargo rejects extra filter arguments. When using a filter, include a test that intentionally matches that filter and record a non-zero discovery check such as:

```bash
cargo test --locked failures_cli -- --list | rg 'failures_'
cargo test --locked failures_cli
```

If the discovery command shows no intended tests, the task is not verified even if Cargo exits successfully.

Docs gate:

- README first screen describes the stable product identity: lightweight OpenAI-compatible key router, explicit local model routes, one client-facing base URL/token, and redacted operator tooling;
- README does not narrate historical support chat, temporary keys, real domains, private server names, personal deployment incidents, or one-off replacement-key commands;
- key replacement, probe, failure, reload, and health workflows appear under operations/troubleshooting, not as the primary product story;
- public docs do not claim upstream `/v1/models` aggregation, active health checks in `doctor`, automatic model exposure, default model injection, Responses-to-Chat support, runtime counters, billing, or UI unless a separate accepted plan implements them.

Ignore-sync gate:

- `.gitignore` and `.dockerignore` must both cover at least `.env*`, `config/*.yaml`, `config/*.yml`, `data/`, `*.keys`, `*.sqlite*`, `*.sqlite3*`, `*.db*`, `*.wal`, `*.shm`, `*.log*`, `dist/`, generated runtime artifacts, and sensitive snapshot/golden output locations;
- ignore rules must be checked before release and after adding any new template, smoke, fixture, or build path;
- if a required ignore entry is missing, fix the ignore file in a separate hygiene commit before staging implementation or release artifacts.

Data-plane client-invisibility gate:

- authenticated `/v1/models` returns only explicit public model ids visible to the client token;
- `/v1/models` does not expose channel ids, provider names, credential state, probe state, reload state, staged registry state, endpoint capability metadata, or management-only reasons;
- successful streaming remains non-buffered; response headers/body forwarding must not wait for a full upstream success body;
- no new client-facing endpoint or compatibility shim is registered by M1-M4 unless the task explicitly names it;
- response/filter/guard failures returned to clients use redacted local error codes and never raw upstream text, matched filter text, secrets, or store paths.

Hot-path change gate:

Any task touching `src/proxy.rs`, `src/route_plan.rs`, request-transform code in `src/provider.rs`, `src/state.rs`, `src/model_catalog.rs`, or `/v1` route registration in `src/main.rs` must add or run tests/static checks proving no request-path YAML parsing, SQLite/store access, management calls, model discovery/catalog calls, background scanner spawn, or full successful-response buffering.

Commit file lists in this plan are expected boundaries, not exhaustive staging commands. Before each commit, include the task's tests and verify the staged file list:

```bash
git diff --cached --name-only
```

Before every commit, run a staged denylist review. Staged files must not include `.env*`, local `config/*.yaml`, `data/`, `*.keys`, `*.sqlite*`, `*.db*`, `*.wal`, `*.shm`, `*.log*`, `dist/`, local release tarballs/checksums, generated runtime artifacts, or golden/snapshot fixtures containing secrets, secret-derived identifiers, raw request/response bodies, or untrusted promotional/injection text.

Before a release:

```bash
scripts/local-ci.sh
git status -sb --untracked-files=all
scripts/build-release-x86_64-linux-docker.sh
git status -sb --untracked-files=all
(cd dist && shasum -a 256 -c one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256)
```

Release artifact smoke tests must extract the built tarball into a tempdir and run the released binary, not `cargo run`, through:

```bash
./one-ai-key --help
./one-ai-key init local --out config/local.yaml --keys data/relay.keys --dry-run
./one-ai-key init local --out config/local.yaml --keys data/relay.keys --yes
./one-ai-key check-config --config config/local.yaml
```

After M2, release smoke must also run the released binary with a local mock upstream and prove `/v1/models` plus one model-bearing request work without real upstream keys. If Linux-only release smoke cannot run directly on the macOS host, run it inside the same local Docker/Nix x86_64 container used for the build.

The release smoke must also compare `check-config --output json` `model_visibility_preview` for the generated client-token reference against the authenticated `/v1/models` response from the started service. The two model-id sets must match exactly for the tempdir fixture.

By the M1-M4 release gate, this smoke must be scriptable as `scripts/release-smoke.sh` or an equivalent committed command block. It must create its own tempdir, generate placeholder config, start a local mock upstream, start the released binary, set placeholder management/client tokens, assert `status` / `reason_code` / `next_action` for representative operator commands, and clean up background processes.

Representative operator-command smoke after M2-M4:

```bash
./one-ai-key doctor --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --output json
./one-ai-key client-tokens list --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --output json
./one-ai-key models list --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --client-token-ref local-client --output json
./one-ai-key route explain --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --model gpt-example --client-token-ref local-client --output json
./one-ai-key keys stats --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --credential-set relay_credentials --output json
./one-ai-key failures tail --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --last 20 --output json
./one-ai-key reload status --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --output json
./one-ai-key reload diff --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --output json
./one-ai-key reload apply --management-url http://127.0.0.1:4101 --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN --dry-run --output json
```

Commands that are not implemented at the time of a milestone smoke must be omitted only when their milestone is not being closed. For the final M1-M4 release, all implemented M1-M4 commands must either succeed with local mock data or return their documented unavailable/deferred reason code; no command may fail due to missing CLI wiring, raw token confusion, malformed JSON output, or zero-test-only coverage. Negative smoke must include a client `/v1` base URL accidentally passed as `--management-url` and must assert `reason_code: client_base_url_used_for_management`.

Streaming non-buffering does not have to be proven by the shell smoke if a named test already covers it. The release gate must name and run that test before release.

The release path must stay local Docker/Nix; do not build on deployment servers.

GitHub release gate:

```bash
VERSION="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name=="one-ai-key") | .version')"
TAG="v${VERSION}"
git status -sb --untracked-files=all
git tag --list "${TAG}"
gh release create "${TAG}" \
  "dist/one-ai-key-${VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
  "dist/one-ai-key-${VERSION}-x86_64-unknown-linux-gnu.tar.gz.sha256" \
  --title "one-ai-key ${VERSION}" \
  --notes-file dist/release-notes-${VERSION}.md
gh release view "${TAG}" --json tagName,name,assets
```

Before creating the release, verify that:

- the tag/version, Cargo package version, tarball filename, checksum filename, and release notes version match;
- release notes state that Responses-to-Chat and runtime counters are not part of the release unless a separate accepted plan implemented them;
- no release note, asset, or checksum contains secrets, private domains, local paths, temporary keys, raw request/response bodies, or untrusted promotional/injection text;
- uploaded assets include exactly the expected tarball and checksum unless a separate packaging task added more artifacts;
- after release creation, download the assets into a tempdir and verify the checksum from the uploaded `.sha256` file.

## Convergence Record

Root issue ledger:

| Root | Resolution |
| --- | --- |
| Read-only commands mixed with write/probe workflows | Split M1/M2 read-only from M3 operator workflows, M4 safe backend additions, and post-M1-M4 deferred gates. |
| CLI scope too vague | Added command contract, side-effect classes, exit codes, common flags, examples, and redaction rules. |
| Risk of duplicating server logic in CLI | Added file map and rule that CLI wraps backend projections instead of recomputing route/runtime semantics. |
| Responses compatibility pressure | Kept capability matrix before protocol adapter and moved adapter out of this implementation plan into a parked follow-up gate. |
| Usage/counters risk | Restricted counters to bounded non-billing metadata and excluded upstream `usage` parsing from this plan. |
| Product core dilution risk | Added product principles and milestone user outcomes so implementation remains a lightweight router workflow, not a platform roadmap. |
| Operator UX drift risk | Added command UX quality bar and report contract so commands are useful, explainable, and safe by default. |
| Roadmap completion ambiguity | Defined M1-M4 plus release gate as the completion point; D2 is conditional post-completion work and D1 requires a separate protocol plan. |
| Abstract feature creep without user workflow proof | Added operator journey acceptance for first run, model visibility, key maintenance, incident diagnosis, and config-change workflows. |
| Dry-run/confirmation ambiguity | Replaced loose prose with a side-effect execution-mode table and central dispatch-table requirement. |
| Client-token reference discoverability gap | Added `client-tokens list` and folded it into M2 model explanation workflows. |
| Discovered model onboarding overclaim | Renamed M3.4 to onboarding plan and stated it does not make models visible without later explicit apply/reload/scope actions. |
| Composite side-effect ambiguity | Added effect vectors so commands can be read-only, store-reading, upstream-touching, store-writing, and runtime-mutating along independent axes. |
| Key-derived identifier leakage risk | Added display/redaction classes. Fingerprints, hashes, prefixes/suffixes, HMACs, and existing backend credential ids are internal/compatibility fields only; M1-M4 CLI may use only non-secret `credential_ref`, otherwise dependent credential workflows remain blocked. |
| Read-only aggregate cost risk | Made latest probe summaries conditional on bounded/indexed projections and prohibited CLI page-scan aggregation. |
| Request-path compatibility creep | Parked default-model injection and required a separate protocol/request-path plan for any future endpoint-default or protocol-bridge behavior. |
| Release client-invisibility gap | Added data-plane release gates proving management/probe/discovery/reload/capability details stay invisible to `/v1` clients. |
| First-run packaging gap | Added release artifact smoke tests using the built binary, local tempdirs, and mock upstreams rather than only development commands. |
| Documentation drift risk | Added a documentation contract so README and technical docs describe stable product usage rather than historical support chat, temporary keys, real domains, or a key-replacement-heavy narrative. |
| Deferred counter hot-path exception | Recast D2 as a deferred performance decision with required budget, frozen dimensions, update points, and abort criteria before any request-path edits. |
| Repository hygiene risk | Added local artifact constraints and staged denylist review before commits. |
| Current baseline drift | Replaced stale M1-era, post-M2.4b, and post-M2.6 preflight text with the current post-M2 stop-card checkpoint and explicit M3 entry instructions. |
| Report envelope drift | Added a common JSON/table report envelope requiring `status`, `reason_code`, `side_effect_class`, `effect_vector`, scope/window metadata, and structured `next_action`. |
| Dynamic command injection risk | Replaced dynamic `safe_command`/`dry_run_command` strings with structured `safe_argv` and allowlisted display rendering. |
| Management error injection risk | Added central tests requiring management JSON/HTML/error bodies and encoded promotional text to be discarded into local redacted reason codes. |
| M2 dependency ambiguity | Serialized `doctor` after `failures tail` because `doctor` uses `failures tail` as a safe next action. |
| M3/M4 parallel conflict risk | Tightened parallel matrix: key workflows are serialized unless split modules are committed; reload/capability CLI display updates are serialized unless a shared DTO/render extension point exists. |
| Reload diff/apply overreach risk | Split reload diff into an availability contract and optional typed diff from an existing staged projection; reload apply requires an enforceable generation/plan precondition or fails closed. |
| Release publication gap | Added GitHub release gate, asset/checksum upload verification, post-upload checksum verification, and release-note exclusion requirements. |
| Zero-test false positive risk | Added mandatory non-zero test discovery evidence for filtered Cargo test commands. |
| Ignore rule drift | Added ignore-sync gate for `.gitignore` and `.dockerignore` covering local configs, key/data files, databases, logs, release artifacts, and sensitive snapshots. |

Propagation audit:

- The read-only invariant appears in the canonical invariants, M1, M2, M3 dry-run rules, and rejected items.
- The no-live-catalog invariant appears in constraints, `models explain`, client matrix, M3 local model-route onboarding, M4 capability tests, and rejected items.
- The Responses adapter boundary appears in canonical invariants, client matrix, D1, and rejected items.
- Redaction requirements appear in constraints, CLI contract, file map, M1/M2/M3/M4 tests, deferred gate tests, and release gate.
- Product principles appear in product principles, milestone user outcomes, non-negotiable constraints, canonical invariants, rejected/deferred items, and stop record.
- Command UX requirements appear in CLI contract, command UX quality bar, report contract, operator examples, milestone tests, and stop cards.
- The roadmap completion stop appears in roadmap completion criteria, plan governance, deferred gate purpose, deferred gate close condition, release gate, and stop record.
- Operator journey acceptance appears in the journey section, report contract, operator examples, M1-M4 tasks, and milestone stop cards.
- Side-effect execution semantics appear in canonical invariants, execution mode table, side-effect classes, effect vector contract, M1.0, M3 tests, and release gate.
- Secret-derived identifier redaction appears in non-negotiable constraints, report contract, M1/M2/M3 tests, and staged denylist.
- Bounded management-query constraints appear in report contract, `keys stats`, `doctor`, `reload diff`, M4 capability tests, and D2 counter gate.
- Client-token ref discoverability appears in command list, report contract, operator examples, first-run transcript, and M2.3.
- The current progress baseline appears in Current Baseline And Preflight, M2 dependency text, the parallel execution matrix, and the stop record.
- The unified report envelope appears in the CLI contract, report contract, M2 stop card, M3/M4 stop cards, release smoke, and convergence record.
- Structured `safe_argv` appears in the report contract, JSON skeleton, M2/M3/M4 next-action obligations, and release smoke expectations.
- GitHub release and artifact verification appear in the release gate and are bounded to local Docker/Nix build output plus `gh release` publication checks.
- Docs cold-read and ignore-sync gates appear in task execution, milestone stop cards, and the release gate.

Stop record:

Current status is internally converged and deliverable as a plan after the latest multi-agent review round. The current roadmap stops after M1-M4 are implemented, documented, locally tested, committed, released through the local Docker/Nix gate, and published through the GitHub release gate. The active implementation checkpoint is post-M2 stop-card: M1 and M2 are complete, M3 is the next implementation gate, and M4 is not started.

Remaining implementation-time gates are narrow and already bounded by the milestone text: M2 must reconcile completed commands against the unified report envelope; M3 individual credential workflows depend on non-secret `credential_ref`; M3 probe-apply depends on a read-only apply plan and a mutating precondition; M4 typed reload diff proceeds only if an existing staged projection exists; M4 reload apply requires enforceable generation/plan preconditions or fails closed; D2 counters remain deferred unless the maintainer records a separate performance decision. M3 import dry-run is locked to CLI-local preview and must disclose that confirmed imports can become route-eligible immediately under current backend semantics. M3 local model-route onboarding closes as a `runtime_readonly` planning workflow, not as upstream discovery or client-visible model exposure. Default-model injection and Responses-to-Chat implementation are not authorized by this plan and require separate protocol/request-path plans.
