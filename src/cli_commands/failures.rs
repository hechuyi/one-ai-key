use serde_json::Value;

const DEFAULT_LAST: usize = 50;
const MAX_LAST: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureTailOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub last: Option<usize>,
    pub filters: FailureFilters,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureExplainOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub request_id: String,
    pub last: Option<usize>,
    pub filters: FailureFilters,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureFilters {
    pub request_id: Option<String>,
    pub public_model: Option<String>,
    pub channel_id: Option<String>,
    pub directive: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailureWindow {
    requested_last: usize,
    effective_last: usize,
    routing_buffered_events: usize,
    routing_offset: usize,
    routing_limit: usize,
    response_filter_buffered_events: usize,
    response_filter_offset: usize,
    response_filter_limit: usize,
}

impl FailureWindow {
    fn metadata(self, returned: usize) -> Value {
        let combined_limit = self
            .routing_limit
            .saturating_add(self.response_filter_limit);
        serde_json::json!({
            "kind": "bounded_recent_events",
            "source": ["routing_telemetry", "response_filter_events"],
            "limit": combined_limit,
            "returned": returned,
            "truncated": self.routing_offset > 0 || self.response_filter_offset > 0,
            "cursor": Value::Null,
            "bounded_reason": "latest_window",
            "requested_last": self.requested_last,
            "per_source_limit": self.effective_last,
            "sources": {
                "routing_telemetry": {
                    "buffered_events": self.routing_buffered_events,
                    "offset": self.routing_offset,
                    "limit": self.routing_limit,
                },
                "response_filter_events": {
                    "buffered_events": self.response_filter_buffered_events,
                    "offset": self.response_filter_offset,
                    "limit": self.response_filter_limit,
                },
            },
        })
    }
}

pub fn parse_failure_last(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("last must be an integer from 1 to {MAX_LAST}"))?;
    if (1..=MAX_LAST).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!("last must be from 1 to {MAX_LAST}"))
    }
}

pub async fn run_tail(
    options: FailureTailOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let (routing, response_filter, window) = fetch_failure_windows(&client, options.last).await?;
    let report = tail_report(&routing, &response_filter, window, &options.filters);
    Ok(render_failure_report(&report, options.output))
}

pub async fn run_explain(
    options: FailureExplainOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let (routing, response_filter, window) = fetch_failure_windows(&client, options.last).await?;
    let mut filters = options.filters.clone();
    filters.request_id = Some(options.request_id.clone());
    let report = explain_report(&routing, &response_filter, window, &filters);
    Ok(render_failure_report(&report, options.output))
}

async fn fetch_failure_windows(
    client: &crate::operator_client::OperatorClient,
    last: Option<usize>,
) -> Result<(Value, Value, FailureWindow), crate::operator_client::OperatorClientError> {
    let requested_last = last.unwrap_or(DEFAULT_LAST);
    let effective_last = requested_last.clamp(1, MAX_LAST);
    let routing_metadata = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::RoutingTelemetry {
            offset: Some(0),
            limit: Some(0),
        })
        .await?;
    let response_filter_metadata = client
        .get_json(
            crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
                offset: Some(0),
                limit: Some(0),
            },
        )
        .await?;
    let routing_buffered_events = buffered_events(&routing_metadata);
    let response_filter_buffered_events = buffered_events(&response_filter_metadata);
    let routing_offset = routing_buffered_events.saturating_sub(effective_last);
    let response_filter_offset = response_filter_buffered_events.saturating_sub(effective_last);
    let routing = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::RoutingTelemetry {
            offset: Some(routing_offset),
            limit: Some(effective_last),
        })
        .await?;
    let response_filter = client
        .get_json(
            crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
                offset: Some(response_filter_offset),
                limit: Some(effective_last),
            },
        )
        .await?;
    let window = FailureWindow {
        requested_last,
        effective_last,
        routing_buffered_events,
        routing_offset,
        routing_limit: effective_last,
        response_filter_buffered_events,
        response_filter_offset,
        response_filter_limit: effective_last,
    };
    Ok((routing, response_filter, window))
}

fn buffered_events(snapshot: &Value) -> usize {
    snapshot
        .get("buffered_events")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

#[cfg(test)]
pub fn tail_endpoint_sequence(
    last: Option<usize>,
    routing_buffered_events: usize,
    response_filter_buffered_events: usize,
) -> Vec<crate::operator_client::ReadOnlyEndpoint> {
    let effective_last = last.unwrap_or(DEFAULT_LAST).min(MAX_LAST).max(1);
    vec![
        crate::operator_client::ReadOnlyEndpoint::RoutingTelemetry {
            offset: Some(0),
            limit: Some(0),
        },
        crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
            offset: Some(0),
            limit: Some(0),
        },
        crate::operator_client::ReadOnlyEndpoint::RoutingTelemetry {
            offset: Some(routing_buffered_events.saturating_sub(effective_last)),
            limit: Some(effective_last),
        },
        crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
            offset: Some(response_filter_buffered_events.saturating_sub(effective_last)),
            limit: Some(effective_last),
        },
    ]
}

#[cfg(test)]
pub fn render_tail_report(
    routing: &Value,
    response_filter: &Value,
    filters: &FailureFilters,
    output: crate::cli_report::OutputFormat,
) -> String {
    let window = test_window(routing, response_filter);
    render_failure_report(
        &tail_report(routing, response_filter, window, filters),
        output,
    )
}

#[cfg(test)]
pub fn render_explain_report(
    routing: &Value,
    response_filter: &Value,
    request_id: &str,
    filters: &FailureFilters,
    output: crate::cli_report::OutputFormat,
) -> String {
    let mut filters = filters.clone();
    filters.request_id = Some(request_id.to_string());
    let window = test_window(routing, response_filter);
    render_failure_report(
        &explain_report(routing, response_filter, window, &filters),
        output,
    )
}

#[cfg(test)]
fn test_window(routing: &Value, response_filter: &Value) -> FailureWindow {
    FailureWindow {
        requested_last: DEFAULT_LAST,
        effective_last: DEFAULT_LAST,
        routing_buffered_events: buffered_events(routing),
        routing_offset: routing.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
        routing_limit: routing.get("limit").and_then(Value::as_u64).unwrap_or(0) as usize,
        response_filter_buffered_events: buffered_events(response_filter),
        response_filter_offset: response_filter
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        response_filter_limit: response_filter
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
    }
}

fn tail_report(
    routing: &Value,
    response_filter: &Value,
    window: FailureWindow,
    filters: &FailureFilters,
) -> Value {
    let failures = filtered_failures(routing, response_filter, filters);
    let status = if failures.is_empty() {
        "ok"
    } else {
        "degraded"
    };
    let reason_code = if failures.is_empty() {
        "no_failures_in_window"
    } else {
        "failures_found_in_window"
    };
    report_envelope(
        "failures tail",
        status,
        reason_code,
        "bounded recent failure events were inspected",
        filters,
        window,
        failures,
    )
}

