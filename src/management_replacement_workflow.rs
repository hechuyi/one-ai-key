use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub(crate) struct ReplacementWorkflowInput<'a> {
    pub credential_set_id: &'a str,
    pub include_credential_refs: bool,
    pub capacity_summary: &'a Value,
    pub replacement_need: &'a Value,
    pub route_impact: &'a Value,
}

pub(crate) fn project_replacement_workflow(input: ReplacementWorkflowInput<'_>) -> Value {
    let need_status = string_field(input.replacement_need, "status");
    let need_reason = string_field(input.replacement_need, "reason_code");
    let route_status = string_field(input.route_impact, "status");
    let candidate_presence = input
        .route_impact
        .get("requested_credential_set")
        .and_then(|value| value.get("candidate_presence"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    let route_unavailable = route_status == "unavailable_without_route_projection";
    let replacement_recommended = need_status == "replacement_recommended";
    let monitor_capacity = need_status == "monitor";
    let not_candidate = candidate_presence == "not_candidate";
    let selected_candidate = candidate_presence == "selected_candidate";
    let fallback_candidate = candidate_presence == "candidate_not_selected";

    let operator_maintenance_priority = if route_unavailable {
        "route_impact_unknown"
    } else if not_candidate {
        "inventory_only"
    } else if replacement_recommended {
        if fallback_candidate {
            "fallback_capacity_low"
        } else {
            "active_capacity_missing"
        }
    } else if monitor_capacity {
        if selected_candidate {
            "selected_capacity_low"
        } else if fallback_candidate {
            "fallback_capacity_low"
        } else {
            "inventory_only"
        }
    } else {
        "inventory_only"
    };

    let blocking_reason_code = if route_unavailable {
        "route_preview_unavailable"
    } else if not_candidate {
        "route_not_candidate"
    } else if replacement_recommended {
        replacement_blocking_reason_for_input(input, need_reason)
    } else {
        "none"
    };

    json!({
        "operator_maintenance_priority": operator_maintenance_priority,
        "blocking_reason_code": blocking_reason_code,
        "current_evidence": {
            "capacity_total": number_field(input.capacity_summary, "total"),
            "capacity_available": number_field(input.capacity_summary, "available"),
            "replacement_need_status": safe_code_or_unknown(need_status),
            "replacement_need_reason_code": safe_code_or_unknown(need_reason),
            "route_impact_status": safe_code_or_unknown(route_status),
            "candidate_presence": safe_code_or_unknown(candidate_presence),
        },
        "next_action": replacement_workflow_next_action(
            input,
            operator_maintenance_priority,
        ),
    })
}

fn replacement_blocking_reason(reason_code: &str) -> &'static str {
    match reason_code {
        "no_available_credentials" => "no_available_credentials",
        "credential_set_empty" | "credential_set_exhausted" | "import_replacement_credentials" => {
            "credential_set_exhausted"
        }
        _ => "no_available_credentials",
    }
}

fn replacement_blocking_reason_for_input(
    input: ReplacementWorkflowInput<'_>,
    reason_code: &str,
) -> &'static str {
    let total = number_field(input.capacity_summary, "total");
    let available = number_field(input.capacity_summary, "available");
    if total == 0 {
        return "credential_set_exhausted";
    }
    if available == 0 {
        return "no_available_credentials";
    }
    replacement_blocking_reason(reason_code)
}

fn replacement_workflow_next_action(
    input: ReplacementWorkflowInput<'_>,
    operator_maintenance_priority: &str,
) -> Value {
    match operator_maintenance_priority {
        "route_impact_unknown" => route_explain_action(input).unwrap_or_else(|| {
            keys_stats_action(input.credential_set_id, input.include_credential_refs)
        }),
        "active_capacity_missing" => keys_import_dry_run_action(input.credential_set_id),
        _ => keys_stats_action(input.credential_set_id, input.include_credential_refs),
    }
}

fn route_explain_action(input: ReplacementWorkflowInput<'_>) -> Option<Value> {
    let model = input.route_impact.get("model").and_then(Value::as_str)?;
    let model = safe_argv_arg(model)?;
    let mut argv = vec![
        Value::from("one-ai-key"),
        Value::from("route"),
        Value::from("explain"),
        Value::from(model),
    ];
    if let Some(client_token_ref) = input
        .route_impact
        .get("client_token_ref")
        .and_then(Value::as_str)
        .and_then(safe_argv_arg)
    {
        argv.push(Value::from("--client-token-ref"));
        argv.push(Value::from(client_token_ref));
    }
    Some(json!({
        "summary": "Re-check route impact through the read-only route explain projection.",
        "template_id": "route_explain",
        "safe_argv": Value::Array(argv),
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    }))
}

fn keys_import_dry_run_action(credential_set_id: &str) -> Value {
    let safe_id = safe_argv_arg(credential_set_id);
    let safe_argv = safe_id
        .map(|credential_set_id| {
            json!([
                "one-ai-key",
                "keys",
                "import",
                "--credential-set",
                credential_set_id,
                "--source",
                "replacement.keys",
                "--dry-run"
            ])
        })
        .unwrap_or(Value::Null);
    json!({
        "summary": "Preview a replacement import locally; this does not send credentials to management.",
        "template_id": "keys_import_dry_run",
        "safe_argv": safe_argv,
        "side_effect_class": "local_preview",
        "requires_confirmation": false,
    })
}

