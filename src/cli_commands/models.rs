use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsListOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub client_token_ref: Option<String>,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsExplainOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub model: String,
    pub client_token_ref: Option<String>,
    pub endpoint_family: Option<String>,
    pub output: crate::cli_report::OutputFormat,
}

pub async fn run_list(
    options: ModelsListOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let model_routes = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::ModelRoutes)
        .await?;
    Ok(render_models_list_report(
        &model_routes,
        options.client_token_ref.as_deref(),
        options.output,
    ))
}

pub async fn run_explain(
    options: ModelsExplainOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let preview = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::RoutingPreview {
            model: options.model.clone(),
            client_token_ref: options.client_token_ref.clone(),
        })
        .await?;
    let runtime_projection =
        crate::cli_commands::runtime_reload_projection::fetch_runtime_reload_projection(&client)
            .await?;
    let availability = if let Some(endpoint_family) = options.endpoint_family.as_ref() {
        Some(
            crate::cli_commands::model_availability_projection::fetch_model_availability(
                &client,
                &options.model,
                endpoint_family,
                options.client_token_ref.as_deref(),
            )
            .await?,
        )
    } else {
        None
    };
    Ok(render_models_explain_report_with_management_projection(
        &preview,
        runtime_projection.as_ref(),
        availability.as_ref(),
        options.output,
    ))
}

pub fn render_models_list_report(
    model_routes: &Value,
    client_token_ref: Option<&str>,
    output: crate::cli_report::OutputFormat,
) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => {
            render_models_list_json(model_routes, client_token_ref)
        }
        crate::cli_report::OutputFormat::Table => {
            render_models_list_table(model_routes, client_token_ref)
        }
    }
}

#[cfg(test)]
pub fn render_models_explain_report(
    preview: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    render_models_explain_report_with_management_projection(preview, None, None, output)
}

fn render_models_explain_report_with_management_projection(
    preview: &Value,
    management_projection: Option<&Value>,
    availability: Option<&Value>,
    output: crate::cli_report::OutputFormat,
) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => {
            render_models_explain_json(preview, management_projection, availability)
        }
        crate::cli_report::OutputFormat::Table => {
            render_models_explain_table(preview, management_projection, availability)
        }
    }
}

fn render_models_list_json(model_routes: &Value, client_token_ref: Option<&str>) -> String {
    let report = sanitized_models_list_report(model_routes, client_token_ref);
    serde_json::to_string_pretty(&report).expect("models list json report should serialize")
}

fn sanitized_models_list_report(model_routes: &Value, client_token_ref: Option<&str>) -> Value {
    let models = model_routes
        .get("routes")
        .and_then(Value::as_array)
        .map(|routes| routes.iter().map(sanitize_model_route).collect::<Vec<_>>())
        .unwrap_or_default();
    let model_count = models.len();
    let visible_model_count = models
        .iter()
        .filter(|model| {
            model
                .get("visible_target_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                > 0
        })
        .count();
    let status = if model_count == 0 {
        "empty"
    } else if visible_model_count == 0 {
        "blocked"
    } else {
        "ok"
    };
    let data = serde_json::json!({
        "command": "models list",
        "reload_diff_status": "unavailable_until_m4",
        "capability_status": "unavailable_until_m4",
        "client_token_ref": client_token_ref,
        "client_token_scope_status": models_list_client_scope_status(client_token_ref),
        "model_count": model_count,
        "visible_model_count": visible_model_count,
        "default_channel": model_routes.get("default_channel").and_then(Value::as_str),
        "unmapped_model_policy": model_routes
            .get("unmapped_model_policy")
            .and_then(Value::as_str),
        "models": models,
    });
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: models_list_reason(status),
        reason_code: models_list_reason_code(status),
        effect: crate::cli_effects::runtime_readonly_effect(),
        scope: serde_json::json!({
            "client_token_ref": client_token_ref,
            "projection": "model_routes",
        }),
        window: Value::Null,
        next_action: models_list_next_action(status, client_token_ref),
        data,
    })
}

fn models_list_reason_code(status: &str) -> &'static str {
    match status {
        "empty" => "no_model_routes_configured",
        "blocked" => "no_runtime_visible_model_routes",
        _ => "model_routes_available",
    }
}

fn models_list_reason(status: &str) -> &'static str {
    match status {
        "empty" => "No compiled runtime model routes are available.",
        "blocked" => "Compiled model routes exist, but none have a runtime-visible target.",
        _ => "Compiled runtime model routes are available.",
    }
}

fn models_list_client_scope_status(client_token_ref: Option<&str>) -> &'static str {
    if client_token_ref.is_some() {
        "not_evaluated_for_list_use_models_explain"
    } else {
        "not_requested"
    }
}

