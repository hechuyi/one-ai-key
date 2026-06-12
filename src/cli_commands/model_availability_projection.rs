use serde_json::Value;

pub(crate) async fn fetch_model_availability(
    client: &crate::operator_client::OperatorClient,
    model: &str,
    endpoint_family: &str,
    client_token_ref: Option<&str>,
) -> Result<Value, crate::operator_client::OperatorClientError> {
    client
        .get_json(
            crate::operator_client::ReadOnlyEndpoint::ModelAvailability {
                model: model.to_string(),
                endpoint_family: endpoint_family.to_string(),
                client_token_ref: client_token_ref.map(str::to_string),
            },
        )
        .await
}

pub(crate) fn sanitize_model_availability(value: &Value) -> Value {
    let reason_code = value
        .get("reason_code")
        .and_then(Value::as_str)
        .filter(|value| is_safe_reason_code(value));
    let blocking_domain = value
        .get("blocking_domain")
        .and_then(Value::as_str)
        .filter(|value| is_safe_reason_code(value));
    let next_step =
        sanitize_management_next_step(value.get("next_step")).unwrap_or(serde_json::Value::Null);
    let client_token = value
        .get("client_token")
        .and_then(Value::as_object)
        .map(|token| {
            serde_json::json!({
                "id": token
                    .get("id")
                    .and_then(Value::as_str)
                    .map(safe_client_token_id_label),
                "name": token
                    .get("name")
                    .and_then(Value::as_str)
                    .map(safe_client_token_name_label),
                "enabled": token.get("enabled").and_then(Value::as_bool),
            })
        });
    serde_json::json!({
        "status": value.get("status").and_then(Value::as_str),
        "can_use": value.get("can_use").and_then(Value::as_bool),
        "blocking_domain": blocking_domain,
        "reason_code": reason_code,
        "next_action": value
            .get("next_action")
            .and_then(Value::as_str)
            .filter(|value| is_safe_reason_code(value)),
        "endpoint_family": value
            .get("endpoint_family")
            .and_then(Value::as_str)
            .filter(|value| is_safe_reason_code(value)),
        "model": value
            .get("model")
            .and_then(Value::as_str)
            .map(safe_public_model_label),
        "public_model": value
            .get("public_model")
            .and_then(Value::as_str)
            .map(safe_public_model_label),
        "client_token_ref": value
            .get("client_token_ref")
            .and_then(Value::as_str)
            .and_then(safe_reference_label_value),
        "route_kind": value
            .get("route_kind")
            .and_then(Value::as_str)
            .filter(|value| is_safe_reason_code(value)),
        "registry_generation": value.get("registry_generation").and_then(Value::as_u64),
        "reload_drift": sanitize_availability_reload_drift(value.get("reload_drift")),
        "recent_failure_hint": sanitize_recent_failure_hint(value.get("recent_failure_hint")),
        "evidence": sanitize_availability_evidence(value.get("evidence")),
        "next_step": next_step,
        "client_token": client_token,
    })
}

pub(crate) fn invalid_model_availability_evidence(availability: &Value) -> Option<Value> {
    let mut invalid_fields = Vec::new();
    if availability
        .get("can_use")
        .and_then(Value::as_bool)
        .is_none()
    {
        invalid_fields.push("can_use");
    }
    if availability
        .get("reason_code")
        .and_then(Value::as_str)
        .is_none()
    {
        invalid_fields.push("reason_code");
    }
    if availability
        .get("blocking_domain")
        .and_then(Value::as_str)
        .is_none()
    {
        invalid_fields.push("blocking_domain");
    }
    if availability
        .get("endpoint_family")
        .and_then(Value::as_str)
        .is_none()
    {
        invalid_fields.push("endpoint_family");
    }
    if availability.get("model").and_then(Value::as_str).is_none() {
        invalid_fields.push("model");
    }
    if availability
        .get("evidence")
        .filter(|value| !value.is_null())
        .is_none()
    {
        invalid_fields.push("evidence");
    }
    if availability
        .get("next_step")
        .filter(|value| !value.is_null())
        .is_none()
    {
        invalid_fields.push("next_step");
    }
    if invalid_fields.is_empty() {
        None
    } else {
        Some(serde_json::json!({
            "projection": "model_availability",
            "invalid_fields": invalid_fields,
        }))
    }
}