fn keys_stats_action(credential_set_id: &str, include_credential_refs: bool) -> Value {
    let mut argv = vec![
        Value::from("one-ai-key"),
        Value::from("keys"),
        Value::from("stats"),
    ];
    if let Some(credential_set_id) = safe_argv_arg(credential_set_id) {
        argv.push(Value::from("--credential-set"));
        argv.push(Value::from(credential_set_id));
    }
    if include_credential_refs {
        argv.push(Value::from("--include-credential-refs"));
    }
    json!({
        "summary": "Refresh credential-set stats from read-only management projections.",
        "template_id": "keys_stats",
        "safe_argv": Value::Array(argv),
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    })
}

fn string_field<'a>(value: &'a Value, field: &str) -> &'a str {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn number_field(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn safe_code_or_unknown(value: &str) -> String {
    safe_argv_arg(value).unwrap_or_else(|| "unknown".to_string())
}

fn safe_argv_arg(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("://")
        || lower.contains("http")
        || lower.contains("www.")
        || lower.contains("token")
        || lower.contains("secret")
    {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workflow(capacity_summary: Value, replacement_need: Value, route_impact: Value) -> Value {
        project_replacement_workflow(ReplacementWorkflowInput {
            credential_set_id: "relay_credentials",
            include_credential_refs: false,
            capacity_summary: &capacity_summary,
            replacement_need: &replacement_need,
            route_impact: &route_impact,
        })
    }

    fn route_with_candidate_presence(candidate_presence: &str) -> Value {
        json!({
            "status": "available",
            "model": "gpt-example",
            "client_token_ref": "local-client",
            "requested_credential_set": {
                "candidate_presence": candidate_presence
            }
        })
    }

    #[test]
    fn replacement_workflow_marks_empty_selected_set_as_exhausted() {
        let report = workflow(
            json!({"total": 0, "available": 0}),
            json!({
                "status": "replacement_recommended",
                "reason_code": "credential_set_empty"
            }),
            route_with_candidate_presence("selected_candidate"),
        );

        assert_eq!(
            report["operator_maintenance_priority"],
            "active_capacity_missing"
        );
        assert_eq!(report["blocking_reason_code"], "credential_set_exhausted");
        assert_eq!(report["next_action"]["template_id"], "keys_import_dry_run");
    }

    #[test]
    fn replacement_workflow_marks_selected_set_without_available_credentials() {
        let report = workflow(
            json!({"total": 3, "available": 0}),
            json!({
                "status": "replacement_recommended",
                "reason_code": "no_available_credentials"
            }),
            route_with_candidate_presence("selected_candidate"),
        );

        assert_eq!(
            report["operator_maintenance_priority"],
            "active_capacity_missing"
        );
        assert_eq!(report["blocking_reason_code"], "no_available_credentials");
        assert_eq!(report["current_evidence"]["capacity_available"], json!(0));
    }

    #[test]
    fn replacement_workflow_marks_selected_partial_loss_for_monitoring() {
        let report = workflow(
            json!({"total": 3, "available": 2}),
            json!({
                "status": "monitor",
                "reason_code": "partial_capacity_loss"
            }),
            route_with_candidate_presence("selected_candidate"),
        );

        assert_eq!(
            report["operator_maintenance_priority"],
            "selected_capacity_low"
        );
        assert_eq!(report["blocking_reason_code"], "none");
        assert_eq!(report["next_action"]["template_id"], "keys_stats");
    }

    #[test]
    fn replacement_workflow_marks_fallback_candidate_as_fallback_capacity_low() {
        let report = workflow(
            json!({"total": 3, "available": 1}),
            json!({
                "status": "monitor",
                "reason_code": "partial_capacity_loss"
            }),
            route_with_candidate_presence("candidate_not_selected"),
        );

        assert_eq!(
            report["operator_maintenance_priority"],
            "fallback_capacity_low"
        );
        assert_eq!(report["blocking_reason_code"], "none");
    }

    #[test]
    fn replacement_workflow_marks_not_candidate_as_inventory_only() {
        let report = workflow(
            json!({"total": 1, "available": 0}),
            json!({
                "status": "replacement_recommended",
                "reason_code": "no_available_credentials"
            }),
            route_with_candidate_presence("not_candidate"),
        );

        assert_eq!(report["operator_maintenance_priority"], "inventory_only");
        assert_eq!(report["blocking_reason_code"], "route_not_candidate");
        assert_eq!(report["next_action"]["template_id"], "keys_stats");
    }

    #[test]
    fn replacement_workflow_marks_missing_set_projection_as_inventory_only() {
        let report = workflow(
            json!({"total": 0, "available": 0}),
            json!({
                "status": "unknown",
                "reason_code": "credential_set_not_projected"
            }),
            json!({
                "status": "unavailable_without_model_context",
                "requested_credential_set": {
                    "candidate_presence": "unknown"
                }
            }),
        );

        assert_eq!(report["operator_maintenance_priority"], "inventory_only");
        assert_eq!(report["blocking_reason_code"], "none");
        assert_eq!(report["next_action"]["template_id"], "keys_stats");
    }

    #[test]
    fn replacement_workflow_marks_route_projection_failure_as_unknown_impact() {
        let report = workflow(
            json!({"total": 2, "available": 1}),
            json!({
                "status": "monitor",
                "reason_code": "partial_capacity_loss"
            }),
            json!({
                "status": "unavailable_without_route_projection",
                "model": "gpt-example",
                "client_token_ref": "local-client",
                "requested_credential_set": {
                    "candidate_presence": "unknown"
                }
            }),
        );

        assert_eq!(
            report["operator_maintenance_priority"],
            "route_impact_unknown"
        );
        assert_eq!(report["blocking_reason_code"], "route_preview_unavailable");
        assert_eq!(report["next_action"]["template_id"], "route_explain");
    }
}
