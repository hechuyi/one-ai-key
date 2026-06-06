use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsOnboardPlanOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub channel_id: String,
    pub public_model: String,
    pub upstream_model: Option<String>,
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
    Ok(render_onboard_plan_report(
        &options,
        Some(&model_routes),
        Some(&channel),
        options.output,
    ))
}

pub fn render_onboard_plan_report(
    options: &ModelsOnboardPlanOptions,
    model_routes: Option<&Value>,
    channel: Option<&Value>,
    output: crate::cli_report::OutputFormat,
) -> String {
    let report = sanitized_onboard_plan_report(options, model_routes, channel);
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
                "channel_projection": Value::Null,
                "configured_public_routes": [],
                "currently_exposed_public_routes": [],
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
            "channel_projection": sanitize_channel(channel),
            "configured_public_routes": configured_public_routes,
            "currently_exposed_public_routes": model_routes_for_channel(model_routes, &options.channel_id),
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
        "planning_only_no_visibility_change",
        "client_visibility_changed",
        "live_discovery_called",
        "management_mutation_sent",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
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

fn display_value(value: &str) -> String {
    crate::cli_report::escape_table_value(value)
}

#[cfg(test)]
mod tests {
    use axum::{
        routing::{get, post},
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
            "health": {"kind": "ready", "reason_code": null}
        });

        let rendered = super::render_onboard_plan_report(
            &options,
            Some(&model_routes),
            Some(&channel),
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
            report["data"]["required_followups"][0]["status"],
            "deferred_by_m1_m4"
        );
        assert!(!rendered.contains("SHOULD_NOT_RENDER_TOKEN_HASH"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_LIVE_CATALOG"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_API_BASE"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_AUTH_HEADER"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_FINGERPRINT"));
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
            mode: super::ModelsOnboardPlanMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        };

        let rendered = super::render_onboard_plan_report(
            &options,
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
        assert_eq!(report["next_action"]["repair_path"], "deferred_by_m1_m4");
    }

    #[tokio::test]
    async fn models_onboard_plan_calls_only_readonly_runtime_projections() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let post_called = Arc::new(AtomicBool::new(false));
        let route_seen = Arc::clone(&seen_paths);
        let channel_seen = Arc::clone(&seen_paths);
        let post_seen = Arc::clone(&post_called);
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
                "/management/model-discovery/sync-plan",
                post(move |Json(_body): Json<Value>| {
                    let post_seen = Arc::clone(&post_seen);
                    async move {
                        post_seen.store(true, Ordering::SeqCst);
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
        assert!(!post_called.load(Ordering::SeqCst));
        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/model-routes".to_string(),
                "/management/channels/relay-a".to_string(),
            ]
        );
    }
}