fn explain_report(
    routing: &Value,
    response_filter: &Value,
    window: FailureWindow,
    filters: &FailureFilters,
) -> Value {
    let failures = filtered_failures(routing, response_filter, filters);
    let status = if failures.is_empty() {
        "not_found"
    } else {
        "degraded"
    };
    let reason_code = if failures.is_empty() {
        "request_failure_not_found_in_window".to_string()
    } else {
        primary_reason_code(&failures)
    };
    let returned = failures.len();
    let data = serde_json::json!({
        "command": "failures explain",
        "failure_count": returned,
        "explanation": request_explanation(&failures),
        "evidence": failures,
    });
    report_envelope_with_data(
        status,
        &reason_code,
        "bounded local failure evidence was inspected",
        filters,
        window,
        returned,
        data,
    )
}

fn report_envelope(
    command: &'static str,
    status: &'static str,
    reason_code: &str,
    reason: &'static str,
    filters: &FailureFilters,
    window: FailureWindow,
    failures: Vec<Value>,
) -> Value {
    let returned = failures.len();
    let data = serde_json::json!({
        "command": command,
        "failure_count": returned,
        "failures": failures,
    });
    report_envelope_with_data(status, reason_code, reason, filters, window, returned, data)
}

fn report_envelope_with_data(
    status: &'static str,
    reason_code: &str,
    reason: &'static str,
    filters: &FailureFilters,
    window: FailureWindow,
    returned: usize,
    data: Value,
) -> Value {
    let failures = data
        .get("failures")
        .or_else(|| data.get("evidence"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let effect = crate::cli_effects::runtime_readonly_store_reads_effect();
    serde_json::json!({
        "status": status,
        "reason": reason,
        "reason_code": reason_code,
        "side_effect_class": crate::cli_effects::side_effect_class_code(effect.side_effect_class),
        "effect_vector": crate::cli_effects::effect_vector_json(effect.effect_vector),
        "scope": filters_metadata(filters),
        "window": window.metadata(returned),
        "next_action": aggregate_next_action(&failures),
        "data": data,
    })
}

fn request_explanation(failures: &[Value]) -> Value {
    let Some(primary) = primary_failure(failures) else {
        return serde_json::json!({
            "stage": "unknown",
            "failure_class": "unknown",
            "router_action": "none",
            "retry_eligibility": "not_applicable",
            "retry_blocked_reason": Value::Null,
            "client_visible_status": "not_found_in_window",
            "final_outcome": "not_found_in_window",
        });
    };
    let final_outcome = match primary
        .get("client_visible_status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
    {
        "not_applicable_retry_before_output" => "recovered_before_client_output",
        "not_applicable" => "management_or_lifecycle_event",
        "local_400"
        | "local_401"
        | "local_404"
        | "local_429"
        | "local_502"
        | "local_503"
        | "upstream_4xx"
        | "upstream_5xx"
        | "stream_committed_failure" => "client_visible_failure",
        _ => "unknown",
    };
    serde_json::json!({
        "stage": taxonomy_string(primary, "stage", "unknown"),
        "public_model": taxonomy_string(primary, "public_model", "unknown"),
        "client_token_ref": primary.get("client_token_ref").cloned().unwrap_or(Value::Null),
        "selected_target": primary.get("selected_target").cloned().unwrap_or(Value::Null),
        "failure_class": taxonomy_string(primary, "failure_class", "unknown"),
        "router_action": taxonomy_string(primary, "router_action", "none"),
        "retry_eligibility": taxonomy_string(primary, "retry_eligibility", "not_applicable"),
        "retry_blocked_reason": primary.get("retry_blocked_reason").cloned().unwrap_or(Value::Null),
        "client_visible_status": taxonomy_string(primary, "client_visible_status", "unknown"),
        "final_outcome": final_outcome,
    })
}

fn primary_failure(failures: &[Value]) -> Option<&Value> {
    failures
        .iter()
        .enumerate()
        .max_by_key(|(index, failure)| (failure_priority(failure), *index))
        .map(|(_, failure)| failure)
}

fn failure_priority(failure: &Value) -> u8 {
    let stage = failure.get("stage").and_then(Value::as_str).unwrap_or("");
    let failure_class = failure
        .get("failure_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    let client_visible_status = failure
        .get("client_visible_status")
        .and_then(Value::as_str)
        .unwrap_or("");
    let router_action = failure
        .get("router_action")
        .and_then(Value::as_str)
        .unwrap_or("");
    if stage == "post_output" || failure_class == "stream_committed_failure" {
        return 100;
    }
    if is_client_visible_failure_status(client_visible_status) {
        return 90;
    }
    if router_action == "returned_local_error" {
        return 80;
    }
    if matches!(router_action, "marked_credential" | "marked_channel") {
        return 70;
    }
    if matches!(
        router_action,
        "retried_before_output" | "fell_back_before_output"
    ) {
        return 40;
    }
    10
}

fn is_client_visible_failure_status(status: &str) -> bool {
    matches!(
        status,
        "local_400"
            | "local_401"
            | "local_404"
            | "local_429"
            | "local_502"
            | "local_503"
            | "upstream_4xx"
            | "upstream_5xx"
            | "stream_committed_failure"
    )
}

fn taxonomy_string(value: &Value, field: &str, fallback: &'static str) -> Value {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(Value::from)
        .unwrap_or_else(|| Value::from(fallback))
}

fn filtered_failures(
    routing: &Value,
    response_filter: &Value,
    filters: &FailureFilters,
) -> Vec<Value> {
    routing
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(classify_routing_event)
        .chain(
            response_filter
                .get("events")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(classify_response_filter_event),
        )
        .filter(|failure| matches_filters(failure, filters))
        .collect::<Vec<_>>()
}

fn classify_routing_event(event: &Value) -> Option<Value> {
    let kind = event
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "upstream_failure_observed" => Some(classify_upstream_failure(event)),
        "channel_health_transition_applied" => Some(classify_channel_transition(event)),
        "credential_transition_applied" | "credential_lifecycle_persistence_dropped" => {
            Some(classify_credential_transition(event))
        }
        "no_route_candidate" | "route_rejected" => Some(classify_route_planning_failure(event)),
        "model_not_visible" | "client_scope_miss" => Some(classify_model_visibility_failure(event)),
        _ => None,
    }
}

fn classify_upstream_failure(event: &Value) -> Value {
    let failure = event.get("failure").unwrap_or(&Value::Null);
    let failure_kind = failure
        .get("failure_kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let status = failure.get("status").and_then(Value::as_u64);
    let directive = safe_directive(failure.get("directive")).unwrap_or("return_error");
    let retry_decision_reason = failure
        .get("retry_decision_reason")
        .or_else(|| failure.get("denial_reason"))
        .and_then(Value::as_str);
    let retry_eligibility = retry_eligibility(failure, directive, retry_decision_reason);
    let failure_class = upstream_failure_class(failure_kind, status);
    let stage = upstream_stage(failure.get("failure_source").and_then(Value::as_str));
    let router_action = router_action_for_directive(directive);
    let request_id = safe_local_id(event.get("request_id"));
    let public_model = safe_local_id(failure.get("public_model"));
    let channel_id = safe_local_id(event.get("channel_id"));
    let client_token_ref = safe_local_id(event.get("client_token_ref"));
    serde_json::json!({
        "source": "routing_telemetry",
        "event_kind": "upstream_failure_observed",
        "request_id": nullable_string(request_id.as_deref()),
        "stage": stage,
        "public_model": public_model.clone().unwrap_or_else(|| "unknown".to_string()),
        "client_token_ref": nullable_string(client_token_ref.as_deref()),
        "selected_target": sanitize_selected_target(channel_id.as_deref()),
        "channel_id": nullable_string(channel_id.as_deref()),
        "failure_class": failure_class,
        "router_action": router_action,
        "retry_eligibility": retry_eligibility,
        "retry_blocked_reason": retry_blocked_reason(&retry_eligibility, retry_decision_reason),
        "client_visible_status": client_visible_status(status, directive, &retry_eligibility),
        "reason_code": reason_code_for_class(failure_class),
        "directive": directive,
        "attempt": failure.get("attempt").and_then(Value::as_u64),
        "next_action": next_action_for(stage, failure_class, request_id.as_deref(), public_model.as_deref(), channel_id.as_deref(), client_token_ref.as_deref()),
    })
}

fn classify_channel_transition(event: &Value) -> Value {
    let state = event
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let failure_class = if state == "cooling_down" || state == "degraded" {
        "credential_unavailable"
    } else {
        "unknown"
    };
    let request_id = safe_local_id(event.get("request_id"));
    let channel_id = safe_local_id(event.get("channel_id"));
    let client_token_ref = safe_local_id(event.get("client_token_ref"));
    serde_json::json!({
        "source": "routing_telemetry",
        "event_kind": "channel_health_transition_applied",
        "request_id": nullable_string(request_id.as_deref()),
        "stage": "credential_selection",
        "public_model": safe_local_id(event.get("public_model")).unwrap_or_else(|| "unknown".to_string()),
        "client_token_ref": nullable_string(client_token_ref.as_deref()),
        "selected_target": sanitize_selected_target(channel_id.as_deref()),
        "channel_id": nullable_string(channel_id.as_deref()),
        "failure_class": failure_class,
        "router_action": "marked_channel",
        "retry_eligibility": "not_applicable",
        "retry_blocked_reason": Value::Null,
        "client_visible_status": "not_applicable",
        "reason_code": stable_reason(event.get("reason").and_then(Value::as_str), "channel_degraded"),
        "directive": Value::Null,
        "next_action": next_action_for("credential_selection", failure_class, request_id.as_deref(), None, channel_id.as_deref(), client_token_ref.as_deref()),
    })
}

fn classify_credential_transition(event: &Value) -> Value {
    let request_id = safe_local_id(event.get("request_id"));
    let channel_id = safe_local_id(event.get("channel_id"));
    let client_token_ref = safe_local_id(event.get("client_token_ref"));
    serde_json::json!({
        "source": "routing_telemetry",
        "event_kind": credential_event_kind(event.get("kind").and_then(Value::as_str)),
        "request_id": nullable_string(request_id.as_deref()),
        "stage": "credential_selection",
        "public_model": safe_local_id(event.get("public_model")).unwrap_or_else(|| "unknown".to_string()),
        "client_token_ref": nullable_string(client_token_ref.as_deref()),
        "selected_target": sanitize_selected_target(channel_id.as_deref()),
        "channel_id": nullable_string(channel_id.as_deref()),
        "failure_class": "credential_unavailable",
        "router_action": "marked_credential",
        "retry_eligibility": "not_applicable",
        "retry_blocked_reason": Value::Null,
        "client_visible_status": "not_applicable",
        "reason_code": stable_reason(event.get("reason").and_then(Value::as_str), "credential_unavailable"),
        "directive": Value::Null,
        "next_action": next_action_for("credential_selection", "credential_unavailable", request_id.as_deref(), None, channel_id.as_deref(), client_token_ref.as_deref()),
    })
}

fn classify_route_planning_failure(event: &Value) -> Value {
    let request_id = safe_local_id(event.get("request_id"));
    let public_model = safe_local_id(event.get("public_model"));
    let channel_id = safe_local_id(event.get("channel_id"));
    let client_token_ref = safe_local_id(event.get("client_token_ref"));
    serde_json::json!({
        "source": "routing_telemetry",
        "event_kind": route_event_kind(event.get("kind").and_then(Value::as_str)),
        "request_id": nullable_string(request_id.as_deref()),
        "stage": "route_planning",
        "public_model": public_model.clone().unwrap_or_else(|| "unknown".to_string()),
        "client_token_ref": nullable_string(client_token_ref.as_deref()),
        "selected_target": Value::Null,
        "channel_id": nullable_string(channel_id.as_deref()),
        "failure_class": "no_route_candidate",
        "router_action": "returned_local_error",
        "retry_eligibility": "not_applicable",
        "retry_blocked_reason": Value::Null,
        "client_visible_status": safe_status(event.get("client_visible_status")).unwrap_or("local_404"),
        "reason_code": "no_route_candidate",
        "directive": Value::Null,
        "next_action": next_action_for("route_planning", "no_route_candidate", request_id.as_deref(), public_model.as_deref(), channel_id.as_deref(), client_token_ref.as_deref()),
    })
}

fn classify_model_visibility_failure(event: &Value) -> Value {
    let request_id = safe_local_id(event.get("request_id"));
    let public_model = safe_local_id(event.get("public_model"));
    let channel_id = safe_local_id(event.get("channel_id"));
    let client_token_ref = safe_local_id(event.get("client_token_ref"));
    serde_json::json!({
        "source": "routing_telemetry",
        "event_kind": visibility_event_kind(event.get("kind").and_then(Value::as_str)),
        "request_id": nullable_string(request_id.as_deref()),
        "stage": "model_visibility",
        "public_model": public_model.clone().unwrap_or_else(|| "unknown".to_string()),
        "client_token_ref": nullable_string(client_token_ref.as_deref()),
        "selected_target": Value::Null,
        "channel_id": nullable_string(channel_id.as_deref()),
        "failure_class": "model_not_visible",
        "router_action": "returned_local_error",
        "retry_eligibility": "not_applicable",
        "retry_blocked_reason": Value::Null,
        "client_visible_status": safe_status(event.get("client_visible_status")).unwrap_or("local_404"),
        "reason_code": "model_not_in_client_scope",
        "directive": Value::Null,
        "next_action": next_action_for("model_visibility", "model_not_visible", request_id.as_deref(), public_model.as_deref(), channel_id.as_deref(), client_token_ref.as_deref()),
    })
}

fn classify_response_filter_event(event: &Value) -> Option<Value> {
    if event.get("outcome").and_then(Value::as_str) != Some("rejected") {
        return None;
    }
    let body_committed = event
        .get("body_committed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let action = safe_directive(event.get("action")).unwrap_or("reject");
    let request_id = safe_local_id(event.get("request_id"));
    let public_model = safe_local_id(event.get("public_model"));
    let channel_id = safe_local_id(event.get("channel_id"));
    let client_token_ref = safe_local_id(event.get("client_token_ref"));
    Some(serde_json::json!({
        "source": "response_filter_events",
        "event_kind": "response_filter_rejected",
        "request_id": nullable_string(request_id.as_deref()),
        "stage": if body_committed { "post_output" } else { "response_filter" },
        "public_model": public_model.clone().unwrap_or_else(|| "unknown".to_string()),
        "client_token_ref": nullable_string(client_token_ref.as_deref()),
        "selected_target": sanitize_selected_target(channel_id.as_deref()),
        "channel_id": nullable_string(channel_id.as_deref()),
        "failure_class": if body_committed { "stream_committed_failure" } else { "response_filter_rejected" },
        "router_action": response_filter_router_action(action),
        "retry_eligibility": if body_committed { "blocked_streaming" } else { "eligible_before_output" },
        "retry_blocked_reason": if body_committed { Value::from("partial_output_started") } else { Value::Null },
        "client_visible_status": if body_committed { "stream_committed_failure" } else { "local_502" },
        "reason_code": stable_reason(event.get("reason_code").and_then(Value::as_str), "response_filter_rejected"),
        "directive": action,
        "content_kind": safe_status(event.get("content_kind")).unwrap_or("unknown"),
        "next_action": next_action_for(if body_committed { "post_output" } else { "response_filter" }, "response_filter_rejected", request_id.as_deref(), public_model.as_deref(), channel_id.as_deref(), client_token_ref.as_deref()),
    }))
}

fn upstream_stage(failure_source: Option<&str>) -> &'static str {
    match failure_source.unwrap_or_default() {
        "response_filter_precommit" => "response_filter",
        "guarded_success_envelope" | "upstream_response_guard" => "upstream_response_guard",
        "upstream_transaction" | "upstream_transport" => "upstream_transport",
        _ => "upstream_transport",
    }
}

fn upstream_failure_class(failure_kind: &str, status: Option<u64>) -> &'static str {
    if matches!(status, Some(500..=599)) {
        return "upstream_5xx";
    }
    match failure_kind {
        "timeout" | "transport_timeout" => "upstream_timeout",
        "response_filter_rejected" => "response_filter_rejected",
        "auth_invalid"
        | "quota_exhausted"
        | "key_switch_cooldown"
        | "relay_balance_unavailable" => "credential_unavailable",
        "provider_unavailable" => "upstream_5xx",
        _ => "unknown",
    }
}

fn router_action_for_directive(directive: &str) -> &'static str {
    match directive {
        "retry" => "retried_before_output",
        "fallback" => "fell_back_before_output",
        "mark_credential" => "marked_credential",
        "mark_channel" => "marked_channel",
        "record_event_only" => "recorded_event_only",
        _ => "returned_local_error",
    }
}

fn retry_eligibility(
    failure: &Value,
    directive: &str,
    retry_decision_reason: Option<&str>,
) -> String {
    if let Some(reason) = retry_decision_reason.and_then(stable_retry_blocked_reason) {
        return format!("blocked_{reason}");
    }
    if failure
        .get("bytes_sent_to_client")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return "blocked_bytes_sent".to_string();
    }
    if failure
        .get("streaming")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && directive != "retry"
    {
        return "blocked_streaming".to_string();
    }
    if directive == "retry" || directive == "fallback" {
        "eligible_before_output".to_string()
    } else {
        "not_applicable".to_string()
    }
}

fn retry_blocked_reason(retry_eligibility: &str, retry_decision_reason: Option<&str>) -> Value {
    if !retry_eligibility.starts_with("blocked_") {
        return Value::Null;
    }
    retry_decision_reason
        .and_then(stable_retry_blocked_reason)
        .map(Value::from)
        .unwrap_or_else(|| Value::from(retry_eligibility.trim_start_matches("blocked_")))
}

fn stable_retry_blocked_reason(value: &str) -> Option<&'static str> {
    match value {
        "streaming" | "blocked_streaming" => Some("streaming"),
        "bytes_sent" | "blocked_bytes_sent" => Some("bytes_sent"),
        "policy" | "blocked_policy" => Some("policy"),
        "duplicate_charge_risk" | "blocked_duplicate_charge_risk" => Some("duplicate_charge_risk"),
        _ => None,
    }
}