pub(crate) fn management_projection_invalid_next_action() -> Value {
    serde_json::json!({
        "summary": "The management model availability projection is incomplete or invalid. No automatic diagnostic action is available.",
        "template_id": "no_action_required",
        "safe_argv": [],
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    })
}

pub(crate) fn append_availability_table_fields(output: &mut String, availability: &Value) {
    crate::cli_report::push_table_field(output, "availability.status", availability.get("status"));
    crate::cli_report::push_table_field(
        output,
        "availability.reason_code",
        availability.get("reason_code"),
    );
    crate::cli_report::push_table_field(
        output,
        "availability.blocking_domain",
        availability.get("blocking_domain"),
    );
    if availability
        .get("evidence")
        .filter(|value| !value.is_null())
        .is_some()
    {
        output.push_str("availability.evidence_source: management_model_availability\n");
    }
    crate::cli_report::push_table_field(
        output,
        "availability.next_action",
        availability.get("next_action"),
    );
    crate::cli_report::push_table_field(
        output,
        "availability.endpoint_family",
        availability.get("endpoint_family"),
    );
    if let Some(reload_drift) = availability
        .get("reload_drift")
        .filter(|value| !value.is_null())
    {
        crate::cli_report::push_table_field(
            output,
            "availability.reload_drift.status",
            reload_drift.get("status"),
        );
        crate::cli_report::push_table_field(
            output,
            "availability.reload_drift.reason_code",
            reload_drift.get("reason_code"),
        );
        crate::cli_report::push_table_field(
            output,
            "availability.reload_drift.runtime_reload_required",
            reload_drift.get("runtime_reload_required"),
        );
    }
    if let Some(recent_failure_hint) = availability
        .get("recent_failure_hint")
        .filter(|value| !value.is_null())
    {
        crate::cli_report::push_table_field(
            output,
            "availability.recent_failure_hint.status",
            recent_failure_hint.get("status"),
        );
        crate::cli_report::push_table_field(
            output,
            "availability.recent_failure_hint.reason_codes",
            recent_failure_hint.get("reason_codes"),
        );
        crate::cli_report::push_table_field(
            output,
            "availability.recent_failure_hint.matched_event_count",
            recent_failure_hint.get("matched_event_count"),
        );
    }
}

fn sanitize_management_next_step(value: Option<&Value>) -> Option<Value> {
    let action = value?.as_object()?;
    if action.get("side_effect_class").and_then(Value::as_str) != Some("runtime_readonly") {
        return None;
    }
    if action.get("requires_confirmation").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    let template_id = action
        .get("template_id")
        .and_then(Value::as_str)
        .filter(|value| is_safe_reason_code(value))?;
    let summary = action
        .get("summary")
        .and_then(Value::as_str)
        .and_then(safe_next_step_summary)?;
    let safe_argv = action
        .get("safe_argv")
        .and_then(Value::as_array)?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()?;
    if !is_allowed_management_next_step_action(template_id, &safe_argv)
        || !safe_argv.iter().copied().all(is_safe_next_step_argv_arg)
    {
        return None;
    }
    Some(serde_json::json!({
        "summary": summary,
        "template_id": template_id,
        "safe_argv": safe_argv,
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    }))
}

