use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsOnboardPlanOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub channel_id: String,
    pub public_model: String,
    pub upstream_model: Option<String>,
    pub client_token_ref: Option<String>,
    pub endpoint_family: Option<String>,
    pub mode: ModelsOnboardPlanMode,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsOnboardPlanMode {
    DryRun,
    DeferredToSeparatePlan,
}

pub async fn run(
    options: ModelsOnboardPlanOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    if matches!(options.mode, ModelsOnboardPlanMode::DeferredToSeparatePlan) {
        return Ok(render_deferred_report(&options));
    }
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let model_routes = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::ModelRoutes)
        .await?;
    let channel = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::Channel {
            channel_id: options.channel_id.clone(),
        })
        .await?;
    let runtime_reload_projection =
        crate::cli_commands::runtime_reload_projection::fetch_runtime_reload_projection(&client)
            .await?;
    let model_availability = match (&options.endpoint_family, &options.client_token_ref) {
        (Some(endpoint_family), Some(client_token_ref)) => Some(
            client
                .get_json(
                    crate::operator_client::ReadOnlyEndpoint::ModelAvailability {
                        model: options.public_model.clone(),
                        endpoint_family: endpoint_family.clone(),
                        client_token_ref: Some(client_token_ref.clone()),
                    },
                )
                .await?,
        ),
        _ => None,
    };
    Ok(render_onboard_plan_report(
        &options,
        Some(&model_routes),
        Some(&channel),
        runtime_reload_projection.as_ref(),
        model_availability.as_ref(),
        options.output,
    ))
}

pub fn render_onboard_plan_report(
    options: &ModelsOnboardPlanOptions,
    model_routes: Option<&Value>,
    channel: Option<&Value>,
    runtime_reload_projection: Option<&Value>,
    model_availability: Option<&Value>,
    output: crate::cli_report::OutputFormat,
) -> String {
    let report = sanitized_onboard_plan_report(
        options,
        model_routes,
        channel,
        runtime_reload_projection,
        model_availability,
    );
    match output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(&report).expect("models onboard plan should serialize")
        }
        crate::cli_report::OutputFormat::Table => render_onboard_plan_table(&report),
    }
}

fn render_deferred_report(options: &ModelsOnboardPlanOptions) -> String {
    let report = onboard_report_envelope(
        "blocked",
        "deferred_to_separate_plan",
        "Live discovery, sync, reload, and client-token scope mutation are deferred to a separate plan.",
        serde_json::json!({
            "command": "models onboard-plan",
            "channel_id": safe_local_string(&options.channel_id),
            "public_model": safe_local_string(&options.public_model),
            "upstream_model": safe_local_string(options.upstream_model.as_deref().unwrap_or(&options.public_model)),
            "client_token_ref": safe_reference_label_value_or_redacted(options.client_token_ref.as_deref()),
            "endpoint_family": safe_reason_code_or_null(options.endpoint_family.as_deref()),
            "planning_only_no_visibility_change": true,
            "client_visibility_changed": false,
            "live_discovery_called": false,
            "management_mutation_sent": false,
            "required_followups": deferred_followups(),
        }),
        next_action_deferred(),
        options,
    );
    match options.output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(&report).expect("models onboard deferred should serialize")
        }
        crate::cli_report::OutputFormat::Table => render_onboard_plan_table(&report),
    }
}

