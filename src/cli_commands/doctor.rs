use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub include_alerts: bool,
    pub include_events: bool,
    pub include_routes: bool,
    pub event_limit: usize,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorProjection {
    name: &'static str,
    endpoint: crate::operator_client::ReadOnlyEndpoint,
}

#[derive(Debug, Clone)]
struct ProjectionResult {
    name: &'static str,
    value: Result<Value, crate::operator_client::OperatorClientError>,
}

pub async fn run(
    options: DoctorOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let mut results = Vec::new();
    for projection in doctor_projections(&options) {
        results.push(ProjectionResult {
            name: projection.name,
            value: client.get_json(projection.endpoint).await,
        });
    }
    Ok(render_doctor_report(&results, &options))
}

fn doctor_projections(options: &DoctorOptions) -> Vec<DoctorProjection> {
    let mut projections = default_doctor_projections();
    if options.include_alerts {
        projections.push(DoctorProjection {
            name: "alerts",
            endpoint: crate::operator_client::ReadOnlyEndpoint::Alerts,
        });
    }
    if options.include_events {
        projections.push(DoctorProjection {
            name: "events",
            endpoint: crate::operator_client::ReadOnlyEndpoint::Events {
                offset: Some(0),
                limit: Some(options.event_limit.clamp(1, 50)),
            },
        });
    }
    if options.include_routes {
        projections.push(DoctorProjection {
            name: "routes",
            endpoint: crate::operator_client::ReadOnlyEndpoint::ModelRoutes,
        });
    }
    projections
}

fn default_doctor_projections() -> Vec<DoctorProjection> {
    vec![
        DoctorProjection {
            name: "explain_runtime",
            endpoint: crate::operator_client::ReadOnlyEndpoint::ExplainRuntime,
        },
        DoctorProjection {
            name: "runtime",
            endpoint: crate::operator_client::ReadOnlyEndpoint::Runtime,
        },
        DoctorProjection {
            name: "serving_health",
            endpoint: crate::operator_client::ReadOnlyEndpoint::ServingHealth,
        },
        DoctorProjection {
            name: "resilience_health",
            endpoint: crate::operator_client::ReadOnlyEndpoint::ResilienceHealth,
        },
    ]
}

fn render_doctor_report(results: &[ProjectionResult], options: &DoctorOptions) -> String {
    let report = sanitized_doctor_report(results, options);
    match options.output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(&report).expect("doctor json report should serialize")
        }
        crate::cli_report::OutputFormat::Table => render_doctor_table_from_report(&report),
    }
}

fn sanitized_doctor_report(results: &[ProjectionResult], options: &DoctorOptions) -> Value {
    let failures = results
        .iter()
        .filter_map(|result| {
            result.value.as_ref().err().map(|error| {
                serde_json::json!({
                    "stage": "management_projection",
                    "projection": result.name,
                    "status": "unavailable",
                    "failure_class": "management_projection_unavailable",
                    "router_action": "none",
                    "retry_eligibility": "not_applicable",
                    "retry_blocked_reason": Value::Null,
                    "client_visible_status": "not_applicable",
                    "reason_code": stable_management_error_reason(error.reason_code()),
                    "next_action": partial_failure_next_action(result.name),
                })
            })
        })
        .collect::<Vec<_>>();
    let runtime = projection_value(results, "runtime");
    let explain_runtime = projection_value(results, "explain_runtime");
    let serving_health = projection_value(results, "serving_health");
    let resilience_health = projection_value(results, "resilience_health");
    let alerts = projection_value(results, "alerts");
    let events = projection_value(results, "events");
    let routes = projection_value(results, "routes");
    let runtime_summary = summarize_runtime(runtime, explain_runtime);
    let serving_summary = summarize_serving(serving_health);
    let resilience_summary = summarize_resilience(resilience_health);
    let reload_required = summarize_reload_required(runtime, explain_runtime);
    let bounded_status = bounded_status(options, alerts, events, routes);
    let status = doctor_status(&failures, &serving_summary, &resilience_summary);
    let reason_code = doctor_reason_code(status, &failures, &serving_summary, &resilience_summary);
    let event_evidence_included = options.include_events && events.is_some();
    let effect = doctor_effect(options);

    serde_json::json!({
        "status": status,
        "reason": doctor_reason(status, reason_code),
        "reason_code": reason_code,
        "side_effect_class": crate::cli_effects::side_effect_class_code(effect.side_effect_class),
        "effect_vector": crate::cli_effects::effect_vector_json(effect.effect_vector),
        "scope": {
            "default_runtime_only": !options.include_alerts && !options.include_events && !options.include_routes,
            "include_alerts": options.include_alerts,
            "include_events": options.include_events,
            "include_routes": options.include_routes,
            "event_limit": options.event_limit.clamp(1, 50),
        },
        "window": doctor_window(options, events),
        "next_action": doctor_next_action(status, event_evidence_included),
        "data": {
            "command": "doctor",
            "runtime": runtime_summary,
            "serving": serving_summary,
            "resilience": resilience_summary,
            "reload_required": reload_required,
            "bounded_status": bounded_status,
            "partial_status": {
                "failed_projection_count": failures.len(),
                "failures": failures,
            },
        },
    })
}

