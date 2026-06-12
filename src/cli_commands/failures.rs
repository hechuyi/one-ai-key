use serde_json::Value;

const DEFAULT_LAST: usize = 50;
const MAX_LAST: usize = 200;
const BOUNDED_EVIDENCE_AVAILABILITY_NOTE: &str =
    "bounded historical evidence only; current availability was not evaluated";

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
    pub endpoint_family: Option<String>,
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
    routing_capacity: usize,
    routing_dropped_events: u64,
    routing_offset: usize,
    routing_limit: usize,
    response_filter_buffered_events: usize,
    response_filter_capacity: usize,
    response_filter_dropped_events: u64,
    response_filter_offset: usize,
    response_filter_limit: usize,
}

impl FailureWindow {
    fn metadata(self, returned: usize) -> Value {
        let combined_limit = self
            .routing_limit
            .saturating_add(self.response_filter_limit);
        let dropped_events = self
            .routing_dropped_events
            .saturating_add(self.response_filter_dropped_events);
        serde_json::json!({
            "kind": "bounded_recent_events",
            "source": ["routing_telemetry", "response_filter_events"],
            "limit": combined_limit,
            "capacity": self.routing_capacity.saturating_add(self.response_filter_capacity),
            "dropped_events": dropped_events,
            "returned": returned,
            "truncated": self.routing_offset > 0 || self.response_filter_offset > 0,
            "cursor": Value::Null,
            "bounded_reason": "latest_window",
            "requested_last": self.requested_last,
            "per_source_limit": self.effective_last,
            "sources": {
                "routing_telemetry": {
                    "buffered_events": self.routing_buffered_events,
                    "capacity": self.routing_capacity,
                    "dropped_events": self.routing_dropped_events,
                    "offset": self.routing_offset,
                    "limit": self.routing_limit,
                },
                "response_filter_events": {
                    "buffered_events": self.response_filter_buffered_events,
                    "capacity": self.response_filter_capacity,
                    "dropped_events": self.response_filter_dropped_events,
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
        routing_capacity: metadata_usize(&routing_metadata, "capacity"),
        routing_dropped_events: metadata_u64(&routing_metadata, "dropped_events"),
        routing_offset,
        routing_limit: effective_last,
        response_filter_buffered_events,
        response_filter_capacity: metadata_usize(&response_filter_metadata, "capacity"),
        response_filter_dropped_events: metadata_u64(&response_filter_metadata, "dropped_events"),
        response_filter_offset,
        response_filter_limit: effective_last,
    };
    Ok((routing, response_filter, window))
}

fn buffered_events(snapshot: &Value) -> usize {
    metadata_usize(snapshot, "buffered_events")
}

fn metadata_usize(snapshot: &Value, key: &str) -> usize {
    snapshot
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn metadata_u64(snapshot: &Value, key: &str) -> u64 {
    snapshot.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
pub fn tail_endpoint_sequence(
    last: Option<usize>,
    routing_buffered_events: usize,
    response_filter_buffered_events: usize,
) -> Vec<crate::operator_client::ReadOnlyEndpoint> {
    let effective_last = last.unwrap_or(DEFAULT_LAST).clamp(1, MAX_LAST);
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
        routing_capacity: metadata_usize(routing, "capacity"),
        routing_dropped_events: metadata_u64(routing, "dropped_events"),
        routing_offset: routing.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
        routing_limit: routing.get("limit").and_then(Value::as_u64).unwrap_or(0) as usize,
        response_filter_buffered_events: buffered_events(response_filter),
        response_filter_capacity: metadata_usize(response_filter, "capacity"),
        response_filter_dropped_events: metadata_u64(response_filter, "dropped_events"),
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
    let failures = filtered_failures(routing, response_filter, window, filters);
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
    let failures = filtered_failures(routing, response_filter, window, filters);
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
        "availability_source": "bounded_evidence",
        "current_availability": false,
        "availability_note": BOUNDED_EVIDENCE_AVAILABILITY_NOTE,
        "next_action": aggregate_next_action(&failures),
        "data": data,
    })
}

fn request_explanation(failures: &[Value]) -> Value {
    let Some(primary) = primary_failure(failures) else {
        return serde_json::json!({
            "stage": "unknown",
            "endpoint_family": "unknown",
            "failure_class": "unknown",
            "route_kind": "unknown",
            "router_action": "none",
            "retry_eligibility": "not_applicable",
            "retry_blocked_reason": Value::Null,
            "client_visible_status": "not_found_in_window",
            "upstream_status": Value::Null,
            "admission": Value::Null,
            "final_outcome": "not_found_in_window",
        });
    };
    serde_json::json!({
        "stage": taxonomy_string(primary, "stage", "unknown"),
        "endpoint_family": taxonomy_string(primary, "endpoint_family", "unknown"),
        "public_model": taxonomy_string(primary, "public_model", "unknown"),
        "client_token_ref": primary.get("client_token_ref").cloned().unwrap_or(Value::Null),
        "selected_target": primary.get("selected_target").cloned().unwrap_or(Value::Null),
        "route_kind": taxonomy_string(primary, "route_kind", "unknown"),
        "failure_class": taxonomy_string(primary, "failure_class", "unknown"),
        "router_action": taxonomy_string(primary, "router_action", "none"),
        "retry_eligibility": taxonomy_string(primary, "retry_eligibility", "not_applicable"),
        "retry_blocked_reason": primary.get("retry_blocked_reason").cloned().unwrap_or(Value::Null),
        "client_visible_status": taxonomy_string(primary, "client_visible_status", "unknown"),
        "upstream_status": primary.get("upstream_status").cloned().unwrap_or(Value::Null),
        "admission": primary.get("admission").cloned().unwrap_or(Value::Null),
        "final_outcome": taxonomy_string(primary, "final_outcome", "unknown"),
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
    window: FailureWindow,
    filters: &FailureFilters,
) -> Vec<Value> {
    routing
        .get("failure_events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(projected_failure_event)
        .filter(|failure| matches_filters(failure, filters))
        .take(window.routing_limit)
        .chain(
            response_filter
                .get("failure_events")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(projected_failure_event)
                .filter(|failure| matches_filters(failure, filters))
                .take(window.response_filter_limit),
        )
        .collect::<Vec<_>>()
}

fn projected_failure_event(event: &Value) -> Option<Value> {
    if !event.is_object() {
        return None;
    }
    let reason_code = projected_string(event, "reason_code", "unknown_failure_class");
    Some(serde_json::json!({
        "source": projected_string(event, "source", "unknown"),
        "event_kind": projected_string(event, "event_kind", "unknown"),
        "request_id": projected_nullable_string(event, "request_id"),
        "stage": projected_string(event, "stage", "unknown"),
        "endpoint_family": projected_nullable_string(event, "endpoint_family"),
        "public_model": projected_string(event, "public_model", "unknown"),
        "client_token_ref": projected_nullable_string(event, "client_token_ref"),
        "selected_target": projected_selected_target(event.get("selected_target")),
        "channel_id": projected_nullable_string(event, "channel_id"),
        "route_kind": projected_nullable_string(event, "route_kind"),
        "failure_class": projected_string(event, "failure_class", "unknown"),
        "router_action": projected_string(event, "router_action", "none"),
        "retry_eligibility": projected_string(event, "retry_eligibility", "not_applicable"),
        "retry_blocked_reason": projected_nullable_string(event, "retry_blocked_reason"),
        "client_visible_status": projected_string(event, "client_visible_status", "unknown"),
        "upstream_status": projected_nullable_status(event, "upstream_status"),
        "final_outcome": projected_string(event, "final_outcome", "unknown"),
        "reason_code": reason_code,
        "blocking_domain": projected_string(event, "blocking_domain", "unknown"),
        "directive": projected_nullable_string(event, "directive"),
        "attempt": event.get("attempt").and_then(Value::as_u64),
        "content_kind": projected_nullable_string(event, "content_kind"),
        "admission": projected_admission(event.get("admission")),
        "next_action": projected_next_action(event.get("next_action"), &reason_code),
    }))
}

fn projected_string(event: &Value, field: &str, fallback: &'static str) -> String {
    event
        .get(field)
        .and_then(Value::as_str)
        .and_then(sanitize_local_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn projected_nullable_string(event: &Value, field: &str) -> Value {
    event
        .get(field)
        .and_then(Value::as_str)
        .and_then(sanitize_local_string)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn projected_nullable_status(event: &Value, field: &str) -> Value {
    event
        .get(field)
        .and_then(Value::as_u64)
        .filter(|status| (100..=599).contains(status))
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn projected_selected_target(value: Option<&Value>) -> Value {
    value
        .and_then(|target| target.get("channel_id"))
        .and_then(Value::as_str)
        .and_then(sanitize_local_string)
        .map(|channel_id| serde_json::json!({ "channel_id": channel_id }))
        .unwrap_or(Value::Null)
}

fn projected_admission(value: Option<&Value>) -> Value {
    let Some(object) = value.and_then(Value::as_object) else {
        return Value::Null;
    };
    let Some(candidate_count) = object.get("candidate_count").and_then(Value::as_u64) else {
        return Value::Null;
    };
    let Some(included_count) = object.get("included_count").and_then(Value::as_u64) else {
        return Value::Null;
    };
    let Some(blocked_count) = object.get("blocked_count").and_then(Value::as_u64) else {
        return Value::Null;
    };
    let Some(hard_blocked_count) = object.get("hard_blocked_count").and_then(Value::as_u64) else {
        return Value::Null;
    };
    let Some(soft_suppressed_count) = object.get("soft_suppressed_count").and_then(Value::as_u64)
    else {
        return Value::Null;
    };
    let Some(last_resort_used) = object.get("last_resort_used").and_then(Value::as_bool) else {
        return Value::Null;
    };

    serde_json::json!({
        "registry_generation": object.get("registry_generation").and_then(Value::as_u64),
        "status": object
            .get("status")
            .and_then(Value::as_str)
            .and_then(sanitize_local_string)
            .unwrap_or_else(|| "unknown".to_string()),
        "primary_reason_code": object
            .get("primary_reason_code")
            .and_then(Value::as_str)
            .and_then(sanitize_local_string)
            .unwrap_or_else(|| "unknown".to_string()),
        "candidate_count": candidate_count,
        "included_count": included_count,
        "blocked_count": blocked_count,
        "hard_blocked_count": hard_blocked_count,
        "soft_suppressed_count": soft_suppressed_count,
        "last_resort_used": last_resort_used,
        "hard_reason_codes": projected_reason_code_array(object.get("hard_reason_codes")),
        "soft_reason_codes": projected_reason_code_array(object.get("soft_reason_codes")),
    })
}

fn projected_reason_code_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(sanitize_local_string)
        .take(8)
        .collect()
}

fn projected_next_action(_value: Option<&Value>, reason_code: &str) -> Value {
    diagnostic_contract_for(reason_code).next_action
}

fn safe_next_action_shape(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    for field in ["summary", "template_id", "side_effect_class"] {
        if object
            .get(field)
            .and_then(Value::as_str)
            .and_then(sanitize_local_text)
            .is_none()
        {
            return false;
        }
    }
    if object
        .get("requires_confirmation")
        .and_then(Value::as_bool)
        .is_none()
    {
        return false;
    }
    object
        .get("safe_argv")
        .and_then(Value::as_array)
        .is_some_and(|argv| argv.iter().all(safe_next_action_arg))
}

fn safe_next_action_arg(value: &Value) -> bool {
    value.as_str().and_then(sanitize_local_text).is_some()
}

fn diagnostic_contract_for(reason_code: &str) -> crate::diagnostic_contract::DiagnosticContract {
    crate::diagnostic_contract::contract_for_reason(reason_code)
        .unwrap_or_else(crate::diagnostic_contract::fallback_contract)
}

fn aggregate_next_action(failures: &[Value]) -> Value {
    primary_failure(failures)
        .and_then(|failure| failure.get("next_action"))
        .filter(|next_action| safe_next_action_shape(next_action))
        .cloned()
        .unwrap_or_else(|| diagnostic_contract_for("no_failures_in_window").next_action)
}

fn primary_reason_code(failures: &[Value]) -> String {
    primary_failure(failures)
        .and_then(|failure| failure.get("reason_code"))
        .and_then(Value::as_str)
        .and_then(sanitize_local_string)
        .unwrap_or_else(|| "failures_found_in_window".to_string())
}

fn matches_filters(failure: &Value, filters: &FailureFilters) -> bool {
    matches_filter_field(
        failure.get("endpoint_family").and_then(Value::as_str),
        filters.endpoint_family.as_deref(),
    ) && matches_filter_field(
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
        "endpoint_family": filters.endpoint_family.as_deref().and_then(sanitize_local_string),
        "request_id": filters.request_id.as_deref().and_then(sanitize_local_string),
        "public_model": filters.public_model.as_deref().and_then(sanitize_local_string),
        "channel_id": filters.channel_id.as_deref().and_then(sanitize_local_string),
        "directive": filters.directive.as_deref().and_then(sanitize_local_string),
    })
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

fn sanitize_local_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_graphic() || byte == b' ')
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
        for field in [
            "kind",
            "limit",
            "per_source_limit",
            "returned",
            "truncated",
            "bounded_reason",
        ] {
            crate::cli_report::push_table_field(
                &mut output,
                &format!("window.{field}"),
                window.get(field),
            );
        }
        if let Some(sources) = window.get("sources").and_then(Value::as_object) {
            for source in ["routing_telemetry", "response_filter_events"] {
                if let Some(source_window) = sources.get(source) {
                    for field in [
                        "buffered_events",
                        "capacity",
                        "dropped_events",
                        "offset",
                        "limit",
                    ] {
                        crate::cli_report::push_table_field(
                            &mut output,
                            &format!("window.sources.{source}.{field}"),
                            source_window.get(field),
                        );
                    }
                }
            }
        }
        output.push_str(&format!(
            "Window: kind={} limit={} returned={} truncated={}\n",
            table_str(window.get("kind")),
            table_str(window.get("limit")),
            table_str(window.get("returned")),
            table_str(window.get("truncated")),
        ));
    }
    crate::cli_report::push_table_field(
        &mut output,
        "availability_source",
        report.get("availability_source"),
    );
    crate::cli_report::push_table_field(
        &mut output,
        "current_availability",
        report.get("current_availability"),
    );
    crate::cli_report::push_table_field(
        &mut output,
        "availability_note",
        report.get("availability_note"),
    );
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
            "- request_id={} stage={} endpoint_family={} failure_class={} blocking_domain={} router_action={} retry_eligibility={} retry_blocked_reason={} client_visible_status={} upstream_status={} reason_code={} model={} channel={} directive={} admission_candidate_count={} admission_included_count={}\n",
            table_str(failure.get("request_id")),
            table_str(failure.get("stage")),
            table_str(failure.get("endpoint_family")),
            table_str(failure.get("failure_class")),
            table_str(failure.get("blocking_domain")),
            table_str(failure.get("router_action")),
            table_str(failure.get("retry_eligibility")),
            table_str(failure.get("retry_blocked_reason")),
            table_str(failure.get("client_visible_status")),
            table_str(failure.get("upstream_status")),
            table_str(failure.get("reason_code")),
            table_str(failure.get("public_model")),
            table_str(failure.get("channel_id")),
            table_str(failure.get("directive")),
            table_str(
                failure
                    .get("admission")
                    .and_then(|admission| admission.get("candidate_count"))
            ),
            table_str(
                failure
                    .get("admission")
                    .and_then(|admission| admission.get("included_count"))
            ),
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

    #[allow(clippy::too_many_arguments)]
    fn projected_failure(
        source: &str,
        event_kind: &str,
        request_id: &str,
        stage: &str,
        public_model: &str,
        channel_id: Option<&str>,
        failure_class: &str,
        router_action: &str,
        retry_eligibility: &str,
        retry_blocked_reason: Option<&str>,
        client_visible_status: &str,
        final_outcome: &str,
        reason_code: &str,
        directive: Option<&str>,
        attempt: Option<u64>,
    ) -> Value {
        let contract = crate::diagnostic_contract::contract_for_reason(reason_code)
            .unwrap_or_else(crate::diagnostic_contract::fallback_contract);
        serde_json::json!({
            "source": source,
            "event_kind": event_kind,
            "request_id": request_id,
            "stage": stage,
            "endpoint_family": "chat_completions",
            "public_model": public_model,
            "client_token_ref": Value::Null,
            "selected_target": channel_id
                .map(|channel_id| serde_json::json!({ "channel_id": channel_id }))
                .unwrap_or(Value::Null),
            "channel_id": channel_id,
            "failure_class": failure_class,
            "router_action": router_action,
            "retry_eligibility": retry_eligibility,
            "retry_blocked_reason": retry_blocked_reason,
            "client_visible_status": client_visible_status,
            "final_outcome": final_outcome,
            "reason_code": reason_code,
            "blocking_domain": contract.blocking_domain,
            "directive": directive,
            "attempt": attempt,
            "next_action": contract.next_action,
        })
    }

    fn routing_fixture() -> Value {
        serde_json::json!({
            "buffered_events": 6,
            "capacity": 1024,
            "dropped_events": 2,
            "offset": 0,
            "limit": 50,
            "failure_events": [
                projected_failure("routing_telemetry", "client_scope_miss", "req_scope", "model_visibility", "gpt-example", None, "model_not_visible", "returned_local_error", "not_applicable", None, "local_404", "client_visible_failure", "model_not_in_client_scope", None, None),
                projected_failure("routing_telemetry", "no_route_candidate", "req_no_route", "route_planning", "gpt-missing", None, "no_route_candidate", "returned_local_error", "not_applicable", None, "local_404", "client_visible_failure", "no_route_candidate", None, None),
                projected_failure("routing_telemetry", "channel_health_transition_applied", "req_cooldown", "credential_selection", "unknown", Some("relay-a"), "credential_unavailable", "marked_channel", "not_applicable", None, "not_applicable", "management_or_lifecycle_event", "channel_cooling_down", None, None),
                projected_failure("routing_telemetry", "credential_transition_applied", "req_credential", "credential_selection", "unknown", Some("relay-a"), "credential_unavailable", "marked_credential", "not_applicable", None, "not_applicable", "management_or_lifecycle_event", "credential_unavailable", None, None),
                projected_failure("routing_telemetry", "upstream_failure_observed", "req_5xx", "upstream_transport", "gpt-example", Some("relay-b"), "upstream_5xx", "returned_local_error", "not_applicable", None, "upstream_5xx", "client_visible_failure", "upstream_5xx", Some("return_error"), Some(0)),
                projected_failure("routing_telemetry", "upstream_failure_observed", "req_timeout", "upstream_transport", "gpt-example", Some("relay-b"), "upstream_timeout", "retried_before_output", "eligible_before_output", None, "not_applicable_retry_before_output", "recovered_before_client_output", "upstream_timeout", Some("retry"), Some(0))
            ],
            "events": []
        })
    }

    fn response_filter_fixture() -> Value {
        serde_json::json!({
            "buffered_events": 2,
            "capacity": 1024,
            "dropped_events": 3,
            "offset": 0,
            "limit": 50,
            "failure_events": [
                projected_failure("response_filter_events", "response_filter_rejected", "req_filter", "response_filter", "gpt-example", Some("relay-a"), "response_filter_rejected", "recorded_event_only", "eligible_before_output", None, "local_502", "client_visible_failure", "response_filter_rejected", Some("reject"), None),
                projected_failure("response_filter_events", "response_filter_rejected", "req_stream", "post_output", "gpt-example", Some("relay-a"), "stream_committed_failure", "recorded_event_only", "blocked_streaming", Some("partial_output_started"), "stream_committed_failure", "client_visible_failure", "stream_committed_failure", Some("reject"), None)
            ],
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
    fn failures_tail_renders_management_projected_failure_events_instead_of_raw_events() {
        let routing = serde_json::json!({
            "buffered_events": 1,
            "capacity": 1024,
            "dropped_events": 0,
            "offset": 0,
            "limit": 50,
            "failure_events": [
                {
                    "source": "routing_telemetry",
                    "event_kind": "upstream_failure_observed",
                    "request_id": "req_projected",
                    "stage": "upstream_transport",
                    "public_model": "gpt-projected",
                    "client_token_ref": null,
                    "selected_target": {"channel_id": "relay-projected"},
                    "channel_id": "relay-projected",
                    "failure_class": "upstream_5xx",
                    "router_action": "returned_local_error",
                    "retry_eligibility": "not_applicable",
                    "retry_blocked_reason": null,
                    "client_visible_status": "upstream_5xx",
                    "reason_code": "upstream_5xx",
                    "blocking_domain": "upstream_provider",
                    "directive": "return_error",
                    "attempt": 1,
                    "next_action": {
                        "summary": "Inspect recent failures.",
                        "template_id": "failures_tail",
                        "safe_argv": ["one-ai-key", "failures", "tail"],
                        "side_effect_class": "runtime_readonly",
                        "requires_confirmation": false
                    }
                }
            ],
            "events": [
                {
                    "kind": "upstream_failure_observed",
                    "request_id": "req_raw_should_be_ignored",
                    "channel_id": "relay-raw",
                    "failure": {
                        "failure_kind": "timeout",
                        "failure_source": "upstream_transport",
                        "directive": "retry",
                        "public_model": "gpt-raw"
                    }
                }
            ]
        });
        let rendered = render_tail_report(
            &routing,
            &serde_json::json!({
                "buffered_events": 0,
                "capacity": 1024,
                "dropped_events": 0,
                "offset": 0,
                "limit": 50,
                "failure_events": [],
                "events": []
            }),
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let failures = report["data"]["failures"].as_array().unwrap();

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["request_id"], "req_projected");
        assert_eq!(failures[0]["failure_class"], "upstream_5xx");
        assert!(!rendered.contains("req_raw_should_be_ignored"));
        assert!(!rendered.contains("gpt-raw"));
    }

    fn oversized_routing_fixture(count: usize, request_id: &str) -> Value {
        let failure_events = (0..count)
            .map(|index| {
                projected_failure(
                    "routing_telemetry",
                    "upstream_failure_observed",
                    request_id,
                    "upstream_transport",
                    &format!("gpt-example-{index}"),
                    Some("relay-a"),
                    "upstream_5xx",
                    "returned_local_error",
                    "not_applicable",
                    None,
                    "upstream_5xx",
                    "client_visible_failure",
                    "upstream_5xx",
                    Some("return_error"),
                    Some(0),
                )
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "buffered_events": count,
            "capacity": 1024,
            "dropped_events": 0,
            "offset": 0,
            "limit": MAX_LAST,
            "failure_events": failure_events,
            "events": [],
        })
    }

    fn oversized_response_filter_fixture(count: usize, request_id: &str) -> Value {
        let failure_events = (0..count)
            .map(|index| {
                projected_failure(
                    "response_filter_events",
                    "response_filter_rejected",
                    request_id,
                    "response_filter",
                    &format!("gpt-example-{index}"),
                    Some("relay-a"),
                    "response_filter_rejected",
                    "recorded_event_only",
                    "eligible_before_output",
                    None,
                    "local_502",
                    "client_visible_failure",
                    "response_filter_rejected",
                    Some("reject"),
                    None,
                )
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "buffered_events": count,
            "capacity": 1024,
            "dropped_events": 0,
            "offset": 0,
            "limit": MAX_LAST,
            "failure_events": failure_events,
            "events": [],
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
    fn failures_tail_table_marks_bounded_evidence_not_current_availability() {
        let rendered = render_tail_report(
            &routing_fixture(),
            &response_filter_fixture(),
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Table,
        );

        assert!(rendered.contains("availability_source: bounded_evidence"));
        assert!(rendered.contains("current_availability: false"));
        assert!(rendered.contains(
            "availability_note: bounded historical evidence only; current availability was not evaluated"
        ));
        assert!(rendered.contains("window.per_source_limit: 50"));
        assert!(rendered.contains("window.sources.routing_telemetry.dropped_events: 2"));
        assert!(rendered.contains("window.sources.response_filter_events.dropped_events: 3"));
        assert!(rendered.contains("blocking_domain=upstream"));
    }

    #[test]
    fn failures_explain_filters_bounded_events_by_request_model_channel_and_directive() {
        let rendered = render_explain_report(
            &routing_fixture(),
            &response_filter_fixture(),
            "req_5xx",
            &FailureFilters {
                endpoint_family: None,
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
    fn failures_explain_preserves_local_admission_denial_evidence_and_upstream_status_boundary() {
        let routing = serde_json::json!({
            "buffered_events": 2,
            "offset": 0,
            "limit": 50,
            "failure_events": [
                {
                    "source": "routing_telemetry",
                    "event_kind": "route_admission_denied",
                    "request_id": "req_local_503",
                    "stage": "route_admission",
                    "endpoint_family": "responses",
                    "public_model": "gpt-route",
                    "client_token_ref": "local-client",
                    "route_kind": "explicit_model_route",
                    "failure_class": "route_admission_denied",
                    "router_action": "returned_local_error",
                    "retry_eligibility": "not_applicable",
                    "retry_blocked_reason": null,
                    "client_visible_status": "local_503",
                    "upstream_status": null,
                    "final_outcome": "client_visible_failure",
                    "reason_code": "no_route_candidate",
                    "blocking_domain": "route",
                    "admission": {
                        "registry_generation": 9,
                        "status": "unavailable",
                        "primary_reason_code": "no_route_candidate",
                        "candidate_count": 3,
                        "included_count": 0,
                        "blocked_count": 3,
                        "hard_blocked_count": 2,
                        "soft_suppressed_count": 1,
                        "last_resort_used": false,
                        "hard_reason_codes": ["channel_cooling_down", "no_available_credentials"],
                        "soft_reason_codes": ["provider_cooling_down"]
                    },
                    "next_action": {
                        "summary": "Inspect route explanation.",
                        "template_id": "route_explain",
                        "safe_argv": ["one-ai-key", "route", "explain", "<model>"],
                        "side_effect_class": "runtime_readonly",
                        "requires_confirmation": false
                    }
                },
                {
                    "source": "routing_telemetry",
                    "event_kind": "upstream_failure_observed",
                    "request_id": "req_upstream_503",
                    "stage": "upstream_transport",
                    "public_model": "gpt-route",
                    "client_token_ref": "local-client",
                    "selected_target": {"channel_id": "relay-a"},
                    "channel_id": "relay-a",
                    "failure_class": "upstream_5xx",
                    "router_action": "returned_local_error",
                    "retry_eligibility": "not_applicable",
                    "retry_blocked_reason": null,
                    "client_visible_status": "upstream_5xx",
                    "upstream_status": 503,
                    "final_outcome": "client_visible_failure",
                    "reason_code": "upstream_5xx",
                    "blocking_domain": "upstream_provider",
                    "directive": "return_error",
                    "attempt": 0,
                    "next_action": {
                        "summary": "Inspect recent failures.",
                        "template_id": "failures_tail",
                        "safe_argv": ["one-ai-key", "failures", "tail"],
                        "side_effect_class": "runtime_readonly",
                        "requires_confirmation": false
                    }
                }
            ],
            "events": []
        });
        let empty_filter = serde_json::json!({
            "buffered_events": 0,
            "offset": 0,
            "limit": 50,
            "failure_events": [],
            "events": []
        });

        let local_rendered = render_explain_report(
            &routing,
            &empty_filter,
            "req_local_503",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let local_report: Value = serde_json::from_str(&local_rendered).unwrap();
        let local_failure = &local_report["data"]["evidence"][0];

        assert_eq!(local_report["reason_code"], "no_route_candidate");
        assert_eq!(
            local_report["data"]["explanation"]["endpoint_family"],
            "responses"
        );
        assert_eq!(
            local_report["data"]["explanation"]["route_kind"],
            "explicit_model_route"
        );
        assert_eq!(local_failure["event_kind"], "route_admission_denied");
        assert_eq!(local_failure["stage"], "route_admission");
        assert_eq!(local_failure["endpoint_family"], "responses");
        assert_eq!(local_failure["route_kind"], "explicit_model_route");
        assert_eq!(local_failure["client_visible_status"], "local_503");
        assert!(local_failure["upstream_status"].is_null());
        assert_eq!(local_failure["admission"]["candidate_count"], 3);
        assert_eq!(local_failure["admission"]["status"], "unavailable");
        assert_eq!(
            local_failure["admission"]["primary_reason_code"],
            "no_route_candidate"
        );
        assert_eq!(local_failure["admission"]["included_count"], 0);
        assert_eq!(
            local_failure["admission"]["hard_reason_codes"][0],
            "channel_cooling_down"
        );

        let upstream_rendered = render_explain_report(
            &routing,
            &empty_filter,
            "req_upstream_503",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let upstream_report: Value = serde_json::from_str(&upstream_rendered).unwrap();
        let upstream_failure = &upstream_report["data"]["evidence"][0];

        assert_eq!(upstream_report["reason_code"], "upstream_5xx");
        assert_eq!(upstream_failure["client_visible_status"], "upstream_5xx");
        assert_eq!(upstream_failure["upstream_status"], 503);
        assert!(upstream_failure["admission"].is_null());

        let local_table = render_explain_report(
            &routing,
            &empty_filter,
            "req_local_503",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Table,
        );
        assert!(local_table.contains("upstream_status=null"));
        assert!(local_table.contains("admission_candidate_count=3"));
        assert!(local_table.contains("admission_included_count=0"));
    }

    #[test]
    fn failures_explain_table_marks_bounded_evidence_not_current_availability() {
        let rendered = render_explain_report(
            &routing_fixture(),
            &response_filter_fixture(),
            "req_5xx",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Table,
        );

        assert!(rendered.contains("availability_source: bounded_evidence"));
        assert!(rendered.contains("current_availability: false"));
        assert!(rendered.contains(
            "availability_note: bounded historical evidence only; current availability was not evaluated"
        ));
        assert!(rendered.contains("window.per_source_limit: 50"));
        assert!(rendered.contains("window.sources.routing_telemetry.dropped_events: 2"));
        assert!(rendered.contains("window.sources.response_filter_events.dropped_events: 3"));
        assert!(rendered.contains("blocking_domain=upstream"));
    }

    #[test]
    fn failures_preserves_m3_retry_directives_for_display_and_filtering() {
        let routing = serde_json::json!({
            "buffered_events": 3,
            "offset": 0,
            "limit": 50,
            "failure_events": [
                projected_failure("routing_telemetry", "upstream_failure_observed", "req_retry_credential", "upstream_transport", "gpt-example", Some("relay-a"), "upstream_5xx", "retried_before_output", "eligible_before_output", None, "not_applicable_retry_before_output", "recovered_before_client_output", "upstream_5xx", Some("retry_credential"), Some(0)),
                projected_failure("routing_telemetry", "upstream_failure_observed", "req_retry_route", "upstream_transport", "gpt-example", Some("relay-a"), "upstream_5xx", "fell_back_before_output", "eligible_before_output", None, "not_applicable_retry_before_output", "recovered_before_client_output", "upstream_5xx", Some("retry_route_target"), Some(0)),
                projected_failure("routing_telemetry", "upstream_failure_observed", "req_retry_same", "upstream_transport", "gpt-example", Some("relay-a"), "upstream_timeout", "retried_before_output", "eligible_before_output", None, "not_applicable_retry_before_output", "recovered_before_client_output", "upstream_timeout", Some("retry_same_target"), Some(0))
            ],
            "events": []
        });
        let response_filter = serde_json::json!({
            "buffered_events": 0,
            "offset": 0,
            "limit": 50,
            "failure_events": [],
            "events": []
        });

        let rendered = render_tail_report(
            &routing,
            &response_filter,
            &FailureFilters {
                endpoint_family: None,
                request_id: None,
                public_model: Some("gpt-example".to_string()),
                channel_id: Some("relay-a".to_string()),
                directive: Some("retry_route_target".to_string()),
            },
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let failures = report["data"]["failures"].as_array().unwrap();

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["request_id"], "req_retry_route");
        assert_eq!(failures[0]["directive"], "retry_route_target");
        assert_eq!(failures[0]["router_action"], "fell_back_before_output");
        assert_eq!(failures[0]["retry_eligibility"], "eligible_before_output");
        assert_eq!(
            failures[0]["client_visible_status"],
            "not_applicable_retry_before_output"
        );

        let rendered = render_tail_report(
            &routing,
            &response_filter,
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let failures = report["data"]["failures"].as_array().unwrap();
        for directive in [
            "retry_credential",
            "retry_route_target",
            "retry_same_target",
        ] {
            assert!(
                failures.iter().any(|failure| {
                    failure["directive"] == directive
                        && failure["router_action"] != "returned_local_error"
                        && failure["retry_eligibility"] == "eligible_before_output"
                }),
                "{directive} should be preserved as bounded retry evidence"
            );
        }
    }

    #[test]
    fn failures_tail_filters_bounded_events_by_endpoint_family() {
        let routing = serde_json::json!({
            "buffered_events": 2,
            "offset": 0,
            "limit": 50,
            "failure_events": [
                projected_failure("routing_telemetry", "upstream_failure_observed", "req_chat", "upstream_transport", "gpt-example", Some("relay-a"), "upstream_5xx", "returned_local_error", "not_applicable", None, "upstream_5xx", "client_visible_failure", "upstream_5xx", Some("return_error"), Some(0)),
                {
                    "source": "routing_telemetry",
                    "event_kind": "route_admission_denied",
                    "request_id": "req_responses",
                    "stage": "route_admission",
                    "endpoint_family": "responses",
                    "public_model": "gpt-example",
                    "client_token_ref": null,
                    "selected_target": null,
                    "channel_id": null,
                    "route_kind": "explicit_model_route",
                    "failure_class": "route_admission_denied",
                    "router_action": "returned_local_error",
                    "retry_eligibility": "not_applicable",
                    "retry_blocked_reason": null,
                    "client_visible_status": "local_503",
                    "upstream_status": null,
                    "final_outcome": "client_visible_failure",
                    "reason_code": "no_route_candidate",
                    "blocking_domain": "route",
                    "directive": null,
                    "attempt": null,
                    "admission": {
                        "registry_generation": 9,
                        "candidate_count": 1,
                        "included_count": 0,
                        "blocked_count": 1,
                        "hard_blocked_count": 1,
                        "soft_suppressed_count": 0,
                        "last_resort_used": false,
                        "hard_reason_codes": ["channel_cooling_down"],
                        "soft_reason_codes": []
                    },
                    "next_action": {
                        "summary": "Inspect route explanation.",
                        "template_id": "route_explain",
                        "safe_argv": ["one-ai-key", "route", "explain", "<model>"],
                        "side_effect_class": "runtime_readonly",
                        "requires_confirmation": false
                    }
                }
            ],
            "events": []
        });
        let response_filter = serde_json::json!({
            "buffered_events": 0,
            "offset": 0,
            "limit": 50,
            "failure_events": [],
            "events": []
        });

        let rendered = render_tail_report(
            &routing,
            &response_filter,
            &FailureFilters {
                endpoint_family: Some("responses".to_string()),
                request_id: None,
                public_model: Some("gpt-example".to_string()),
                channel_id: None,
                directive: None,
            },
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let failures = report["data"]["failures"].as_array().unwrap();

        assert_eq!(report["scope"]["endpoint_family"], "responses");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["request_id"], "req_responses");
        assert_eq!(failures[0]["endpoint_family"], "responses");
        assert_eq!(failures[0]["route_kind"], "explicit_model_route");
        assert!(!rendered.contains("req_chat"));
    }

    #[test]
    fn failures_explain_endpoint_family_filter_prevents_cross_family_request_matches() {
        let rendered = render_explain_report(
            &routing_fixture(),
            &response_filter_fixture(),
            "req_5xx",
            &FailureFilters {
                endpoint_family: Some("responses".to_string()),
                request_id: None,
                public_model: None,
                channel_id: None,
                directive: None,
            },
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "not_found");
        assert_eq!(report["reason_code"], "request_failure_not_found_in_window");
        assert_eq!(report["scope"]["request_id"], "req_5xx");
        assert_eq!(report["scope"]["endpoint_family"], "responses");
        assert_eq!(report["data"]["failure_count"], 0);
        assert_eq!(report["data"]["explanation"]["endpoint_family"], "unknown");
    }

    #[test]
    fn failures_explain_chooses_terminal_request_outcome_from_multiple_events() {
        let routing = serde_json::json!({
            "buffered_events": 2,
            "offset": 0,
            "limit": 50,
            "failure_events": [
                projected_failure("routing_telemetry", "upstream_failure_observed", "req_multi", "upstream_transport", "gpt-example", Some("relay-a"), "upstream_5xx", "retried_before_output", "eligible_before_output", None, "not_applicable_retry_before_output", "recovered_before_client_output", "upstream_5xx", Some("retry"), Some(0)),
                projected_failure("routing_telemetry", "upstream_failure_observed", "req_multi", "upstream_transport", "gpt-example", Some("relay-b"), "upstream_5xx", "returned_local_error", "not_applicable", None, "upstream_5xx", "client_visible_failure", "upstream_5xx", Some("return_error"), Some(0))
            ],
            "events": []
        });
        let rendered = render_explain_report(
            &routing,
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "failure_events": [], "events": []}),
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

        assert_eq!(next_action["template_id"], "failures_tail");
        assert!(next_action["safe_argv"].is_array());
        let legacy_safe_field = ["safe", "command"].join("_");
        let legacy_dry_run_field = ["dry", "run", "command"].join("_");
        assert!(!rendered.contains(&legacy_safe_field));
        assert!(!rendered.contains(&legacy_dry_run_field));
        assert!(!rendered.contains("keys probe"));
    }

    #[test]
    fn failures_safe_argv_validation_rejects_unknown_templates_and_unsafe_args() {
        assert!(!crate::diagnostic_contract::is_valid_safe_next_action(
            &serde_json::json!({
                "summary": "unsafe",
                "template_id": "unknown_explain",
                "safe_argv": ["one-ai-key", "unknown", "explain", "--management-url", "<url>", "--management-token-env", "<env>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            })
        ));
        assert!(!crate::diagnostic_contract::is_valid_safe_next_action(
            &serde_json::json!({
                "summary": "unsafe",
                "template_id": "route_explain",
                "safe_argv": ["one-ai-key", "route", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "/tmp/private-model"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false
            })
        ));
    }

    #[test]
    fn failures_rejects_projected_next_action_not_in_diagnostic_allowlist() {
        let injected_next_action = serde_json::json!({
            "summary": "Looks read-only but is not a registered diagnostic contract.",
            "template_id": "unknown_explain",
            "safe_argv": [
                "one-ai-key",
                "doctor",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>"
            ],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false
        });
        let mut failure = projected_failure(
            "routing_telemetry",
            "no_route_candidate",
            "req_unknown_next_action",
            "route_planning",
            "gpt-example",
            None,
            "no_route_candidate",
            "returned_local_error",
            "not_applicable",
            None,
            "local_503",
            "client_visible_failure",
            "no_route_candidate",
            None,
            None,
        );
        failure["next_action"] = injected_next_action;
        let routing = serde_json::json!({
            "buffered_events": 1,
            "capacity": 1024,
            "dropped_events": 0,
            "offset": 0,
            "limit": 50,
            "failure_events": [failure],
            "events": []
        });

        let rendered = render_explain_report(
            &routing,
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "failure_events": [], "events": []}),
            "req_unknown_next_action",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let contract =
            crate::diagnostic_contract::contract_for_reason("no_route_candidate").unwrap();

        assert_eq!(report["next_action"], contract.next_action);
        assert_eq!(
            report["data"]["evidence"][0]["next_action"],
            contract.next_action
        );
        assert!(!rendered.contains("unknown_explain"));
        assert!(!rendered.contains("Looks read-only"));
    }

    #[test]
    fn failures_model_visibility_next_action_uses_safe_client_token_ref_when_available() {
        let mut failure = projected_failure(
            "routing_telemetry",
            "client_scope_miss",
            "req_scope_token",
            "model_visibility",
            "gpt-example",
            None,
            "model_not_visible",
            "returned_local_error",
            "not_applicable",
            None,
            "local_404",
            "client_visible_failure",
            "model_not_in_client_scope",
            None,
            None,
        );
        failure["client_token_ref"] = serde_json::json!("local-client");
        let routing = serde_json::json!({
            "buffered_events": 1,
            "offset": 0,
            "limit": 50,
            "failure_events": [failure],
            "events": []
        });
        let rendered = render_explain_report(
            &routing,
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "failure_events": [], "events": []}),
            "req_scope_token",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let contract =
            crate::diagnostic_contract::contract_for_reason("model_not_in_client_scope").unwrap();
        let argv = report["next_action"]["safe_argv"].as_array().unwrap();

        assert_eq!(report["next_action"], contract.next_action);
        assert!(rendered.contains("<client-token-ref>"));
        assert!(!argv.iter().any(|arg| arg == "local-client"));
    }

    #[test]
    fn failures_explain_is_bounded_evidence_and_uses_diagnostic_contract() {
        let rendered = render_explain_report(
            &routing_fixture(),
            &response_filter_fixture(),
            "req_scope",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let contract = crate::diagnostic_contract::contract_for_reason("model_not_in_client_scope")
            .expect("stage 2 failure reason should have a diagnostic contract");

        assert_eq!(report["availability_source"], "bounded_evidence");
        assert_eq!(report["current_availability"], false);
        assert_eq!(
            report["data"]["evidence"][0]["blocking_domain"],
            contract.blocking_domain
        );
        assert_eq!(report["next_action"], contract.next_action);
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "runtime_readonly"
        );
        assert_eq!(report["next_action"]["requires_confirmation"], false);
    }

    #[test]
    fn failures_response_filter_lifecycle_action_maps_to_credential_mark() {
        let response_filter = serde_json::json!({
            "buffered_events": 1,
            "offset": 0,
            "limit": 50,
            "failure_events": [
                projected_failure("response_filter_events", "response_filter_rejected", "req_expire", "response_filter", "gpt-example", Some("relay-a"), "response_filter_rejected", "marked_credential", "eligible_before_output", None, "local_502", "client_visible_failure", "response_filter_rejected", Some("reject_and_expire_credential"), None)
            ],
            "events": []
        });
        let rendered = render_explain_report(
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "failure_events": [], "events": []}),
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
                endpoint_family: None,
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
        assert!(rendered.contains("endpoint_family=chat_completions"));
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
        assert_eq!(report["window"]["dropped_events"], 5);
        assert_eq!(
            report["window"]["sources"]["routing_telemetry"]["limit"],
            50
        );
        assert_eq!(
            report["window"]["sources"]["routing_telemetry"]["capacity"],
            1024
        );
        assert_eq!(
            report["window"]["sources"]["routing_telemetry"]["dropped_events"],
            2
        );
        assert_eq!(
            report["window"]["sources"]["response_filter_events"]["limit"],
            50
        );
        assert_eq!(
            report["window"]["sources"]["response_filter_events"]["capacity"],
            1024
        );
        assert_eq!(
            report["window"]["sources"]["response_filter_events"]["dropped_events"],
            3
        );
    }

    #[test]
    fn failures_tail_never_returns_more_than_the_bounded_source_windows() {
        let rendered = render_tail_report(
            &oversized_routing_fixture(MAX_LAST + 50, "req_many"),
            &oversized_response_filter_fixture(MAX_LAST + 50, "req_many"),
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let failures = report["data"]["failures"].as_array().unwrap();

        assert_eq!(report["window"]["limit"], serde_json::json!(MAX_LAST * 2));
        assert_eq!(
            report["window"]["returned"],
            serde_json::json!(MAX_LAST * 2)
        );
        assert_eq!(
            report["data"]["failure_count"],
            serde_json::json!(MAX_LAST * 2)
        );
        assert_eq!(failures.len(), MAX_LAST * 2);
    }

    #[test]
    fn failures_explain_never_returns_more_than_the_bounded_source_windows() {
        let rendered = render_explain_report(
            &oversized_routing_fixture(MAX_LAST + 50, "req_many"),
            &oversized_response_filter_fixture(MAX_LAST + 50, "req_many"),
            "req_many",
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let evidence = report["data"]["evidence"].as_array().unwrap();

        assert_eq!(report["window"]["limit"], serde_json::json!(MAX_LAST * 2));
        assert_eq!(
            report["window"]["returned"],
            serde_json::json!(MAX_LAST * 2)
        );
        assert_eq!(
            report["data"]["failure_count"],
            serde_json::json!(MAX_LAST * 2)
        );
        assert_eq!(evidence.len(), MAX_LAST * 2);
    }

    #[test]
    fn failures_window_treats_missing_capacity_and_dropped_events_as_zero() {
        let rendered = render_tail_report(
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "events": []}),
            &serde_json::json!({"buffered_events": 0, "offset": 0, "limit": 50, "events": []}),
            &FailureFilters::default(),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["window"]["dropped_events"], 0);
        assert_eq!(
            report["window"]["sources"]["routing_telemetry"]["capacity"],
            0
        );
        assert_eq!(
            report["window"]["sources"]["routing_telemetry"]["dropped_events"],
            0
        );
        assert_eq!(
            report["window"]["sources"]["response_filter_events"]["capacity"],
            0
        );
        assert_eq!(
            report["window"]["sources"]["response_filter_events"]["dropped_events"],
            0
        );
    }
}