fn client_visible_status(status: Option<u64>, directive: &str, retry_eligibility: &str) -> String {
    if retry_eligibility == "eligible_before_output" {
        return "not_applicable_retry_before_output".to_string();
    }
    match status {
        Some(400..=499) => "upstream_4xx".to_string(),
        Some(500..=599) => "upstream_5xx".to_string(),
        Some(value) => format!("upstream_status_{value}"),
        None if directive == "retry" => "not_applicable_retry_before_output".to_string(),
        None => "unknown".to_string(),
    }
}

fn reason_code_for_class(failure_class: &str) -> &'static str {
    match failure_class {
        "credential_unavailable" => "credential_unavailable",
        "upstream_5xx" => "upstream_5xx",
        "upstream_timeout" => "upstream_timeout",
        "response_filter_rejected" => "response_filter_rejected",
        _ => "unknown_failure_class",
    }
}

fn stable_reason(value: Option<&str>, fallback: &'static str) -> &'static str {
    match value.unwrap_or_default() {
        "channel_degraded" => "channel_degraded",
        "channel_cooling_down" => "channel_cooling_down",
        "credential_unavailable" => "credential_unavailable",
        "credential_cooling_down" => "credential_unavailable",
        "credential_quota_exhausted" => "credential_unavailable",
        "response_filter_rejected" => "response_filter_rejected",
        "stream_committed_failure" => "stream_committed_failure",
        "model_not_in_client_scope" => "model_not_in_client_scope",
        "no_route_candidate" => "no_route_candidate",
        "upstream_5xx" => "upstream_5xx",
        "upstream_timeout" => "upstream_timeout",
        _ => fallback,
    }
}

fn response_filter_router_action(action: &str) -> &'static str {
    match action {
        "reject_and_expire_credential" => "marked_credential",
        "reject_and_cooldown_channel" => "marked_channel",
        "reject_and_disable_channel" => "marked_channel",
        _ => "recorded_event_only",
    }
}