fn projection_value<'a>(results: &'a [ProjectionResult], name: &str) -> Option<&'a Value> {
    results
        .iter()
        .find(|result| result.name == name)
        .and_then(|result| result.value.as_ref().ok())
}

fn doctor_status(failures: &[Value], serving: &Value, resilience: &Value) -> &'static str {
    if status_field(serving).is_some_and(is_unhealthy_status)
        || status_field(resilience).is_some_and(is_unhealthy_status)
    {
        "degraded"
    } else if !failures.is_empty() {
        "partial"
    } else {
        "ok"
    }
}

fn doctor_reason_code(
    status: &str,
    failures: &[Value],
    serving: &Value,
    resilience: &Value,
) -> &'static str {
    match status {
        "degraded" if status_field(serving).is_some_and(is_unhealthy_status) => {
            "serving_health_degraded"
        }
        "degraded" if status_field(resilience).is_some_and(is_unhealthy_status) => {
            "resilience_health_degraded"
        }
        "partial" => "management_projection_partial",
        _ => {
            let _ = failures;
            "runtime_health_available"
        }
    }
}

fn doctor_reason(_status: &str, reason_code: &str) -> &'static str {
    match reason_code {
        "management_projection_partial" => {
            "Doctor completed with at least one unavailable management projection."
        }
        "serving_health_degraded" => "Serving health reports a non-ready status.",
        "resilience_health_degraded" => "Resilience health reports operator-visible degradation.",
        _ => "Runtime, serving, and resilience projections are available.",
    }
}

fn doctor_effect(options: &DoctorOptions) -> crate::cli_effects::CommandEffect {
    let mut effect = crate::cli_effects::runtime_readonly_effect();
    if options.include_alerts || options.include_events {
        effect.effect_vector.reads_management_store = true;
    }
    effect
}

fn summarize_runtime(runtime: Option<&Value>, explain_runtime: Option<&Value>) -> Value {
    serde_json::json!({
        "status": if runtime.is_some() || explain_runtime.is_some() { "available" } else { "unavailable" },
        "runtime_generation": first_u64(&[
            runtime.and_then(|value| value.get("runtime_generation")),
            runtime.and_then(|value| value.get("generation")),
            explain_runtime.and_then(|value| value.get("runtime_generation")),
            explain_runtime.and_then(|value| value.get("generation")),
        ]),
        "model_route_count": first_u64(&[
            runtime.and_then(|value| value.get("model_route_count")),
            explain_runtime.and_then(|value| value.get("model_route_count")),
        ]),
        "credential_set_count": first_u64(&[
            runtime.and_then(|value| value.get("credential_set_count")),
            explain_runtime.and_then(|value| value.get("credential_set_count")),
        ]),
    })
}

fn summarize_serving(serving: Option<&Value>) -> Value {
    let status = projection_status(serving).unwrap_or("unavailable");
    serde_json::json!({
        "status": status,
        "reason_code": projection_reason_code(serving),
        "accepting_requests": serving
            .and_then(|value| value.get("accepting_requests"))
            .and_then(Value::as_bool),
        "needs_operator_input": serving
            .and_then(|value| value.get("needs_operator_input"))
            .and_then(Value::as_bool),
        "operator_input_alerts": serving
            .and_then(|value| value.get("operator_input_alerts"))
            .and_then(Value::as_u64),
    })
}