fn sanitized_onboard_plan_report(
    options: &ModelsOnboardPlanOptions,
    model_routes: Option<&Value>,
    channel: Option<&Value>,
    runtime_reload_projection: Option<&Value>,
    model_availability: Option<&Value>,
) -> Value {
    let upstream_model = options
        .upstream_model
        .as_deref()
        .unwrap_or(&options.public_model);
    let configured_public_routes = model_routes
        .and_then(|value| value.get("routes"))
        .and_then(Value::as_array)
        .map(|routes| routes.iter().map(sanitize_route).collect::<Vec<_>>())
        .unwrap_or_default();
    let credential_set_id = channel
        .and_then(|value| value.get("credential_set_id"))
        .and_then(Value::as_str)
        .and_then(safe_local_string);
    let reload_summary =
        crate::cli_commands::runtime_reload_projection::summarize_runtime_reload_projection(
            runtime_reload_projection,
        );
    let client_visibility = client_visibility_summary(options, model_availability);
    let endpoint_capabilities = endpoint_capability_summary(channel);

    if model_routes.is_none() || channel.is_none() {
        return onboard_report_envelope(
            "blocked",
            "onboard_plan_projection_unavailable",
            "Read-only runtime projections required for model onboarding planning are unavailable.",
            serde_json::json!({
                "command": "models onboard-plan",
                "channel_id": safe_local_string(&options.channel_id),
                "public_model": safe_local_string(&options.public_model),
                "upstream_model": safe_local_string(upstream_model),
                "credential_set_id": Value::Null,
                "credential_set_ref": Value::Null,
                "key_set_ref": Value::Null,
                "channel_projection": Value::Null,
                "configured_public_routes": [],
                "currently_exposed_public_routes": [],
                "current_conflicts": current_conflicts(model_routes, &options.public_model, &options.channel_id, upstream_model),
                "endpoint_capability_status": "unknown",
                "endpoint_capability_reason_code": "not_reported",
                "endpoint_capabilities": Value::Null,
                "client_visibility": client_visibility,
                "active_registry_generation": reload_summary.active_registry_generation,
                "active_registry_version": reload_summary.active_registry_version,
                "staged_registry_version": reload_summary.staged_registry_version,
                "runtime_reload_required": reload_summary.runtime_reload_required,
                "reload_diff_status": reload_summary.reload_diff_status,
                "reload_diff_reason_code": reload_summary.reload_diff_reason_code,
                "reload_diff_next_action": reload_summary.reload_diff_next_action,
                "proposed_route": proposed_route(options, upstream_model),
                "planning_only_no_visibility_change": true,
                "client_visibility_changed": false,
                "live_discovery_called": false,
                "management_mutation_sent": false,
                "required_followups": deferred_followups(),
            }),
            next_action_deferred(),
            options,
        );
    }

    let status = if credential_set_id.is_some() {
        "dry_run"
    } else {
        "blocked"
    };
    let reason_code = if credential_set_id.is_some() {
        "models_onboard_plan_projected"
    } else {
        "onboard_plan_projection_unavailable"
    };
    let reason = if credential_set_id.is_some() {
        "Model route onboarding plan was built from read-only runtime projections without changing visibility."
    } else {
        "The requested channel was not present in the bounded credential-set projection."
    };
    onboard_report_envelope(
        status,
        reason_code,
        reason,
        serde_json::json!({
            "command": "models onboard-plan",
            "channel_id": safe_local_string(&options.channel_id),
            "public_model": safe_local_string(&options.public_model),
            "upstream_model": safe_local_string(upstream_model),
            "credential_set_id": credential_set_id,
            "credential_set_ref": credential_set_id,
            "key_set_ref": credential_set_id,
            "channel_projection": sanitize_channel(channel),
            "configured_public_routes": configured_public_routes,
            "currently_exposed_public_routes": model_routes_for_channel(model_routes, &options.channel_id),
            "current_conflicts": current_conflicts(model_routes, &options.public_model, &options.channel_id, upstream_model),
            "endpoint_capability_status": endpoint_capabilities.0,
            "endpoint_capability_reason_code": endpoint_capabilities.1,
            "endpoint_capabilities": endpoint_capabilities.2,
            "client_visibility": client_visibility,
            "active_registry_generation": reload_summary.active_registry_generation,
            "active_registry_version": reload_summary.active_registry_version,
            "staged_registry_version": reload_summary.staged_registry_version,
            "runtime_reload_required": reload_summary.runtime_reload_required,
            "reload_diff_status": reload_summary.reload_diff_status,
            "reload_diff_reason_code": reload_summary.reload_diff_reason_code,
            "reload_diff_next_action": reload_summary.reload_diff_next_action,
            "possible_upstream_onboarding": "not_discovered",
            "proposed_route": proposed_route(options, upstream_model),
            "planning_only_no_visibility_change": true,
            "client_visibility_changed": false,
            "live_discovery_called": false,
            "management_mutation_sent": false,
            "required_followups": deferred_followups(),
        }),
        next_action_for_plan(status),
        options,
    )
}