fn credential_event_kind(kind: Option<&str>) -> &'static str {
    match kind {
        Some("credential_lifecycle_persistence_dropped") => {
            "credential_lifecycle_persistence_dropped"
        }
        _ => "credential_transition_applied",
    }
}

fn route_event_kind(kind: Option<&str>) -> &'static str {
    match kind {
        Some("route_rejected") => "route_rejected",
        _ => "no_route_candidate",
    }
}

fn visibility_event_kind(kind: Option<&str>) -> &'static str {
    match kind {
        Some("client_scope_miss") => "client_scope_miss",
        _ => "model_not_visible",
    }
}

fn sanitize_selected_target(channel_id: Option<&str>) -> Value {
    channel_id
        .and_then(sanitize_local_string)
        .map(|channel_id| serde_json::json!({ "channel_id": channel_id }))
        .unwrap_or(Value::Null)
}

fn next_action_for(
    stage: &str,
    failure_class: &str,
    request_id: Option<&str>,
    public_model: Option<&str>,
    channel_id: Option<&str>,
    client_token_ref: Option<&str>,
) -> Value {
    match (stage, failure_class) {
        ("model_visibility", _,) => next_action(
            "models_explain_visibility",
            "Inspect the compiled model route and client-token scope for this public model.",
            model_explain_argv(public_model, client_token_ref),
            "runtime_readonly",
            false,
        ),
        ("route_planning", "no_route_candidate") => next_action(
            "route_explain_no_candidate",
            "Inspect route candidates for this public model.",
            route_explain_argv(public_model),
            "runtime_readonly",
            false,
        ),
        ("credential_selection", "credential_unavailable") => next_action(
            "failures_tail_channel",
            "Inspect recent bounded failures for the affected channel. Credential probing is unavailable until M3.",
            failures_tail_argv(channel_id),
            "runtime_readonly",
            false,
        ),
        ("response_filter", _) | ("post_output", _) => next_action(
            "failures_explain_request",
            "Inspect this bounded failure record. Do not retry after client-visible output.",
            failures_explain_argv(request_id),
            "runtime_readonly",
            false,
        ),
        ("upstream_transport", "upstream_5xx") | ("upstream_transport", "upstream_timeout") => {
            next_action(
                "route_explain_upstream_failure",
                "Inspect route candidates and recent failures before changing configuration.",
                route_explain_argv(public_model),
                "runtime_readonly",
                false,
            )
        }
        _ => next_action(
            "failures_tail",
            "Inspect recent bounded failures for related events.",
            failures_tail_argv(channel_id),
            "runtime_readonly",
            false,
        ),
    }
}