fn summarize_resilience(resilience: Option<&Value>) -> Value {
    let status = projection_status(resilience).unwrap_or("unavailable");
    serde_json::json!({
        "status": status,
        "reason_code": projection_reason_code(resilience),
        "needs_operator_input": resilience
            .and_then(|value| value.get("needs_operator_input"))
            .and_then(Value::as_bool),
        "operator_input_alerts": resilience
            .and_then(|value| value.get("operator_input_alerts"))
            .and_then(Value::as_u64),
        "cooling_down": resilience
            .and_then(|value| value.get("cooling_down"))
            .and_then(Value::as_u64),
        "exhausted": resilience
            .and_then(|value| value.get("exhausted"))
            .and_then(Value::as_u64),
    })
}

fn summarize_reload_required(runtime: Option<&Value>, explain_runtime: Option<&Value>) -> Value {
    let required = first_bool(&[
        runtime.and_then(|value| value.get("reload_required")),
        runtime.and_then(|value| {
            value
                .get("reload")
                .and_then(|reload| reload.get("required"))
        }),
        explain_runtime.and_then(|value| value.get("reload_required")),
        explain_runtime.and_then(|value| {
            value
                .get("reload")
                .and_then(|reload| reload.get("required"))
        }),
    ]);
    let reload_diff_status = match required {
        Some(true) => "available",
        Some(false) => "not_required",
        None => "unknown",
    };
    serde_json::json!({
        "status": if required.is_some() { "available" } else { "unknown_until_reload_projection" },
        "required": required,
        "reload_diff_status": reload_diff_status,
    })
}

fn bounded_status(
    options: &DoctorOptions,
    alerts: Option<&Value>,
    events: Option<&Value>,
    routes: Option<&Value>,
) -> Value {
    serde_json::json!({
        "alerts": opt_in_alerts_status(options.include_alerts, alerts),
        "events": opt_in_events_status(options.include_events, events, options.event_limit.clamp(1, 50)),
        "routes": opt_in_routes_status(options.include_routes, routes),
        "events_window": if options.include_events {
            serde_json::json!({"offset": 0, "limit": options.event_limit.clamp(1, 50), "status": "bounded"})
        } else {
            Value::Null
        },
    })
}

fn opt_in_alerts_status(requested: bool, value: Option<&Value>) -> Value {
    if !requested {
        return serde_json::json!({"status": "not_requested"});
    }
    serde_json::json!({
        "status": if value.is_some() { "available_summary" } else { "unavailable" },
        "source": "store_backed_projection",
        "completeness": "summary_only_payload_omitted",
        "total_alerts": value
            .and_then(|value| value.get("total_alerts"))
            .and_then(Value::as_u64),
        "payload": "omitted",
    })
}

fn opt_in_events_status(requested: bool, value: Option<&Value>, limit: usize) -> Value {
    if !requested {
        return serde_json::json!({"status": "not_requested"});
    }
    let returned = value
        .and_then(|value| value.get("events"))
        .and_then(Value::as_array)
        .map(Vec::len);
    serde_json::json!({
        "status": if value.is_some() { "available_bounded" } else { "unavailable" },
        "source": "store_backed_bounded_projection",
        "completeness": "latest_window",
        "offset": 0,
        "limit": limit,
        "returned": returned,
        "truncated": Value::Null,
        "payload": "omitted",
    })
}

fn opt_in_routes_status(requested: bool, value: Option<&Value>) -> Value {
    if !requested {
        return serde_json::json!({"status": "not_requested"});
    }
    let returned = value
        .and_then(|value| value.get("routes"))
        .and_then(Value::as_array)
        .map(Vec::len);
    serde_json::json!({
        "status": if value.is_some() { "available_complete" } else { "unavailable" },
        "source": "runtime_projection",
        "completeness": "complete_runtime_projection",
        "returned": returned,
        "truncated": false,
        "payload": "omitted",
    })
}