fn onboard_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    data: Value,
    next_action: Value,
    options: &ModelsOnboardPlanOptions,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect: crate::cli_effects::runtime_readonly_effect(),
        scope: serde_json::json!({
            "channel_id": safe_local_string(&options.channel_id),
            "public_model": safe_local_string(&options.public_model),
            "projection": "model_routes_and_channel",
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn proposed_route(options: &ModelsOnboardPlanOptions, upstream_model: &str) -> Value {
    serde_json::json!({
        "public_model": safe_local_string(&options.public_model),
        "upstream_model": safe_local_string(upstream_model),
        "target": {
            "channel_id": safe_local_string(&options.channel_id),
            "upstream_model": safe_local_string(upstream_model),
            "enabled": true,
        },
        "visibility_after_this_command": false,
    })
}

fn sanitize_route(route: &Value) -> Value {
    let targets = route
        .get("targets")
        .and_then(Value::as_array)
        .map(|targets| targets.iter().map(sanitize_target).collect::<Vec<_>>())
        .unwrap_or_default();
    serde_json::json!({
        "model": route.get("model").and_then(Value::as_str).and_then(safe_local_string),
        "strategy": route.get("strategy").and_then(Value::as_str).and_then(safe_local_string),
        "target_count": targets.len(),
        "targets": targets,
    })
}

fn sanitize_target(target: &Value) -> Value {
    serde_json::json!({
        "channel_id": target.get("channel_id").and_then(Value::as_str).and_then(safe_local_string),
        "upstream_model": target.get("upstream_model").and_then(Value::as_str).and_then(safe_local_string),
        "provider_kind": target.get("provider_kind").and_then(Value::as_str).and_then(safe_local_string),
        "priority": target.get("priority").and_then(Value::as_u64),
        "weight": target.get("weight").and_then(Value::as_u64),
        "enabled": target.get("enabled").and_then(Value::as_bool),
        "health_kind": target.get("health").and_then(|health| health.get("kind")).and_then(Value::as_str).and_then(safe_local_string),
        "health_reason_code": target.get("health").and_then(|health| health.get("reason_code")).and_then(Value::as_str).and_then(safe_local_string),
    })
}

fn sanitize_channel(channel: Option<&Value>) -> Value {
    let Some(channel) = channel else {
        return Value::Null;
    };
    serde_json::json!({
        "channel_id": channel.get("name").and_then(Value::as_str).and_then(safe_local_string),
        "credential_set_id": channel.get("credential_set_id").and_then(Value::as_str).and_then(safe_local_string),
        "provider_kind": channel.get("provider_kind").and_then(Value::as_str).and_then(safe_local_string),
        "configured_enabled": channel.get("configured_enabled").and_then(Value::as_bool),
        "total_credentials": channel.get("total_credentials").and_then(Value::as_u64),
        "available_credentials": channel.get("available_credentials").and_then(Value::as_u64),
        "endpoint_capabilities": crate::cli_report::sanitize_endpoint_capabilities(
            channel.get("endpoint_capabilities")
        ),
        "health_kind": channel
            .get("health")
            .and_then(|health| health.get("kind"))
            .and_then(Value::as_str)
            .and_then(safe_local_string),
        "health_reason_code": channel
            .get("health")
            .and_then(|health| health.get("reason_code"))
            .and_then(Value::as_str)
            .and_then(safe_local_string),
    })
}

fn current_conflicts(
    model_routes: Option<&Value>,
    public_model: &str,
    channel_id: &str,
    upstream_model: &str,
) -> Value {
    let mut entries = Vec::new();
    let routes = model_routes
        .and_then(|value| value.get("routes"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    for route in routes {
        let route_model = route
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if route_model == public_model {
            entries.push(serde_json::json!({
                "code": "public_model_already_configured",
                "public_model": safe_local_string(route_model),
            }));
        }

        for target in route
            .get("targets")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let target_channel = target
                .get("channel_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let target_upstream = target
                .get("upstream_model")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if target_channel != channel_id {
                continue;
            }
            if route_model == public_model && target_upstream == upstream_model {
                entries.push(serde_json::json!({
                    "code": "same_target_already_present",
                    "public_model": safe_local_string(route_model),
                    "channel_id": safe_local_string(target_channel),
                    "upstream_model": safe_local_string(target_upstream),
                }));
            } else if route_model == public_model && target_upstream != upstream_model {
                entries.push(serde_json::json!({
                    "code": "same_channel_different_upstream",
                    "public_model": safe_local_string(route_model),
                    "channel_id": safe_local_string(target_channel),
                    "existing_upstream_model": safe_local_string(target_upstream),
                    "proposed_upstream_model": safe_local_string(upstream_model),
                }));
            } else if route_model != public_model {
                entries.push(serde_json::json!({
                    "code": "channel_used_by_other_public_models",
                    "other_public_model": safe_local_string(route_model),
                    "channel_id": safe_local_string(target_channel),
                    "upstream_model": safe_local_string(target_upstream),
                }));
            }
        }
    }

    serde_json::json!({
        "has_conflicts": !entries.is_empty(),
        "entry_count": entries.len(),
        "entries": entries,
    })
}

fn endpoint_capability_summary(channel: Option<&Value>) -> (&'static str, &'static str, Value) {
    let capabilities = crate::cli_report::sanitize_endpoint_capabilities(
        channel.and_then(|channel| channel.get("endpoint_capabilities")),
    );
    if capabilities.is_object() {
        ("available", "reported", capabilities)
    } else {
        ("unknown", "not_reported", Value::Null)
    }
}

fn client_visibility_summary(
    options: &ModelsOnboardPlanOptions,
    model_availability: Option<&Value>,
) -> Value {
    if let Some(model_availability) = model_availability {
        return sanitize_model_availability(model_availability);
    }

    serde_json::json!({
        "status": "not_evaluated",
        "reason_code": "missing_visibility_args",
        "reason": "client visibility requires both --client-token-ref and --endpoint-family",
        "client_token_ref": safe_reference_label_value_or_redacted(options.client_token_ref.as_deref()),
        "endpoint_family": safe_reason_code_or_null(options.endpoint_family.as_deref()),
        "can_use": Value::Null,
        "source": "not_evaluated",
    })
}

fn sanitize_model_availability(value: &Value) -> Value {
    let client_token = value
        .get("client_token")
        .and_then(Value::as_object)
        .map(|token| {
            serde_json::json!({
                "id": safe_reference_label_value_or_redacted(token.get("id").and_then(Value::as_str)),
                "name": safe_reference_label_value_or_redacted(token.get("name").and_then(Value::as_str)),
                "enabled": token.get("enabled").and_then(Value::as_bool),
            })
        })
        .unwrap_or(Value::Null);

    serde_json::json!({
        "status": safe_reason_code_or_null(value.get("status").and_then(Value::as_str)),
        "can_use": value.get("can_use").and_then(Value::as_bool),
        "blocking_domain": safe_reason_code_or_null(value.get("blocking_domain").and_then(Value::as_str)),
        "reason_code": safe_reason_code_or_null(value.get("reason_code").and_then(Value::as_str)),
        "next_action": safe_reason_code_or_null(value.get("next_action").and_then(Value::as_str)),
        "endpoint_family": safe_reason_code_or_null(value.get("endpoint_family").and_then(Value::as_str)),
        "model": value.get("model").and_then(Value::as_str).and_then(safe_local_string),
        "public_model": value.get("public_model").and_then(Value::as_str).and_then(safe_local_string),
        "client_token_ref": safe_reference_label_value_or_redacted(value.get("client_token_ref").and_then(Value::as_str)),
        "route_kind": safe_reason_code_or_null(value.get("route_kind").and_then(Value::as_str)),
        "registry_generation": value.get("registry_generation").and_then(Value::as_u64),
        "evidence": sanitize_availability_evidence(value.get("evidence")),
        "client_token": client_token,
        "source": "management_model_availability",
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
    Value::Object(sanitized)
}

fn model_routes_for_channel(model_routes: Option<&Value>, channel_id: &str) -> Value {
    let routes = model_routes
        .and_then(|value| value.get("routes"))
        .and_then(Value::as_array)
        .map(|routes| {
            routes
                .iter()
                .filter_map(|route| {
                    let route_targets = route
                        .get("targets")
                        .and_then(Value::as_array)
                        .map(|targets| {
                            targets
                                .iter()
                                .filter(|target| {
                                    target.get("channel_id").and_then(Value::as_str)
                                        == Some(channel_id)
                                })
                                .map(sanitize_target)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if route_targets.is_empty() {
                        None
                    } else {
                        Some(serde_json::json!({
                            "model": route.get("model").and_then(Value::as_str).and_then(safe_local_string),
                            "strategy": route.get("strategy").and_then(Value::as_str).and_then(safe_local_string),
                            "targets": route_targets,
                        }))
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(routes)
}

fn deferred_followups() -> Value {
    serde_json::json!([
        {
            "action": "edit_local_config_model_route",
            "status": "deferred_by_m1_m4",
            "effect_class": "local_write"
        },
        {
            "action": "reload_runtime_after_manual_config_change",
            "status": "deferred_by_m1_m4",
            "effect_class": "management_write"
        },
        {
            "action": "update_client_token_scope_if_needed",
            "status": "deferred_by_m1_m4",
            "effect_class": "management_write"
        }
    ])
}

fn next_action_for_plan(status: &str) -> Value {
    if status == "dry_run" {
        serde_json::json!({
            "summary": "Review this planning-only report. Add explicit local model-route configuration and use later reload/scope workflows when available.",
            "template_id": "manual_config_review",
            "safe_argv": [],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
            "repair_path": "deferred_by_m1_m4",
        })
    } else {
        next_action_deferred()
    }
}

fn next_action_deferred() -> Value {
    serde_json::json!({
        "summary": "Model onboarding apply, discovery, sync, reload, and client-token scope changes are deferred to a separate plan.",
        "template_id": "deferred_to_separate_plan",
        "safe_argv": [],
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
        "repair_path": "deferred_by_m1_m4",
    })
}

fn render_onboard_plan_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("Model onboarding plan\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "channel_id",
        "public_model",
        "upstream_model",
        "credential_set_id",
        "credential_set_ref",
        "key_set_ref",
        "endpoint_capability_status",
        "endpoint_capability_reason_code",
        "active_registry_generation",
        "active_registry_version",
        "staged_registry_version",
        "runtime_reload_required",
        "reload_diff_status",
        "reload_diff_reason_code",
        "planning_only_no_visibility_change",
        "client_visibility_changed",
        "live_discovery_called",
        "management_mutation_sent",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    if let Some(capabilities) = report.get("endpoint_capabilities") {
        output.push_str(&format!(
            "{}\n",
            crate::cli_report::endpoint_capabilities_table_summary(Some(capabilities))
        ));
    }
    if let Some(visibility) = report.get("client_visibility") {
        for field in [
            "status",
            "reason_code",
            "blocking_domain",
            "can_use",
            "endpoint_family",
            "client_token_ref",
            "source",
        ] {
            crate::cli_report::push_table_field(
                &mut output,
                &format!("client_visibility.{field}"),
                visibility.get(field),
            );
        }
    }
    if let Some(conflicts) = report.get("current_conflicts") {
        crate::cli_report::push_table_field(
            &mut output,
            "current_conflicts.has_conflicts",
            conflicts.get("has_conflicts"),
        );
        crate::cli_report::push_table_field(
            &mut output,
            "current_conflicts.entry_count",
            conflicts.get("entry_count"),
        );
        for entry in conflicts
            .get("entries")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            crate::cli_report::push_table_field(
                &mut output,
                "current_conflicts.entry",
                entry.get("code"),
            );
        }
    }
    output.push_str("configured_public_routes:\n");
    for route in report
        .get("configured_public_routes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        output.push_str(&format!(
            "- model={} targets={}\n",
            display_value(
                route
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
            ),
            route
                .get("target_count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        ));
    }
    output
}

fn safe_local_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > 200
        || trimmed.starts_with("sk-")
        || trimmed.chars().any(|ch| ch.is_control())
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn safe_reason_code_or_null(value: Option<&str>) -> Value {
    value
        .filter(|value| is_safe_reason_code(value))
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn is_safe_reason_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn safe_reference_label_value_or_redacted(value: Option<&str>) -> Value {
    match value.and_then(safe_reference_label_value) {
        Some(value) => Value::from(value),
        None if value.is_some() => Value::from("<redacted-reference>"),
        None => Value::Null,
    }
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

fn display_value(value: &str) -> String {
    crate::cli_report::escape_table_value(value)
}

#[cfg(test)]
mod tests {
    use axum::{
        routing::{get, post, put},
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    async fn spawn_management_fixture(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[test]
    fn models_onboard_plan_json_is_planning_only_and_redacted() {
        let options = super::ModelsOnboardPlanOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            channel_id: "relay-a".to_string(),
            public_model: "coding".to_string(),
            upstream_model: Some("vendor/coding".to_string()),
            client_token_ref: Some("operator-client".to_string()),
            endpoint_family: Some("chat_completions".to_string()),
            mode: super::ModelsOnboardPlanMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        };
        let model_routes = json!({
            "routes": [
                {
                    "model": "fast",
                    "strategy": "priority",
                    "targets": [
                        {
                            "channel_id": "relay-a",
                            "upstream_model": "vendor/fast",
                            "provider_kind": "relay",
                            "priority": 10,
                            "weight": 1,
                            "enabled": true,
                            "token_hash": "SHOULD_NOT_RENDER_TOKEN_HASH",
                            "health": {"kind": "ready", "reason_code": null}
                        }
                    ]
                }
            ],
            "raw_upstream_catalog": ["SHOULD_NOT_RENDER_LIVE_CATALOG"]
        });
        let channel = json!({
            "name": "relay-a",
            "credential_set_id": "relay-credentials",
            "provider_kind": "relay",
            "configured_enabled": true,
            "total_credentials": 2,
            "available_credentials": 1,
            "api_base": "SHOULD_NOT_RENDER_API_BASE",
            "auth_header": "SHOULD_NOT_RENDER_AUTH_HEADER",
            "fingerprint": "SHOULD_NOT_RENDER_FINGERPRINT",
            "endpoint_capabilities": {
                "chat_completions": "supported",
                "responses": "unknown",
                "embeddings": "unsupported",
                "models": "local_projection",
                "diagnostic_labels": ["relay", "line\nlabel"],
                "raw_probe": "SHOULD_NOT_RENDER_CAPABILITY_RAW_PROBE"
            },
            "health": {"kind": "ready", "reason_code": null}
        });
        let reload_diff = json!({
            "status": "ok",
            "reason_code": "reload_diff_available",
            "active_registry_generation": 7,
            "active_registry_version": 11,
            "staged_registry_version": 12,
            "runtime_reload_required": true,
            "resource_changes": [{"secret": "SHOULD_NOT_RENDER_RELOAD_RESOURCE_DETAIL"}]
        });
        let availability = json!({
            "status": "available",
            "can_use": true,
            "blocking_domain": "none",
            "reason_code": "available",
            "next_action": "none",
            "endpoint_family": "chat_completions",
            "model": "coding",
            "public_model": "coding",
            "client_token_ref": "operator-client",
            "route_kind": "configured",
            "registry_generation": 7,
            "evidence": {
                "client_token_ref_supplied": true,
                "client_token_known": true,
                "client_token_enabled": true,
                "model_allowed": true,
                "model_visible": true,
                "route_present": true,
                "selected_target_present": true,
                "route_target_count": 1,
                "endpoint_family_target_count": 1,
                "candidate_reason_codes": ["selected"]
            },
            "client_token": {
                "id": "operator-client",
                "name": "safe token",
                "enabled": true,
                "raw_token": "SHOULD_NOT_RENDER_CLIENT_TOKEN"
            }
        });

        let rendered = super::render_onboard_plan_report(
            &options,
            Some(&model_routes),
            Some(&channel),
            Some(&reload_diff),
            Some(&availability),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["reason_code"], "models_onboard_plan_projected");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["reads_management_runtime"], true);
        assert_eq!(report["effect_vector"]["reads_management_store"], false);
        assert_eq!(report["effect_vector"]["calls_upstream"], false);
        assert_eq!(report["effect_vector"]["writes_local_files"], false);
        assert_eq!(report["effect_vector"]["writes_management_store"], false);
        assert_eq!(report["effect_vector"]["mutates_runtime"], false);
        assert_eq!(report["scope"]["channel_id"], "relay-a");
        assert_eq!(report["scope"]["public_model"], "coding");
        assert_eq!(report["planning_only_no_visibility_change"], true);
        assert_eq!(report["client_visibility_changed"], false);
        assert_eq!(report["live_discovery_called"], false);
        assert_eq!(report["management_mutation_sent"], false);
        assert_eq!(report["data"]["credential_set_id"], "relay-credentials");
        assert_eq!(
            report["data"]["channel_projection"]["channel_id"],
            "relay-a"
        );
        assert_eq!(
            report["data"]["configured_public_routes"][0]["model"],
            "fast"
        );
        assert_eq!(report["data"]["proposed_route"]["public_model"], "coding");
        assert_eq!(
            report["data"]["proposed_route"]["upstream_model"],
            "vendor/coding"
        );
        assert_eq!(
            report["data"]["endpoint_capabilities"]["chat_completions"],
            "supported"
        );
        assert_eq!(report["data"]["endpoint_capability_status"], "available");
        assert_eq!(report["data"]["client_visibility"]["status"], "available");
        assert_eq!(report["data"]["client_visibility"]["can_use"], true);
        assert_eq!(
            report["data"]["client_visibility"]["evidence"]["endpoint_family_target_count"],
            1
        );
        assert_eq!(report["data"]["runtime_reload_required"], true);
        assert_eq!(report["data"]["reload_diff_status"], "ok");
        assert_eq!(
            report["data"]["reload_diff_reason_code"],
            "reload_diff_available"
        );
        assert_eq!(report["data"]["staged_registry_version"], 12);
        assert_eq!(
            report["data"]["required_followups"][0]["status"],
            "deferred_by_m1_m4"
        );
        assert!(!rendered.contains("SHOULD_NOT_RENDER_TOKEN_HASH"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_LIVE_CATALOG"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_API_BASE"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_AUTH_HEADER"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_FINGERPRINT"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_CAPABILITY_RAW_PROBE"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_RELOAD_RESOURCE_DETAIL"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN"));
    }

    #[test]
    fn models_onboard_plan_reports_unavailable_projection_without_live_work() {
        let options = super::ModelsOnboardPlanOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            channel_id: "relay-a".to_string(),
            public_model: "coding".to_string(),
            upstream_model: None,
            client_token_ref: None,
            endpoint_family: None,
            mode: super::ModelsOnboardPlanMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        };

        let rendered = super::render_onboard_plan_report(
            &options,
            None,
            None,
            None,
            None,
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["reason_code"], "onboard_plan_projection_unavailable");
        assert_eq!(report["planning_only_no_visibility_change"], true);
        assert_eq!(report["live_discovery_called"], false);
        assert_eq!(report["management_mutation_sent"], false);
        assert_eq!(
            report["data"]["client_visibility"]["status"],
            "not_evaluated"
        );
        assert_eq!(
            report["data"]["client_visibility"]["reason_code"],
            "missing_visibility_args"
        );
        assert_eq!(report["data"]["reload_diff_status"], "unknown");
        assert_eq!(report["next_action"]["repair_path"], "deferred_by_m1_m4");
    }

    #[test]
    fn models_onboard_plan_reports_current_conflicts_for_existing_public_and_channel_target() {
        let options = super::ModelsOnboardPlanOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            channel_id: "relay-a".to_string(),
            public_model: "coding".to_string(),
            upstream_model: Some("vendor/coding".to_string()),
            client_token_ref: None,
            endpoint_family: None,
            mode: super::ModelsOnboardPlanMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        };
        let model_routes = json!({
            "routes": [
                {
                    "model": "coding",
                    "strategy": "priority",
                    "targets": [
                        {
                            "channel_id": "relay-a",
                            "upstream_model": "vendor/coding",
                            "enabled": true
                        },
                        {
                            "channel_id": "relay-a",
                            "upstream_model": "vendor/other",
                            "enabled": true
                        }
                    ]
                },
                {
                    "model": "drafting",
                    "strategy": "priority",
                    "targets": [
                        {
                            "channel_id": "relay-a",
                            "upstream_model": "vendor/drafting",
                            "enabled": true
                        }
                    ]
                }
            ]
        });
        let channel = json!({
            "name": "relay-a",
            "credential_set_id": "relay-credentials",
            "provider_kind": "relay",
            "configured_enabled": true,
            "total_credentials": 2,
            "available_credentials": 1,
            "health": {"kind": "ready"}
        });

        let rendered = super::render_onboard_plan_report(
            &options,
            Some(&model_routes),
            Some(&channel),
            None,
            None,
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["data"]["current_conflicts"]["has_conflicts"], true);
        let codes = report["data"]["current_conflicts"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["code"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(codes.contains(&"public_model_already_configured"));
        assert!(codes.contains(&"same_target_already_present"));
        assert!(codes.contains(&"same_channel_different_upstream"));
        assert!(codes.contains(&"channel_used_by_other_public_models"));
        assert!(report["data"]["current_conflicts"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["public_model"] == "coding"));
        assert!(report["data"]["current_conflicts"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["upstream_model"] == "vendor/coding"));
        assert!(report["data"]["current_conflicts"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["other_public_model"] == "drafting"));
    }

    #[tokio::test]
    async fn models_onboard_plan_calls_only_readonly_runtime_projections() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let post_called = Arc::new(AtomicBool::new(false));
        let put_called = Arc::new(AtomicBool::new(false));
        let route_seen = Arc::clone(&seen_paths);
        let channel_seen = Arc::clone(&seen_paths);
        let reload_seen = Arc::clone(&seen_paths);
        let availability_seen = Arc::clone(&seen_paths);
        let post_seen = Arc::clone(&post_called);
        let put_seen = Arc::clone(&put_called);
        let router = Router::new()
            .route(
                "/management/model-routes",
                get(move || {
                    let route_seen = Arc::clone(&route_seen);
                    async move {
                        route_seen
                            .lock()
                            .unwrap()
                            .push("/management/model-routes".to_string());
                        Json(json!({
                            "routes": [{
                                "model": "fast",
                                "strategy": "priority",
                                "targets": [{
                                    "channel_id": "relay-a",
                                    "upstream_model": "vendor/fast",
                                    "enabled": true,
                                    "health": {"kind": "ready"}
                                }]
                            }]
                        }))
                    }
                }),
            )
            .route(
                "/management/channels/relay-a",
                get(move || {
                    let channel_seen = Arc::clone(&channel_seen);
                    async move {
                        channel_seen
                            .lock()
                            .unwrap()
                            .push("/management/channels/relay-a".to_string());
                        Json(json!({
                            "name": "relay-a",
                            "credential_set_id": "relay-credentials",
                            "provider_kind": "relay",
                            "configured_enabled": true,
                            "total_credentials": 2,
                            "available_credentials": 1,
                            "health": {"kind": "ready"}
                        }))
                    }
                }),
            )
            .route(
                "/management/runtime/reload-diff",
                get(move || {
                    let reload_seen = Arc::clone(&reload_seen);
                    async move {
                        reload_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime/reload-diff".to_string());
                        Json(json!({
                            "status": "ok",
                            "reason_code": "reload_diff_empty",
                            "active_registry_generation": 3,
                            "active_registry_version": 4,
                            "staged_registry_version": 4,
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
                        Json(json!({
                            "status": "unavailable",
                            "can_use": false,
                            "blocking_domain": "model",
                            "reason_code": "model_not_visible",
                            "next_action": "inspect_model_route",
                            "endpoint_family": "chat_completions",
                            "model": "coding",
                            "public_model": "coding",
                            "client_token_ref": "operator-client",
                            "route_kind": "configured",
                            "registry_generation": 3,
                            "evidence": {
                                "client_token_ref_supplied": true,
                                "client_token_known": true,
                                "client_token_enabled": true,
                                "model_allowed": true,
                                "model_visible": false,
                                "route_present": false,
                                "selected_target_present": false,
                                "route_target_count": 0,
                                "endpoint_family_target_count": 0,
                                "candidate_reason_codes": ["no_route"]
                            }
                        }))
                    }
                }),
            )
            .route(
                "/management/model-discovery/sync-plan",
                post(move |Json(_body): Json<Value>| {
                    let post_seen = Arc::clone(&post_seen);
                    async move {
                        post_seen.store(true, Ordering::SeqCst);
                        Json(json!({"unexpected": true}))
                    }
                }),
            )
            .route(
                "/management/runtime/reload",
                put(move |Json(_body): Json<Value>| {
                    let put_seen = Arc::clone(&put_seen);
                    async move {
                        put_seen.store(true, Ordering::SeqCst);
                        Json(json!({"unexpected": true}))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!("ONE_AI_KEY_TEST_ONBOARD_PLAN_TOKEN_{}", std::process::id());
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::ModelsOnboardPlanOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            channel_id: "relay-a".to_string(),
            public_model: "coding".to_string(),
            upstream_model: Some("vendor/coding".to_string()),
            client_token_ref: Some("operator-client".to_string()),
            endpoint_family: Some("chat_completions".to_string()),
            mode: super::ModelsOnboardPlanMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["live_discovery_called"], false);
        assert_eq!(report["management_mutation_sent"], false);
        assert_eq!(report["data"]["client_visibility"]["status"], "unavailable");
        assert_eq!(
            report["data"]["reload_diff_reason_code"],
            "reload_diff_empty"
        );
        assert!(!post_called.load(Ordering::SeqCst));
        assert!(!put_called.load(Ordering::SeqCst));
        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/model-routes".to_string(),
                "/management/channels/relay-a".to_string(),
                "/management/runtime/reload-diff".to_string(),
                "/management/model-availability".to_string(),
            ]
        );
    }
}