fn next_action(
    template_id: &'static str,
    summary: &'static str,
    safe_argv: Vec<String>,
    side_effect_class: &'static str,
    requires_confirmation: bool,
) -> Value {
    serde_json::json!({
        "summary": summary,
        "template_id": template_id,
        "safe_argv": validate_safe_argv(safe_argv),
        "side_effect_class": side_effect_class,
        "requires_confirmation": requires_confirmation,
    })
}

fn model_explain_argv(public_model: Option<&str>, client_token_ref: Option<&str>) -> Vec<String> {
    let mut argv = common_management_argv("models", "explain");
    argv.push("--model".to_string());
    argv.push(public_model.unwrap_or("<public-model>").to_string());
    argv.push("--client-token-ref".to_string());
    argv.push(client_token_ref.unwrap_or("<client-token-ref>").to_string());
    argv
}

fn route_explain_argv(public_model: Option<&str>) -> Vec<String> {
    let mut argv = common_management_argv("route", "explain");
    argv.push(public_model.unwrap_or("<public-model>").to_string());
    argv
}

fn failures_tail_argv(channel_id: Option<&str>) -> Vec<String> {
    let mut argv = common_management_argv("failures", "tail");
    argv.push("--last".to_string());
    argv.push("50".to_string());
    if let Some(channel_id) = channel_id {
        argv.push("--channel".to_string());
        argv.push(channel_id.to_string());
    }
    argv
}

fn failures_explain_argv(request_id: Option<&str>) -> Vec<String> {
    let mut argv = common_management_argv("failures", "explain");
    argv.push(request_id.unwrap_or("<request-id>").to_string());
    argv.push("--last".to_string());
    argv.push("50".to_string());
    argv
}

fn common_management_argv(command: &'static str, subcommand: &'static str) -> Vec<String> {
    vec![
        "one-ai-key".to_string(),
        command.to_string(),
        subcommand.to_string(),
        "--management-url".to_string(),
        "<url>".to_string(),
        "--management-token-env".to_string(),
        "<env>".to_string(),
    ]
}

fn validate_safe_argv(argv: Vec<String>) -> Vec<Value> {
    if is_allowlisted_safe_argv(&argv) && argv.iter().all(|arg| is_safe_argv_arg(arg)) {
        argv.into_iter().map(Value::from).collect()
    } else {
        Vec::new()
    }
}

fn is_allowlisted_safe_argv(argv: &[String]) -> bool {
    if argv.is_empty() {
        return true;
    }
    if argv.len() < 7
        || argv[0] != "one-ai-key"
        || argv[3] != "--management-url"
        || argv[4] != "<url>"
        || argv[5] != "--management-token-env"
        || argv[6] != "<env>"
    {
        return false;
    }
    match (
        argv.get(1).map(String::as_str),
        argv.get(2).map(String::as_str),
    ) {
        (Some("models"), Some("explain")) => {
            argv.len() == 11 && argv[7] == "--model" && argv[9] == "--client-token-ref"
        }
        (Some("route"), Some("explain")) => argv.len() == 8,
        (Some("failures"), Some("tail")) => {
            (argv.len() == 9 && argv[7] == "--last" && argv[8] == "50")
                || (argv.len() == 11
                    && argv[7] == "--last"
                    && argv[8] == "50"
                    && argv[9] == "--channel")
        }
        (Some("failures"), Some("explain")) => {
            argv.len() == 10 && argv[8] == "--last" && argv[9] == "50"
        }
        _ => false,
    }
}

fn is_safe_argv_arg(arg: &str) -> bool {
    const SAFE_PLACEHOLDERS: &[&str] = &[
        "<url>",
        "<env>",
        "<client-token-ref>",
        "<public-model>",
        "<request-id>",
    ];
    if SAFE_PLACEHOLDERS.contains(&arg) {
        return true;
    }
    sanitize_local_string(arg).is_some()
}

fn aggregate_next_action(failures: &[Value]) -> Value {
    primary_failure(failures)
        .and_then(|failure| failure.get("next_action"))
        .cloned()
        .unwrap_or_else(|| {
            next_action(
                "no_action_required",
                "No failures matched the bounded local event window.",
                Vec::new(),
                "runtime_readonly",
                false,
            )
        })
}

fn primary_reason_code(failures: &[Value]) -> String {
    primary_failure(failures)
        .and_then(|failure| failure.get("reason_code"))
        .and_then(Value::as_str)
        .map(|value| stable_reason(Some(value), "failures_found_in_window"))
        .unwrap_or("failures_found_in_window")
        .to_string()
}

fn matches_filters(failure: &Value, filters: &FailureFilters) -> bool {
    matches_filter_field(
        failure.get("request_id").and_then(Value::as_str),
        filters.request_id.as_deref(),
    ) && matches_filter_field(
        failure.get("public_model").and_then(Value::as_str),
        filters.public_model.as_deref(),
    ) && matches_filter_field(
        failure.get("channel_id").and_then(Value::as_str),
        filters.channel_id.as_deref(),
    ) && matches_filter_field(
        failure.get("directive").and_then(Value::as_str),
        filters.directive.as_deref(),
    )
}

fn matches_filter_field(value: Option<&str>, filter: Option<&str>) -> bool {
    match filter {
        Some(filter) => match sanitize_local_string(filter) {
            Some(filter) => value == Some(filter.as_str()),
            None => false,
        },
        None => true,
    }
}

