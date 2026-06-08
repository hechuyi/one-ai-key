use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteExplainOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub model: String,
    pub client_token_ref: Option<String>,
    pub output: crate::cli_report::OutputFormat,
}

pub async fn run(
    options: RouteExplainOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let mut preview = client.get_json(route_explain_endpoint(&options)).await?;
    let runtime_projection =
        crate::cli_commands::runtime_reload_projection::fetch_runtime_reload_projection(&client)
            .await?;
    crate::cli_commands::runtime_reload_projection::attach_runtime_reload_projection(
        &mut preview,
        runtime_projection,
    );
    Ok(render_route_explain_report(&preview, options.output))
}

fn route_explain_endpoint(
    options: &RouteExplainOptions,
) -> crate::operator_client::ReadOnlyEndpoint {
    crate::operator_client::ReadOnlyEndpoint::RoutingPreview {
        model: options.model.clone(),
        client_token_ref: options.client_token_ref.clone(),
    }
}

pub fn render_route_explain_report(
    preview: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => render_route_explain_json(preview),
        crate::cli_report::OutputFormat::Table => render_route_explain_table(preview),
    }
}

fn render_route_explain_json(preview: &Value) -> String {
    let report = sanitized_route_explain_report(preview);
    serde_json::to_string_pretty(&report).expect("route explain json report should serialize")
}