fn models_list_next_action(status: &str, client_token_ref: Option<&str>) -> Value {
    match status {
        "empty" => serde_json::json!({
            "summary": "No compiled runtime model routes are available. Add explicit model routes, then run check-config. Reload status is unavailable until M4.",
            "template_id": "check_config",
            "safe_argv": ["one-ai-key", "check-config", "--config", "<config>"],
            "side_effect_class": "offline_readonly",
            "requires_confirmation": false,
            "reload_status": "unavailable_until_m4",
        }),
        "blocked" => serde_json::json!({
            "summary": "Compiled model routes exist, but none have a runtime-visible enabled target in this bounded projection. Use models explain for a single public model.",
            "template_id": "models_explain",
            "safe_argv": model_explain_argv(client_token_ref),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
        _ => serde_json::json!({
            "summary": "Compiled runtime model routes are available. Use models explain for a single public model and optional client-token reference.",
            "template_id": "models_explain",
            "safe_argv": model_explain_argv(client_token_ref),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    }
}

fn model_explain_argv(client_token_ref: Option<&str>) -> Value {
    let mut argv = vec![
        Value::from("one-ai-key"),
        Value::from("models"),
        Value::from("explain"),
        Value::from("--management-url"),
        Value::from("<url>"),
        Value::from("--management-token-env"),
        Value::from("<env>"),
        Value::from("--model"),
        Value::from("<public-model>"),
    ];
    if client_token_ref.is_some() {
        argv.push(Value::from("--client-token-ref"));
        argv.push(Value::from("<client-token-ref>"));
    }
    Value::Array(argv)
}

fn sanitize_model_route(route: &Value) -> Value {
    let targets = route
        .get("targets")
        .and_then(Value::as_array)
        .map(|targets| {
            targets
                .iter()
                .map(sanitize_model_target)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let visible_target_count = targets
        .iter()
        .filter(|target| {
            target.get("enabled").and_then(Value::as_bool) == Some(true)
                && target
                    .get("health_kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind != "disabled")
        })
        .count();
    serde_json::json!({
        "model": route.get("model").and_then(Value::as_str),
        "strategy": route.get("strategy").and_then(Value::as_str),
        "target_count": targets.len(),
        "visible_target_count": visible_target_count,
        "targets": targets,
    })
}

fn sanitize_model_target(target: &Value) -> Value {
    serde_json::json!({
        "channel_id": target.get("channel_id").and_then(Value::as_str),
        "upstream_model": target.get("upstream_model").and_then(Value::as_str),
        "provider_kind": target.get("provider_kind").and_then(Value::as_str),
        "priority": target.get("priority").and_then(Value::as_u64),
        "weight": target.get("weight").and_then(Value::as_u64),
        "enabled": target.get("enabled").and_then(Value::as_bool),
        "health_kind": target
            .get("health")
            .and_then(|health| health.get("kind"))
            .and_then(Value::as_str),
        "health_reason_code": target
            .get("health")
            .and_then(|health| health.get("reason_code"))
            .and_then(Value::as_str),
        "health_generation": target
            .get("health")
            .and_then(|health| health.get("generation"))
            .and_then(Value::as_u64),
    })
}

fn render_models_explain_json(
    preview: &Value,
    management_projection: Option<&Value>,
    availability: Option<&Value>,
) -> String {
    let report = sanitized_models_explain_report(preview, management_projection, availability);
    serde_json::to_string_pretty(&report).expect("models explain json report should serialize")
}

fn sanitized_models_explain_report(
    preview: &Value,
    management_projection: Option<&Value>,
    availability: Option<&Value>,
) -> Value {
    let client_token = sanitize_preview_client_token(preview.get("client_token"));
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
    let selected_target = sanitize_selected_target(preview.get("selected_target"));
    let has_selected_target = !selected_target.is_null();
    let client_scope_status = client_scope_status(&client_token, preview);
    let capability_status =
        crate::cli_report::endpoint_capability_status_from_candidates(&candidates);
    let runtime_reload =
        crate::cli_commands::runtime_reload_projection::summarize_runtime_reload_projection(
            management_projection
                .or_else(|| preview.get("runtime_reload_projection"))
                .or_else(|| preview.get("runtime_reload")),
        );
    let availability = availability
        .map(crate::cli_commands::model_availability_projection::sanitize_model_availability);
    let invalid_management_projection = availability.as_ref().and_then(
        crate::cli_commands::model_availability_projection::invalid_model_availability_evidence,
    );
    let has_invalid_management_projection = invalid_management_projection.is_some();
    let availability_can_use = availability
        .as_ref()
        .and_then(|value| value.get("can_use"))
        .and_then(Value::as_bool)
        .filter(|_| !has_invalid_management_projection)
        .or_else(|| has_invalid_management_projection.then_some(false));
    let status = if let Some(can_use) = availability_can_use {
        if can_use {
            "ok"
        } else {
            "blocked"
        }
    } else if has_selected_target {
        "ok"
    } else {
        "blocked"
    };
    let reason_code = if has_invalid_management_projection {
        "management_projection_invalid"
    } else {
        availability
            .as_ref()
            .and_then(|value| value.get("reason_code"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| models_explain_reason_code(has_selected_target, client_scope_status))
    };
    let diagnostic_contract = crate::diagnostic_contract::contract_for_reason(reason_code);
    let blocking_domain = if has_invalid_management_projection {
        Some("management_projection")
    } else {
        availability
            .as_ref()
            .and_then(|value| value.get("blocking_domain"))
            .and_then(Value::as_str)
            .or_else(|| {
                diagnostic_contract
                    .as_ref()
                    .map(|contract| contract.blocking_domain)
            })
    };
    let report_model = if availability.is_some() {
        availability
            .as_ref()
            .and_then(|value| value.get("model"))
            .and_then(Value::as_str)
    } else {
        preview.get("model").and_then(Value::as_str)
    };
    let report_endpoint_family = availability
        .as_ref()
        .and_then(|value| value.get("endpoint_family"))
        .and_then(Value::as_str);
    let report_client_token_ref = availability
        .as_ref()
        .and_then(|value| value.get("client_token_ref"))
        .and_then(Value::as_str)
        .or_else(|| client_token.get("name").and_then(Value::as_str));
    let reload_drift = availability
        .as_ref()
        .and_then(|value| value.get("reload_drift"))
        .filter(|value| !value.is_null())
        .cloned();
    let recent_failure_hint = availability
        .as_ref()
        .and_then(|value| value.get("recent_failure_hint"))
        .filter(|value| !value.is_null())
        .cloned();
    let next_action = if has_invalid_management_projection {
        diagnostic_contract
            .map(|contract| contract.next_action)
            .unwrap_or_else(crate::cli_commands::model_availability_projection::management_projection_invalid_next_action)
    } else {
        availability
            .as_ref()
            .and_then(|value| value.get("next_step"))
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| {
                models_explain_next_action(status, preview, client_scope_status, reason_code)
            })
    };
    let evidence = invalid_management_projection.or_else(|| {
        availability
            .as_ref()
            .and_then(|value| value.get("evidence"))
            .cloned()
    });
    let data = serde_json::json!({
        "command": "models explain",
        "active_registry_generation": runtime_reload.active_registry_generation,
        "active_registry_version": runtime_reload.active_registry_version,
        "staged_registry_version": runtime_reload.staged_registry_version,
        "runtime_reload_required": runtime_reload.runtime_reload_required,
        "reload_diff_status": runtime_reload.reload_diff_status,
        "reload_diff_reason_code": runtime_reload.reload_diff_reason_code,
        "reload_diff_next_action": runtime_reload.reload_diff_next_action,
        "capability_status": capability_status,
        "can_use": availability_can_use,
        "blocking_domain": blocking_domain,
        "endpoint_family": report_endpoint_family,
        "model": report_model,
        "client_token_ref": report_client_token_ref,
        "evidence": evidence,
        "reload_drift": reload_drift,
        "recent_failure_hint": recent_failure_hint,
        "route_kind": preview.get("route_kind").and_then(Value::as_str),
        "registry_generation": preview.get("registry_generation").and_then(Value::as_u64),
        "candidate_limit": preview.get("candidate_limit").and_then(Value::as_u64),
        "client_token": client_token,
        "client_scope_status": client_scope_status,
        "availability": availability.clone(),
        "selected_target": selected_target,
        "candidates": candidates,
    });
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: models_explain_reason(reason_code),
        reason_code,
        effect: crate::cli_effects::runtime_readonly_effect(),
        scope: serde_json::json!({
            "model": report_model,
            "client_token_ref": report_client_token_ref,
            "endpoint_family": report_endpoint_family,
        }),
        window: Value::Null,
        next_action,
        data,
    })
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

fn client_scope_status(client_token: &Value, preview: &Value) -> &'static str {
    if preview.get("route_kind").and_then(Value::as_str) == Some("client_model_denied") {
        return "model_not_in_client_scope";
    }
    let unrestricted = client_token
        .get("unrestricted_model_groups")
        .and_then(Value::as_bool)
        == Some(true);
    if unrestricted {
        return "unrestricted";
    }
    if client_token
        .get("allowed_model_groups")
        .and_then(Value::as_array)
        .is_some()
    {
        "allowed_by_runtime_catalog"
    } else {
        "unknown"
    }
}

fn models_explain_reason_code(
    has_selected_target: bool,
    client_scope_status: &'static str,
) -> &'static str {
    if client_scope_status == "model_not_in_client_scope" {
        "model_not_in_client_scope"
    } else if has_selected_target {
        "model_visible_to_client"
    } else {
        "no_runtime_route_candidate"
    }
}

fn models_explain_reason(reason_code: &str) -> &'static str {
    match reason_code {
        "management_projection_invalid" => {
            "The management model availability projection is incomplete or invalid."
        }
        "model_not_in_client_scope" => "The public model is not in this client-token scope.",
        "no_runtime_route_candidate" => "The public model has no selected runtime route candidate.",
        _ => "The public model has a selected runtime route candidate.",
    }
}

fn models_explain_next_action(
    status: &str,
    preview: &Value,
    client_scope_status: &str,
    reason_code: &str,
) -> Value {
    if let Some(contract) = crate::diagnostic_contract::contract_for_reason(reason_code) {
        return contract.next_action;
    }
    let model = preview
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("<public-model>");
    if status == "ok" {
        serde_json::json!({
            "summary": "The public model has a selected runtime route candidate for this client-token scope. No repair action is required by models explain.",
            "template_id": "no_action_required",
            "safe_argv": [],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        })
    } else if client_scope_status == "model_not_in_client_scope" {
        serde_json::json!({
            "summary": format!("Public model {model} is not in this client-token scope. This command is read-only and has no client-token scope mutation CLI; edit the supported config or staged registry path, then run check-config and inspect reload status or reload diff as needed."),
            "template_id": "check_config",
            "safe_argv": ["one-ai-key", "check-config", "--config", "<config>"],
            "side_effect_class": "offline_readonly",
            "requires_confirmation": false,
            "repair_path": "deferred_by_m1_m4",
            "reload_status": "available",
        })
    } else {
        serde_json::json!({
            "summary": format!("Public model {model} has no selected runtime route candidate. Inspect route explain for route target and runtime channel details."),
            "template_id": "route_explain",
            "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "<public-model>"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        })
    }
}

fn sanitize_preview_client_token(client_token: Option<&Value>) -> Value {
    let Some(client_token) = client_token else {
        return Value::Null;
    };
    serde_json::json!({
        "id": client_token
            .get("id")
            .and_then(Value::as_str)
            .map(safe_client_token_id_label),
        "name": client_token
            .get("name")
            .and_then(Value::as_str)
            .map(safe_client_token_name_label),
        "unrestricted_model_groups": client_token
            .get("unrestricted_model_groups")
            .and_then(Value::as_bool),
        "unrestricted_channels": client_token
            .get("unrestricted_channels")
            .and_then(Value::as_bool),
        "allowed_model_groups": client_token
            .get("allowed_model_groups")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default(),
        "allowed_channels": client_token
            .get("allowed_channels")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}

fn sanitize_selected_target(target: Option<&Value>) -> Value {
    let Some(target) = target else {
        return Value::Null;
    };
    if target.is_null() {
        return Value::Null;
    }
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
            .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
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

fn render_models_list_table(model_routes: &Value, client_token_ref: Option<&str>) -> String {
    let report = sanitized_models_list_report(model_routes, client_token_ref);
    let mut output = String::new();
    output.push_str("Models\n");
    append_common_header(&mut output, &report);
    if let Some(client_token_ref) = report.get("client_token_ref").and_then(Value::as_str) {
        output.push_str(&format!(
            "client_token_ref: {}\n",
            display_value(client_token_ref)
        ));
        output.push_str("client_token_scope_status: not_evaluated_for_list_use_models_explain\n");
    }
    output.push_str(&format!(
        "model_count: {}\n",
        report
            .get("model_count")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    ));
    output.push_str("models:\n");
    for model in report
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        output.push_str(&format!(
            "- model={} strategy={} targets={} visible_targets={}\n",
            display_value(
                model
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
            ),
            display_value(
                model
                    .get("strategy")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
            ),
            model
                .get("target_count")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            model
                .get("visible_target_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ));
    }
    output
}

fn render_models_explain_table(
    preview: &Value,
    management_projection: Option<&Value>,
    availability: Option<&Value>,
) -> String {
    let report = sanitized_models_explain_report(preview, management_projection, availability);
    let mut output = String::new();
    output.push_str(&format!(
        "Model explanation for {}\n",
        display_value(
            report
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>")
        )
    ));
    append_common_header(&mut output, &report);
    output.push_str(&format!(
        "client_scope_status: {}\n",
        display_value(
            report
                .get("client_scope_status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        )
    ));
    if let Some(availability) = report.get("availability").filter(|value| !value.is_null()) {
        crate::cli_commands::model_availability_projection::append_availability_table_fields(
            &mut output,
            availability,
        );
    }
    let selected = report
        .get("selected_target")
        .and_then(|target| target.get("channel_id"))
        .and_then(Value::as_str)
        .unwrap_or("none");
    output.push_str(&format!("selected_target: {}\n", display_value(selected)));
    output.push_str("candidates:\n");
    for candidate in report
        .get("candidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        output.push_str(&format!(
            "- channel={} included={} selected={} health={} reasons={} {}\n",
            display_value(
                candidate
                    .get("channel_id")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
            ),
            candidate
                .get("included")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            candidate
                .get("selected")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            display_value(
                candidate
                    .get("health_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ),
            display_value(
                &candidate
                    .get("reasons")
                    .and_then(Value::as_array)
                    .map(|values| values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(","))
                    .unwrap_or_default()
            ),
            crate::cli_report::endpoint_capabilities_table_summary(
                candidate.get("endpoint_capabilities")
            )
        ));
    }
    output
}

fn append_common_header(output: &mut String, report: &Value) {
    crate::cli_report::append_report_envelope_table_fields(output, report);
    crate::cli_commands::runtime_reload_projection::append_runtime_reload_table_fields(
        output, report,
    );
    crate::cli_report::push_table_field(
        output,
        "capability_status",
        report.get("capability_status"),
    );
}

fn display_value(value: &str) -> String {
    crate::cli_report::escape_table_value(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::get, Json, Router};
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
    fn models_list_json_uses_model_routes_projection_without_token_matrix() {
        let input = serde_json::json!({
            "default_channel": "fallback",
            "unmapped_model_policy": "default_channel",
            "routes": [
                {
                    "model": "gpt-public",
                    "strategy": "priority",
                    "targets": [
                        {
                            "channel_id": "primary",
                            "upstream_model": "vendor-model",
                            "provider_kind": "openai",
                            "priority": 10,
                            "weight": 1,
                            "enabled": true,
                            "token_hash": "SHOULD_NOT_RENDER_TOKEN_HASH",
                            "health": {
                                "kind": "ready",
                                "reason_code": null,
                                "generation": 3
                            }
                        }
                    ]
                }
            ]
        });

        let rendered = render_models_list_report(
            &input,
            Some("local-client"),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "model_routes_available");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["reads_management_runtime"], true);
        assert_eq!(report["window"], serde_json::Value::Null);
        assert!(report["next_action"]["safe_argv"].is_array());
        let legacy_dry_run_field = ["dry", "run", "command"].join("_");
        assert!(report["next_action"].get(&legacy_dry_run_field).is_none());
        assert_eq!(report["client_token_ref"], "local-client");
        assert_eq!(
            report["client_token_scope_status"],
            "not_evaluated_for_list_use_models_explain"
        );
        assert_eq!(report["reload_diff_status"], "unavailable_until_m4");
        assert_eq!(report["capability_status"], "unavailable_until_m4");
        assert_eq!(report["models"][0]["model"], "gpt-public");
        assert_eq!(report["models"][0]["visible_target_count"], 1);
        assert!(!rendered.contains("SHOULD_NOT_RENDER_TOKEN_HASH"));
        assert!(!rendered.contains("client_tokens"));
    }

    #[test]
    fn models_explain_json_reports_client_scope_reason_and_structured_action() {
        let input = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "client_model_denied",
            "registry_generation": 7,
            "candidate_limit": 8,
            "client_token": {
                "id": "local-client",
                "name": "Local Client",
                "token_hash": "SHOULD_NOT_RENDER_TOKEN_HASH",
                "unrestricted_model_groups": false,
                "unrestricted_channels": true,
                "allowed_model_groups": ["other-model"],
                "allowed_channels": []
            },
            "selected_target": null,
            "candidates": [
                {
                    "target_index": 0,
                    "channel_id": "primary",
                    "upstream_model": "vendor-model",
                    "provider_kind": "openai",
                    "priority": 10,
                    "weight": 1,
                    "target_enabled": true,
                    "included": false,
                    "selected": false,
                    "plan_position": null,
                    "reasons": ["model_not_allowed"],
                    "health": {
                        "kind": "ready",
                        "reason_code": null,
                        "generation": 3
                    },
                    "credential_set_id": "primary-set",
                    "selector_generation": 4,
                    "credentials": {
                        "total": 2,
                        "available": 1,
                        "token_hash": "SHOULD_NOT_RENDER_TOKEN_HASH"
                    }
                }
            ]
        });

        let rendered = render_models_explain_report(&input, crate::cli_report::OutputFormat::Json);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["reason_code"], "model_not_in_client_scope");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["reads_management_runtime"], true);
        assert_eq!(report["window"], serde_json::Value::Null);
        assert_eq!(report["client_scope_status"], "model_not_in_client_scope");
        assert_eq!(report["reload_diff_status"], "unknown");
        assert_eq!(report["capability_status"], "unknown");
        let contract =
            crate::diagnostic_contract::contract_for_reason("model_not_in_client_scope").unwrap();
        assert_eq!(report["blocking_domain"], contract.blocking_domain);
        assert_eq!(report["next_action"], contract.next_action);
        assert!(report["next_action"].get("repair_path").is_none());
        assert!(report["next_action"].get("reload_status").is_none());
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "runtime_readonly"
        );
        assert_eq!(report["candidates"][0]["credentials"]["total"], 2);
        let legacy_safe_field = ["safe", "command"].join("_");
        let legacy_dry_run_field = ["dry", "run", "command"].join("_");
        assert!(report["next_action"].get(&legacy_safe_field).is_none());
        assert!(report["next_action"].get(&legacy_dry_run_field).is_none());
        assert_eq!(
            report["next_action"]["safe_argv"],
            contract.next_action["safe_argv"]
        );
        assert!(!rendered.contains("SHOULD_NOT_RENDER_TOKEN_HASH"));
        assert!(!rendered.contains("token_hash"));
    }

    #[test]
    fn models_explain_table_reports_selected_target_when_visible() {
        let input = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "configured",
            "client_token": {
                "id": "local-client",
                "name": "Local Client",
                "unrestricted_model_groups": true,
                "unrestricted_channels": true,
                "allowed_model_groups": [],
                "allowed_channels": []
            },
            "selected_target": {
                "channel_id": "primary",
                "plan_position": 0
            },
            "candidates": []
        });

        let rendered = render_models_explain_report(&input, crate::cli_report::OutputFormat::Table);

        assert!(rendered.contains("status: ok"));
        assert!(rendered.contains("reason_code: model_visible_to_client"));
        assert!(rendered.contains("side_effect_class: runtime_readonly"));
        assert!(rendered.contains("effect.reads_management_runtime: true"));
        assert!(rendered.contains("client_scope_status: unrestricted"));
        assert!(rendered.contains("selected_target: primary"));
        assert!(rendered.contains("reload_diff_status: unknown"));
        assert!(rendered.contains("capability_status: unknown"));
        assert!(rendered.contains("next_action.safe_argv: []"));
    }

    #[test]
    fn endpoint_capability_cli_models_explain_shows_static_endpoint_capabilities() {
        let input = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "configured",
            "client_token": {
                "id": "local-client",
                "name": "Local Client",
                "unrestricted_model_groups": true,
                "unrestricted_channels": true,
                "allowed_model_groups": [],
                "allowed_channels": []
            },
            "selected_target": {
                "channel_id": "primary",
                "plan_position": 0
            },
            "candidates": [
                {
                    "target_index": 0,
                    "channel_id": "primary",
                    "upstream_model": "vendor-model",
                    "provider_kind": "openai",
                    "target_enabled": true,
                    "included": true,
                    "selected": true,
                    "plan_position": 0,
                    "reasons": ["available"],
                    "health": {"kind": "ready", "generation": 3},
                    "credential_set_id": "primary-set",
                    "selector_generation": 4,
                    "credentials": {"total": 2, "available": 1},
                    "endpoint_capabilities": {
                        "chat_completions": "supported",
                        "responses": "unsupported",
                        "embeddings": "unknown",
                        "models": "local_projection",
                        "diagnostic_labels": ["openai_compatible", "line\nlabel"],
                        "raw_secret": "SHOULD_NOT_RENDER"
                    }
                }
            ]
        });

        let rendered_json =
            render_models_explain_report(&input, crate::cli_report::OutputFormat::Json);
        let report: Value = serde_json::from_str(&rendered_json).unwrap();

        assert_eq!(report["capability_status"], "available");
        assert_eq!(
            report["candidates"][0]["endpoint_capabilities"],
            serde_json::json!({
                "chat_completions": "supported",
                "responses": "unsupported",
                "embeddings": "unknown",
                "models": "local_projection",
                "diagnostic_labels": ["openai_compatible", "line\nlabel"]
            })
        );
        assert!(report["candidates"][0]["endpoint_capabilities"]
            .get("raw_secret")
            .is_none());
        assert!(!rendered_json.contains("SHOULD_NOT_RENDER"));

        let rendered_table =
            render_models_explain_report(&input, crate::cli_report::OutputFormat::Table);

        assert!(rendered_table.contains("capability_status: available"));
        assert!(rendered_table.contains(
            "endpoint_capabilities.chat_completions=supported endpoint_capabilities.responses=unsupported endpoint_capabilities.embeddings=unknown endpoint_capabilities.models=local_projection endpoint_capabilities.diagnostic_labels=openai_compatible,line\\nlabel"
        ));
        assert!(!rendered_table.contains("line\nlabel"));
    }

    #[test]
    fn endpoint_capability_cli_models_explain_ignores_illegal_capability_shapes() {
        let input = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "configured",
            "selected_target": null,
            "candidates": [
                {
                    "channel_id": "primary",
                    "included": false,
                    "selected": false,
                    "health": {"kind": "ready"},
                    "endpoint_capabilities": ["supported"]
                },
                {
                    "channel_id": "fallback",
                    "included": false,
                    "selected": false,
                    "health": {"kind": "ready"},
                    "endpoint_capabilities": {
                        "chat_completions": {"nested": "supported"},
                        "diagnostic_labels": "openai_compatible",
                        "raw_secret": "SHOULD_NOT_RENDER"
                    }
                }
            ]
        });

        let rendered = render_models_explain_report(&input, crate::cli_report::OutputFormat::Json);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["capability_status"], "unknown");
        assert!(report["candidates"][0]["endpoint_capabilities"].is_null());
        assert!(report["candidates"][1]["endpoint_capabilities"].is_null());
        assert!(!rendered.contains("SHOULD_NOT_RENDER"));
    }

    #[test]
    fn models_explain_json_uses_server_availability_as_canonical_top_level_projection() {
        let preview = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "explicit_model_route",
            "registry_generation": 5,
            "client_token": {
                "id": "https://preview.example/v1?secret=sk-PREVIEW_SHOULD_NOT_RENDER",
                "name": "\t/tmp/preview/sk-PREVIEW_SHOULD_NOT_RENDER",
                "unrestricted_model_groups": true,
                "unrestricted_channels": true
            },
            "selected_target": null,
            "candidates": [
                {
                    "channel_id": "preview-blocked",
                    "included": false,
                    "selected": false,
                    "health": {"kind": "disabled", "reason_code": "channel_disabled"},
                    "credentials": {"total": 0, "available": 0}
                }
            ]
        });
        let availability = serde_json::json!({
            "status": "available",
            "can_use": true,
            "blocking_domain": "none",
            "reason_code": "available",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "route_kind": "explicit_model_route",
            "registry_generation": 7,
            "evidence": {
                "route_target_count": 2,
                "endpoint_family_target_count": 1,
                "candidate_reason_codes": ["no_available_credentials", "token_hash_SHOULD_NOT_RENDER"],
                "raw_secret": "SHOULD_NOT_RENDER"
            },
            "next_step": {
                "summary": "No repair action is required.",
                "template_id": "no_action_required",
                "safe_argv": [],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false,
                "unsafe_path": "/tmp/SHOULD_NOT_RENDER"
            },
            "next_action": "none",
            "client_token": {
                "id": "\tcontrol-client-id",
                "name": "https://relay.example/tmp/private?secret=sk-SHOULD_NOT_RENDER\njoin-now",
                "enabled": true
            },
            "raw_token": "SHOULD_NOT_RENDER"
        });

        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["can_use"], true);
        assert_eq!(report["blocking_domain"], "none");
        assert_eq!(report["reason_code"], "available");
        assert_eq!(report["endpoint_family"], "chat_completions");
        assert_eq!(report["model"], "gpt-public");
        assert_eq!(report["client_token_ref"], "local-client");
        assert_eq!(report["evidence"]["route_target_count"], 2);
        assert_eq!(report["evidence"]["endpoint_family_target_count"], 1);
        assert_eq!(
            report["evidence"]["candidate_reason_codes"],
            serde_json::json!(["no_available_credentials"])
        );
        assert_eq!(report["next_action"]["template_id"], "no_action_required");
        assert_eq!(report["next_action"]["safe_argv"], serde_json::json!([]));
        assert_eq!(report["availability"]["can_use"], true);
        assert_eq!(report["availability"]["blocking_domain"], "none");
        assert_eq!(report["client_token"]["id"], "<redacted-client-token-id>");
        assert_eq!(
            report["client_token"]["name"],
            "<redacted-client-token-name>"
        );
        assert_eq!(
            report["availability"]["client_token"]["id"],
            "<redacted-client-token-id>"
        );
        assert_eq!(
            report["availability"]["client_token"]["name"],
            "<redacted-client-token-name>"
        );
        assert_eq!(report["availability"]["client_token"]["enabled"], true);
        assert_eq!(report["selected_target"], serde_json::Value::Null);
        assert_eq!(report["route_kind"], "explicit_model_route");
        assert_eq!(report["registry_generation"], 5);
        assert!(!rendered.contains("SHOULD_NOT_RENDER"));
        assert!(!rendered.contains("secret="));
        assert!(!rendered.contains("https://relay.example"));
        assert!(!rendered.contains("/tmp/private"));
        assert!(!rendered.contains("sk-SHOULD_NOT_RENDER"));
        assert!(!rendered.contains("control-client-id"));
        assert!(!rendered.contains("https://preview.example"));
        assert!(!rendered.contains("/tmp/preview"));
        assert!(!rendered.contains("sk-PREVIEW_SHOULD_NOT_RENDER"));
        assert!(!rendered.contains("PREVIEW_SHOULD_NOT_RENDER"));
        assert!(!rendered.contains("join-now"));
        assert!(!rendered.contains("/tmp/"));
        assert!(!rendered.contains("raw_secret"));
    }

    #[test]
    fn models_explain_json_reports_unavailable_server_projection_top_level() {
        let preview = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "explicit_model_route",
            "registry_generation": 5,
            "client_token": {"name": "local-client"},
            "selected_target": {"channel_id": "preview-selected", "plan_position": 0},
            "candidates": []
        });
        let availability = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "endpoint_family",
            "reason_code": "endpoint_family_mismatch",
            "endpoint_family": "embeddings",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "route_kind": "explicit_model_route",
            "registry_generation": 7,
            "evidence": {
                "route_target_count": 1,
                "endpoint_family_target_count": 0,
                "unsupported_target_count": 0,
                "unknown_or_missing_target_count": 1
            },
            "next_step": {
                "summary": "Inspect runtime route target endpoint capabilities.",
                "template_id": "route_explain",
                "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "<public-model>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            },
            "next_action": "configure_endpoint_capabilities_or_route"
        });

        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["can_use"], false);
        assert_eq!(report["blocking_domain"], "endpoint_family");
        assert_eq!(report["reason_code"], "endpoint_family_mismatch");
        assert_eq!(report["next_action"]["template_id"], "route_explain");
        assert_eq!(
            report["next_action"]["safe_argv"],
            serde_json::json!([
                "one-ai-key",
                "route",
                "explain",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "<public-model>"
            ])
        );
    }

    #[test]
    fn models_explain_table_reports_endpoint_family_blocking_domain_and_evidence_source() {
        let preview = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "explicit_model_route",
            "registry_generation": 5,
            "client_token": {"name": "local-client"},
            "selected_target": {"channel_id": "preview-selected", "plan_position": 0},
            "candidates": []
        });
        let availability = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "endpoint_family",
            "reason_code": "endpoint_family_mismatch",
            "endpoint_family": "embeddings",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "route_kind": "explicit_model_route",
            "registry_generation": 7,
            "evidence": {
                "route_target_count": 1,
                "endpoint_family_target_count": 0,
                "unknown_or_missing_target_count": 1,
                "raw_secret": "SHOULD_NOT_RENDER"
            },
            "next_step": {
                "summary": "Inspect runtime route target endpoint capabilities.",
                "template_id": "route_explain",
                "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "<public-model>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            },
            "next_action": "configure_endpoint_capabilities_or_route"
        });

        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability),
            crate::cli_report::OutputFormat::Table,
        );

        assert!(rendered.contains("availability.blocking_domain: endpoint_family"));
        assert!(rendered.contains("availability.evidence_source: management_model_availability"));
        assert!(rendered.contains("availability.endpoint_family: embeddings"));
        assert!(!rendered.contains("raw_secret"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER"));
    }

    #[test]
    fn models_explain_management_availability_projection_is_canonical() {
        let preview = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "explicit_model_route",
            "registry_generation": 5,
            "client_token": {"name": "local-client"},
            "selected_target": {"channel_id": "preview-selected", "plan_position": 0},
            "candidates": []
        });
        let availability = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "route",
            "reason_code": "no_route",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "route_kind": "no_route",
            "registry_generation": 7,
            "evidence": {
                "route_target_count": 0,
                "endpoint_family_target_count": 0,
                "candidate_reason_codes": ["no_route"]
            },
            "next_step": {
                "summary": "Inspect the management-projected route state.",
                "template_id": "management_route_projection",
                "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "<public-model>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            },
            "next_action": "inspect_management_route_projection"
        });

        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(report["blocking_domain"], "route");
        assert_eq!(
            report["availability"]["next_action"],
            "inspect_management_route_projection"
        );
        assert_eq!(
            report["availability"]["next_step"]["template_id"],
            "management_route_projection"
        );
        assert_eq!(
            report["availability"]["next_step"]["safe_argv"],
            serde_json::json!([
                "one-ai-key",
                "route",
                "explain",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "<public-model>"
            ])
        );
        assert_eq!(report["next_action"], report["availability"]["next_step"]);
    }

    #[test]
    fn models_explain_management_next_step_rejects_list_actions_and_accepts_stage2_readonly() {
        let preview = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "explicit_model_route",
            "registry_generation": 5,
            "client_token": {"name": "local-client"},
            "selected_target": null,
            "candidates": []
        });
        let availability_with_models_list = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "model",
            "reason_code": "model_missing",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "evidence": {
                "route_target_count": 0,
                "endpoint_family_target_count": 0
            },
            "next_step": {
                "summary": "Do not surface legacy models list.",
                "template_id": "models_list",
                "safe_argv": ["one-ai-key", "models", "list", "--management-url", "<url>", "--management-token-env", "<env>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }
        });
        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability_with_models_list),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(report["availability"]["next_step"], serde_json::Value::Null);
        assert_ne!(report["next_action"]["template_id"], "models_list");

        let availability_with_client_tokens_list = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "client_token",
            "reason_code": "token_unknown",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "evidence": {
                "route_target_count": 0,
                "endpoint_family_target_count": 0
            },
            "next_step": {
                "summary": "Do not surface legacy client token list.",
                "template_id": "client_tokens_list",
                "safe_argv": ["one-ai-key", "client-tokens", "list", "--management-url", "<url>", "--management-token-env", "<env>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }
        });
        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability_with_client_tokens_list),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(report["availability"]["next_step"], serde_json::Value::Null);
        assert_ne!(report["next_action"]["template_id"], "client_tokens_list");

        let availability_with_reload_status = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "runtime",
            "reason_code": "runtime_unavailable",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "evidence": {
                "route_target_count": 1,
                "endpoint_family_target_count": 1
            },
            "next_step": {
                "summary": "Inspect read-only runtime reload status.",
                "template_id": "reload_status",
                "safe_argv": ["one-ai-key", "reload", "status", "--management-url", "<url>", "--management-token-env", "<env>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }
        });
        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability_with_reload_status),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(
            report["availability"]["next_step"]["template_id"],
            "reload_status"
        );
        assert_eq!(report["next_action"], report["availability"]["next_step"]);

        let availability_with_mismatched_template = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "runtime",
            "reason_code": "runtime_unavailable",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "evidence": {
                "route_target_count": 1,
                "endpoint_family_target_count": 1
            },
            "next_step": {
                "summary": "Inspect read-only runtime doctor projection.",
                "template_id": "client_tokens_list",
                "safe_argv": ["one-ai-key", "doctor", "--management-url", "<url>", "--management-token-env", "<env>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }
        });
        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability_with_mismatched_template),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(report["availability"]["next_step"], serde_json::Value::Null);
        assert_ne!(report["next_action"]["template_id"], "client_tokens_list");
    }

    #[test]
    fn canonical_models_explain_rejects_missing_management_next_step_without_local_fallback() {
        let preview = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "explicit_model_route",
            "registry_generation": 5,
            "client_token": {"name": "local-client"},
            "selected_target": null,
            "candidates": []
        });
        let availability = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "route",
            "reason_code": "no_route",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "evidence": {
                "route_target_count": 0,
                "endpoint_family_target_count": 0,
                "candidate_reason_codes": ["no_route"]
            }
        });

        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["can_use"], false);
        assert_eq!(report["reason_code"], "management_projection_invalid");
        assert_eq!(report["blocking_domain"], "management_projection");
        assert_eq!(
            report["next_action"]["template_id"],
            "no_action_required",
            "canonical endpoint-family path must not locally derive route_explain when management next_step is missing"
        );
        assert_eq!(report["next_action"]["safe_argv"], serde_json::json!([]));
        assert_eq!(report["evidence"]["projection"], "model_availability");
        assert_eq!(
            report["evidence"]["invalid_fields"],
            serde_json::json!(["next_step"])
        );
        assert_eq!(report["availability"]["next_step"], serde_json::Value::Null);
    }

    #[test]
    fn canonical_models_explain_rejects_invalid_management_projection_fields_without_unsafe_action()
    {
        let preview = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "explicit_model_route",
            "registry_generation": 5,
            "client_token": {"name": "local-client"},
            "selected_target": {"channel_id": "preview-selected", "plan_position": 0},
            "candidates": []
        });
        let availability = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "../route",
            "reason_code": "no_route",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "evidence": {
                "route_target_count": 0,
                "endpoint_family_target_count": 0,
                "candidate_reason_codes": ["no_route"]
            },
            "next_step": {
                "summary": "Apply runtime reload.",
                "template_id": "reload_apply",
                "safe_argv": ["one-ai-key", "reload", "apply", "--yes"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }
        });

        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let next_action_argv = report["next_action"]["safe_argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .unwrap()
            .join(" ");

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["can_use"], false);
        assert_eq!(report["reason_code"], "management_projection_invalid");
        assert_eq!(report["blocking_domain"], "management_projection");
        assert_eq!(report["next_action"]["template_id"], "no_action_required");
        assert_eq!(
            report["evidence"]["invalid_fields"],
            serde_json::json!(["blocking_domain", "next_step"])
        );
        for forbidden in [
            "reload apply",
            "keys import",
            "keys probe",
            "curl",
            "route explain",
        ] {
            assert!(
                !next_action_argv.contains(forbidden),
                "invalid canonical projection must not produce local or unsafe suggestion `{forbidden}`"
            );
        }
    }

    #[test]
    fn models_explain_surfaces_management_reload_and_recent_failure_hint() {
        let preview = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "explicit_model_route",
            "registry_generation": 5,
            "client_token": {"name": "local-client"},
            "selected_target": null,
            "candidates": []
        });
        let availability = serde_json::json!({
            "status": "unavailable",
            "can_use": false,
            "blocking_domain": "target",
            "reason_code": "no_usable_key_or_target",
            "endpoint_family": "chat_completions",
            "model": "gpt-public",
            "public_model": "gpt-public",
            "client_token_ref": "local-client",
            "evidence": {
                "route_target_count": 1,
                "endpoint_family_target_count": 0,
                "candidate_reason_codes": ["no_usable_key_or_target"]
            },
            "reload_drift": {
                "status": "unknown",
                "reason_code": "staged_registry_version_unavailable",
                "active_registry_generation": 7,
                "active_registry_version": null,
                "staged_registry_version": null,
                "runtime_reload_required": null,
                "last_reload_at_unix_seconds": null,
                "last_reload_error_reason_code": null
            },
            "recent_failure_hint": {
                "status": "none",
                "source": "routing_telemetry_bounded_window",
                "window_event_count": 0,
                "matched_event_count": 0,
                "reason_codes": [],
                "channel_ids": [],
                "raw_upstream_text": "SHOULD_NOT_RENDER"
            },
            "next_step": {
                "summary": "Inspect route candidates and admission details for this public model.",
                "template_id": "route_explain",
                "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "<public-model>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            }
        });

        let rendered = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(report["reload_drift"]["status"], "unknown");
        assert_eq!(
            report["reload_drift"]["reason_code"],
            "staged_registry_version_unavailable"
        );
        assert_eq!(report["recent_failure_hint"]["status"], "none");
        assert_eq!(
            report["recent_failure_hint"]["reason_codes"],
            serde_json::json!([])
        );
        assert!(!rendered.contains("SHOULD_NOT_RENDER"));
        assert!(!rendered.contains("raw_upstream_text"));

        let table = super::render_models_explain_report_with_management_projection(
            &preview,
            None,
            Some(&availability),
            crate::cli_report::OutputFormat::Table,
        );
        assert!(table.contains("availability.reload_drift.status: unknown"));
        assert!(table.contains(
            "availability.reload_drift.reason_code: staged_registry_version_unavailable"
        ));
        assert!(table.contains("availability.recent_failure_hint.status: none"));
        assert!(table.contains("availability.recent_failure_hint.reason_codes: []"));
        assert!(!table.contains("SHOULD_NOT_RENDER"));
        assert!(!table.contains("raw_upstream_text"));
    }

    #[test]
    fn models_explain_json_reports_runtime_reload_projection_without_sensitive_fields() {
        let legacy_safe_field = ["safe", "command"].join("_");
        let legacy_dry_run_field = ["dry", "run", "command"].join("_");
        let input = serde_json::json!({
            "model": "gpt-public",
            "route_kind": "configured",
            "client_token": {
                "id": "local-client",
                "name": "Local Client",
                "unrestricted_model_groups": true,
                "unrestricted_channels": true,
                "allowed_model_groups": [],
                "allowed_channels": []
            },
            "selected_target": {
                "channel_id": "primary",
                "plan_position": 0
            },
            "candidates": []
        });
        let projection = serde_json::json!({
            "status": "ok",
            "reason_code": "reload_diff_available",
            "active_registry_generation": 21,
            "active_registry_version": 4,
            "staged_registry_version": 5,
            "runtime_reload_required": true,
            "next_action": {
                "safe_argv": ["unsafe-raw-path"],
                legacy_safe_field: "DO_NOT_LEAK",
                legacy_dry_run_field: "DO_NOT_LEAK"
            },
            "raw_path": "/tmp/secret-registry.yaml",
            "raw_yaml": "RAW_YAML_SHOULD_NOT_APPEAR"
        });

        let rendered = super::render_models_explain_report_with_management_projection(
            &input,
            Some(&projection),
            None,
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

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
            report["reload_diff_next_action"]["safe_argv"],
            serde_json::json!([
                "one-ai-key",
                "reload",
                "diff",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>"
            ])
        );
        assert!(!rendered.contains("/tmp/secret-registry.yaml"));
        assert!(!rendered.contains("RAW_YAML_SHOULD_NOT_APPEAR"));
        assert!(!rendered.contains("unsafe-raw-path"));
        assert!(!rendered.contains("DO_NOT_LEAK"));
        assert_ne!(report["reload_diff_status"], "unavailable_until_m4");
    }

    #[tokio::test]
    async fn endpoint_capability_cli_does_not_probe_upstream_models_explain() {
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
                        Json(serde_json::json!({
                            "model": "gpt-public",
                            "selected_target": {"channel_id": "primary", "plan_position": 0},
                            "candidates": [{
                                "channel_id": "primary",
                                "included": true,
                                "selected": true,
                                "health": {"kind": "ready"},
                                "endpoint_capabilities": {
                                    "chat_completions": "supported",
                                    "responses": "unsupported",
                                    "embeddings": "unknown",
                                    "models": "local_projection",
                                    "diagnostic_labels": ["openai_compatible"]
                                }
                            }]
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
                        Json(serde_json::json!({
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
        let env_name = format!(
            "ONE_AI_KEY_TEST_MODELS_EXPLAIN_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run_explain(super::ModelsExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-public".to_string(),
            client_token_ref: None,
            endpoint_family: None,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

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
        assert_eq!(report["capability_status"], "available");
    }

    #[tokio::test]
    async fn models_explain_run_fetches_management_model_availability_when_endpoint_family_requested(
    ) {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let preview_seen = Arc::clone(&seen_paths);
        let diff_seen = Arc::clone(&seen_paths);
        let availability_seen = Arc::clone(&seen_paths);
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
                        Json(serde_json::json!({
                            "model": "gpt-public",
                            "selected_target": {"channel_id": "primary", "plan_position": 0},
                            "client_token": {"name": "local-client"},
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
                        Json(serde_json::json!({
                            "status": "ok",
                            "reason_code": "reload_diff_empty",
                            "runtime_reload_required": false
                        }))
                    }
                }),
            )
            .route(
                "/management/model-availability",
                get(move || {
                    let availability_seen = Arc::clone(&availability_seen);
                    async move {
                        availability_seen
                            .lock()
                            .unwrap()
                            .push("/management/model-availability".to_string());
                        Json(serde_json::json!({
                            "status": "available",
                            "can_use": true,
                            "blocking_domain": "none",
                            "reason_code": "available",
                            "next_action": "none",
                            "endpoint_family": "chat_completions",
                            "model": "gpt-public",
                            "client_token_ref": "local-client",
                            "evidence": {
                                "route_target_count": 1,
                                "endpoint_family_target_count": 1,
                                "selected_target_present": true,
                                "candidate_reason_codes": ["available", "raw_SHOULD_NOT_RENDER"]
                            },
                            "next_step": {
                                "summary": "No repair action is required.",
                                "template_id": "no_action_required",
                                "safe_argv": [],
                                "side_effect_class": "runtime_readonly",
                                "requires_confirmation": false
                            },
                            "public_model": "https://relay.example/v1?token=sk-SHOULD_NOT_RENDER",
                            "route_kind": "explicit_model_route",
                            "registry_generation": 7,
                            "client_token": {
                                "id": "client-a",
                                "name": "local-client",
                                "enabled": true,
                                "token_hash": "SHOULD_NOT_RENDER"
                            },
                            "raw_token": "SHOULD_NOT_RENDER",
                            "upstream_url": "https://relay.example/v1?token=SHOULD_NOT_RENDER"
                        }))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_MODELS_EXPLAIN_AVAILABILITY_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run_explain(super::ModelsExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-public".to_string(),
            client_token_ref: Some("local-client".to_string()),
            endpoint_family: Some("chat_completions".to_string()),
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/routing/preview".to_string(),
                "/management/runtime/reload-diff".to_string(),
                "/management/model-availability".to_string(),
            ]
        );
        assert_eq!(report["availability"]["status"], "available");
        assert_eq!(report["availability"]["reason_code"], "available");
        assert_eq!(report["can_use"], true);
        assert_eq!(report["blocking_domain"], "none");
        assert_eq!(report["reason_code"], "available");
        assert_eq!(report["endpoint_family"], "chat_completions");
        assert_eq!(report["model"], "gpt-public");
        assert_eq!(report["client_token_ref"], "local-client");
        assert_eq!(
            report["evidence"]["candidate_reason_codes"],
            serde_json::json!(["available"])
        );
        assert_eq!(report["next_action"]["template_id"], "no_action_required");
        assert_eq!(report["next_action"]["safe_argv"], serde_json::json!([]));
        assert_eq!(
            report["availability"]["endpoint_family"],
            "chat_completions"
        );
        assert_eq!(report["scope"]["endpoint_family"], "chat_completions");
        assert!(!rendered.contains("SHOULD_NOT_RENDER"));
        assert!(!rendered.contains("sk-"));
        assert!(!rendered.contains("token="));
        assert!(!rendered.contains("https://relay.example/v1"));
    }

    #[tokio::test]
    async fn models_explain_run_reports_unavailable_model_availability_as_top_level_block() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let preview_seen = Arc::clone(&seen_paths);
        let diff_seen = Arc::clone(&seen_paths);
        let availability_seen = Arc::clone(&seen_paths);
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
                        Json(serde_json::json!({
                            "model": "gpt-public",
                            "route_kind": "explicit_model_route",
                            "selected_target": {"channel_id": "primary", "plan_position": 0},
                            "client_token": {"name": "local-client"},
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
                        Json(serde_json::json!({
                            "status": "ok",
                            "reason_code": "reload_diff_empty",
                            "runtime_reload_required": false
                        }))
                    }
                }),
            )
            .route(
                "/management/model-availability",
                get(move || {
                    let availability_seen = Arc::clone(&availability_seen);
                    async move {
                        availability_seen
                            .lock()
                            .unwrap()
                            .push("/management/model-availability".to_string());
                        Json(serde_json::json!({
                            "status": "unavailable",
                            "can_use": false,
                            "blocking_domain": "endpoint_family",
                            "reason_code": "endpoint_family_mismatch",
                            "next_action": "configure_endpoint_capabilities_or_route",
                            "endpoint_family": "embeddings",
                            "model": "gpt-public",
                            "public_model": "gpt-public",
                            "client_token_ref": "local-client",
                            "route_kind": "explicit_model_route",
                            "registry_generation": 7,
                            "evidence": {
                                "route_target_count": 1,
                                "endpoint_family_target_count": 0,
                                "unknown_or_missing_target_count": 1
                            },
                            "next_step": {
                                "summary": "Inspect runtime route target endpoint capabilities.",
                                "template_id": "route_explain",
                                "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "<public-model>"],
                                "side_effect_class": "runtime_readonly",
                                "requires_confirmation": false
                            }
                        }))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_MODELS_EXPLAIN_UNAVAILABLE_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run_explain(super::ModelsExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-public".to_string(),
            client_token_ref: Some("local-client".to_string()),
            endpoint_family: Some("embeddings".to_string()),
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/routing/preview".to_string(),
                "/management/runtime/reload-diff".to_string(),
                "/management/model-availability".to_string(),
            ]
        );
        assert_eq!(report["status"], "blocked");
        assert_eq!(report["can_use"], false);
        assert_eq!(report["blocking_domain"], "endpoint_family");
        assert_eq!(report["reason_code"], "endpoint_family_mismatch");
        assert_eq!(report["selected_target"]["channel_id"], "primary");
        assert_eq!(report["next_action"]["template_id"], "route_explain");
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "runtime_readonly"
        );
        assert_eq!(report["next_action"]["requires_confirmation"], false);
    }

    #[tokio::test]
    async fn models_explain_run_does_not_hide_reload_diff_auth_failure() {
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
                        Json(serde_json::json!({
                            "model": "gpt-public",
                            "selected_target": {"channel_id": "primary", "plan_position": 0},
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
                        Json(serde_json::json!({"runtime_reload_required": false}))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_MODELS_EXPLAIN_AUTH_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let error = super::run_explain(super::ModelsExplainOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            model: "gpt-public".to_string(),
            client_token_ref: None,
            endpoint_family: None,
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
    fn models_table_escapes_control_characters() {
        let input = serde_json::json!({
            "model": "gpt\npublic\u{1b}[31m",
            "route_kind": "configured",
            "client_token": {
                "id": "local-client",
                "name": "Local Client",
                "unrestricted_model_groups": true,
                "unrestricted_channels": true,
                "allowed_model_groups": [],
                "allowed_channels": []
            },
            "selected_target": null,
            "candidates": [
                {
                    "target_index": 0,
                    "channel_id": "primary\rchannel",
                    "upstream_model": "vendor-model",
                    "provider_kind": "openai",
                    "priority": 10,
                    "weight": 1,
                    "target_enabled": true,
                    "included": false,
                    "selected": false,
                    "plan_position": null,
                    "reasons": ["line\nbreak", "ansi\u{1b}[31m"],
                    "health": {"kind": "ready", "generation": 3},
                    "credential_set_id": "primary-set",
                    "selector_generation": 4,
                    "credentials": {"total": 2, "available": 1}
                }
            ]
        });

        let rendered = render_models_explain_report(&input, crate::cli_report::OutputFormat::Table);

        assert!(rendered.contains("gpt\\npublic\\u{1b}[31m"));
        assert!(rendered.contains("primary\\rchannel"));
        assert!(rendered.contains("line\\nbreak,ansi\\u{1b}[31m"));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains("gpt\npublic"));
        assert!(!rendered.contains("primary\rchannel"));
    }
}