fn filters_metadata(filters: &FailureFilters) -> Value {
    serde_json::json!({
        "request_id": filters.request_id.as_deref().and_then(sanitize_local_string),
        "public_model": filters.public_model.as_deref().and_then(sanitize_local_string),
        "channel_id": filters.channel_id.as_deref().and_then(sanitize_local_string),
        "directive": filters.directive.as_deref().and_then(sanitize_local_string),
    })
}

fn nullable_string(value: Option<&str>) -> Value {
    value.map(Value::from).unwrap_or(Value::Null)
}

fn safe_local_id(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(sanitize_local_string)
}

fn sanitize_local_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("://")
        || lower.contains("http")
        || lower.contains("telegram")
        || lower.contains("promo")
        || lower.contains("invite")
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn safe_directive(value: Option<&Value>) -> Option<&'static str> {
    match value.and_then(Value::as_str).unwrap_or_default() {
        "retry" => Some("retry"),
        "fallback" => Some("fallback"),
        "return_error" => Some("return_error"),
        "mark_credential" => Some("mark_credential"),
        "mark_channel" => Some("mark_channel"),
        "record_event_only" => Some("record_event_only"),
        "reject" => Some("reject"),
        "reject_and_expire_credential" => Some("reject_and_expire_credential"),
        "reject_and_cooldown_channel" => Some("reject_and_cooldown_channel"),
        "reject_and_disable_channel" => Some("reject_and_disable_channel"),
        _ => None,
    }
}

fn safe_status(value: Option<&Value>) -> Option<&'static str> {
    match value.and_then(Value::as_str).unwrap_or_default() {
        "local_400" => Some("local_400"),
        "local_401" => Some("local_401"),
        "local_404" => Some("local_404"),
        "local_429" => Some("local_429"),
        "local_502" => Some("local_502"),
        "local_503" => Some("local_503"),
        "upstream_4xx" => Some("upstream_4xx"),
        "upstream_5xx" => Some("upstream_5xx"),
        "stream_committed_failure" => Some("stream_committed_failure"),
        "text" => Some("text"),
        "json" => Some("json"),
        _ => None,
    }
}

pub fn render_failure_report(report: &Value, output: crate::cli_report::OutputFormat) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(report).expect("failure report should serialize")
        }
        crate::cli_report::OutputFormat::Table => render_failure_table(report),
    }
}

fn render_failure_table(report: &Value) -> String {
    let mut output = String::new();
    push_table_line(&mut output, "Status", report.get("status"));
    push_table_line(&mut output, "Reason", report.get("reason"));
    push_table_line(&mut output, "Reason code", report.get("reason_code"));
    push_table_line(&mut output, "Side effect", report.get("side_effect_class"));
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
            crate::cli_report::push_table_field(
                &mut output,
                &format!("effect.{field}"),
                effect.get(field),
            );
        }
    }
    if let Some(scope) = report.get("scope").and_then(Value::as_object) {
        for (key, value) in scope {
            crate::cli_report::push_table_field(&mut output, &format!("scope.{key}"), Some(value));
        }
    }
    if let Some(window) = report.get("window") {
        for field in ["kind", "limit", "returned", "truncated", "bounded_reason"] {
            crate::cli_report::push_table_field(
                &mut output,
                &format!("window.{field}"),
                window.get(field),
            );
        }
        output.push_str(&format!(
            "Window: kind={} limit={} returned={} truncated={}\n",
            table_str(window.get("kind")),
            table_str(window.get("limit")),
            table_str(window.get("returned")),
            table_str(window.get("truncated")),
        ));
    }
    if let Some(summary) = report
        .get("next_action")
        .and_then(|next_action| next_action.get("summary"))
    {
        push_table_line(&mut output, "Next action", Some(summary));
    }
    if let Some(template_id) = report
        .get("next_action")
        .and_then(|next_action| next_action.get("template_id"))
    {
        push_table_line(&mut output, "Next action template", Some(template_id));
    }
    if let Some(side_effect_class) = report
        .get("next_action")
        .and_then(|next_action| next_action.get("side_effect_class"))
    {
        push_table_line(
            &mut output,
            "Next action side effect",
            Some(side_effect_class),
        );
    }
    if let Some(requires_confirmation) = report
        .get("next_action")
        .and_then(|next_action| next_action.get("requires_confirmation"))
    {
        push_table_line(
            &mut output,
            "Next action requires confirmation",
            Some(requires_confirmation),
        );
    }
    if let Some(argv) = report
        .get("next_action")
        .and_then(|next_action| next_action.get("safe_argv"))
        .and_then(Value::as_array)
    {
        if argv.is_empty() {
            output.push_str("Next action safe_argv: []\n");
        } else {
            for (index, arg) in argv.iter().enumerate() {
                output.push_str(&format!(
                    "Next action safe_argv[{index}]: {}\n",
                    table_str(Some(arg)),
                ));
            }
        }
    }
    output.push_str("Failures:\n");
    let failures = report
        .get("data")
        .and_then(|data| data.get("failures"))
        .or_else(|| report.get("data").and_then(|data| data.get("evidence")))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for failure in failures {
        output.push_str(&format!(
            "- request_id={} stage={} failure_class={} router_action={} retry_eligibility={} retry_blocked_reason={} client_visible_status={} reason_code={} model={} channel={} directive={}\n",
            table_str(failure.get("request_id")),
            table_str(failure.get("stage")),
            table_str(failure.get("failure_class")),
            table_str(failure.get("router_action")),
            table_str(failure.get("retry_eligibility")),
            table_str(failure.get("retry_blocked_reason")),
            table_str(failure.get("client_visible_status")),
            table_str(failure.get("reason_code")),
            table_str(failure.get("public_model")),
            table_str(failure.get("channel_id")),
            table_str(failure.get("directive")),
        ));
    }
    output
}

fn push_table_line(output: &mut String, label: &str, value: Option<&Value>) {
    output.push_str(label);
    output.push_str(": ");
    output.push_str(&table_str(value));
    output.push('\n');
}

