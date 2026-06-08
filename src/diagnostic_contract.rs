use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticContract {
    pub blocking_domain: &'static str,
    pub next_action: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContractEntry {
    reason_code: &'static str,
    blocking_domain: &'static str,
    action: SafeAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafeAction {
    None,
    ModelsExplain,
    ModelsExplainWithClientTokenRef,
    RouteExplain,
    FailuresTail,
    FailuresExplainRequest,
    Doctor,
    ReloadStatus,
}

const CONTRACTS: &[ContractEntry] = &[
    ContractEntry {
        reason_code: "available",
        blocking_domain: "none",
        action: SafeAction::None,
    },
    ContractEntry {
        reason_code: "model_visible_to_client",
        blocking_domain: "none",
        action: SafeAction::None,
    },
    ContractEntry {
        reason_code: "no_failures_in_window",
        blocking_domain: "none",
        action: SafeAction::None,
    },
    ContractEntry {
        reason_code: "request_failure_not_found_in_window",
        blocking_domain: "none",
        action: SafeAction::None,
    },
    ContractEntry {
        reason_code: "token_missing",
        blocking_domain: "client_token",
        action: SafeAction::ModelsExplainWithClientTokenRef,
    },
    ContractEntry {
        reason_code: "token_unknown",
        blocking_domain: "client_token",
        action: SafeAction::Doctor,
    },
    ContractEntry {
        reason_code: "token_disabled",
        blocking_domain: "client_token",
        action: SafeAction::Doctor,
    },
    ContractEntry {
        reason_code: "model_not_in_client_scope",
        blocking_domain: "client_token",
        action: SafeAction::ModelsExplainWithClientTokenRef,
    },
    ContractEntry {
        reason_code: "client_channel_scope",
        blocking_domain: "client_token",
        action: SafeAction::ModelsExplainWithClientTokenRef,
    },
    ContractEntry {
        reason_code: "model_missing",
        blocking_domain: "model",
        action: SafeAction::ModelsExplain,
    },
    ContractEntry {
        reason_code: "no_runtime_route_candidate",
        blocking_domain: "route",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "no_route",
        blocking_domain: "route",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "no_route_candidate",
        blocking_domain: "route",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "target_disabled",
        blocking_domain: "target",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "no_available_credentials",
        blocking_domain: "target",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "no_usable_key_or_target",
        blocking_domain: "target",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "channel_disabled",
        blocking_domain: "target",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "channel_cooling_down",
        blocking_domain: "target",
        action: SafeAction::FailuresTail,
    },
    ContractEntry {
        reason_code: "channel_degraded",
        blocking_domain: "target",
        action: SafeAction::FailuresTail,
    },
    ContractEntry {
        reason_code: "credential_unavailable",
        blocking_domain: "target",
        action: SafeAction::FailuresTail,
    },
    ContractEntry {
        reason_code: "unknown_channel",
        blocking_domain: "target",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "runtime_unavailable",
        blocking_domain: "runtime",
        action: SafeAction::ReloadStatus,
    },
    ContractEntry {
        reason_code: "provider_cooling_down",
        blocking_domain: "target",
        action: SafeAction::FailuresTail,
    },
    ContractEntry {
        reason_code: "degraded_last_resort",
        blocking_domain: "target",
        action: SafeAction::FailuresTail,
    },
    ContractEntry {
        reason_code: "provider_cooling_down_last_resort",
        blocking_domain: "target",
        action: SafeAction::FailuresTail,
    },
    ContractEntry {
        reason_code: "unsupported_endpoint_family",
        blocking_domain: "endpoint_family",
        action: SafeAction::ModelsExplain,
    },
    ContractEntry {
        reason_code: "endpoint_family_unsupported",
        blocking_domain: "endpoint_family",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "endpoint_family_mismatch",
        blocking_domain: "endpoint_family",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "upstream_5xx",
        blocking_domain: "upstream",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "upstream_timeout",
        blocking_domain: "upstream",
        action: SafeAction::RouteExplain,
    },
    ContractEntry {
        reason_code: "response_filter_rejected",
        blocking_domain: "response_filter",
        action: SafeAction::FailuresExplainRequest,
    },
    ContractEntry {
        reason_code: "stream_committed_failure",
        blocking_domain: "response_filter",
        action: SafeAction::FailuresExplainRequest,
    },
    ContractEntry {
        reason_code: "failures_found_in_window",
        blocking_domain: "failure_window",
        action: SafeAction::FailuresTail,
    },
    ContractEntry {
        reason_code: "unknown_failure_class",
        blocking_domain: "failure_window",
        action: SafeAction::FailuresTail,
    },
    ContractEntry {
        reason_code: "unknown",
        blocking_domain: "unknown",
        action: SafeAction::Doctor,
    },
    ContractEntry {
        reason_code: "admission_summary_missing",
        blocking_domain: "unknown",
        action: SafeAction::Doctor,
    },
];

#[cfg(test)]
pub const STAGE2_REASON_CODES: &[&str] = &[
    "available",
    "model_visible_to_client",
    "no_failures_in_window",
    "request_failure_not_found_in_window",
    "token_missing",
    "token_unknown",
    "token_disabled",
    "model_not_in_client_scope",
    "client_channel_scope",
    "model_missing",
    "no_runtime_route_candidate",
    "no_route",
    "no_route_candidate",
    "target_disabled",
    "no_available_credentials",
    "no_usable_key_or_target",
    "channel_disabled",
    "channel_cooling_down",
    "channel_degraded",
    "credential_unavailable",
    "unknown_channel",
    "runtime_unavailable",
    "provider_cooling_down",
    "degraded_last_resort",
    "provider_cooling_down_last_resort",
    "unsupported_endpoint_family",
    "endpoint_family_unsupported",
    "endpoint_family_mismatch",
    "upstream_5xx",
    "upstream_timeout",
    "response_filter_rejected",
    "stream_committed_failure",
    "failures_found_in_window",
    "unknown_failure_class",
];

pub fn contract_for_reason(reason_code: &str) -> Option<DiagnosticContract> {
    CONTRACTS
        .iter()
        .find(|entry| entry.reason_code == reason_code)
        .map(|entry| DiagnosticContract {
            blocking_domain: entry.blocking_domain,
            next_action: next_action_value(entry.action),
        })
}

pub fn fallback_contract() -> DiagnosticContract {
    contract_for_reason("unknown").expect("unknown diagnostic contract should be registered")
}

#[cfg(test)]
pub fn contract_count_for_reason(reason_code: &str) -> usize {
    CONTRACTS
        .iter()
        .filter(|entry| entry.reason_code == reason_code)
        .count()
}

#[cfg(test)]
pub fn is_valid_safe_next_action(value: &Value) -> bool {
    let Some(action) = value.as_object() else {
        return false;
    };
    if action.get("side_effect_class").and_then(Value::as_str) != Some("runtime_readonly") {
        return false;
    }
    if action.get("requires_confirmation").and_then(Value::as_bool) != Some(false) {
        return false;
    }
    let Some(template_id) = action.get("template_id").and_then(Value::as_str) else {
        return false;
    };
    if !is_safe_identifier(template_id) {
        return false;
    }
    let Some(argv) = action.get("safe_argv").and_then(Value::as_array) else {
        return false;
    };
    let argv = argv.iter().map(Value::as_str).collect::<Option<Vec<_>>>();
    let Some(argv) = argv else {
        return false;
    };
    is_allowed_safe_argv(&argv) && argv.iter().copied().all(is_safe_argv_arg)
}

fn next_action_value(action: SafeAction) -> Value {
    let spec = action_spec(action);
    serde_json::json!({
        "summary": spec.summary,
        "template_id": spec.template_id,
        "safe_argv": spec.safe_argv,
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    })
}

#[derive(Debug, Clone, Copy)]
struct ActionSpec {
    summary: &'static str,
    template_id: &'static str,
    safe_argv: &'static [&'static str],
}

fn action_spec(action: SafeAction) -> ActionSpec {
    match action {
        SafeAction::None => ActionSpec {
            summary: "No additional diagnostic action is required.",
            template_id: "no_action_required",
            safe_argv: &[],
        },
        SafeAction::ModelsExplain => ActionSpec {
            summary: "Inspect the runtime model projection for this public model.",
            template_id: "models_explain",
            safe_argv: &[
                "one-ai-key",
                "models",
                "explain",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "--model",
                "<public-model>",
            ],
        },
        SafeAction::ModelsExplainWithClientTokenRef => ActionSpec {
            summary: "Inspect the runtime model projection with an explicit client-token reference.",
            template_id: "models_explain_visibility",
            safe_argv: &[
                "one-ai-key",
                "models",
                "explain",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "--model",
                "<public-model>",
                "--client-token-ref",
                "<client-token-ref>",
            ],
        },
        SafeAction::RouteExplain => ActionSpec {
            summary: "Inspect route candidates and admission details for this public model.",
            template_id: "route_explain",
            safe_argv: &[
                "one-ai-key",
                "route",
                "explain",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "<public-model>",
            ],
        },
        SafeAction::FailuresTail => ActionSpec {
            summary: "Inspect recent bounded failure evidence.",
            template_id: "failures_tail",
            safe_argv: &[
                "one-ai-key",
                "failures",
                "tail",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "--last",
                "50",
            ],
        },
        SafeAction::FailuresExplainRequest => ActionSpec {
            summary: "Inspect the bounded failure record for this request.",
            template_id: "failures_explain_request",
            safe_argv: &[
                "one-ai-key",
                "failures",
                "explain",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "<request-id>",
                "--last",
                "50",
            ],
        },
        SafeAction::Doctor => ActionSpec {
            summary: "Inspect the read-only runtime doctor projection.",
            template_id: "doctor",
            safe_argv: &[
                "one-ai-key",
                "doctor",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
            ],
        },
        SafeAction::ReloadStatus => ActionSpec {
            summary: "Inspect read-only runtime reload status.",
            template_id: "reload_status",
            safe_argv: &[
                "one-ai-key",
                "reload",
                "status",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
            ],
        },
    }
}

#[cfg(test)]
fn is_allowed_safe_argv(argv: &[&str]) -> bool {
    if argv.is_empty() {
        return true;
    }
    matches!(
        argv,
        [
            "one-ai-key",
            "models",
            "explain",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>",
            "--model",
            "<public-model>"
        ] | [
            "one-ai-key",
            "models",
            "explain",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>",
            "--model",
            "<public-model>",
            "--client-token-ref",
            "<client-token-ref>"
        ] | [
            "one-ai-key",
            "models",
            "explain",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>",
            "--model",
            "<public-model>",
            "--endpoint-family",
            "chat_completions"
        ] | [
            "one-ai-key",
            "route",
            "explain",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>",
            "<public-model>"
        ] | [
            "one-ai-key",
            "failures",
            "tail",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>",
            "--last",
            "50"
        ] | [
            "one-ai-key",
            "failures",
            "explain",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>",
            "<request-id>",
            "--last",
            "50"
        ] | [
            "one-ai-key",
            "doctor",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>"
        ] | [
            "one-ai-key",
            "reload",
            "status",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>"
        ] | [
            "one-ai-key",
            "reload",
            "diff",
            "--management-url",
            "<url>",
            "--management-token-env",
            "<env>"
        ]
    )
}

#[cfg(test)]
fn is_safe_argv_arg(arg: &str) -> bool {
    if arg.is_empty()
        || arg.len() > 128
        || arg.starts_with('/')
        || arg.starts_with("~/")
        || arg.starts_with("./")
        || arg.starts_with("../")
        || arg.contains("://")
        || arg.contains('\\')
        || arg.chars().any(char::is_control)
        || looks_like_windows_absolute_path(arg)
    {
        return false;
    }
    if arg.bytes().any(|byte| {
        matches!(
            byte,
            b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'$' | b'`' | b'\'' | b'"'
        )
    }) {
        return false;
    }
    let lower = arg.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("secret")
        || lower.contains("authorization")
        || lower.contains("bearer")
        || lower.contains("api_key")
        || lower.contains("apikey")
    {
        return false;
    }
    arg.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'<' | b'>')
    })
}

