# Next Development Plan Stop Card

plan_id: `next-development-plan`

release_or_scope_name: `runtime-diagnosis-publication-source-closure`

closed_capability: `Runtime Reliability Semantics v1; Operator Diagnosis Loop v1; Model Publication Workflow v1`

implemented_scope: `route admission taxonomy; proxy secondary gate alignment; typed relay/error classification; real failure-to-state transition summaries; conservative M3 non-expansion boundary; pre-output guard and response-filter lifecycle stability; canonical models explain diagnosis; management diagnosis projection; route/models/failures projection alignment; safe read-only diagnosis next actions; doctor/reload role boundaries; read-only model publication plan; staged model-route apply response with validation and redacted audit summaries; explicit reload diff/apply precondition; visibility verification before and after reload; local release smoke publication workflow`

deferred_scope: `Safe Credential Replacement Workflow v1; production deployment smoke; release artifact publishing; broader credential replacement UX; route/app boundary extraction; config resolver/compiler boundary extraction`

parked_or_rejected_items: `live upstream model catalog aggregation; background health checks; adaptive routing; persistent failure ledger; UI/dashboard work; billing/usage platform; broad protocol conversion; automatic model/client-scope/key mutation; automatic reload; YAML presets; private deployment state as release evidence`

operator_contract_evidence: `pass: canonical models explain with endpoint family reports can_use, reason_code, blocking_domain, endpoint_family, model, client_token_ref, bounded evidence, and read-only next_action; route explain, failures explain, doctor, reload status, and reload diff remain read-only diagnosis surfaces; publication plan/apply/reload/verify remains explicit and staged`

targeted_test_evidence: `pass: routing_telemetry_failure_transition_summary matched 5 tests; failure_transition_summary matched 11 tests; registry_model_route matched 6 tests; provider_cooling matched 7 tests; models_explain matched 20 unit tests plus 2 local release contract tests; reload_diff matched 17 tests`

local_ci_result: `pass: CARGO_TARGET_DIR outside the repository, scripts/local-ci.sh exit 0; local-ci ran 1123 unit tests, 15 local_release_contract tests, and 5 pre_output_stability_boundary_contract tests`

release_artifact_result: `pass: scripts/build-release-x86_64-linux-docker.sh exit 0; rebuilt dist/one-ai-key-0.1.10-x86_64-unknown-linux-gnu.tar.gz and matching .sha256 from current source`

release_smoke_result: `pass: scripts/release-smoke.sh exit 0 after rebuild; extracted artifact smoke passed for one-ai-key 0.1.10`

redaction_and_denylist_result: `pass: git diff --check exited 0; executable staged-path denylist exited 0; staged files are limited to docs/plans, source, and tests and exclude dist/, target/, config/, data/, SQLite, logs, keys, raw fixtures, private scripts, and AGENTS.md`

compatibility_result: `pass: local-ci and release smoke cover /v1/models local catalog behavior, /v1/chat/completions mock data-plane requests, /v1/responses boundary contracts, invalid client token, management/client URL misuse, canonical diagnosis, and model publication plan/apply/reload/verify workflow`

anti_platform_gate_result: `pass: implementation adds tests and bounded projections only; no live catalog aggregation, background probing, persistent ledger, UI, billing, broad adapter, or automatic mutation was added`

support_residue_scan_result: `pass: plan and stop-card text use stable product behavior and placeholders; no private domains, raw keys, token material, deployment transcript, chat history, or one-off support workaround is recorded`

deployment_boundary_result: `not_run_by_design: no production server, DNS, systemd unit, gateway pin, token rotation, or public traffic is part of this source closure`

known_blockers: `none for Stage 1 through Stage 3 source closure after final gates pass`

next_version_candidates: `Safe Credential Replacement Workflow v1; source-to-release artifact packaging; production operator smoke harness; route/app boundary extraction; config resolver/compiler boundary extraction`