fn table_str(value: Option<&Value>) -> String {
    let raw = match value {
        Some(Value::String(value)) => value.as_str().to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) | None => "null".to_string(),
        Some(other) => other.to_string(),
    };
    crate::cli_report::escape_table_value(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routing_fixture() -> Value {
        serde_json::json!({
            "buffered_events": 6,
            "offset": 0,
            "limit": 50,
            "events": [
                {
                    "kind": "client_scope_miss",
                    "request_id": "req_scope",
                    "public_model": "gpt-example",
                    "client_visible_status": "local_404"
                },
                {
                    "kind": "no_route_candidate",
                    "request_id": "req_no_route",
                    "public_model": "gpt-missing",
                    "client_visible_status": "local_404"
                },
                {
                    "kind": "channel_health_transition_applied",
                    "request_id": "req_cooldown",
                    "channel_id": "relay-a",
                    "state": "cooling_down",
                    "reason": "channel_cooling_down"
                },
                {
                    "kind": "credential_transition_applied",
                    "request_id": "req_credential",
                    "channel_id": "relay-a",
                    "reason": "credential_quota_exhausted"
                },
                {
                    "kind": "upstream_failure_observed",
                    "request_id": "req_5xx",
                    "channel_id": "relay-b",
                    "failure": {
                        "failure_kind": "provider_unavailable",
                        "failure_source": "upstream_transport",
                        "status": 503,
                        "directive": "return_error",
                        "public_model": "gpt-example"
                    }
                },
                {
                    "kind": "upstream_failure_observed",
                    "request_id": "req_timeout",
                    "channel_id": "relay-b",
                    "failure": {
                        "failure_kind": "timeout",
                        "failure_source": "upstream_transport",
                        "directive": "retry",
                        "public_model": "gpt-example"
                    }
                }
            ]
        })
    }

    fn response_filter_fixture() -> Value {
        serde_json::json!({
            "buffered_events": 2,
            "offset": 0,
            "limit": 50,
            "events": [
                {
                    "outcome": "rejected",
                    "request_id": "req_filter",
                    "public_model": "gpt-example",
                    "channel_id": "relay-a",
                    "reason_code": "response_filter_rejected",
                    "body_committed": false,
                    "matched_text": "untrusted text must not be copied"
                },
                {
                    "outcome": "rejected",
                    "request_id": "req_stream",
                    "public_model": "gpt-example",
                    "channel_id": "relay-a",
                    "reason_code": "stream_committed_failure",
                    "body_committed": true,
                    "matched_text": "untrusted stream text must not be copied"
                }
            ]
        })
    }

    #[test]
    fn failures_tail_classifies_scope_route_credential_upstream_filter_and_post_output() {
        let rendered = render_tail_report(
            &routing_fixture(),
            &response_filter_fixture(),
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let failures = report["data"]["failures"].as_array().unwrap();

        assert_eq!(report["status"], "degraded");
        assert_eq!(report["reason_code"], "failures_found_in_window");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["calls_upstream"], false);
        assert_eq!(report["window"]["kind"], "bounded_recent_events");
        assert_eq!(report["window"]["limit"], 100);
        assert_eq!(report["window"]["per_source_limit"], 50);
        assert_eq!(report["window"]["returned"], 8);

        for request_id in [
            "req_scope",
            "req_no_route",
            "req_cooldown",
            "req_credential",
            "req_5xx",
            "req_timeout",
            "req_filter",
            "req_stream",
        ] {
            assert!(
                failures
                    .iter()
                    .any(|failure| failure["request_id"] == request_id),
                "{request_id} should be represented"
            );
        }
        assert!(failures.iter().any(|failure| {
            failure["request_id"] == "req_scope"
                && failure["stage"] == "model_visibility"
                && failure["failure_class"] == "model_not_visible"
                && failure["router_action"] == "returned_local_error"
        }));
        assert!(failures.iter().any(|failure| {
            failure["request_id"] == "req_no_route"
                && failure["stage"] == "route_planning"
                && failure["failure_class"] == "no_route_candidate"
        }));
        assert!(failures.iter().any(|failure| {
            failure["request_id"] == "req_5xx"
                && failure["stage"] == "upstream_transport"
                && failure["failure_class"] == "upstream_5xx"
        }));
        assert!(failures.iter().any(|failure| {
            failure["request_id"] == "req_filter"
                && failure["stage"] == "response_filter"
                && failure["failure_class"] == "response_filter_rejected"
                && failure["retry_eligibility"] == "eligible_before_output"
        }));
        assert!(failures.iter().any(|failure| {
            failure["request_id"] == "req_stream"
                && failure["stage"] == "post_output"
                && failure["failure_class"] == "stream_committed_failure"
                && failure["retry_eligibility"] == "blocked_streaming"
                && failure["retry_blocked_reason"] == "partial_output_started"
        }));
    }

    #[test]
    fn failures_explain_filters_bounded_events_by_request_model_channel_and_directive() {
        let rendered = render_explain_report(
            &routing_fixture(),
            &response_filter_fixture(),
            "req_5xx",
            &FailureFilters {
                request_id: None,
                public_model: Some("gpt-example".to_string()),
                channel_id: Some("relay-b".to_string()),
                directive: Some("return_error".to_string()),
            },
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "degraded");
        assert_eq!(report["reason_code"], "upstream_5xx");
        assert_eq!(report["scope"]["request_id"], "req_5xx");
        assert_eq!(report["scope"]["public_model"], "gpt-example");
        assert_eq!(report["scope"]["channel_id"], "relay-b");
        assert_eq!(report["data"]["failure_count"], 1);
        assert_eq!(report["data"]["evidence"][0]["request_id"], "req_5xx");
        assert_eq!(report["data"]["explanation"]["stage"], "upstream_transport");
        assert_eq!(
            report["data"]["explanation"]["failure_class"],
            "upstream_5xx"
        );
        assert_eq!(
            report["data"]["explanation"]["final_outcome"],
            "client_visible_failure"
        );
    }

    #[test]
    fn failures_explain_chooses_terminal_request_outcome_from_multiple_events() {
        let routing = serde_json::json!({
            "buffered_events": 2,
            "offset": 0,
            "limit": 50,
            "events": [
                {
                    "kind": "upstream_failure_observed",
                    "request_id": "req_multi",
                    "channel_id": "relay-a",
                    "failure": {
                        "failure_kind": "provider_unavailable",
                        "failure_source": "upstream_transport",
                        "status": 503,
                        "directive": "retry",
                        "public_model": "gpt-example"
                    }
                },
                {
                    "kind": "upstream_failure_observed",
                    "request_id": "req_multi",
                    "channel_id": "relay-b",
                    "failure": {
                        "failure_kind": "provider_unavailable",
                        "failure_source": "upstream_transport",
                        "status": 503,
                        "directive": "return_error",
                        "public_model": "gpt-example"
                    }
                }
            ]
        });
        let rendered = render_explain_report(
            &routing,
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "events": []}),
            "req_multi",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["data"]["failure_count"], 2);
        assert_eq!(
            report["data"]["explanation"]["client_visible_status"],
            "upstream_5xx"
        );
        assert_eq!(
            report["data"]["explanation"]["router_action"],
            "returned_local_error"
        );
        assert_eq!(report["reason_code"], "upstream_5xx");
        assert_eq!(
            report["data"]["explanation"]["final_outcome"],
            "client_visible_failure"
        );
    }

    #[test]
    fn failures_next_actions_use_safe_argv_and_do_not_suggest_probe_in_m2() {
        let rendered = render_explain_report(
            &routing_fixture(),
            &response_filter_fixture(),
            "req_cooldown",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let next_action = &report["next_action"];

        assert_eq!(next_action["template_id"], "failures_tail_channel");
        assert!(next_action["safe_argv"].is_array());
        let legacy_safe_field = ["safe", "command"].join("_");
        let legacy_dry_run_field = ["dry", "run", "command"].join("_");
        assert!(!rendered.contains(&legacy_safe_field));
        assert!(!rendered.contains(&legacy_dry_run_field));
        assert!(!rendered.contains("keys probe"));
    }

    #[test]
    fn failures_safe_argv_validation_rejects_unknown_templates_and_unsafe_args() {
        assert!(validate_safe_argv(vec![
            "one-ai-key".to_string(),
            "unknown".to_string(),
            "explain".to_string(),
            "--management-url".to_string(),
            "<url>".to_string(),
            "--management-token-env".to_string(),
            "<env>".to_string(),
        ])
        .is_empty());
        assert!(validate_safe_argv(vec![
            "one-ai-key".to_string(),
            "route".to_string(),
            "explain".to_string(),
            "--management-url".to_string(),
            "<url>".to_string(),
            "--management-token-env".to_string(),
            "<env>".to_string(),
            "bad/model".to_string(),
        ])
        .is_empty());
    }

    #[test]
    fn failures_model_visibility_next_action_uses_safe_client_token_ref_when_available() {
        let routing = serde_json::json!({
            "buffered_events": 1,
            "offset": 0,
            "limit": 50,
            "events": [
                {
                    "kind": "client_scope_miss",
                    "request_id": "req_scope_token",
                    "public_model": "gpt-example",
                    "client_token_ref": "local-client",
                    "client_visible_status": "local_404"
                }
            ]
        });
        let rendered = render_explain_report(
            &routing,
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "events": []}),
            "req_scope_token",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let argv = report["next_action"]["safe_argv"].as_array().unwrap();

        assert_eq!(
            report["next_action"]["template_id"],
            "models_explain_visibility"
        );
        assert!(argv.iter().any(|arg| arg == "local-client"));
        assert!(!rendered.contains("<client-token-ref>"));
    }

    #[test]
    fn failures_response_filter_lifecycle_action_maps_to_credential_mark() {
        let response_filter = serde_json::json!({
            "buffered_events": 1,
            "offset": 0,
            "limit": 50,
            "events": [
                {
                    "outcome": "rejected",
                    "request_id": "req_expire",
                    "public_model": "gpt-example",
                    "channel_id": "relay-a",
                    "action": "reject_and_expire_credential",
                    "reason_code": "response_filter_rejected",
                    "body_committed": false
                }
            ]
        });
        let rendered = render_explain_report(
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "events": []}),
            &response_filter,
            "req_expire",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            report["data"]["explanation"]["router_action"],
            "marked_credential"
        );
        assert_eq!(
            report["data"]["evidence"][0]["directive"],
            "reject_and_expire_credential"
        );
    }

    #[test]
    fn failures_output_redacts_untrusted_text_and_secret_like_values() {
        let routing = serde_json::json!({
            "buffered_events": 1,
            "offset": 0,
            "limit": 50,
            "events": [
                {
                    "kind": "upstream_failure_observed",
                    "request_id": "req_malicious\nansi\u{1b}",
                    "channel_id": "https://invalid.example/key-secret",
                    "failure": {
                        "failure_kind": "provider_unavailable",
                        "failure_source": "upstream_transport",
                        "status": 503,
                        "directive": "return_error",
                        "public_model": "http-model",
                        "message": "raw upstream body must not appear"
                    },
                    "raw_body": "raw request body must not appear"
                }
            ]
        });
        let response_filter = serde_json::json!({
            "buffered_events": 1,
            "offset": 0,
            "limit": 50,
            "events": [
                {
                    "outcome": "rejected",
                    "request_id": "req_filter",
                    "public_model": "gpt-example",
                    "channel_id": "relay-a",
                    "matched_text": "matched response filter text must not appear",
                    "body_committed": false
                }
            ]
        });

        let json = render_tail_report(
            &routing,
            &response_filter,
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let table = render_tail_report(
            &routing,
            &response_filter,
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Table,
        );
        for output in [&json, &table] {
            assert!(!output.contains("raw upstream body"));
            assert!(!output.contains("raw request body"));
            assert!(!output.contains("matched response filter text"));
            assert!(!output.contains("key-secret"));
            assert!(!output.contains("https://invalid.example"));
            assert!(!output.contains("http-model"));
            assert!(!output.contains('\u{1b}'));
        }
    }

    #[test]
    fn failures_invalid_filters_do_not_expand_the_result_window() {
        let rendered = render_tail_report(
            &routing_fixture(),
            &response_filter_fixture(),
            &FailureFilters {
                request_id: Some("req_scope/unsafe".to_string()),
                public_model: None,
                channel_id: None,
                directive: None,
            },
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["scope"]["request_id"], Value::Null);
        assert_eq!(report["data"]["failure_count"], 0);
        assert_eq!(report["data"]["failures"].as_array().unwrap().len(), 0);
        assert!(!rendered.contains("req_scope/unsafe"));
    }

    #[test]
    fn failures_table_preserves_json_taxonomy_fields() {
        let rendered = render_tail_report(
            &routing_fixture(),
            &response_filter_fixture(),
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Table,
        );

        assert!(rendered.contains("Status: degraded"));
        assert!(rendered.contains("Reason code: failures_found_in_window"));
        assert!(rendered.contains("Side effect: runtime_readonly"));
        assert!(rendered.contains("effect.reads_management_runtime: true"));
        assert!(rendered.contains("effect.reads_management_store: true"));
        assert!(rendered.contains("scope.request_id: unknown"));
        assert!(rendered.contains("window.limit: 100"));
        assert!(rendered.contains("Window: kind=bounded_recent_events"));
        assert!(rendered.contains("stage=upstream_transport"));
        assert!(rendered.contains("failure_class=upstream_5xx"));
        assert!(rendered.contains("retry_eligibility=blocked_streaming"));
        assert!(rendered.contains("client_visible_status=stream_committed_failure"));
        assert!(rendered.contains("Next action template:"));
        assert!(rendered.contains("Next action safe_argv[0]: one-ai-key"));
        assert!(rendered.contains("Next action safe_argv[1]:"));
    }

    #[test]
    fn failures_tail_uses_bounded_metadata_and_last_window_endpoints() {
        assert_eq!(
            parse_failure_last("200").expect("max should parse"),
            MAX_LAST
        );
        assert!(parse_failure_last("201").is_err());
        assert_eq!(
            tail_endpoint_sequence(Some(20), 110, 5),
            vec![
                crate::operator_client::ReadOnlyEndpoint::RoutingTelemetry {
                    offset: Some(0),
                    limit: Some(0),
                },
                crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
                    offset: Some(0),
                    limit: Some(0),
                },
                crate::operator_client::ReadOnlyEndpoint::RoutingTelemetry {
                    offset: Some(90),
                    limit: Some(20),
                },
                crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
                    offset: Some(0),
                    limit: Some(20),
                },
            ]
        );
    }

    #[test]
    fn failures_window_reports_combined_limit_for_two_bounded_sources() {
        let rendered = render_tail_report(
            &routing_fixture(),
            &response_filter_fixture(),
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["window"]["limit"], 100);
        assert_eq!(
            report["window"]["sources"]["routing_telemetry"]["limit"],
            50
        );
        assert_eq!(
            report["window"]["sources"]["response_filter_events"]["limit"],
            50
        );
    }
}