#[cfg(test)]
fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage2_reason_has_exactly_one_safe_readonly_contract() {
        assert!(!STAGE2_REASON_CODES.is_empty());
        for reason_code in STAGE2_REASON_CODES {
            assert_eq!(
                contract_count_for_reason(reason_code),
                1,
                "{reason_code} should map to exactly one diagnostic contract"
            );
            let contract = contract_for_reason(reason_code).unwrap();
            assert_eq!(
                contract.next_action["side_effect_class"],
                "runtime_readonly"
            );
            assert_eq!(contract.next_action["requires_confirmation"], false);
            assert!(
                is_valid_safe_next_action(&contract.next_action),
                "{reason_code} should map to a safe read-only next action"
            );
        }
    }

    #[test]
    fn unsafe_diagnostic_next_actions_are_rejected() {
        let unsafe_actions = [
            serde_json::json!({
                "summary": "unsafe",
                "template_id": "keys_import",
                "safe_argv": ["one-ai-key", "keys", "import", "--file", "<path>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }),
            serde_json::json!({
                "summary": "unsafe",
                "template_id": "keys_probe",
                "safe_argv": ["one-ai-key", "keys", "probe", "--target", "<channel>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }),
            serde_json::json!({
                "summary": "unsafe",
                "template_id": "reload_apply",
                "safe_argv": ["one-ai-key", "reload", "apply", "--dry-run", "--yes"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }),
            serde_json::json!({
                "summary": "unsafe",
                "template_id": "curl",
                "safe_argv": ["curl", "https://example.invalid/path"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }),
            serde_json::json!({
                "summary": "unsafe",
                "template_id": "route_explain",
                "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "/tmp/private-model"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }),
            serde_json::json!({
                "summary": "unsafe",
                "template_id": "route_explain",
                "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "$(touch /tmp/x)"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }),
        ];

        for action in unsafe_actions {
            assert!(
                !is_valid_safe_next_action(&action),
                "unsafe action should be rejected: {action}"
            );
        }
    }
}