fn doctor_next_action(status: &str, event_evidence_included: bool) -> Value {
    if !event_evidence_included {
        return serde_json::json!({
            "summary": "Doctor did not include event evidence. Inspect bounded recent failures before diagnosing an incident.",
            "template_id": "failures_tail",
            "safe_argv": ["one-ai-key", "failures", "tail", "--last", "20"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        });
    }
    serde_json::json!({
        "summary": if status == "ok" {
            "Runtime projections are available and bounded event evidence was included."
        } else {
            "Review the partial or degraded projections and bounded event evidence."
        },
        "template_id": if status == "ok" { "no_action_required" } else { "review_doctor_projection" },
        "safe_argv": [],
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    })
}

fn partial_failure_next_action(_projection: &str) -> Value {
    serde_json::json!({
        "summary": "A management projection was unavailable. The report omits its body and keeps token material redacted.",
        "template_id": "failures_tail",
        "safe_argv": ["one-ai-key", "failures", "tail", "--last", "20"],
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    })
}

fn doctor_window(options: &DoctorOptions, events: Option<&Value>) -> Value {
    if !options.include_events {
        return Value::Null;
    }
    let limit = options.event_limit.clamp(1, 50);
    let returned = events
        .and_then(|value| value.get("events"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    serde_json::json!({
        "kind": "bounded_management_events",
        "source": ["events"],
        "limit": limit,
        "returned": returned,
        "truncated": Value::Null,
        "cursor": Value::Null,
        "bounded_reason": "opt_in_latest_window",
    })
}

fn render_doctor_table_from_report(report: &Value) -> String {
    let mut output = String::new();
    let data = report.get("data");
    output.push_str("Doctor report\n");
    push_field(&mut output, "status", report.get("status"));
    push_field(&mut output, "reason_code", report.get("reason_code"));
    push_field(&mut output, "reason", report.get("reason"));
    push_field(
        &mut output,
        "side_effect_class",
        report.get("side_effect_class"),
    );
    if let Some(scope) = report.get("scope") {
        for field in [
            "default_runtime_only",
            "include_alerts",
            "include_events",
            "include_routes",
            "event_limit",
        ] {
            push_field(&mut output, &format!("scope.{field}"), scope.get(field));
        }
    }
    if let Some(window) = report.get("window").filter(|value| !value.is_null()) {
        for field in ["kind", "limit", "returned", "truncated", "bounded_reason"] {
            push_field(&mut output, &format!("window.{field}"), window.get(field));
        }
    }
    if let Some(effect) = report.get("effect_vector") {
        for field in [
            "reads_management_runtime",
            "reads_management_store",
            "writes_management_store",
            "calls_upstream",
            "mutates_runtime",
        ] {
            push_field(&mut output, &format!("effect.{field}"), effect.get(field));
        }
    }
    push_field(
        &mut output,
        "runtime_generation",
        data.and_then(|data| data.get("runtime"))
            .and_then(|runtime| runtime.get("runtime_generation")),
    );
    push_field(
        &mut output,
        "serving.status",
        data.and_then(|data| data.get("serving"))
            .and_then(|serving| serving.get("status")),
    );
    push_field(
        &mut output,
        "serving.reason_code",
        data.and_then(|data| data.get("serving"))
            .and_then(|serving| serving.get("reason_code")),
    );
    push_field(
        &mut output,
        "resilience.status",
        data.and_then(|data| data.get("resilience"))
            .and_then(|resilience| resilience.get("status")),
    );
    push_field(
        &mut output,
        "resilience.reason_code",
        data.and_then(|data| data.get("resilience"))
            .and_then(|resilience| resilience.get("reason_code")),
    );
    push_field(
        &mut output,
        "reload_required",
        data.and_then(|data| data.get("reload_required"))
            .and_then(|reload| reload.get("required")),
    );
    push_field(
        &mut output,
        "reload_diff_status",
        data.and_then(|data| data.get("reload_required"))
            .and_then(|reload| reload.get("reload_diff_status")),
    );
    if let Some(bounded) = data.and_then(|data| data.get("bounded_status")) {
        for field in ["alerts", "events", "routes"] {
            push_field(
                &mut output,
                &format!("{field}.status"),
                bounded.get(field).and_then(|value| value.get("status")),
            );
            push_field(
                &mut output,
                &format!("{field}.completeness"),
                bounded
                    .get(field)
                    .and_then(|value| value.get("completeness")),
            );
            push_field(
                &mut output,
                &format!("{field}.returned"),
                bounded.get(field).and_then(|value| value.get("returned")),
            );
        }
    }
    if let Some(failures) = report
        .get("data")
        .and_then(|data| data.get("partial_status"))
        .and_then(|partial| partial.get("failures"))
        .and_then(Value::as_array)
    {
        output.push_str("partial_failures:\n");
        for failure in failures {
            let projection = failure
                .get("projection")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let reason_code = failure
                .get("reason_code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            output.push_str(&format!(
                "- projection={} reason_code={}\n",
                display_value(projection),
                display_value(reason_code)
            ));
        }
    }
    if let Some(summary) = report
        .get("next_action")
        .and_then(|next_action| next_action.get("summary"))
        .and_then(Value::as_str)
    {
        output.push_str(&format!("next_action: {}\n", display_value(summary)));
    }
    if let Some(template_id) = report
        .get("next_action")
        .and_then(|next_action| next_action.get("template_id"))
        .and_then(Value::as_str)
    {
        output.push_str(&format!(
            "next_action.template_id: {}\n",
            display_value(template_id)
        ));
    }
    if let Some(argv) = report
        .get("next_action")
        .and_then(|next_action| next_action.get("safe_argv"))
        .and_then(Value::as_array)
    {
        if argv.is_empty() {
            output.push_str("next_action.safe_argv: []\n");
        } else {
            for (index, arg) in argv.iter().enumerate() {
                push_field(
                    &mut output,
                    &format!("next_action.safe_argv[{index}]"),
                    Some(arg),
                );
            }
        }
    }
    output
}

fn push_field(output: &mut String, name: &str, value: Option<&Value>) {
    let rendered = match value {
        Some(Value::String(value)) => display_value(value),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Null) | None => "unknown".to_string(),
        Some(other) => display_value(&other.to_string()),
    };
    output.push_str(&format!("{name}: {rendered}\n"));
}

fn projection_status(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .and_then(stable_projection_status)
}

fn projection_reason_code(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(|value| value.get("reason_code"))
        .and_then(Value::as_str)
        .and_then(stable_projection_reason_code)
}

fn is_unhealthy_status(status: &str) -> bool {
    matches!(
        status,
        "degraded" | "blocked" | "unavailable" | "not_ready" | "error"
    )
}

fn status_field(value: &Value) -> Option<&str> {
    value.get("status").and_then(Value::as_str)
}

fn stable_projection_status(value: &str) -> Option<&'static str> {
    match value {
        "ok" | "ready" | "healthy" | "serving" => Some("ok"),
        "degraded" => Some("degraded"),
        "blocked" => Some("blocked"),
        "unavailable" => Some("unavailable"),
        "not_ready" => Some("not_ready"),
        "error" => Some("error"),
        "partial" => Some("partial"),
        _ => None,
    }
}