fn safe_next_step_summary(summary: &str) -> Option<String> {
    let trimmed = summary.trim();
    if trimmed.is_empty()
        || trimmed.len() > 240
        || trimmed.chars().any(char::is_control)
        || trimmed.contains("://")
        || trimmed.starts_with('/')
        || trimmed.starts_with("~/")
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || looks_like_windows_absolute_path(trimmed)
    {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("sk_")
        || lower.contains("secret")
        || lower.contains("authorization")
        || lower.contains("bearer")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("/tmp/")
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn is_allowed_management_next_step_action(template_id: &str, argv: &[&str]) -> bool {
    if argv.is_empty() {
        return template_id == "no_action_required";
    }
    matches!(
        (template_id, argv),
        (
            "models_explain",
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
            ]
        ) | (
            "models_explain_visibility",
            [
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
            ]
        ) | (
            "use_supported_endpoint_family",
            [
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
            ]
        ) | (
            "route_explain" | "management_route_projection",
            [
                "one-ai-key",
                "route",
                "explain",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "<public-model>"
            ]
        ) | (
            "failures_tail",
            [
                "one-ai-key",
                "failures",
                "tail",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "--last",
                "50"
            ]
        ) | (
            "failures_explain_request",
            [
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
            ]
        ) | (
            "doctor",
            [
                "one-ai-key",
                "doctor",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>"
            ]
        ) | (
            "reload_status",
            [
                "one-ai-key",
                "reload",
                "status",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>"
            ]
        ) | (
            "reload_diff",
            [
                "one-ai-key",
                "reload",
                "diff",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>"
            ]
        )
    )
}

fn is_safe_next_step_argv_arg(arg: &str) -> bool {
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
        || lower.contains("sk_")
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

fn sanitize_availability_evidence(evidence: Option<&Value>) -> Value {
    let Some(evidence) = evidence.and_then(Value::as_object) else {
        return Value::Null;
    };
    let mut sanitized = serde_json::Map::new();
    for key in [
        "client_token_ref_supplied",
        "client_token_known",
        "client_token_enabled",
        "model_allowed",
        "model_visible",
        "route_present",
        "selected_target_present",
    ] {
        if let Some(value) = evidence.get(key).and_then(Value::as_bool) {
            sanitized.insert(key.to_string(), Value::Bool(value));
        }
    }
    for key in [
        "route_target_count",
        "endpoint_family_target_count",
        "unsupported_target_count",
        "unknown_or_missing_target_count",
        "preview_candidate_count",
        "candidate_limit",
    ] {
        if let Some(value) = evidence.get(key).and_then(Value::as_u64) {
            sanitized.insert(key.to_string(), Value::from(value));
        }
    }
    if let Some(reason_codes) = evidence
        .get("candidate_reason_codes")
        .and_then(Value::as_array)
    {
        let reason_codes = reason_codes
            .iter()
            .filter_map(Value::as_str)
            .filter(|value| is_safe_reason_code(value))
            .take(8)
            .map(Value::from)
            .collect::<Vec<_>>();
        sanitized.insert(
            "candidate_reason_codes".to_string(),
            Value::Array(reason_codes),
        );
    }
    if let Some(primary_reason) = evidence
        .get("admission_primary_reason_code")
        .and_then(Value::as_str)
        .filter(|value| is_safe_reason_code(value))
    {
        sanitized.insert(
            "admission_primary_reason_code".to_string(),
            Value::from(primary_reason),
        );
    }
    Value::Object(sanitized)
}

fn sanitize_availability_reload_drift(reload_drift: Option<&Value>) -> Value {
    let Some(reload_drift) = reload_drift.and_then(Value::as_object) else {
        return Value::Null;
    };
    let status = reload_drift
        .get("status")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "current" | "drift" | "unknown"));
    let reason_code = reload_drift
        .get("reason_code")
        .and_then(Value::as_str)
        .filter(|value| is_safe_reason_code(value));
    serde_json::json!({
        "status": status,
        "reason_code": reason_code,
        "active_registry_generation": reload_drift
            .get("active_registry_generation")
            .and_then(Value::as_u64),
        "active_registry_version": reload_drift
            .get("active_registry_version")
            .and_then(Value::as_u64),
        "staged_registry_version": reload_drift
            .get("staged_registry_version")
            .and_then(Value::as_u64),
        "runtime_reload_required": reload_drift
            .get("runtime_reload_required")
            .and_then(Value::as_bool),
        "last_reload_at_unix_seconds": reload_drift
            .get("last_reload_at_unix_seconds")
            .and_then(Value::as_u64),
        "last_reload_error_reason_code": reload_drift
            .get("last_reload_error_reason_code")
            .and_then(Value::as_str)
            .filter(|value| is_safe_reason_code(value)),
    })
}

