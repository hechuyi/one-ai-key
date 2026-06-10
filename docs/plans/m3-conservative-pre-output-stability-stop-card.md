# M3 Conservative Pre-Output Stability Stop Card

plan_id: `product-improvement-roadmap`

release_or_scope_name: `m3-conservative-pre-output-stability-v1`

closed_capability: `Conservative Pre-Output Stability v1`

implemented_scope: `selected-target pre-output retry gate; endpoint-family allowlist for non-streaming Chat Completions and Responses; named-pool retry exclusion; one-extra-attempt budget across credential, route-target, and same-target continuations; redacted retry and denial telemetry; local route-admission 503 evidence separated from selected-upstream 503 evidence; README, operations, configuration, technical design, and roadmap behavior docs`

deferred_scope: `deployment pin update; production deployment smoke; broader protocol conversion; endpoint fallback; active background health probing; persistent adaptive routing; usage or billing ledger`

parked_or_rejected_items: `streaming retry; partial-output fallback; /v1/models retry; Embeddings retry; named-pool M3 retry; unknown endpoint retry; default 429 retry; cross-provider default fallback outside frozen route policy; YAML retry presets; live upstream catalog aggregation; background health scanning; UI, billing, multi-tenancy, and broad OpenAI protocol conversion`

operator_contract_evidence: `pass: README and docs/operations.md describe the single conservative pre-output retry, excluded endpoints, telemetry denial reasons, and diagnostic behavior without private deployment traces or secret material. docs/operations.md now distinguishes local route admission failures from selected-upstream failures: local no-route capacity evidence reports route_admission_denied, local_503, upstream_status=null, and bounded admission counts, while selected upstream 503 evidence reports upstream_5xx, upstream_status=503, and no admission object`

test_evidence: `pass: cargo test --locked exited 0 with 1142 unit tests, 16 local_release_contract tests, and 5 pre_output_stability_boundary_contract tests; release_smoke_script_covers_local_admission_and_upstream_503_failure_evidence passed; scripts/release-smoke.sh syntax check passed; release-smoke static contract now covers upstream_5xx/upstream_status=503/admission=null, route_admission_denied/no_route_candidate/local_503/upstream_status=null/admission.included_count=0, management-report leak scanning for the local markers, and zero upstream chat-completion POSTs for local admission denial`

non_empty_filtered_test_evidence: `pass: release_smoke_script_covers_local_admission_and_upstream_503_failure_evidence filter ran 1 intended test; --test local_release_contract ran 16 tests; --test pre_output_stability_boundary_contract ran 5 tests; zero-test filtered cargo output was rejected and rerun with the correct integration-test target`

local_ci_result: `pass: CARGO_TARGET_DIR outside the repository, scripts/local-ci.sh exit 0`

release_artifact_result: `pass: package version bumped to 0.1.12; scripts/build-release-x86_64-linux-docker.sh exited 0; dist/one-ai-key-0.1.12-x86_64-unknown-linux-gnu.tar.gz and matching .sha256 were produced from current source; sidecar contains archive basename only; checksum verifies; tarball contains only one-ai-key`

release_smoke_result: `pass: scripts/release-smoke.sh exited 0 against the freshly built 0.1.12 release artifact; smoke verified checksum, extracted binary execution, generated local config, authenticated /v1/models, one model-bearing request, operator reports, model publication workflow, redacted management reports, selected-upstream 503 evidence, local no-route admission 503 evidence, and client /v1 URL rejection for management-url misuse`

artifact_sha: `12f181ba1d4c5b0c95c6550bf927a9840e9a53e5da6734a925fd105b2e2a3038`

published_asset_verification: `pass: GitHub Release v0.1.12 was created at https://github.com/hechuyi/one-ai-key/releases/tag/v0.1.12; uploaded tarball and .sha256 assets were downloaded through the configured local proxy into a tempdir; checksum verified from the uploaded .sha256 sidecar; tarball contains only one-ai-key; release is not draft or prerelease`

redaction_and_denylist_result: `pass: git diff --check exited 0; executable staged-path denylist exited 0; staged files are limited to Cargo release metadata, public docs, source, and tests and exclude dist/, target/, key-pool-router/, config/, data/, SQLite, logs, keys, raw fixtures, private scripts, and AGENTS.md`

anti_platform_gate_result: `pass: no live upstream catalog aggregation, background health check loop, persistent failure ledger, billing/usage analytics, UI surface, or broad protocol adapter was added`

support_residue_scan_result: `pass: changed docs use stable product behavior, placeholder terms, status codes, and reason codes rather than private deployment names, real gateway domains, raw keys, chat transcripts, or one-off replacement procedures`

deployment_boundary_result: `not_run_by_design: no production server, DNS, systemd unit, gateway pin, token rotation, or public traffic was touched`

known_blockers: `none for M3 source and release artifact closure`

next_version_candidates: `route/app boundary extraction; config resolver/compiler boundary extraction; failures projection/classification split; explicit plan for any future Responses-to-Chat adapter or endpoint compatibility work`
