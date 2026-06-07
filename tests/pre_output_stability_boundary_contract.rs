use std::fs;

fn read_repo_file(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

fn test_body<'a>(source: &'a str, name: &str) -> &'a str {
    let needle = format!("fn {name}");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("missing characterization test `{name}`"));
    let suffix = &source[start..];
    let end = suffix[1..]
        .find("\n    #[")
        .map(|offset| offset + 1)
        .unwrap_or(suffix.len());
    &suffix[..end]
}

fn assert_contains_all(source: &str, context: &str, expected: &[&str]) {
    for needle in expected {
        assert!(source.contains(needle), "{context} must contain `{needle}`");
    }
}

#[test]
fn existing_runtime_tests_cover_pre_output_retry_and_retry_denials() {
    let main = read_repo_file("src/main.rs");

    assert_contains_all(
        test_body(
            &main,
            "m3_non_streaming_chat_and_responses_retry_502_503_504_once_before_output",
        ),
        "M3 Chat/Responses 502/503/504 retry matrix",
        &[
            "\"/v1/chat/completions\"",
            "\"/v1/responses\"",
            "StatusCode::BAD_GATEWAY",
            "StatusCode::SERVICE_UNAVAILABLE",
            "StatusCode::GATEWAY_TIMEOUT",
            "retry_same_target",
            "upstream_hits.load(Ordering::SeqCst), 2",
            "duplicate_charge_risk == \"unknown\"",
        ],
    );

    assert_contains_all(
        test_body(
            &main,
            "m3_ineligible_endpoint_families_and_named_pool_do_not_retry",
        ),
        "M3 ineligible endpoint and named-pool retry denial characterization",
        &[
            "\"/v1/embeddings\"",
            "\"/v1/unknown\"",
            "\"/pools/test/v1/chat/completions\"",
            "upstream_hits.load(Ordering::SeqCst), 1",
            "failure_not_retryable",
            "duplicate_charge_risk == \"none\"",
        ],
    );

    assert_contains_all(
        test_body(
            &main,
            "error_body_read_failure_does_not_bypass_streaming_fallback_gate",
        ),
        "streaming retry denial characterization",
        &[
            "\"stream\":true",
            "StatusCode::BAD_GATEWAY",
            "*fallback_hits.lock().await, 0",
        ],
    );

    assert_contains_all(
        test_body(
            &main,
            "partial_output_started_body_error_does_not_route_fallback",
        ),
        "partial-output retry denial characterization",
        &[
            "partial_output_started",
            "StatusCode::OK",
            "*fallback_hits.lock().await, 0",
            "denial_reason.as_deref() == Some(\"partial_output_started\")",
            "duplicate_charge_risk == \"none\"",
        ],
    );
}

#[test]
fn existing_guarded_success_tests_cover_2xx_body_boundary() {
    let main = read_repo_file("src/main.rs");

    assert_contains_all(
        test_body(
            &main,
            "guarded_success_json_error_returns_error_without_route_fallback",
        ),
        "guarded 2xx error-envelope characterization",
        &[
            "StatusCode::OK",
            "guarded_success_envelope",
            "StatusCode::BAD_GATEWAY",
            "fallback_hits.load(Ordering::SeqCst), 0",
            "denial_reason",
            "failure_not_retryable",
            "duplicate_charge_risk",
            "none",
        ],
    );

    assert_contains_all(
        test_body(
            &main,
            "guarded_success_prefix_passes_through_response_filter_and_strips_body_headers",
        ),
        "guarded 2xx success-body filtering characterization",
        &[
            "StatusCode::CREATED",
            "CONTENT_LENGTH",
            "CONTENT_ENCODING",
            "[filtered]",
            "unsafe-marker",
        ],
    );

    assert_contains_all(
        test_body(
            &main,
            "guarded_success_no_body_204_skips_guard_and_success_records",
        ),
        "guarded 2xx no-body characterization",
        &[
            "StatusCode::NO_CONTENT",
            "guarded_success_envelope",
            "assert!(!guarded_failure_seen)",
        ],
    );
}

#[test]
fn existing_transition_tests_cover_stable_denial_and_duplicate_charge_codes() {
    let routing = read_repo_file("src/routing.rs");
    let failure_observer = read_repo_file("src/failure_observer.rs");

    assert_contains_all(
        test_body(
            &routing,
            "phase3b_global_retry_gates_still_deny_credential_exhaustion_route_fallback",
        ),
        "transition retry-denial boundary",
        &[
            "BodyNotReplayable",
            "StreamingNotRetryable",
            "PartialOutputStarted",
        ],
    );

    assert_contains_all(
        test_body(
            &routing,
            "retry_gate_allows_only_initial_m3_endpoint_families",
        ),
        "M3 endpoint-family retry allowlist boundary",
        &[
            "EndpointKind::Models",
            "EndpointKind::Embeddings",
            "FailureNotRetryable",
        ],
    );

    assert_contains_all(
        test_body(&routing, "retry_gate_denies_named_channel_selection"),
        "M3 named-pool retry denial boundary",
        &["SelectionReason::NamedChannel", "FailureNotRetryable"],
    );

    assert_contains_all(
        test_body(
            &routing,
            "upstream_transaction_same_target_retry_has_unknown_duplicate_charge_risk",
        ),
        "same-target duplicate-charge risk characterization",
        &[
            "RetryDirective::RetrySameTarget",
            "DuplicateChargeRisk::Unknown",
        ],
    );

    assert_contains_all(
        test_body(
            &failure_observer,
            "retry_decision_telemetry_uses_stable_success_reason_codes",
        ),
        "stable retry telemetry code characterization",
        &["retry_same_target", "pre_output_transient_retry_allowed"],
    );

    assert_contains_all(
        &failure_observer,
        "stable retry denial telemetry codes",
        &["\"streaming_not_retryable\"", "\"partial_output_started\""],
    );
}