fn sanitize_recent_failure_hint(recent_failure_hint: Option<&Value>) -> Value {
    let Some(hint) = recent_failure_hint.and_then(Value::as_object) else {
        return Value::Null;
    };
    let status = hint
        .get("status")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "none" | "present" | "unknown"));
    let source = hint
        .get("source")
        .and_then(Value::as_str)
        .filter(|value| *value == "routing_telemetry_bounded_window");
    let reason_codes = hint
        .get("reason_codes")
        .and_then(Value::as_array)
        .map(|reason_codes| {
            reason_codes
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| is_safe_reason_code(value))
                .take(8)
                .map(Value::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let channel_ids = hint
        .get("channel_ids")
        .and_then(Value::as_array)
        .map(|channel_ids| {
            channel_ids
                .iter()
                .filter_map(Value::as_str)
                .map(|value| {
                    safe_reference_label_value(value)
                        .unwrap_or_else(|| "<redacted-channel-id>".to_string())
                })
                .take(8)
                .map(Value::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "status": status,
        "source": source,
        "window_event_count": hint
            .get("window_event_count")
            .and_then(Value::as_u64),
        "matched_event_count": hint
            .get("matched_event_count")
            .and_then(Value::as_u64),
        "reason_codes": reason_codes,
        "channel_ids": channel_ids,
    })
}

fn is_safe_reason_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn safe_reference_label_value(reference: &str) -> Option<String> {
    if reference.chars().any(char::is_control) {
        return None;
    }
    let trimmed = reference.trim();
    if !trimmed.is_empty()
        && trimmed.len() <= 128
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !trimmed.to_ascii_lowercase().contains("secret")
        && !trimmed.to_ascii_lowercase().contains("authorization")
        && !trimmed.to_ascii_lowercase().contains("api_key")
        && !trimmed.to_ascii_lowercase().contains("apikey")
        && !trimmed.to_ascii_lowercase().contains("bearer")
        && !trimmed.to_ascii_lowercase().contains("sk-")
        && !trimmed.to_ascii_lowercase().contains("sk_")
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn safe_client_token_id_label(id: &str) -> String {
    if safe_reference_label_value(id).is_some() {
        id.trim().to_string()
    } else {
        "<redacted-client-token-id>".to_string()
    }
}

fn safe_client_token_name_label(name: &str) -> String {
    if safe_reference_label_value(name).is_some() {
        name.trim().to_string()
    } else {
        "<redacted-client-token-name>".to_string()
    }
}

fn safe_public_model_label(public_model: &str) -> String {
    let trimmed = public_model.trim();
    if is_safe_public_model_label(trimmed) {
        trimmed.to_string()
    } else {
        "<redacted-public-model>".to_string()
    }
}

fn is_safe_public_model_label(public_model: &str) -> bool {
    if public_model.is_empty() || public_model.len() > 128 {
        return false;
    }
    if public_model.starts_with('/')
        || public_model.starts_with("~/")
        || public_model.starts_with("./")
        || public_model.starts_with("../")
        || public_model.contains("/../")
        || public_model.contains('\\')
        || public_model.contains("://")
        || public_model.contains('?')
        || public_model.contains('&')
        || public_model.contains('=')
        || public_model.chars().any(char::is_control)
    {
        return false;
    }
    let lower = public_model.to_ascii_lowercase();
    if looks_like_windows_absolute_path(public_model)
        || looks_like_url_scheme(&lower)
        || lower.contains("token")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("secret")
        || lower.contains("authorization")
        || lower.contains("bearer")
        || lower.contains("sk-")
        || lower.contains("sk_")
    {
        return false;
    }
    public_model.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
    })
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn looks_like_url_scheme(lower: &str) -> bool {
    ["http:", "https:", "file:", "ftp:", "s3:", "gs:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}