fn stable_projection_reason_code(value: &str) -> Option<&'static str> {
    match value {
        "serving" => Some("serving"),
        "healthy" => Some("healthy"),
        "ready" => Some("ready"),
        "runtime_health_available" => Some("runtime_health_available"),
        "serving_health_degraded" => Some("serving_health_degraded"),
        "resilience_health_degraded" => Some("resilience_health_degraded"),
        "credential_set_transition_required" => Some("credential_set_transition_required"),
        "operator_input_required" => Some("operator_input_required"),
        "management_projection_partial" => Some("management_projection_partial"),
        _ => Some("unknown"),
    }
}

fn stable_management_error_reason(value: &str) -> &'static str {
    match value {
        "management_token_missing" => "management_token_missing",
        "management_unauthorized" => "management_unauthorized",
        "management_forbidden" => "management_forbidden",
        "management_not_found" => "management_not_found",
        "management_timeout" => "management_timeout",
        "management_transport_error" => "management_transport_error",
        "management_non_json_error" => "management_non_json_error",
        "management_http_error" => "management_http_error",
        "management_malformed_json" => "management_malformed_json",
        "management_body_error" => "management_body_error",
        _ => "management_projection_unavailable",
    }
}

fn first_u64(values: &[Option<&Value>]) -> Option<u64> {
    values
        .iter()
        .find_map(|value| value.and_then(Value::as_u64))
}

fn first_bool(values: &[Option<&Value>]) -> Option<bool> {
    values
        .iter()
        .find_map(|value| value.and_then(Value::as_bool))
}

