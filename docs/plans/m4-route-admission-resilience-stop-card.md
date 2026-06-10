# M4 Route Admission Resilience Stop Card

plan_id: `m4-runtime-resilience-plan`

release_or_scope_name: `m4-route-admission-resilience-source-closure`

closed_capability: `Route Admission Resilience v1`

implemented_scope: `route admission hard/soft taxonomy; provider/account soft-cooling last-resort admission; proxy secondary gate alignment with the frozen route plan; state-transition hard/soft/credential/request taxonomy audit; shared route_admission_summary projection; bounded local admission evidence distinct from selected-upstream failure evidence; CLI rendering from management projections; release-smoke static and script coverage for selected upstream 503, local admission 503, and soft last-resort behavior; user, operator, architecture, and technical-design documentation`

deferred_scope: `production deployment smoke; release artifact publication; broader route/app extraction; config compiler extraction; endpoint-family compatibility adapters; active probe or health-check systems`

parked_or_rejected_items: `streaming retry; partial-output fallback; endpoint-family fallback; Responses-to-Chat conversion; live upstream /v1/models aggregation; background health checks; persistent adaptive routing; usage or billing ledger; dashboard or TUI; automatic key repair, model scope mutation, reload, discover/apply, or route mutation; new YAML switches for M4; production SSH/systemd mutation in release tests`

route_admission_taxonomy_result: `pass: route_plan owns hard blocker, soft suppression, and last-resort reason classification; available, degraded, and provider/account soft-cooling ordering is locked by tests; hard blockers remain excluded from last-resort admission`

state_transition_taxonomy_result: `pass: routing transition characterization covers credential expiration, credential cooldown, credential quota exhaustion, relay-balance channel hard cooldown, non-upstream relay-balance no-op, provider/account degraded state, provider/account soft cooldown, response-filter credential lifecycle actions, response-filter hard channel cooldown, key-switch no-op, request/model client-error no-op, and unsupported provider-adapter no-op`

soft_cooling_last_resort_result: `pass: provider/account soft-cooling candidates are suppressed while better candidates exist and may be selected only as last-resort first upstream attempts when no better otherwise-valid target remains`

hard_blocker_zero_upstream_result: `pass: hard blockers such as channel hard cooldown and empty credential pools still fail closed before upstream; runtime and release-smoke contracts distinguish local admission 503 with zero upstream hits from selected-upstream 503`

proxy_secondary_gate_result: `pass: proxy attempt gating allows a provider-cooling target that was selected as a frozen last resort, preserves skip-to-later-target behavior when a better frozen target remains, and does not synthesize a new route`

m3_retry_non_expansion_result: `pass: M4 does not expand M3 retry; soft last-resort admission is a first attempt from the compiled route plan, not a retry. Streaming, partial-output, embeddings, /v1/models, named-pool, unknown endpoint, and exhausted-deadline retry exclusions remain covered by the M3 boundary tests`

management_projection_result: `pass: /management/routing/preview serializes RouteAdmissionSummary from route_plan; management no longer owns duplicate admission taxonomy helpers and exposes bounded counts and stable reason codes`

admission_evidence_result: `pass: local route admission denial evidence remains bounded and redacted with client-visible local_503, upstream_status=null, stable reason codes, and admission counts; selected upstream 503 evidence remains separate with upstream_status=503 and no admission object`

cli_rendering_result: `pass: route explain renders backend-projected admission_summary; models explain --endpoint-family treats management model availability as canonical; failures explain reports bounded historical evidence and does not recompute current admission`

operator_contract_evidence: `pass: README, docs/operations.md, docs/architecture.md, and docs/technical-design.md describe no_route_candidate, hard blockers, soft suppressions, credential/request-scoped states, and M4 non-goals without private deployment traces or secret material`

test_evidence: `pass: targeted cargo tests for route_plan, proxy attempt gate, routing state transition matrix, management projection, CLI rendering, local release contracts, and pre-output stability boundary passed during M4 implementation; scripts/local-ci.sh passed with CARGO_TARGET_DIR outside the repository after Task 7 and Task 8 documentation edits`

non_empty_filtered_test_evidence: `pass: all recorded filtered cargo commands matched at least one intended test; zero-test filtered output was not used as evidence`

local_ci_result: `pass: CARGO_TARGET_DIR=/tmp/one-ai-key-cargo-target scripts/local-ci.sh exited 0 after Task 8 documentation edits; reported 1152 unit tests, 16 local_release_contract tests, and 5 pre_output_stability_boundary_contract tests passing`

release_artifact_result: `pass: scripts/build-release-x86_64-linux-docker.sh exited 0; dist/one-ai-key-0.1.12-x86_64-unknown-linux-gnu.tar.gz and matching .sha256 were produced from current source; checksum sidecar contains the archive basename only; checksum verifies; tarball contains only one-ai-key`

release_smoke_result: `pass: scripts/release-smoke.sh exited 0 against the freshly built 0.1.12 release artifact; smoke verified checksum, extracted binary execution, generated local config, authenticated /v1/models, model-bearing client requests, operator reports, model publication workflow, redacted management reports, selected-upstream 503 evidence, local no-route admission 503 evidence, provider/account soft-cooling last-resort behavior, and client /v1 URL rejection for management-url misuse`

artifact_sha: `5d09ed34a66b63835b5d94737c9a4c39f931f1745d1c4dda197ccc84f0232d78`

production_smoke_result: `not_run_by_design: production smoke is an operator-run deployment check, not a source release dependency; no production host, DNS, token rotation, or systemd/NixOS mutation is part of this source closure`

redaction_and_denylist_result: `pass: git diff --check exited 0; scripts/check-staged-denylist.sh exited 0; staged files must remain limited to public docs and tracked source/test changes and exclude dist/, target/, key-pool-router/, config/, data/, SQLite, logs, keys, raw production fixtures, private scripts, and AGENTS.md`

anti_platform_gate_result: `pass: M4 added no live upstream catalog aggregation, background probing, persistent adaptive state, usage analytics, billing, UI, endpoint adapter, or broad platform behavior`

support_residue_scan_result: `pass: changed public docs and plan text use stable product behavior, generic route states, and reason codes rather than private domains, raw keys, token material, production incident transcripts, or one-off support procedures`

deployment_boundary_result: `not_run_by_design: deployment hosts consume published release artifacts; no remote deployment mutation is included in M4 source closure`

known_blockers: `none for M4 source and release artifact closure`

next_version_candidates: `route/app boundary extraction; config resolver/compiler extraction; endpoint-family compatibility plan; optional manual probe UX; operational diagnosis improvements that remain read-only and do not add request-path storage joins`
