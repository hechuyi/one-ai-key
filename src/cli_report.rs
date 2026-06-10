use clap::ValueEnum;
use serde_json::Value;

pub const LIST_RELOAD_STATUS_NOT_EVALUATED: &str = "not_evaluated_for_list_use_reload_status";
pub const LIST_CAPABILITY_STATUS_NOT_EVALUATED: &str = "not_evaluated_for_list_use_models_explain";
pub const MANUAL_FOLLOWUP_REQUIRED: &str = "manual_followup_required";
pub const MANUAL_CONFIG_OR_REGISTRY_UPDATE_REQUIRED: &str =
    "manual_config_or_registry_update_required";
pub const BOUNDED_STATS_PROJECTION_AVAILABLE: &str = "bounded_stats_projection_available";
pub const BOUNDED_CREDENTIAL_REFS_REQUIRED: &str = "bounded_credential_refs_required";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
}

pub fn escape_table_value(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| ch.escape_default())
        .collect::<String>()
}

pub struct ReportEnvelope<'a> {
    pub status: &'a str,
    pub reason: &'a str,
    pub reason_code: &'a str,
    pub effect: crate::cli_effects::CommandEffect,
    pub scope: Value,
    pub window: Value,
    pub next_action: Value,
    pub data: Value,
}

pub fn report_envelope_with_legacy_fields(input: ReportEnvelope<'_>) -> Value {
    let mut report = serde_json::json!({
        "status": input.status,
        "reason": input.reason,
        "reason_code": input.reason_code,
        "side_effect_class": crate::cli_effects::side_effect_class_code(input.effect.side_effect_class),
        "effect_vector": crate::cli_effects::effect_vector_json(input.effect.effect_vector),
        "scope": input.scope,
        "window": input.window,
        "next_action": input.next_action,
        "data": input.data,
    });
    let data_clone = report.get("data").cloned();
    if let (Some(report), Some(data)) = (report.as_object_mut(), data_clone) {
        if let Some(data) = data.as_object() {
            for (key, value) in data {
                report.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    report
}

pub fn append_report_envelope_table_fields(output: &mut String, report: &Value) {
    push_table_field(output, "status", report.get("status"));
    push_table_field(output, "reason_code", report.get("reason_code"));
    push_table_field(output, "reason", report.get("reason"));
    push_table_field(output, "side_effect_class", report.get("side_effect_class"));
    if let Some(effect) = report.get("effect_vector") {
        for field in [
            "reads_local_files",
            "reads_management_runtime",
            "reads_management_store",
            "writes_local_files",
            "writes_management_store",
            "calls_upstream",
            "mutates_runtime",
        ] {
            push_table_field(output, &format!("effect.{field}"), effect.get(field));
        }
    }
    if let Some(scope) = report.get("scope").and_then(Value::as_object) {
        for (key, value) in scope {
            push_table_field(output, &format!("scope.{key}"), Some(value));
        }
    }
    if let Some(window) = report.get("window").filter(|value| !value.is_null()) {
        for field in ["kind", "limit", "returned", "truncated", "bounded_reason"] {
            push_table_field(output, &format!("window.{field}"), window.get(field));
        }
    }
    append_next_action_table_fields(output, report.get("next_action"));
}

pub fn append_next_action_table_fields(output: &mut String, next_action: Option<&Value>) {
    let Some(next_action) = next_action else {
        return;
    };
    push_table_field(output, "next_action", next_action.get("summary"));
    push_table_field(
        output,
        "next_action.template_id",
        next_action.get("template_id"),
    );
    push_table_field(
        output,
        "next_action.side_effect_class",
        next_action.get("side_effect_class"),
    );
    push_table_field(
        output,
        "next_action.requires_confirmation",
        next_action.get("requires_confirmation"),
    );
    if let Some(argv) = next_action.get("safe_argv").and_then(Value::as_array) {
        if argv.is_empty() {
            output.push_str("next_action.safe_argv: []\n");
        } else {
            for (index, arg) in argv.iter().enumerate() {
                push_table_field(
                    output,
                    &format!("next_action.safe_argv[{index}]"),
                    Some(arg),
                );
            }
        }
    }
}

pub fn push_table_field(output: &mut String, name: &str, value: Option<&Value>) {
    let rendered = match value {
        Some(Value::String(value)) => escape_table_value(value),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Null) | None => "unknown".to_string(),
        Some(value) => escape_table_value(&value.to_string()),
    };
    output.push_str(&format!("{name}: {rendered}\n"));
}

pub fn sanitize_endpoint_capabilities(value: Option<&Value>) -> Value {
    let Some(value) = value.and_then(Value::as_object) else {
        return Value::Null;
    };
    let Some(chat_completions) = endpoint_support_field(value, "chat_completions") else {
        return Value::Null;
    };
    let Some(responses) = endpoint_support_field(value, "responses") else {
        return Value::Null;
    };
    let Some(embeddings) = endpoint_support_field(value, "embeddings") else {
        return Value::Null;
    };
    let Some(models) = models_capability_field(value, "models") else {
        return Value::Null;
    };
    let Some(diagnostic_labels) = value
        .get("diagnostic_labels")
        .and_then(Value::as_array)
        .and_then(|labels| labels.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
    else {
        return Value::Null;
    };

    serde_json::json!({
        "chat_completions": chat_completions,
        "responses": responses,
        "embeddings": embeddings,
        "models": models,
        "diagnostic_labels": diagnostic_labels,
    })
}

pub fn endpoint_capability_status_from_candidates(candidates: &[Value]) -> &'static str {
    if candidates.iter().any(|candidate| {
        candidate
            .get("endpoint_capabilities")
            .is_some_and(|capabilities| capabilities.as_object().is_some())
    }) {
        "available"
    } else {
        "unknown"
    }
}

pub fn endpoint_capabilities_table_summary(capabilities: Option<&Value>) -> String {
    let Some(capabilities) = capabilities.and_then(Value::as_object) else {
        return "endpoint_capabilities=unknown".to_string();
    };
    let chat_completions = capabilities
        .get("chat_completions")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let responses = capabilities
        .get("responses")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let embeddings = capabilities
        .get("embeddings")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let models = capabilities
        .get("models")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let labels = capabilities
        .get("diagnostic_labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();

    format!(
        "endpoint_capabilities.chat_completions={} endpoint_capabilities.responses={} endpoint_capabilities.embeddings={} endpoint_capabilities.models={} endpoint_capabilities.diagnostic_labels={}",
        escape_table_value(chat_completions),
        escape_table_value(responses),
        escape_table_value(embeddings),
        escape_table_value(models),
        escape_table_value(&labels),
    )
}

fn endpoint_support_field<'a>(
    value: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Option<&'a str> {
    let value = value.get(field)?.as_str()?;
    matches!(value, "supported" | "unsupported" | "unknown").then_some(value)
}

fn models_capability_field<'a>(
    value: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Option<&'a str> {
    let value = value.get(field)?.as_str()?;
    matches!(value, "local_projection" | "unsupported" | "unknown").then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_report_output_format_defaults_to_table() {
        assert_eq!(OutputFormat::default(), OutputFormat::Table);
    }

    #[test]
    fn escape_table_value_escapes_control_characters() {
        assert_eq!(
            escape_table_value("line\nreturn\rtab\tansi\u{1b}[31m"),
            "line\\nreturn\\rtab\\tansi\\u{1b}[31m"
        );
    }

    #[test]
    fn endpoint_capability_cli_helper_sanitizes_known_shape() {
        let value = serde_json::json!({
            "chat_completions": "supported",
            "responses": "unknown",
            "embeddings": "unsupported",
            "models": "local_projection",
            "diagnostic_labels": ["relay", "line\nlabel"],
            "secret": "SHOULD_NOT_RENDER"
        });

        let sanitized = sanitize_endpoint_capabilities(Some(&value));

        assert_eq!(sanitized["chat_completions"], "supported");
        assert!(sanitized.get("secret").is_none());
        assert_eq!(
            endpoint_capabilities_table_summary(Some(&sanitized)),
            "endpoint_capabilities.chat_completions=supported endpoint_capabilities.responses=unknown endpoint_capabilities.embeddings=unsupported endpoint_capabilities.models=local_projection endpoint_capabilities.diagnostic_labels=relay,line\\nlabel"
        );
    }

    #[test]
    fn report_envelope_with_legacy_fields_preserves_contract_and_does_not_allow_data_override() {
        let report = report_envelope_with_legacy_fields(ReportEnvelope {
            status: "ok",
            reason: "operator report is available",
            reason_code: "report_available",
            effect: crate::cli_effects::runtime_readonly_effect(),
            scope: serde_json::json!({"model": "gpt-example"}),
            window: serde_json::json!({"kind": "bounded_recent_events", "limit": 50}),
            next_action: serde_json::json!({
                "summary": "No action required.",
                "template_id": "no_action_required",
                "safe_argv": [],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false,
            }),
            data: serde_json::json!({
                "status": "forged",
                "reason": "forged",
                "reason_code": "forged_reason",
                "side_effect_class": "management_write",
                "next_action": {"safe_argv": ["unsafe"]},
                "legacy_count": 3,
            }),
        });

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason"], "operator report is available");
        assert_eq!(report["reason_code"], "report_available");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["next_action"]["template_id"], "no_action_required");
        assert_eq!(report["legacy_count"], 3);
        assert_eq!(report["data"]["status"], "forged");
    }

    #[test]
    fn append_report_envelope_table_fields_renders_effect_window_and_safe_argv() {
        let report = report_envelope_with_legacy_fields(ReportEnvelope {
            status: "degraded",
            reason: "bounded evidence was inspected",
            reason_code: "failures_found_in_window",
            effect: crate::cli_effects::runtime_readonly_store_reads_effect(),
            scope: serde_json::json!({
                "model": "gpt-example",
                "client_token_ref": "local-client",
            }),
            window: serde_json::json!({
                "kind": "bounded_recent_events",
                "limit": 100,
                "returned": 2,
                "truncated": false,
                "bounded_reason": "latest_window",
            }),
            next_action: serde_json::json!({
                "summary": "Inspect route state.",
                "template_id": "route_explain",
                "safe_argv": ["one-ai-key", "route", "explain", "gpt-example"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false,
            }),
            data: serde_json::json!({"command": "failures tail"}),
        });
        let mut rendered = String::new();

        append_report_envelope_table_fields(&mut rendered, &report);

        assert!(rendered.contains("status: degraded"));
        assert!(rendered.contains("reason_code: failures_found_in_window"));
        assert!(rendered.contains("side_effect_class: runtime_readonly"));
        assert!(rendered.contains("effect.reads_management_runtime: true"));
        assert!(rendered.contains("effect.reads_management_store: true"));
        assert!(rendered.contains("scope.client_token_ref: local-client"));
        assert!(rendered.contains("window.kind: bounded_recent_events"));
        assert!(rendered.contains("window.limit: 100"));
        assert!(rendered.contains("window.returned: 2"));
        assert!(rendered.contains("next_action.safe_argv[0]: one-ai-key"));
        assert!(rendered.contains("next_action.safe_argv[3]: gpt-example"));
    }
}