fn display_value(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| ch.escape_default())
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use serde_json::Value;

    fn options() -> super::DoctorOptions {
        super::DoctorOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example/v1".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            include_alerts: false,
            include_events: false,
            include_routes: false,
            event_limit: 20,
            output: crate::cli_report::OutputFormat::Json,
        }
    }

    #[test]
    fn doctor_cli_default_endpoint_list_is_runtime_only() {
        let options = options();
        let paths = super::doctor_projections(&options)
            .into_iter()
            .map(|projection| projection.endpoint.test_request_parts().unwrap().0)
            .collect::<Vec<_>>();

        assert_eq!(
            paths,
            vec![
                "/management/explain/runtime",
                "/management/runtime",
                "/management/health/serving",
                "/management/health/resilience",
            ]
        );
        assert!(!paths.contains(&"/management/alerts".to_string()));
        assert!(!paths.contains(&"/management/events".to_string()));
        assert!(!paths.contains(&"/management/model-routes".to_string()));
    }

    #[test]
    fn doctor_cli_alerts_events_and_routes_are_opt_in_and_bounded() {
        let mut options = options();
        options.include_alerts = true;
        options.include_events = true;
        options.include_routes = true;
        options.event_limit = 500;

        let parts = super::doctor_projections(&options)
            .into_iter()
            .map(|projection| {
                (
                    projection.name,
                    projection.endpoint.test_request_parts().unwrap(),
                )
            })
            .collect::<Vec<_>>();

        assert!(parts
            .iter()
            .any(|(name, (path, _))| *name == "alerts" && path == "/management/alerts"));
        assert!(parts.iter().any(|(name, (path, query))| {
            *name == "events"
                && path == "/management/events"
                && query
                    == &vec![
                        ("offset".to_string(), "0".to_string()),
                        ("limit".to_string(), "50".to_string()),
                    ]
        }));
        assert!(parts
            .iter()
            .any(|(name, (path, _))| *name == "routes" && path == "/management/model-routes"));
    }

    #[test]
    fn doctor_cli_opt_in_projection_payloads_are_summarized() {
        let mut options = options();
        options.include_alerts = true;
        options.include_events = true;
        options.include_routes = true;
        options.event_limit = 3;
        let results = vec![
            super::ProjectionResult {
                name: "explain_runtime",
                value: Ok(json!({"runtime_generation": 2})),
            },
            super::ProjectionResult {
                name: "runtime",
                value: Ok(json!({"generation": 2})),
            },
            super::ProjectionResult {
                name: "serving_health",
                value: Ok(json!({"status": "ok", "reason_code": "serving"})),
            },
            super::ProjectionResult {
                name: "resilience_health",
                value: Ok(json!({"status": "ok", "reason_code": "healthy"})),
            },
            super::ProjectionResult {
                name: "alerts",
                value: Ok(json!({
                    "total_alerts": 1,
                    "alerts": [{"message": "secret alert payload"}],
                })),
            },
            super::ProjectionResult {
                name: "events",
                value: Ok(json!({
                    "events": [{"detail": "secret event payload"}, {"detail": "other"}],
                })),
            },
            super::ProjectionResult {
                name: "routes",
                value: Ok(json!({
                    "routes": [{
                        "model": "hidden-model-name",
                        "targets": [{"credential_id": "secret-derived-id"}],
                    }],
                })),
            },
        ];

        let rendered = super::render_doctor_report(&results, &options);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            report["data"]["bounded_status"]["alerts"]["completeness"],
            "summary_only_payload_omitted"
        );
        assert_eq!(
            report["data"]["bounded_status"]["events"]["completeness"],
            "latest_window"
        );
        assert_eq!(report["data"]["bounded_status"]["events"]["limit"], 3);
        assert_eq!(report["data"]["bounded_status"]["events"]["returned"], 2);
        assert_eq!(
            report["data"]["bounded_status"]["routes"]["completeness"],
            "complete_runtime_projection"
        );
        assert_eq!(report["data"]["bounded_status"]["routes"]["returned"], 1);
        assert!(!rendered.contains("secret alert payload"));
        assert!(!rendered.contains("secret event payload"));
        assert!(!rendered.contains("hidden-model-name"));
        assert!(!rendered.contains("secret-derived-id"));
    }

    #[test]
    fn doctor_cli_report_redacts_untrusted_and_secret_like_fields() {
        let options = options();
        let results = vec![
            super::ProjectionResult {
                name: "explain_runtime",
                value: Ok(json!({
                    "runtime_generation": 9,
                    "raw_token": "secret-token-material",
                    "upstream_error": "provider body should not appear"
                })),
            },
            super::ProjectionResult {
                name: "runtime",
                value: Ok(json!({
                    "generation": 9,
                    "reload_required": true,
                    "management_token": "secret-management-token"
                })),
            },
            super::ProjectionResult {
                name: "serving_health",
                value: Ok(json!({
                    "status": "ok",
                    "reason_code": "serving",
                    "accepting_requests": true,
                    "credential_id": "secret-derived-id"
                })),
            },
            super::ProjectionResult {
                name: "resilience_health",
                value: Ok(json!({
                    "status": "ok",
                    "reason_code": "healthy",
                    "operator_input_alerts": 0,
                    "fingerprint": "secret-fingerprint"
                })),
            },
        ];

        let rendered = super::render_doctor_report(&results, &options);

        assert!(rendered.contains("\"status\": \"ok\""));
        assert!(rendered.contains("\"reason_code\": \"runtime_health_available\""));
        assert!(rendered.contains("\"side_effect_class\": \"runtime_readonly\""));
        assert!(rendered.contains("\"effect_vector\""));
        assert!(rendered.contains("\"scope\""));
        assert!(rendered.contains("\"data\""));
        assert!(rendered.contains("\"runtime_generation\": 9"));
        assert!(rendered.contains("\"required\": true"));
        assert!(rendered.contains("\"safe_argv\""));
        assert!(rendered.contains("\"failures\""));
        let legacy_command_field = ["safe", "command"].join("_");
        assert!(!rendered.contains(&legacy_command_field));
        assert!(!rendered.contains("secret-token-material"));
        assert!(!rendered.contains("secret-management-token"));
        assert!(!rendered.contains("secret-derived-id"));
        assert!(!rendered.contains("secret-fingerprint"));
        assert!(!rendered.contains("provider body should not appear"));
    }

    #[test]
    fn doctor_cli_reload_required_summary_does_not_suggest_apply() {
        let options = options();
        let results = vec![
            super::ProjectionResult {
                name: "explain_runtime",
                value: Ok(json!({
                    "runtime_generation": 9,
                    "reload_required": true
                })),
            },
            super::ProjectionResult {
                name: "runtime",
                value: Ok(json!({
                    "generation": 9,
                    "reload_required": true
                })),
            },
            super::ProjectionResult {
                name: "serving_health",
                value: Ok(json!({"status": "ok", "reason_code": "serving"})),
            },
            super::ProjectionResult {
                name: "resilience_health",
                value: Ok(json!({"status": "ok", "reason_code": "healthy"})),
            },
        ];

        let rendered = super::render_doctor_report(&results, &options);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["data"]["reload_required"]["required"], true);
        assert_ne!(
            report["data"]["reload_required"]["reload_diff_status"],
            "unavailable_until_m4"
        );
        assert!(!rendered.contains("reload apply"));
        assert!(!rendered.contains("\"apply\""));
        assert!(!rendered.contains("--yes"));
        assert!(!rendered.contains("unavailable_until_m4"));
    }

    #[test]
    fn doctor_cli_partial_failure_uses_reason_code_without_error_body_or_token() {
        let options = options();
        let results = vec![
            super::ProjectionResult {
                name: "explain_runtime",
                value: Ok(json!({"runtime_generation": 2})),
            },
            super::ProjectionResult {
                name: "runtime",
                value: Err(crate::operator_client::OperatorClientError::new(
                    "management_unauthorized",
                    "management API rejected credential material",
                )),
            },
            super::ProjectionResult {
                name: "serving_health",
                value: Ok(json!({"status": "ok", "reason_code": "serving"})),
            },
            super::ProjectionResult {
                name: "resilience_health",
                value: Ok(json!({"status": "ok", "reason_code": "healthy"})),
            },
        ];

        let rendered = super::render_doctor_report(&results, &options);

        assert!(rendered.contains("\"status\": \"partial\""));
        assert!(rendered.contains("\"reason_code\": \"management_projection_partial\""));
        assert!(rendered.contains("\"stage\": \"management_projection\""));
        assert!(rendered.contains("\"projection\": \"runtime\""));
        assert!(rendered.contains("\"reason_code\": \"management_unauthorized\""));
        assert!(rendered.contains("\"safe_argv\""));
        assert!(!rendered.contains("management API rejected credential"));
    }

    #[test]
    fn doctor_cli_next_action_uses_safe_argv_not_shell_string() {
        let options = options();
        let results = vec![
            super::ProjectionResult {
                name: "explain_runtime",
                value: Ok(json!({"runtime_generation": 2})),
            },
            super::ProjectionResult {
                name: "runtime",
                value: Ok(json!({"generation": 2})),
            },
            super::ProjectionResult {
                name: "serving_health",
                value: Ok(json!({"status": "ok", "reason_code": "serving"})),
            },
            super::ProjectionResult {
                name: "resilience_health",
                value: Ok(json!({"status": "ok", "reason_code": "healthy"})),
            },
        ];

        let rendered = super::render_doctor_report(&results, &options);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["next_action"]["template_id"], "failures_tail");
        assert_eq!(
            report["next_action"]["safe_argv"],
            json!(["one-ai-key", "failures", "tail", "--last", "20"])
        );
        let legacy_command_field = ["safe", "command"].join("_");
        assert!(report["next_action"].get(&legacy_command_field).is_none());
    }

    #[test]
    fn doctor_cli_malformed_health_projection_is_not_ok() {
        let options = options();
        let results = vec![
            super::ProjectionResult {
                name: "explain_runtime",
                value: Ok(json!({"runtime_generation": 2})),
            },
            super::ProjectionResult {
                name: "runtime",
                value: Ok(json!({"generation": 2})),
            },
            super::ProjectionResult {
                name: "serving_health",
                value: Ok(json!({"status": "ok\nforged", "reason_code": "ready"})),
            },
            super::ProjectionResult {
                name: "resilience_health",
                value: Ok(json!({"status": "ok", "reason_code": "healthy"})),
            },
        ];

        let rendered = super::render_doctor_report(&results, &options);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_ne!(report["status"], "ok");
        assert_eq!(report["reason_code"], "serving_health_degraded");
        assert_eq!(report["data"]["serving"]["status"], "unavailable");
        assert!(!rendered.contains("ok\nforged"));
    }

    #[test]
    fn doctor_cli_table_escapes_control_characters() {
        let mut options = options();
        options.output = crate::cli_report::OutputFormat::Table;
        let results = vec![
            super::ProjectionResult {
                name: "explain_runtime",
                value: Ok(json!({"runtime_generation": 2})),
            },
            super::ProjectionResult {
                name: "runtime",
                value: Ok(json!({"generation": 2})),
            },
            super::ProjectionResult {
                name: "serving_health",
                value: Ok(json!({"status": "ok\nforged", "reason_code": "ready\u{1b}[31m"})),
            },
            super::ProjectionResult {
                name: "resilience_health",
                value: Ok(json!({"status": "ok", "reason_code": "healthy"})),
            },
        ];

        let rendered = super::render_doctor_report(&results, &options);

        assert!(!rendered.contains("ok\nforged"));
        assert!(!rendered.contains("\u{1b}"));
        assert!(!rendered.contains("ok\\nforged"));
        assert!(!rendered.contains("ready\\u{1b}"));
        assert!(rendered.contains("serving.status: unavailable"));
        assert!(rendered.contains("serving.reason_code: unknown"));
    }

    #[test]
    fn doctor_cli_table_preserves_envelope_and_safe_argv_semantics() {
        let mut options = options();
        options.output = crate::cli_report::OutputFormat::Table;
        let results = vec![
            super::ProjectionResult {
                name: "explain_runtime",
                value: Ok(json!({"runtime_generation": 2})),
            },
            super::ProjectionResult {
                name: "runtime",
                value: Ok(json!({"generation": 2, "reload_required": false})),
            },
            super::ProjectionResult {
                name: "serving_health",
                value: Ok(json!({"status": "ok", "reason_code": "serving"})),
            },
            super::ProjectionResult {
                name: "resilience_health",
                value: Ok(json!({"status": "ok", "reason_code": "healthy"})),
            },
        ];

        let rendered = super::render_doctor_report(&results, &options);

        assert!(rendered.contains("status: ok"));
        assert!(rendered.contains("reason_code: runtime_health_available"));
        assert!(rendered.contains("side_effect_class: runtime_readonly"));
        assert!(rendered.contains("scope.default_runtime_only: true"));
        assert!(rendered.contains("scope.event_limit: 20"));
        assert!(rendered.contains("effect.reads_management_runtime: true"));
        assert!(rendered.contains("runtime_generation: 2"));
        assert!(rendered.contains("serving.status: ok"));
        assert!(rendered.contains("resilience.status: ok"));
        assert!(rendered.contains("next_action.template_id: failures_tail"));
        assert!(rendered.contains("next_action.safe_argv[0]: one-ai-key"));
    }
}