fn sanitized_route_explain_report(preview: &Value) -> Value {
    let effect = crate::cli_effects::runtime_readonly_effect();
    let runtime_reload =
        crate::cli_commands::runtime_reload_projection::summarize_runtime_reload_projection(
            preview
                .get("runtime_reload_projection")
                .or_else(|| preview.get("runtime_reload")),
        );
    let candidates = preview
        .get("candidates")
        .and_then(Value::as_array)
        .map(|candidates| {
            candidates
                .iter()
                .map(sanitize_candidate)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let capability_status =
        crate::cli_report::endpoint_capability_status_from_candidates(&candidates);
    let admission_summary = sanitize_admission_summary(preview.get("admission_summary"));
    let status = admission_summary
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let reason_code = admission_summary
        .get("reason_code")
        .and_then(Value::as_str)
        .unwrap_or("admission_summary_missing");
    let reason = route_admission_reason(status, reason_code);
    let data = serde_json::json!({
        "command": "route explain",
        "active_registry_generation": runtime_reload.active_registry_generation,
        "active_registry_version": runtime_reload.active_registry_version,
        "staged_registry_version": runtime_reload.staged_registry_version,
        "runtime_reload_required": runtime_reload.runtime_reload_required,
        "reload_diff_status": runtime_reload.reload_diff_status,
        "reload_diff_reason_code": runtime_reload.reload_diff_reason_code,
        "reload_diff_next_action": runtime_reload.reload_diff_next_action,
        "capability_status": capability_status,
        "model": preview.get("model").and_then(Value::as_str),
        "route_kind": preview.get("route_kind").and_then(Value::as_str),
        "registry_generation": preview.get("registry_generation").and_then(Value::as_u64),
        "candidate_limit": preview.get("candidate_limit").and_then(Value::as_u64),
        "policy_summary": sanitize_policy_summary(preview.get("policy_summary")),
        "client_token": sanitize_client_token(preview.get("client_token")),
        "selected_target": sanitize_target(preview.get("selected_target")),
        "admission_summary": admission_summary,
        "next_action": route_next_action(status, preview),
        "candidates": candidates,
    });
    envelope_with_legacy_fields(
        status,
        &reason,
        reason_code,
        effect,
        serde_json::json!({
            "model": preview.get("model").and_then(Value::as_str),
            "client_token_ref": preview
                .get("client_token")
                .and_then(|client_token| client_token.get("name"))
                .and_then(Value::as_str),
        }),
        route_next_action(status, preview),
        data,
    )
}

fn route_admission_reason(status: &str, reason_code: &str) -> String {
    match status {
        "available" => "Backend route admission projection reports an available route.".to_string(),
        "last_resort" => {
            format!(
                "Backend route admission projection reports last-resort admission: {reason_code}."
            )
        }
        "unavailable" => {
            format!(
                "Backend route admission projection reports admission unavailable: {reason_code}."
            )
        }
        _ => "Backend route admission projection is unavailable.".to_string(),
    }
}

fn route_next_action(status: &str, preview: &Value) -> Value {
    let model = preview
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    if matches!(status, "available" | "last_resort") {
        serde_json::json!({
            "summary": "A runtime route candidate is selected. No repair action is required by route explain.",
            "template_id": "no_action_required",
            "safe_argv": [],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        })
    } else {
        serde_json::json!({
            "summary": format!("No runtime route candidate is selected for public model {model}. Inspect model visibility and channel health when the corresponding read-only commands are available."),
            "template_id": "models_explain",
            "safe_argv": ["one-ai-key", "models", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "--model", "<public-model>"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        })
    }
}

fn sanitize_admission_summary(summary: Option<&Value>) -> Value {
    let Some(summary) = summary.and_then(Value::as_object) else {
        return serde_json::json!({
            "status": "unknown",
            "reason_code": "admission_summary_missing",
            "selected_target": Value::Null,
            "candidate_count": Value::Null,
            "included_count": Value::Null,
            "blocked_count": Value::Null,
            "soft_suppressed_count": Value::Null,
            "hard_blocked_count": Value::Null,
            "last_resort_used": false,
            "last_resort_reason": Value::Null,
        });
    };

    serde_json::json!({
        "status": safe_admission_code(summary.get("status")),
        "reason_code": safe_admission_code(summary.get("reason_code")),
        "selected_target": sanitize_target(summary.get("selected_target")),
        "candidate_count": summary.get("candidate_count").and_then(Value::as_u64),
        "included_count": summary.get("included_count").and_then(Value::as_u64),
        "blocked_count": summary.get("blocked_count").and_then(Value::as_u64),
        "soft_suppressed_count": summary
            .get("soft_suppressed_count")
            .and_then(Value::as_u64),
        "hard_blocked_count": summary
            .get("hard_blocked_count")
            .and_then(Value::as_u64),
        "last_resort_used": summary
            .get("last_resort_used")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "last_resort_reason": safe_optional_admission_code(summary.get("last_resort_reason")),
    })
}

fn safe_admission_code(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .filter(|value| safe_admission_code_str(value))
        .map(Value::from)
        .unwrap_or_else(|| Value::from("unknown"))
}

fn safe_optional_admission_code(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .filter(|value| safe_admission_code_str(value))
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn safe_admission_code_str(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn envelope_with_legacy_fields(
    status: &str,
    reason: &str,
    reason_code: &str,
    effect: crate::cli_effects::CommandEffect,
    scope: Value,
    next_action: Value,
    data: Value,
) -> Value {
    let mut report = serde_json::json!({
        "status": status,
        "reason": reason,
        "reason_code": reason_code,
        "side_effect_class": crate::cli_effects::side_effect_class_code(effect.side_effect_class),
        "effect_vector": crate::cli_effects::effect_vector_json(effect.effect_vector),
        "scope": scope,
        "window": Value::Null,
        "next_action": next_action,
        "data": data,
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

fn sanitize_policy_summary(policy: Option<&Value>) -> Value {
    let Some(policy) = policy else {
        return Value::Null;
    };
    serde_json::json!({
        "route_target_retry_enabled": policy
            .get("route_target_retry_enabled")
            .and_then(Value::as_bool),
        "same_request_credential_retry_enabled": policy
            .get("same_request_credential_retry_enabled")
            .and_then(Value::as_bool),
        "max_same_request_retries": policy
            .get("max_same_request_retries")
            .and_then(Value::as_u64),
        "candidate_limit": policy.get("candidate_limit").and_then(Value::as_u64),
    })
}

fn sanitize_client_token(client_token: Option<&Value>) -> Value {
    let Some(client_token) = client_token else {
        return Value::Null;
    };
    serde_json::json!({
        "id": client_token.get("id").and_then(Value::as_str),
        "name": client_token.get("name").and_then(Value::as_str),
        "enabled": client_token.get("enabled").and_then(Value::as_bool),
    })
}

fn sanitize_target(target: Option<&Value>) -> Value {
    let Some(target) = target else {
        return Value::Null;
    };
    serde_json::json!({
        "channel_id": target.get("channel_id").and_then(Value::as_str),
        "plan_position": target.get("plan_position").and_then(Value::as_u64),
    })
}

fn sanitize_candidate(candidate: &Value) -> Value {
    serde_json::json!({
        "target_index": candidate.get("target_index").and_then(Value::as_u64),
        "channel_id": candidate.get("channel_id").and_then(Value::as_str),
        "upstream_model": candidate.get("upstream_model").and_then(Value::as_str),
        "provider_kind": candidate.get("provider_kind").and_then(Value::as_str),
        "priority": candidate.get("priority").and_then(Value::as_u64),
        "weight": candidate.get("weight").and_then(Value::as_u64),
        "target_enabled": candidate.get("target_enabled").and_then(Value::as_bool),
        "included": candidate.get("included").and_then(Value::as_bool),
        "selected": candidate.get("selected").and_then(Value::as_bool),
        "plan_position": candidate.get("plan_position").and_then(Value::as_u64),
        "reasons": candidate
            .get("reasons")
            .and_then(Value::as_array)
            .map(|reasons| reasons.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default(),
        "health_kind": candidate
            .get("health")
            .and_then(|health| health.get("kind"))
            .and_then(Value::as_str),
        "health_reason_code": candidate
            .get("health")
            .and_then(|health| health.get("reason_code"))
            .and_then(Value::as_str),
        "health_generation": candidate
            .get("health")
            .and_then(|health| health.get("generation"))
            .and_then(Value::as_u64),
        "credential_set_id": candidate.get("credential_set_id").and_then(Value::as_str),
        "selector_generation": candidate.get("selector_generation").and_then(Value::as_u64),
        "credentials": sanitize_credential_counts(candidate.get("credentials")),
        "endpoint_capabilities": crate::cli_report::sanitize_endpoint_capabilities(
            candidate.get("endpoint_capabilities")
        ),
    })
}

fn sanitize_credential_counts(credentials: Option<&Value>) -> Value {
    let Some(credentials) = credentials else {
        return Value::Null;
    };
    let mut counts = serde_json::Map::new();
    for field in [
        "total",
        "available",
        "cooling_down",
        "expired",
        "quota_exhausted",
        "disabled",
    ] {
        if let Some(value) = credentials.get(field).and_then(Value::as_u64) {
            counts.insert(field.to_string(), Value::from(value));
        }
    }
    Value::Object(counts)
}

fn render_route_explain_table(preview: &Value) -> String {
    let report = sanitized_route_explain_report(preview);
    render_route_explain_table_from_report(&report)
}

fn render_route_explain_table_from_report(report: &Value) -> String {
    let model = report
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let mut output = String::new();
    output.push_str(&format!("Route plan for {}\n", display_value(model)));
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    crate::cli_commands::runtime_reload_projection::append_runtime_reload_table_fields(
        &mut output,
        report,
    );
    crate::cli_report::push_table_field(
        &mut output,
        "capability_status",
        report.get("capability_status"),
    );
    append_admission_summary_table_fields(&mut output, report.get("admission_summary"));

    if let Some(route_kind) = report.get("route_kind").and_then(Value::as_str) {
        output.push_str(&format!("route_kind: {}\n", display_value(route_kind)));
    }
    if let Some(registry_generation) = report.get("registry_generation").and_then(Value::as_u64) {
        output.push_str(&format!("registry_generation: {registry_generation}\n"));
    }
    if let Some(candidate_limit) = report.get("candidate_limit").and_then(Value::as_u64) {
        output.push_str(&format!("candidate_limit: {candidate_limit}\n"));
    }
    if let Some(policy) = report.get("policy_summary") {
        output.push_str(&format!(
            "route_target_retry_enabled: {}\n",
            policy
                .get("route_target_retry_enabled")
                .and_then(Value::as_bool)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ));
        output.push_str(&format!(
            "same_request_credential_retry_enabled: {}\n",
            policy
                .get("same_request_credential_retry_enabled")
                .and_then(Value::as_bool)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ));
        output.push_str(&format!(
            "max_same_request_retries: {}\n",
            policy
                .get("max_same_request_retries")
                .and_then(Value::as_u64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }
    let selected = report.get("selected_target");
    let selected_channel = selected
        .and_then(|target| target.get("channel_id"))
        .and_then(Value::as_str)
        .unwrap_or("none");
    output.push_str(&format!(
        "selected_target: {}\n",
        display_value(selected_channel)
    ));
    if let Some(plan_position) = selected
        .and_then(|target| target.get("plan_position"))
        .and_then(Value::as_u64)
    {
        output.push_str(&format!("selected_plan_position: {plan_position}\n"));
    }
    output.push_str("candidates:\n");

    for candidate in report
        .get("candidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let channel = candidate
            .get("channel_id")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>");
        let included = candidate
            .get("included")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let selected = candidate
            .get("selected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let health = candidate
            .get("health_kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let reasons = candidate
            .get("reasons")
            .and_then(Value::as_array)
            .map(|reasons| {
                reasons
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .filter(|reasons| !reasons.is_empty())
            .unwrap_or_else(|| "none".to_string());
        let upstream_model = candidate
            .get("upstream_model")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let credentials = candidate
            .get("credentials")
            .map(credential_summary)
            .unwrap_or_else(|| "credentials: unknown".to_string());
        let credential_set = candidate
            .get("credential_set_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let selector_generation = candidate
            .get("selector_generation")
            .and_then(Value::as_u64)
            .map(|generation| generation.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let endpoint_capabilities = crate::cli_report::endpoint_capabilities_table_summary(
            candidate.get("endpoint_capabilities"),
        );
        output.push_str(&format!(
            "- {} upstream_model={} included={included} selected={selected} health={} reasons={} credential_set={} selector_generation={} {credentials} {endpoint_capabilities}\n",
            display_value(channel),
            display_value(upstream_model),
            display_value(health),
            display_value(&reasons),
            display_value(credential_set),
            selector_generation,
        ));
    }
    output
}

fn append_admission_summary_table_fields(output: &mut String, admission_summary: Option<&Value>) {
    let Some(admission_summary) = admission_summary else {
        return;
    };
    crate::cli_report::push_table_field(
        output,
        "admission.status",
        admission_summary.get("status"),
    );
    crate::cli_report::push_table_field(
        output,
        "admission.reason_code",
        admission_summary.get("reason_code"),
    );
    let selected = admission_summary.get("selected_target");
    let selected_channel = selected
        .and_then(|target| target.get("channel_id"))
        .and_then(Value::as_str)
        .map(Value::from)
        .unwrap_or(Value::Null);
    crate::cli_report::push_table_field(
        output,
        "admission.selected_target",
        Some(&selected_channel),
    );
    if let Some(plan_position) = selected
        .and_then(|target| target.get("plan_position"))
        .and_then(Value::as_u64)
    {
        output.push_str(&format!(
            "admission.selected_plan_position: {plan_position}\n"
        ));
    }
    for field in [
        "candidate_count",
        "included_count",
        "blocked_count",
        "soft_suppressed_count",
        "hard_blocked_count",
        "last_resort_used",
        "last_resort_reason",
    ] {
        crate::cli_report::push_table_field(
            output,
            &format!("admission.{field}"),
            admission_summary.get(field),
        );
    }
}

fn credential_summary(credentials: &Value) -> String {
    let available = credentials
        .get("available")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let expired = credentials
        .get("expired")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!("credentials.available={available} credentials.expired={expired}")
}

fn display_value(value: &str) -> String {
    crate::cli_report::escape_table_value(value)
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, routing::get, Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    async fn spawn_management_fixture(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[test]
    fn route_explain_report_contains_candidates_and_omits_secret_like_fields() {
        let preview = json!({
            "model": "gpt-4o",
            "route_kind": "explicit_model_route",
            "registry_generation": 12,
            "candidate_limit": 16,
            "policy_summary": {
                "route_target_retry_enabled": true,
                "same_request_credential_retry_enabled": false,
                "max_same_request_retries": 0,
                "candidate_limit": 16
            },
            "client_token": {
                "id": "client-local",
                "name": "local-client",
                "unrestricted_model_groups": true,
                "unrestricted_channels": true
            },
            "selected_target": {
                "channel_id": "relay-a",
                "plan_position": 0
            },
            "admission_summary": {
                "status": "available",
                "reason_code": "available",
                "selected_target": {"channel_id": "relay-a", "plan_position": 0},
                "candidate_count": 2,
                "included_count": 1,
                "blocked_count": 1,
                "soft_suppressed_count": 0,
                "hard_blocked_count": 1,
                "last_resort_used": false,
                "last_resort_reason": null
            },
            "candidates": [
                {
                    "target_index": 0,
                    "channel_id": "relay-a",
                    "upstream_model": "provider/gpt-4o",
                    "provider_kind": "openai_compatible",
                    "target_enabled": true,
                    "plan_position": 0,
                    "included": true,
                    "selected": true,
                    "reasons": ["available"],
                    "health": {"kind": "available", "generation": 3},
                    "credential_set_id": "shared-credentials",
                    "selector_generation": 7,
                    "credentials": {"available": 2, "expired": 0, "total": 2}
                },
                {
                    "target_index": 1,
                    "channel_id": "relay-b",
                    "provider_kind": "openai_compatible",
                    "target_enabled": true,
                    "plan_position": 1,
                    "included": false,
                    "selected": false,
                    "reasons": ["channel_cooling_down"],
                    "health": {"kind": "cooling_down", "reason": "secret-token", "generation": 4},
                    "credential_set_id": "shared-credentials",
                    "selector_generation": 8,
                    "credentials": {"available": 0, "expired": 1, "total": 1}
                }
            ],
            "raw_token": "RAW_SECRET_SHOULD_NOT_APPEAR"
        });

        let rendered =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Table);

        assert!(rendered.contains("Route plan for gpt-4o"));
        assert!(rendered.contains("status: available"));
        assert!(rendered.contains("reason_code: available"));
        assert!(rendered.contains("side_effect_class: runtime_readonly"));
        assert!(rendered.contains("effect.reads_management_runtime: true"));
        assert!(rendered.contains("scope.model: gpt-4o"));
        assert!(rendered.contains("scope.client_token_ref: local-client"));
        assert!(rendered.contains("next_action:"));
        assert!(rendered.contains("next_action.safe_argv: []"));
        assert!(rendered.contains("admission.status: available"));
        assert!(rendered.contains("admission.reason_code: available"));
        assert!(rendered.contains("admission.candidate_count: 2"));
        assert!(rendered.contains("admission.included_count: 1"));
        assert!(rendered.contains("admission.blocked_count: 1"));
        assert!(rendered.contains("admission.hard_blocked_count: 1"));
        assert!(rendered.contains("selected_target: relay-a"));
        assert!(rendered.contains("selected_plan_position: 0"));
        assert!(rendered.contains("route_target_retry_enabled: true"));
        assert!(rendered.contains("same_request_credential_retry_enabled: false"));
        assert!(rendered.contains("relay-b"));
        assert!(rendered.contains("health=cooling_down"));
        assert!(rendered.contains("credential_set=shared-credentials"));
        assert!(rendered.contains("selector_generation=8"));
        assert!(rendered.contains("channel_cooling_down"));
        assert!(rendered.contains("reload_diff_status: unknown"));
        assert!(!rendered.contains("RAW_SECRET_SHOULD_NOT_APPEAR"));
        assert!(!rendered.contains("secret-token"));
    }

    #[test]
    fn route_explain_json_report_is_sanitized() {
        let preview = json!({
            "model": "gpt-4o",
            "policy_summary": {
                "route_target_retry_enabled": true,
                "same_request_credential_retry_enabled": false,
                "max_same_request_retries": 0,
                "candidate_limit": 16
            },
            "selected_target": {
                "channel_id": "relay-a",
                "plan_position": 0,
                "raw_key": "RAW_SECRET_SHOULD_NOT_APPEAR"
            },
            "admission_summary": {
                "status": "available",
                "reason_code": "available",
                "selected_target": {
                    "channel_id": "relay-a",
                    "plan_position": 0,
                    "raw_key": "RAW_SECRET_SHOULD_NOT_APPEAR"
                },
                "candidate_count": 1,
                "included_count": 1,
                "blocked_count": 0,
                "soft_suppressed_count": 0,
                "hard_blocked_count": 0,
                "last_resort_used": false,
                "last_resort_reason": null,
                "raw_body": "RAW_SECRET_SHOULD_NOT_APPEAR"
            },
            "candidates": [{
                "channel_id": "relay-a",
                "upstream_model": "provider/gpt-4o",
                "included": true,
                "selected": true,
                "reasons": ["available"],
                "target_enabled": true,
                "plan_position": 0,
                "credential_set_id": "shared-credentials",
                "selector_generation": 7,
                "health": {"kind": "available", "reason": "secret-token", "generation": 3},
                "credentials": {"available": 2, "expired": 0, "fingerprint": "abcd"}
            }],
            "raw_token": "RAW_SECRET_SHOULD_NOT_APPEAR"
        });

        let rendered =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Json);
        let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["reload_diff_status"], "unknown");
        assert_eq!(report["status"], "available");
        assert_eq!(report["reason_code"], "available");
        assert_eq!(report["admission_summary"]["status"], "available");
        assert_eq!(report["admission_summary"]["reason_code"], "available");
        assert_eq!(report["admission_summary"]["candidate_count"], 1);
        assert_eq!(report["admission_summary"]["included_count"], 1);
        assert_eq!(report["admission_summary"]["blocked_count"], 0);
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["reads_management_runtime"], true);
        assert_eq!(report["window"], serde_json::Value::Null);
        assert!(report["next_action"]["safe_argv"].is_array());
        let legacy_safe_field = ["safe", "command"].join("_");
        let legacy_dry_run_field = ["dry", "run", "command"].join("_");
        assert!(report["next_action"].get(&legacy_safe_field).is_none());
        assert!(report["next_action"].get(&legacy_dry_run_field).is_none());
        assert!(rendered.contains("\"channel_id\": \"relay-a\""));
        assert!(rendered.contains("\"plan_position\": 0"));
        assert!(rendered.contains("\"route_target_retry_enabled\": true"));
        assert!(rendered.contains("\"same_request_credential_retry_enabled\": false"));
        assert!(rendered.contains("\"health_kind\": \"available\""));
        assert!(rendered.contains("\"credential_set_id\": \"shared-credentials\""));
        assert!(rendered.contains("\"selector_generation\": 7"));
        assert!(!rendered.contains("RAW_SECRET_SHOULD_NOT_APPEAR"));
        assert!(!rendered.contains("secret-token"));
        assert!(!rendered.contains("fingerprint"));
    }

    #[test]
    fn route_explain_admission_summary_uses_backend_projection_without_reclassification() {
        let preview = json!({
            "model": "gpt-4o",
            "selected_target": {"channel_id": "relay-a", "plan_position": 0},
            "admission_summary": {
                "status": "last_resort",
                "reason_code": "provider_cooling_down_last_resort",
                "selected_target": {"channel_id": "relay-a", "plan_position": 0},
                "candidate_count": 3,
                "included_count": 1,
                "blocked_count": 2,
                "soft_suppressed_count": 1,
                "hard_blocked_count": 1,
                "last_resort_used": true,
                "last_resort_reason": "provider_cooling_down_last_resort"
            },
            "candidates": [{
                "channel_id": "relay-a",
                "included": true,
                "selected": true,
                "target_enabled": true,
                "plan_position": 0,
                "reasons": ["provider_cooling_down_last_resort"],
                "health": {"kind": "degraded", "generation": 3},
                "credentials": {"available": 1, "expired": 0}
            }]
        });

        let rendered_json =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Json);
        let report: serde_json::Value = serde_json::from_str(&rendered_json).unwrap();

        assert_eq!(report["status"], "last_resort");
        assert_eq!(report["reason_code"], "provider_cooling_down_last_resort");
        assert_eq!(report["admission_summary"]["status"], "last_resort");
        assert_eq!(
            report["admission_summary"]["reason_code"],
            "provider_cooling_down_last_resort"
        );
        assert_eq!(
            report["admission_summary"]["selected_target"]["channel_id"],
            "relay-a"
        );
        assert_eq!(report["admission_summary"]["candidate_count"], 3);
        assert_eq!(report["admission_summary"]["included_count"], 1);
        assert_eq!(report["admission_summary"]["blocked_count"], 2);
        assert_eq!(report["admission_summary"]["soft_suppressed_count"], 1);
        assert_eq!(report["admission_summary"]["hard_blocked_count"], 1);
        assert_eq!(report["admission_summary"]["last_resort_used"], true);
        assert_eq!(
            report["admission_summary"]["last_resort_reason"],
            "provider_cooling_down_last_resort"
        );
        assert_ne!(report["status"], "ok");
        assert_ne!(report["reason_code"], "route_candidate_selected");

        let rendered_table =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Table);
        assert!(rendered_table.contains("status: last_resort"));
        assert!(rendered_table.contains("reason_code: provider_cooling_down_last_resort"));
        assert!(rendered_table.contains("admission.status: last_resort"));
        assert!(rendered_table.contains("admission.reason_code: provider_cooling_down_last_resort"));
        assert!(rendered_table.contains("admission.selected_target: relay-a"));
        assert!(rendered_table.contains("admission.selected_plan_position: 0"));
        assert!(rendered_table.contains("admission.candidate_count: 3"));
        assert!(rendered_table.contains("admission.included_count: 1"));
        assert!(rendered_table.contains("admission.blocked_count: 2"));
        assert!(rendered_table.contains("admission.soft_suppressed_count: 1"));
        assert!(rendered_table.contains("admission.hard_blocked_count: 1"));
        assert!(rendered_table.contains("admission.last_resort_used: true"));
        assert!(rendered_table
            .contains("admission.last_resort_reason: provider_cooling_down_last_resort"));
    }

    #[test]
    fn endpoint_capability_cli_route_explain_shows_static_endpoint_capabilities() {
        let preview = json!({
            "model": "gpt-4o",
            "route_kind": "explicit_model_route",
            "selected_target": {"channel_id": "relay-a", "plan_position": 0},
            "candidates": [{
                "channel_id": "relay-a",
                "upstream_model": "provider/gpt-4o",
                "included": true,
                "selected": true,
                "target_enabled": true,
                "plan_position": 0,
                "reasons": ["available"],
                "health": {"kind": "available", "generation": 3},
                "credential_set_id": "shared-credentials",
                "selector_generation": 7,
                "credentials": {"available": 2, "expired": 0},
                "endpoint_capabilities": {
                    "chat_completions": "supported",
                    "responses": "unknown",
                    "embeddings": "unsupported",
                    "models": "local_projection",
                    "diagnostic_labels": ["relay", "line\nlabel"],
                    "raw_secret": "RAW_SECRET_SHOULD_NOT_APPEAR"
                }
            }]
        });

        let rendered_json =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Json);
        let report: serde_json::Value = serde_json::from_str(&rendered_json).unwrap();

        assert_eq!(report["capability_status"], "available");
        assert_eq!(
            report["candidates"][0]["endpoint_capabilities"],
            json!({
                "chat_completions": "supported",
                "responses": "unknown",
                "embeddings": "unsupported",
                "models": "local_projection",
                "diagnostic_labels": ["relay", "line\nlabel"]
            })
        );
        assert!(!rendered_json.contains("RAW_SECRET_SHOULD_NOT_APPEAR"));

        let rendered_table =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Table);
        assert!(rendered_table.contains("capability_status: available"));
        assert!(rendered_table.contains(
            "endpoint_capabilities.chat_completions=supported endpoint_capabilities.responses=unknown endpoint_capabilities.embeddings=unsupported endpoint_capabilities.models=local_projection endpoint_capabilities.diagnostic_labels=relay,line\\nlabel"
        ));
        assert!(!rendered_table.contains("line\nlabel"));
    }

    #[test]
    fn endpoint_capability_cli_preserves_table_json_semantics() {
        let preview = json!({
            "model": "gpt-4o",
            "selected_target": null,
            "candidates": [{
                "channel_id": "relay-a",
                "included": false,
                "selected": false,
                "health": {"kind": "available"},
                "endpoint_capabilities": {
                    "chat_completions": {"nested": "supported"},
                    "responses": "unknown",
                    "embeddings": "unsupported",
                    "models": "local_projection",
                    "diagnostic_labels": ["relay"],
                    "raw_secret": "RAW_SECRET_SHOULD_NOT_APPEAR"
                }
            }]
        });

        let rendered_json =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Json);
        let report: serde_json::Value = serde_json::from_str(&rendered_json).unwrap();
        assert_eq!(report["capability_status"], "unknown");
        assert!(report["candidates"][0]["endpoint_capabilities"].is_null());
        assert!(!rendered_json.contains("RAW_SECRET_SHOULD_NOT_APPEAR"));

        let rendered_table =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Table);
        assert!(rendered_table.contains("capability_status: unknown"));
        assert!(rendered_table.contains("endpoint_capabilities=unknown"));
        assert!(!rendered_table.contains("RAW_SECRET_SHOULD_NOT_APPEAR"));
    }

    #[test]
    fn route_explain_json_reports_runtime_reload_projection_without_sensitive_fields() {
        let legacy_safe_field = ["safe", "command"].join("_");
        let legacy_dry_run_field = ["dry", "run", "command"].join("_");
        let preview = json!({
            "model": "gpt-4o",
            "selected_target": {"channel_id": "relay-a", "plan_position": 0},
            "candidates": [],
            "runtime_reload_projection": {
                "status": "ok",
                "reason_code": "reload_diff_available",
                "active_registry_generation": 21,
                "active_registry_version": 4,
                "staged_registry_version": 5,
                "runtime_reload_required": true,
                "reload_apply_status": "dry_run_available",
                "next_action": {
                    "summary": "backend summary should not be copied",
                    "template_id": "reload_diff_available",
                    "safe_argv": ["unsafe-raw-path"],
                    legacy_safe_field: "DO_NOT_LEAK",
                    legacy_dry_run_field: "DO_NOT_LEAK"
                },
                "raw_yaml": "RAW_YAML_SHOULD_NOT_APPEAR",
                "token": "TOKEN_SHOULD_NOT_APPEAR"
            }
        });

        let rendered =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Json);
        let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["active_registry_generation"], 21);
        assert_eq!(report["active_registry_version"], 4);
        assert_eq!(report["staged_registry_version"], 5);
        assert_eq!(report["runtime_reload_required"], true);
        assert_eq!(report["reload_diff_status"], "ok");
        assert_eq!(report["reload_diff_reason_code"], "reload_diff_available");
        assert_eq!(
            report["reload_diff_next_action"]["template_id"],
            "reload_diff_available"
        );
        assert_eq!(
            report["reload_diff_next_action"]["side_effect_class"],
            "runtime_readonly"
        );
        assert_eq!(
            report["reload_diff_next_action"]["requires_confirmation"],
            false
        );
        assert_eq!(
            report["reload_diff_next_action"]["safe_argv"],
            json!([
                "one-ai-key",
                "reload",
                "diff",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>"
            ])
        );
        assert!(!rendered.contains("RAW_YAML_SHOULD_NOT_APPEAR"));
        assert!(!rendered.contains("TOKEN_SHOULD_NOT_APPEAR"));
        assert!(!rendered.contains("unsafe-raw-path"));
        assert!(!rendered.contains("DO_NOT_LEAK"));
        assert!(!rendered.contains("unavailable_until_m4\""));
    }

    #[test]
    fn route_explain_table_reports_runtime_reload_projection() {
        let preview = json!({
            "model": "gpt-4o",
            "selected_target": {"channel_id": "relay-a", "plan_position": 0},
            "candidates": [],
            "runtime_reload_projection": {
                "status": "unavailable",
                "reason_code": "unavailable_without_staged_projection",
                "active_registry_generation": 21,
                "active_registry_version": 4,
                "staged_registry_version": 5,
                "runtime_reload_required": true
            }
        });

        let rendered =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Table);

        assert!(rendered.contains("active_registry_generation: 21"));
        assert!(rendered.contains("active_registry_version: 4"));
        assert!(rendered.contains("staged_registry_version: 5"));
        assert!(rendered.contains("runtime_reload_required: true"));
        assert!(rendered.contains("reload_diff_status: unavailable"));
        assert!(rendered.contains("reload_diff_reason_code: unavailable_without_staged_projection"));
    }

    #[tokio::test]
    async fn route_explain_run_reads_routing_preview_then_reload_diff_projection() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let preview_seen = Arc::clone(&seen_paths);
        let diff_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/routing/preview",
                get(move || {
                    let preview_seen = Arc::clone(&preview_seen);
                    async move {
                        preview_seen
                            .lock()
                            .unwrap()
                            .push("/management/routing/preview".to_string());
                        Json(json!({
                            "model": "gpt-4o",
                            "selected_target": {"channel_id": "relay-a", "plan_position": 0},
                            "candidates": []
                        }))
                    }
                }),
            )
            .route(
                "/management/runtime/reload-diff",
                get(move || {
                    let diff_seen = Arc::clone(&diff_seen);
                    async move {
                        diff_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime/reload-diff".to_string());
                        Json(json!({
                            "status": "ok",
                            "reason_code": "reload_diff_empty",
                            "active_registry_generation": 33,
                            "active_registry_version": 7,
                            "staged_registry_version": 7,
                            "runtime_reload_required": false
                        }))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!("ONE_AI_KEY_TEST_ROUTE_EXPLAIN_TOKEN_{}", std::process::id());
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::RouteExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-4o".to_string(),
            client_token_ref: None,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/routing/preview".to_string(),
                "/management/runtime/reload-diff".to_string(),
            ]
        );
        assert_eq!(report["reload_diff_status"], "ok");
        assert_eq!(report["reload_diff_reason_code"], "reload_diff_empty");
        assert_eq!(report["active_registry_generation"], 33);
        assert_eq!(report["runtime_reload_required"], false);
    }

    #[tokio::test]
    async fn route_explain_run_falls_back_to_explain_runtime_when_reload_diff_is_unavailable() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let preview_seen = Arc::clone(&seen_paths);
        let diff_seen = Arc::clone(&seen_paths);
        let explain_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/routing/preview",
                get(move || {
                    let preview_seen = Arc::clone(&preview_seen);
                    async move {
                        preview_seen
                            .lock()
                            .unwrap()
                            .push("/management/routing/preview".to_string());
                        Json(json!({
                            "model": "gpt-4o",
                            "selected_target": null,
                            "candidates": []
                        }))
                    }
                }),
            )
            .route(
                "/management/runtime/reload-diff",
                get(move || {
                    let diff_seen = Arc::clone(&diff_seen);
                    async move {
                        diff_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime/reload-diff".to_string());
                        StatusCode::NOT_FOUND
                    }
                }),
            )
            .route(
                "/management/explain/runtime",
                get(move || {
                    let explain_seen = Arc::clone(&explain_seen);
                    async move {
                        explain_seen
                            .lock()
                            .unwrap()
                            .push("/management/explain/runtime".to_string());
                        Json(json!({
                            "active_registry_generation": 34,
                            "active_registry_version": 8,
                            "staged_registry_version": 9,
                            "runtime_reload_required": true,
                            "raw_path": "/tmp/secret-registry.yaml",
                            "raw_yaml": "RAW_YAML_SHOULD_NOT_APPEAR"
                        }))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_ROUTE_EXPLAIN_FALLBACK_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::RouteExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-4o".to_string(),
            client_token_ref: None,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/routing/preview".to_string(),
                "/management/runtime/reload-diff".to_string(),
                "/management/explain/runtime".to_string(),
            ]
        );
        assert_eq!(report["active_registry_generation"], 34);
        assert_eq!(report["staged_registry_version"], 9);
        assert_eq!(report["runtime_reload_required"], true);
        assert_eq!(report["reload_diff_status"], "unknown");
        assert!(!rendered.contains("/tmp/secret-registry.yaml"));
        assert!(!rendered.contains("RAW_YAML_SHOULD_NOT_APPEAR"));
    }

    #[tokio::test]
    async fn route_explain_run_does_not_hide_reload_diff_auth_failure() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let preview_seen = Arc::clone(&seen_paths);
        let diff_seen = Arc::clone(&seen_paths);
        let explain_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/routing/preview",
                get(move || {
                    let preview_seen = Arc::clone(&preview_seen);
                    async move {
                        preview_seen
                            .lock()
                            .unwrap()
                            .push("/management/routing/preview".to_string());
                        Json(json!({
                            "model": "gpt-4o",
                            "selected_target": {"channel_id": "relay-a", "plan_position": 0},
                            "candidates": []
                        }))
                    }
                }),
            )
            .route(
                "/management/runtime/reload-diff",
                get(move || {
                    let diff_seen = Arc::clone(&diff_seen);
                    async move {
                        diff_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime/reload-diff".to_string());
                        StatusCode::UNAUTHORIZED
                    }
                }),
            )
            .route(
                "/management/explain/runtime",
                get(move || {
                    let explain_seen = Arc::clone(&explain_seen);
                    async move {
                        explain_seen
                            .lock()
                            .unwrap()
                            .push("/management/explain/runtime".to_string());
                        Json(json!({"runtime_reload_required": false}))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_ROUTE_EXPLAIN_AUTH_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let error = super::run(super::RouteExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-4o".to_string(),
            client_token_ref: None,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap_err();
        std::env::remove_var(env_name);

        assert_eq!(error.reason_code(), "management_unauthorized");
        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/routing/preview".to_string(),
                "/management/runtime/reload-diff".to_string(),
            ]
        );
    }

    #[test]
    fn route_explain_uses_only_routing_preview_get_endpoint_with_requested_query() {
        let options = super::RouteExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example/v1".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-4o".to_string(),
            client_token_ref: Some("local-client".to_string()),
            output: crate::cli_report::OutputFormat::Json,
        };

        let (path, query) = super::route_explain_endpoint(&options)
            .test_request_parts()
            .expect("route explain endpoint should build");

        assert_eq!(path, "/management/routing/preview");
        assert_eq!(
            query,
            vec![
                ("model".to_string(), "gpt-4o".to_string()),
                ("client_token".to_string(), "local-client".to_string())
            ]
        );
    }

    #[test]
    fn route_explain_table_escapes_control_characters() {
        let preview = json!({
            "model": "gpt-4o\nforged",
            "selected_target": {"channel_id": "relay-a\u{1b}[31m"},
            "candidates": [{
                "channel_id": "relay-b\nnext",
                "upstream_model": "provider/gpt-4o\rbad",
                "included": true,
                "selected": false,
                "reasons": ["available\nforged"],
                "health": {"kind": "available"},
                "credentials": {"available": 1, "expired": 0}
            }]
        });

        let rendered =
            super::render_route_explain_report(&preview, crate::cli_report::OutputFormat::Table);

        assert!(!rendered.contains("gpt-4o\nforged"));
        assert!(!rendered.contains("\u{1b}"));
        assert!(rendered.contains("gpt-4o\\nforged"));
        assert!(rendered.contains("available\\nforged"));
    }
}
