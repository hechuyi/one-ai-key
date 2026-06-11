use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    client_token_store::ClientTokenStoreHandle,
    credential_repository::CredentialStoreHandle,
    credentials::short_hash,
    events::{
        ManagementAuditEvent, ManagementEventActor, ResponseFilterEvent, RetryPressureCounters,
        RoutingTelemetry, UpstreamFailureTelemetry,
    },
    management_alerts::{
        model_route_all_target_suppression_alerts, response_filter_contamination_alerts_for_state,
        ManagementAlertStatus,
    },
    management_errors::{registry_store_error, ManagementServiceError},
    management_registry::resolve_staged_registry_document,
    management_status::{
        credential_pool_alerts, runtime_readiness_projection, CredentialPoolAlertStatus,
        CredentialSetSnapshotCache, RuntimeCredentialCounts, RuntimeReadinessProjection,
    },
    registry_store::RegistryStoreHandle,
    state::{AppState, ChannelHealth, RuntimeTopologySummary},
};

#[derive(Debug, Serialize)]
pub struct ResponseFilterEventsResponse {
    pub buffered_events: usize,
    pub capacity: usize,
    pub dropped_events: u64,
    pub offset: usize,
    pub limit: usize,
    pub failure_events: Vec<Value>,
    pub events: Vec<Value>,
}

pub fn response_filter_events_response(
    snapshot: Vec<ResponseFilterEvent>,
    capacity: usize,
    dropped_events: u64,
    offset: usize,
    limit: usize,
) -> ResponseFilterEventsResponse {
    let buffered_events = snapshot.len();
    let window = snapshot
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let failure_events = response_filter_failure_events(&window);
    let events = window
        .into_iter()
        .map(sanitize_response_filter_event)
        .collect();
    ResponseFilterEventsResponse {
        buffered_events,
        capacity,
        dropped_events,
        offset,
        limit,
        failure_events,
        events,
    }
}

pub fn response_filter_events_snapshot_response(
    state: &AppState,
    offset: usize,
    limit: usize,
) -> ResponseFilterEventsResponse {
    let (snapshot, capacity, dropped_events) = {
        let events = state
            .response_filter_events
            .lock()
            .expect("response filter events mutex poisoned");
        (
            events.snapshot(),
            events.capacity(),
            total_dropped_events(
                events.dropped_events(),
                &state.response_filter_event_lock_contention_drops,
            ),
        )
    };
    response_filter_events_response(snapshot, capacity, dropped_events, offset, limit)
}

#[derive(Debug, Serialize)]
pub struct RoutingTelemetryResponse {
    pub buffered_events: usize,
    pub capacity: usize,
    pub dropped_events: u64,
    pub offset: usize,
    pub limit: usize,
    pub events: Vec<Value>,
    pub failure_events: Vec<Value>,
    pub failure_transition_summaries: Vec<FailureTransitionSummary>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct FailureTransitionSummary {
    pub request_id: Option<String>,
    pub channel_id: Option<String>,
    pub public_model: Option<String>,
    pub client_token_ref: Option<String>,
    pub selected_credential_id_hash: Option<String>,
    pub attempt: usize,
    pub failure_source: String,
    pub failure_kind: String,
    pub failure_scope: String,
    pub retryable: bool,
    pub status: Option<u16>,
    pub mutation_kind: String,
    pub transition_action: String,
    pub affected_resource_kind: String,
    pub affected_credential_id_hash: Option<String>,
    pub resulting_state: String,
    pub reason: Option<String>,
    pub directive: Option<String>,
    pub denial_reason: Option<String>,
    pub retry_decision: Option<String>,
    pub retry_decision_reason: Option<String>,
}

pub fn routing_telemetry_response(
    snapshot: Vec<RoutingTelemetry>,
    capacity: usize,
    dropped_events: u64,
    offset: usize,
    limit: usize,
) -> RoutingTelemetryResponse {
    let buffered_events = snapshot.len();
    let failure_transition_summaries = failure_transition_summaries(&snapshot);
    let window = snapshot
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let failure_events = routing_failure_events(&window);
    let events = window.into_iter().map(sanitize_routing_telemetry).collect();
    RoutingTelemetryResponse {
        buffered_events,
        capacity,
        dropped_events,
        offset,
        limit,
        events,
        failure_events,
        failure_transition_summaries,
    }
}

pub fn routing_telemetry_snapshot_response(
    state: &AppState,
    offset: usize,
    limit: usize,
) -> RoutingTelemetryResponse {
    let (snapshot, capacity, dropped_events) = {
        let telemetry = state
            .routing_telemetry
            .lock()
            .expect("routing telemetry mutex poisoned");
        (
            telemetry.snapshot(),
            telemetry.capacity(),
            total_dropped_events(
                telemetry.dropped_events(),
                &state.routing_telemetry_lock_contention_drops,
            ),
        )
    };
    routing_telemetry_response(snapshot, capacity, dropped_events, offset, limit)
}

struct FailureTransitionSummaryBuilder {
    request_id: String,
    channel_id: String,
    summary: FailureTransitionSummary,
}

fn failure_transition_summaries(snapshot: &[RoutingTelemetry]) -> Vec<FailureTransitionSummary> {
    let mut summaries: Vec<FailureTransitionSummaryBuilder> =
        Vec::with_capacity(snapshot.len().min(128));
    for event in snapshot {
        match event {
            RoutingTelemetry::UpstreamFailureObserved {
                request_id,
                channel_id,
                failure,
            } => summaries.push(FailureTransitionSummaryBuilder {
                request_id: request_id.clone(),
                channel_id: channel_id.clone(),
                summary: failure_transition_summary_from_failure(request_id, channel_id, failure),
            }),
            RoutingTelemetry::CredentialTransitionApplied {
                request_id,
                channel_id,
                credential_id,
                state,
                reason,
            } => {
                if let Some(builder) =
                    pending_failure_transition_summary(&mut summaries, request_id, channel_id)
                {
                    apply_credential_transition_summary(
                        &mut builder.summary,
                        credential_id,
                        state,
                        reason,
                    );
                }
            }
            RoutingTelemetry::ChannelHealthTransitionApplied {
                request_id,
                channel_id,
                state,
                reason,
            } => {
                if let Some(builder) =
                    pending_failure_transition_summary(&mut summaries, request_id, channel_id)
                {
                    apply_channel_transition_summary(&mut builder.summary, state, reason);
                }
            }
            RoutingTelemetry::RouteSelected { .. }
            | RoutingTelemetry::RouteAdmissionDenied { .. }
            | RoutingTelemetry::TransitionApplied { .. }
            | RoutingTelemetry::CredentialLifecyclePersistenceDropped { .. } => {}
        }
    }
    summaries
        .into_iter()
        .map(|builder| builder.summary)
        .collect()
}

fn pending_failure_transition_summary<'a>(
    summaries: &'a mut [FailureTransitionSummaryBuilder],
    request_id: &str,
    channel_id: &str,
) -> Option<&'a mut FailureTransitionSummaryBuilder> {
    summaries.iter_mut().rev().find(|builder| {
        builder.request_id == request_id
            && builder.channel_id == channel_id
            && builder.summary.mutation_kind == "noop"
    })
}

fn failure_transition_summary_from_failure(
    request_id: &str,
    channel_id: &str,
    failure: &UpstreamFailureTelemetry,
) -> FailureTransitionSummary {
    FailureTransitionSummary {
        request_id: safe_management_id(request_id),
        channel_id: safe_management_id(channel_id),
        public_model: failure.public_model.as_deref().and_then(safe_management_id),
        client_token_ref: None,
        selected_credential_id_hash: safe_management_id(&failure.credential_id_hash),
        attempt: failure.attempt,
        failure_source: safe_management_code(&failure.failure_source),
        failure_kind: safe_management_code(&failure.failure_kind),
        failure_scope: safe_management_code(&failure.failure_scope),
        retryable: failure.retryable,
        status: failure.status,
        mutation_kind: "noop".to_string(),
        transition_action: "noop".to_string(),
        affected_resource_kind: "none".to_string(),
        affected_credential_id_hash: None,
        resulting_state: "noop".to_string(),
        reason: None,
        directive: safe_management_id(&failure.directive),
        denial_reason: failure
            .denial_reason
            .as_deref()
            .and_then(safe_management_id),
        retry_decision: safe_management_id(&failure.retry_decision),
        retry_decision_reason: failure
            .retry_decision_reason
            .as_deref()
            .and_then(safe_management_id),
    }
}

fn apply_credential_transition_summary(
    summary: &mut FailureTransitionSummary,
    credential_id: &str,
    state: &str,
    reason: &str,
) {
    summary.mutation_kind = "credential_transition".to_string();
    summary.transition_action = credential_transition_action(state);
    summary.affected_resource_kind = "credential".to_string();
    summary.affected_credential_id_hash = Some(short_hash(credential_id));
    summary.resulting_state = safe_management_code(state);
    summary.reason = safe_management_id(reason);
}

fn apply_channel_transition_summary(
    summary: &mut FailureTransitionSummary,
    state: &str,
    reason: &str,
) {
    summary.mutation_kind = "channel_health_transition".to_string();
    summary.transition_action = channel_transition_action(state, reason);
    summary.affected_resource_kind = "channel".to_string();
    summary.resulting_state = safe_management_code(state);
    summary.reason = safe_management_id(reason);
}

fn credential_transition_action(state: &str) -> String {
    match state {
        "expired" => "expire_credential",
        "cooling_down" => "mark_credential_cooling_down",
        "quota_exhausted" => "mark_credential_quota_exhausted",
        _ => "credential_transition",
    }
    .to_string()
}

fn channel_transition_action(state: &str, reason: &str) -> String {
    match (state, reason) {
        ("cooling_down", "relay_balance_unavailable") => "mark_relay_balance_channel_cooling_down",
        ("cooling_down", "upstream_provider_unavailable") => {
            "mark_provider_account_channel_cooling_down"
        }
        ("degraded", "upstream_provider_unavailable") => "mark_provider_account_channel_degraded",
        ("cooling_down", _) => "mark_channel_cooling_down",
        ("degraded", _) => "mark_channel_degraded",
        _ => "channel_health_transition",
    }
    .to_string()
}

fn routing_failure_events(snapshot: &[RoutingTelemetry]) -> Vec<Value> {
    let summaries = failure_transition_summaries(snapshot);
    let mut summary_iter = summaries.into_iter();
    let mut events = Vec::with_capacity(snapshot.len().min(128));
    for event in snapshot {
        match event {
            RoutingTelemetry::UpstreamFailureObserved { .. } => {
                if let Some(summary) = summary_iter.next() {
                    events.push(project_routing_failure_event(summary));
                }
            }
            RoutingTelemetry::RouteAdmissionDenied { .. } => {
                if let Some(event) = project_route_admission_denied_event(event) {
                    events.push(event);
                }
            }
            _ => {}
        }
    }
    events
}

fn project_routing_failure_event(summary: FailureTransitionSummary) -> Value {
    let failure_class = routing_failure_class(&summary);
    let reason_code = reason_code_for_failure_class(failure_class);
    let contract = diagnostic_contract_for(reason_code);
    let retry_eligibility = routing_retry_eligibility(&summary);
    let retry_blocked_reason =
        retry_blocked_reason(&retry_eligibility, summary.retry_decision_reason.as_deref());
    let client_visible_status = client_visible_status(
        summary.status,
        summary.directive.as_deref(),
        &retry_eligibility,
    );
    let router_action = routing_router_action(&summary);
    json!({
        "source": "routing_telemetry",
        "event_kind": "upstream_failure_observed",
        "request_id": summary.request_id,
        "stage": upstream_stage(&summary.failure_source),
        "public_model": summary.public_model.unwrap_or_else(|| "unknown".to_string()),
        "client_token_ref": summary.client_token_ref,
        "selected_target": selected_target(summary.channel_id.as_deref()),
        "channel_id": summary.channel_id,
        "failure_class": failure_class,
        "router_action": router_action,
        "retry_eligibility": retry_eligibility,
        "retry_blocked_reason": retry_blocked_reason,
        "client_visible_status": client_visible_status,
        "upstream_status": summary.status,
        "final_outcome": final_outcome(&client_visible_status),
        "reason_code": reason_code,
        "blocking_domain": contract.blocking_domain,
        "directive": summary.directive,
        "attempt": summary.attempt,
        "next_action": contract.next_action,
    })
}

fn project_route_admission_denied_event(event: &RoutingTelemetry) -> Option<Value> {
    let RoutingTelemetry::RouteAdmissionDenied {
        request_id,
        registry_generation,
        endpoint_family,
        public_model,
        client_token_ref,
        route_kind,
        reason_code,
        blocking_domain,
        client_visible_status,
        candidate_count,
        included_count,
        blocked_count,
        hard_blocked_count,
        soft_suppressed_count,
        last_resort_used,
        hard_reason_codes,
        soft_reason_codes,
        ..
    } = event
    else {
        return None;
    };
    let reason_code = stable_admission_reason_code(reason_code);
    let contract = diagnostic_contract_for(reason_code);
    let client_visible_status = local_client_visible_status(*client_visible_status);
    Some(json!({
        "source": "routing_telemetry",
        "event_kind": "route_admission_denied",
        "request_id": safe_management_id(request_id),
        "stage": "route_admission",
        "endpoint_family": safe_management_code(endpoint_family),
        "public_model": public_model.as_deref().and_then(safe_management_id).unwrap_or_else(|| "unknown".to_string()),
        "client_token_ref": client_token_ref.as_deref().and_then(safe_management_id),
        "route_kind": safe_management_code(route_kind),
        "failure_class": "route_admission_denied",
        "router_action": "returned_local_error",
        "retry_eligibility": "not_applicable",
        "retry_blocked_reason": Value::Null,
        "client_visible_status": client_visible_status,
        "upstream_status": Value::Null,
        "final_outcome": final_outcome(&client_visible_status),
        "reason_code": reason_code,
        "blocking_domain": safe_management_code(blocking_domain),
        "admission": {
            "registry_generation": registry_generation,
            "candidate_count": candidate_count,
            "included_count": included_count,
            "blocked_count": blocked_count,
            "hard_blocked_count": hard_blocked_count,
            "soft_suppressed_count": soft_suppressed_count,
            "last_resort_used": last_resort_used,
            "hard_reason_codes": safe_reason_codes(hard_reason_codes),
            "soft_reason_codes": safe_reason_codes(soft_reason_codes),
        },
        "next_action": contract.next_action,
    }))
}

fn response_filter_failure_events(events: &[ResponseFilterEvent]) -> Vec<Value> {
    events
        .iter()
        .filter(|event| event.outcome == "rejected")
        .map(project_response_filter_failure_event)
        .collect()
}

fn project_response_filter_failure_event(event: &ResponseFilterEvent) -> Value {
    let body_committed = event.body_committed;
    let reason_code = stable_reason_code(
        &event.reason_code,
        if body_committed {
            "stream_committed_failure"
        } else {
            "response_filter_rejected"
        },
    );
    let contract = diagnostic_contract_for(reason_code);
    let client_visible_status = if body_committed {
        "stream_committed_failure"
    } else {
        "local_502"
    };
    json!({
        "source": "response_filter_events",
        "event_kind": "response_filter_rejected",
        "request_id": safe_management_id(&event.request_id),
        "stage": if body_committed { "post_output" } else { "response_filter" },
        "public_model": safe_management_code(&event.public_model),
        "selected_target": selected_target(Some(&event.channel_id)),
        "channel_id": safe_management_id(&event.channel_id),
        "failure_class": if body_committed { "stream_committed_failure" } else { "response_filter_rejected" },
        "router_action": response_filter_router_action(&event.action),
        "retry_eligibility": if body_committed { "blocked_streaming" } else { "eligible_before_output" },
        "retry_blocked_reason": if body_committed { Value::from("partial_output_started") } else { Value::Null },
        "client_visible_status": client_visible_status,
        "final_outcome": final_outcome(client_visible_status),
        "reason_code": reason_code,
        "blocking_domain": contract.blocking_domain,
        "directive": safe_management_id(&event.action),
        "content_kind": safe_management_code(&event.content_kind),
        "next_action": contract.next_action,
    })
}

fn routing_failure_class(summary: &FailureTransitionSummary) -> &'static str {
    if matches!(summary.status, Some(500..=599)) {
        return "upstream_5xx";
    }
    if matches!(summary.status, Some(400..=499)) && summary.failure_scope == "credential" {
        return "credential_unavailable";
    }
    match summary.failure_kind.as_str() {
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

fn routing_router_action(summary: &FailureTransitionSummary) -> &'static str {
    match summary.mutation_kind.as_str() {
        "credential_transition" => "marked_credential",
        "channel_health_transition" => "marked_channel",
        _ => router_action_for_directive(summary.directive.as_deref()),
    }
}

fn router_action_for_directive(directive: Option<&str>) -> &'static str {
    match directive.unwrap_or_default() {
        "retry" | "retry_credential" | "retry_same_target" => "retried_before_output",
        "fallback" | "retry_route_target" => "fell_back_before_output",
        "mark_credential" => "marked_credential",
        "mark_channel" => "marked_channel",
        "record_event_only" => "recorded_event_only",
        _ => "returned_local_error",
    }
}

fn routing_retry_eligibility(summary: &FailureTransitionSummary) -> String {
    let directive = summary.directive.as_deref().unwrap_or_default();
    let retry_decision = summary.retry_decision.as_deref().unwrap_or_default();
    if retry_directive_is_pre_output_continuation(directive)
        || retry_directive_is_pre_output_continuation(retry_decision)
    {
        return "eligible_before_output".to_string();
    }
    if let Some(reason) = summary
        .retry_decision_reason
        .as_deref()
        .and_then(stable_retry_blocked_reason)
    {
        return format!("blocked_{reason}");
    }
    "not_applicable".to_string()
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
        "failure_not_retryable" | "blocked_failure_not_retryable" => Some("failure_not_retryable"),
        "attempt_limit_reached" | "blocked_attempt_limit_reached" => Some("attempt_limit_reached"),
        _ => None,
    }
}

fn retry_directive_is_pre_output_continuation(value: &str) -> bool {
    matches!(
        value,
        "retry" | "fallback" | "retry_credential" | "retry_route_target" | "retry_same_target"
    )
}

fn client_visible_status(
    status: Option<u16>,
    directive: Option<&str>,
    retry_eligibility: &str,
) -> String {
    if retry_eligibility == "eligible_before_output" {
        return "not_applicable_retry_before_output".to_string();
    }
    match status {
        Some(400..=499) => "upstream_4xx".to_string(),
        Some(500..=599) => "upstream_5xx".to_string(),
        Some(value) => format!("upstream_status_{value}"),
        None if directive == Some("retry") => "not_applicable_retry_before_output".to_string(),
        None => "unknown".to_string(),
    }
}

fn upstream_stage(failure_source: &str) -> &'static str {
    match failure_source {
        "response_filter_precommit" => "response_filter",
        "guarded_success_envelope" | "upstream_response_guard" => "upstream_response_guard",
        "upstream_transaction" | "upstream_transport" => "upstream_transport",
        _ => "upstream_transport",
    }
}

fn reason_code_for_failure_class(failure_class: &str) -> &'static str {
    match failure_class {
        "credential_unavailable" => "credential_unavailable",
        "upstream_5xx" => "upstream_5xx",
        "upstream_timeout" => "upstream_timeout",
        "response_filter_rejected" => "response_filter_rejected",
        _ => "unknown_failure_class",
    }
}

fn stable_admission_reason_code(value: &str) -> &'static str {
    match value {
        "no_route_candidate" => "no_route_candidate",
        "unsupported_endpoint_family" => "unsupported_endpoint_family",
        "credential_unavailable" => "credential_unavailable",
        _ => "no_route_candidate",
    }
}

fn stable_reason_code(value: &str, fallback: &'static str) -> &'static str {
    match value {
        "response_filter_rejected" => "response_filter_rejected",
        "stream_committed_failure" => "stream_committed_failure",
        _ => fallback,
    }
}

fn local_client_visible_status(status: u16) -> String {
    match status {
        400 => "local_400",
        401 => "local_401",
        403 => "local_403",
        404 => "local_404",
        429 => "local_429",
        502 => "local_502",
        503 => "local_503",
        _ => "local_error",
    }
    .to_string()
}

fn response_filter_router_action(action: &str) -> &'static str {
    match action {
        "reject_and_expire_credential" => "marked_credential",
        "reject_and_cooldown_channel" => "marked_channel",
        "reject_and_disable_channel" => "marked_channel",
        _ => "recorded_event_only",
    }
}

fn selected_target(channel_id: Option<&str>) -> Value {
    channel_id
        .and_then(safe_management_id)
        .map(|channel_id| json!({ "channel_id": channel_id }))
        .unwrap_or(Value::Null)
}

fn diagnostic_contract_for(reason_code: &str) -> crate::diagnostic_contract::DiagnosticContract {
    crate::diagnostic_contract::contract_for_reason(reason_code)
        .unwrap_or_else(crate::diagnostic_contract::fallback_contract)
}

fn final_outcome(client_visible_status: &str) -> &'static str {
    match client_visible_status {
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
    }
}

fn total_dropped_events(buffer_dropped_events: u64, lock_contention_drops: &AtomicU64) -> u64 {
    buffer_dropped_events.saturating_add(lock_contention_drops.load(Ordering::Relaxed))
}

fn sanitize_routing_telemetry(event: RoutingTelemetry) -> Value {
    match event {
        RoutingTelemetry::RouteSelected {
            request_id,
            registry_generation,
            channel_id,
        } => json!({
            "kind": "route_selected",
            "request_id": safe_management_id(&request_id),
            "registry_generation": registry_generation,
            "channel_id": safe_management_id(&channel_id),
        }),
        RoutingTelemetry::UpstreamFailureObserved {
            request_id,
            channel_id,
            failure,
        } => json!({
            "kind": "upstream_failure_observed",
            "request_id": safe_management_id(&request_id),
            "channel_id": safe_management_id(&channel_id),
            "failure": sanitize_upstream_failure(*failure),
        }),
        RoutingTelemetry::RouteAdmissionDenied {
            request_id,
            registry_generation,
            endpoint_family,
            public_model,
            client_token_ref,
            route_kind,
            reason_code,
            blocking_domain,
            client_visible_status,
            candidate_count,
            included_count,
            blocked_count,
            hard_blocked_count,
            soft_suppressed_count,
            last_resort_used,
            hard_reason_codes,
            soft_reason_codes,
            ..
        } => json!({
            "kind": "route_admission_denied",
            "request_id": safe_management_id(&request_id),
            "registry_generation": registry_generation,
            "endpoint_family": safe_management_id(&endpoint_family),
            "public_model": public_model.as_deref().and_then(safe_management_id),
            "client_token_ref": client_token_ref.as_deref().and_then(safe_management_id),
            "route_kind": safe_management_id(&route_kind),
            "reason_code": safe_management_id(&reason_code),
            "blocking_domain": safe_management_id(&blocking_domain),
            "client_visible_status": client_visible_status,
            "upstream_status": Value::Null,
            "candidate_count": candidate_count,
            "included_count": included_count,
            "blocked_count": blocked_count,
            "hard_blocked_count": hard_blocked_count,
            "soft_suppressed_count": soft_suppressed_count,
            "last_resort_used": last_resort_used,
            "hard_reason_codes": safe_reason_codes(&hard_reason_codes),
            "soft_reason_codes": safe_reason_codes(&soft_reason_codes),
        }),
        RoutingTelemetry::TransitionApplied {
            request_id,
            channel_id,
        } => json!({
            "kind": "transition_applied",
            "request_id": safe_management_id(&request_id),
            "channel_id": safe_management_id(&channel_id),
        }),
        RoutingTelemetry::ChannelHealthTransitionApplied {
            request_id,
            channel_id,
            state,
            reason,
        } => json!({
            "kind": "channel_health_transition_applied",
            "request_id": safe_management_id(&request_id),
            "channel_id": safe_management_id(&channel_id),
            "state": safe_management_id(&state),
            "reason": safe_management_id(&reason),
        }),
        RoutingTelemetry::CredentialTransitionApplied {
            request_id,
            channel_id,
            credential_id,
            state,
            reason,
        } => json!({
            "kind": "credential_transition_applied",
            "request_id": safe_management_id(&request_id),
            "channel_id": safe_management_id(&channel_id),
            "credential_id_hash": short_hash(&credential_id),
            "state": safe_management_id(&state),
            "reason": safe_management_id(&reason),
        }),
        RoutingTelemetry::CredentialLifecyclePersistenceDropped {
            request_id,
            channel_id,
            credential_id,
            state,
            reason,
            drop_reason,
        } => json!({
            "kind": "credential_lifecycle_persistence_dropped",
            "request_id": safe_management_id(&request_id),
            "channel_id": safe_management_id(&channel_id),
            "credential_id_hash": short_hash(&credential_id),
            "state": safe_management_id(&state),
            "reason": safe_management_id(&reason),
            "drop_reason": safe_management_id(&drop_reason),
        }),
    }
}

fn sanitize_upstream_failure(failure: UpstreamFailureTelemetry) -> Value {
    json!({
        "public_model": failure.public_model.as_deref().and_then(safe_management_id),
        "credential_id_hash": safe_management_id(&failure.credential_id_hash),
        "attempt": failure.attempt,
        "failure_source": safe_management_id(&failure.failure_source),
        "failure_kind": safe_management_id(&failure.failure_kind),
        "failure_scope": safe_management_id(&failure.failure_scope),
        "retryable": failure.retryable,
        "confidence": safe_management_id(&failure.confidence),
        "status": failure.status,
        "classifier_id": safe_management_id(&failure.classifier_id),
        "classifier_version": safe_management_id(&failure.classifier_version),
        "adaptation_rule_id": failure.adaptation_rule_id.as_deref().and_then(safe_management_id),
        "retry_after_source": failure.retry_after_source.as_deref().and_then(safe_management_id),
        "cooldown_seconds": failure.cooldown_seconds,
        "directive": safe_management_id(&failure.directive),
        "denial_reason": failure.denial_reason.as_deref().and_then(safe_management_id),
        "duplicate_charge_risk": safe_management_id(&failure.duplicate_charge_risk),
        "effective_deadline_remaining_ms": failure.effective_deadline_remaining_ms,
        "retry_pressure_accounted": failure.retry_pressure_accounted,
        "retry_decision": safe_management_id(&failure.retry_decision),
        "retry_decision_reason": failure.retry_decision_reason.as_deref().and_then(safe_management_id),
    })
}

fn sanitize_response_filter_event(event: ResponseFilterEvent) -> Value {
    json!({
        "event_id": event.event_id,
        "created_at_unix_seconds": event.created_at_unix_seconds,
        "request_id": safe_management_id(&event.request_id),
        "channel_id": safe_management_id(&event.channel_id),
        "public_model": safe_management_id(&event.public_model),
        "rule_id": safe_management_id(&event.rule_id),
        "action": safe_management_id(&event.action),
        "content_kind": safe_management_id(&event.content_kind),
        "reason_code": safe_management_id(&event.reason_code),
        "outcome": safe_management_id(&event.outcome),
        "body_committed": event.body_committed,
    })
}

fn safe_management_id(value: &str) -> Option<String> {
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

fn safe_management_code(value: &str) -> String {
    safe_management_id(value).unwrap_or_else(|| "unknown".to_string())
}

fn safe_reason_codes(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| safe_management_id(value))
        .take(8)
        .collect()
}

#[derive(Debug, Serialize)]
pub struct RuntimeResponse {
    pub uptime_seconds: u64,
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub runtime_reload_required: bool,
    pub channels: usize,
    pub client_tokens: usize,
    pub credentials: RuntimeCredentialCounts,
    pub credential_pool_alerts: Vec<CredentialPoolAlertStatus>,
    pub retry_policy: RuntimeRetryPolicy,
    pub retry_profile_summary: RuntimeRetryProfileSummary,
    pub retry_pressure_capacity: RuntimeRetryPressureCapacity,
    pub recent_retry_counters: RuntimeRecentRetryCounters,
    pub management_events: usize,
    pub management_event_window_capacity: usize,
    pub routing_telemetry_events: usize,
    pub routing_telemetry_capacity: usize,
    pub routing_telemetry_dropped_events: u64,
    pub response_filter_events: usize,
    pub response_filter_event_capacity: usize,
    pub request_limits: RequestLimits,
    pub timeout_seconds: TimeoutSeconds,
}

#[derive(Debug, Serialize)]
pub struct RuntimeExplainResponse {
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub runtime_reload_required: bool,
    pub registry_source: RuntimeSourceExplanation,
    pub credential_source: RuntimeSourceExplanation,
    pub client_token_source: RuntimeSourceExplanation,
    pub staged_vs_runtime: StagedRuntimeExplanation,
    pub last_reload: RuntimeReloadExplanation,
}

#[derive(Debug, Serialize)]
pub struct RuntimeSourceExplanation {
    pub source_kind: &'static str,
    pub writable: bool,
    pub persisted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_authoritative: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct StagedRuntimeExplanation {
    pub active_matches_staged: bool,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub active_registry_generation: u64,
    pub reload_required_reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReloadExplanation {
    pub last_reload_at_unix_seconds: Option<u64>,
    pub last_reload_error_reason_code: Option<String>,
}

pub struct RuntimeResponseParts {
    pub uptime_seconds: u64,
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub channels: usize,
    pub client_tokens: usize,
    pub credentials: RuntimeCredentialCounts,
    pub retry_policy: RuntimeRetryPolicy,
    pub retry_profile_summary: RuntimeRetryProfileSummary,
    pub retry_pressure_capacity: RuntimeRetryPressureCapacity,
    pub recent_retry_counters: RuntimeRecentRetryCounters,
    pub management_events: usize,
    pub management_event_window_capacity: usize,
    pub routing_telemetry_events: usize,
    pub routing_telemetry_capacity: usize,
    pub routing_telemetry_dropped_events: u64,
    pub response_filter_events: usize,
    pub response_filter_event_capacity: usize,
    pub max_request_body_bytes: usize,
    pub max_model_catalog_body_bytes: usize,
    pub max_model_catalog_channels: usize,
    pub max_error_body_bytes: usize,
    pub max_route_candidates: usize,
    pub connect_timeout: Duration,
    pub non_streaming_total_timeout: Duration,
    pub streaming_idle_timeout: Duration,
}

pub fn runtime_response_from_parts(parts: RuntimeResponseParts) -> RuntimeResponse {
    let credential_pool_alerts = credential_pool_alerts(&parts.credentials);
    RuntimeResponse {
        uptime_seconds: parts.uptime_seconds,
        active_registry_generation: parts.active_registry_generation,
        active_registry_version: parts.active_registry_version,
        staged_registry_version: parts.staged_registry_version,
        runtime_reload_required: parts.staged_registry_version != parts.active_registry_version,
        channels: parts.channels,
        client_tokens: parts.client_tokens,
        credential_pool_alerts,
        credentials: parts.credentials,
        retry_policy: parts.retry_policy,
        retry_profile_summary: parts.retry_profile_summary,
        retry_pressure_capacity: parts.retry_pressure_capacity,
        recent_retry_counters: parts.recent_retry_counters,
        management_events: parts.management_events,
        management_event_window_capacity: parts.management_event_window_capacity,
        routing_telemetry_events: parts.routing_telemetry_events,
        routing_telemetry_capacity: parts.routing_telemetry_capacity,
        routing_telemetry_dropped_events: parts.routing_telemetry_dropped_events,
        response_filter_events: parts.response_filter_events,
        response_filter_event_capacity: parts.response_filter_event_capacity,
        request_limits: RequestLimits {
            max_request_body_bytes: parts.max_request_body_bytes,
            max_model_catalog_body_bytes: parts.max_model_catalog_body_bytes,
            max_model_catalog_channels: parts.max_model_catalog_channels,
            max_error_body_bytes: parts.max_error_body_bytes,
            max_route_candidates: parts.max_route_candidates,
        },
        timeout_seconds: TimeoutSeconds {
            connect: duration_secs(parts.connect_timeout),
            non_streaming_total: duration_secs(parts.non_streaming_total_timeout),
            streaming_idle: duration_secs(parts.streaming_idle_timeout),
        },
    }
}

pub async fn runtime_response(
    state: &AppState,
    staged_registry_version: Option<u64>,
) -> RuntimeResponse {
    let sample = collect_runtime_snapshot(state).await;
    let same_target_transient_retry_attempt_capacity = sample
        .retry_profile_summary
        .route_target_retry_enabled_channels;

    runtime_response_from_parts(RuntimeResponseParts {
        uptime_seconds: state.started_at.elapsed().as_secs(),
        active_registry_generation: sample.active_registry_generation,
        active_registry_version: sample.active_registry_version,
        staged_registry_version,
        channels: sample.channels,
        client_tokens: sample.client_tokens,
        credentials: sample.credentials,
        retry_policy: sample.retry_policy,
        retry_profile_summary: sample.retry_profile_summary,
        retry_pressure_capacity: RuntimeRetryPressureCapacity {
            same_request_credential_retry_attempts: sample
                .same_request_credential_retry_attempt_capacity,
            route_target_fallback_candidates: state.routing.max_route_candidates,
            same_target_transient_retry_attempts: same_target_transient_retry_attempt_capacity,
        },
        recent_retry_counters: RuntimeRecentRetryCounters::from_window(
            sample.recent_retry_counters,
            sample.routing_telemetry_capacity,
        ),
        management_events: sample.management_events,
        management_event_window_capacity: sample.management_event_window_capacity,
        routing_telemetry_events: sample.routing_telemetry_events,
        routing_telemetry_capacity: sample.routing_telemetry_capacity,
        routing_telemetry_dropped_events: sample.routing_telemetry_dropped_events,
        response_filter_events: sample.response_filter_events,
        response_filter_event_capacity: sample.response_filter_event_capacity,
        max_request_body_bytes: state.max_request_body_bytes,
        max_model_catalog_body_bytes: state.max_model_catalog_body_bytes,
        max_model_catalog_channels: state.routing.max_model_catalog_channels,
        max_error_body_bytes: state.max_error_body_bytes,
        max_route_candidates: state.routing.max_route_candidates,
        connect_timeout: state.timeout_profile.connect,
        non_streaming_total_timeout: state.timeout_profile.non_streaming_total,
        streaming_idle_timeout: state.timeout_profile.streaming_idle,
    })
}

pub async fn runtime_response_for_state(
    state: &AppState,
) -> Result<RuntimeResponse, ManagementServiceError> {
    let staged_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?;
    Ok(runtime_response(state, staged_registry_version).await)
}

pub async fn runtime_explain_response_for_state(
    state: &AppState,
) -> Result<RuntimeExplainResponse, ManagementServiceError> {
    let staged_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?;
    Ok(runtime_explain_response(state, staged_registry_version).await)
}

pub async fn runtime_explain_response(
    state: &AppState,
    staged_registry_version: Option<u64>,
) -> RuntimeExplainResponse {
    let sample = collect_runtime_snapshot(state).await;
    let runtime_reload_required = staged_registry_version != sample.active_registry_version;
    let last_reload = state.runtime_reload_status_snapshot();
    RuntimeExplainResponse {
        active_registry_generation: sample.active_registry_generation,
        active_registry_version: sample.active_registry_version,
        staged_registry_version,
        runtime_reload_required,
        registry_source: registry_source_explanation(&state.registry_store),
        credential_source: credential_source_explanation(&state.credential_store),
        client_token_source: client_token_source_explanation(&state.client_token_store),
        staged_vs_runtime: StagedRuntimeExplanation {
            active_matches_staged: !runtime_reload_required,
            active_registry_version: sample.active_registry_version,
            staged_registry_version,
            active_registry_generation: sample.active_registry_generation,
            reload_required_reason: runtime_reload_required.then_some("staged_registry_differs"),
        },
        last_reload: RuntimeReloadExplanation {
            last_reload_at_unix_seconds: last_reload.last_reload_at_unix_seconds,
            last_reload_error_reason_code: last_reload.last_reload_error_reason_code,
        },
    }
}

fn registry_source_explanation(store: &RegistryStoreHandle) -> RuntimeSourceExplanation {
    match store {
        RegistryStoreHandle::ReadOnly => RuntimeSourceExplanation {
            source_kind: "read_only_bootstrap",
            writable: false,
            persisted: false,
            lifecycle_authoritative: None,
        },
        RegistryStoreHandle::Sqlite(_) => RuntimeSourceExplanation {
            source_kind: "sqlite_registry_store",
            writable: true,
            persisted: true,
            lifecycle_authoritative: None,
        },
    }
}

fn credential_source_explanation(store: &CredentialStoreHandle) -> RuntimeSourceExplanation {
    match store {
        CredentialStoreHandle::ReadOnlyFileBootstrap => RuntimeSourceExplanation {
            source_kind: "file_bootstrap",
            writable: false,
            persisted: false,
            lifecycle_authoritative: Some(false),
        },
        CredentialStoreHandle::Sqlite(_) => RuntimeSourceExplanation {
            source_kind: "sqlite_credential_store",
            writable: true,
            persisted: true,
            lifecycle_authoritative: Some(store.has_lifecycle_snapshot_authority()),
        },
    }
}

fn client_token_source_explanation(store: &ClientTokenStoreHandle) -> RuntimeSourceExplanation {
    match store {
        ClientTokenStoreHandle::ReadOnlyBootstrap => RuntimeSourceExplanation {
            source_kind: "bootstrap",
            writable: false,
            persisted: false,
            lifecycle_authoritative: None,
        },
        ClientTokenStoreHandle::Sqlite(_) => RuntimeSourceExplanation {
            source_kind: "sqlite_client_token_store",
            writable: true,
            persisted: true,
            lifecycle_authoritative: None,
        },
    }
}

#[derive(Debug)]
struct RuntimeSnapshotSample {
    active_registry_generation: u64,
    active_registry_version: Option<u64>,
    channels: usize,
    client_tokens: usize,
    credentials: RuntimeCredentialCounts,
    retry_policy: RuntimeRetryPolicy,
    retry_profile_summary: RuntimeRetryProfileSummary,
    same_request_credential_retry_attempt_capacity: usize,
    recent_retry_counters: RetryPressureCounters,
    management_events: usize,
    management_event_window_capacity: usize,
    routing_telemetry_events: usize,
    routing_telemetry_capacity: usize,
    routing_telemetry_dropped_events: u64,
    response_filter_events: usize,
    response_filter_event_capacity: usize,
    serving_channels: usize,
    credential_set_blocking_alerts: usize,
    operator_input_alerts: usize,
    credential_sets_without_spare: usize,
    channels_cooling_down: usize,
}

#[derive(Debug)]
struct RuntimeReloadSample {
    active_registry_generation: u64,
    active_registry_version: Option<u64>,
    channels: usize,
}

async fn collect_runtime_snapshot(state: &AppState) -> RuntimeSnapshotSample {
    let topology_summary = state.runtime_catalogs.runtime_topology_summary();
    let mut credentials = RuntimeCredentialCounts::default();
    let mut snapshot_cache = CredentialSetSnapshotCache::default();
    let mut serving_channels = 0;
    let mut operator_input_alerts = 0usize;
    let mut credential_set_blocking_alerts = 0usize;
    let mut credential_sets_without_spare = 0usize;
    let mut channels_cooling_down = 0usize;

    let credential_set_topologies = state.runtime_catalogs.credential_set_topologies();
    let channel_topologies = state.runtime_catalogs.channel_topologies();

    for topology in credential_set_topologies {
        for channel_id in topology.channel_ids {
            let Some(pool_state) = state.channels.get(&channel_id) else {
                continue;
            };
            let sample = snapshot_cache.runtime_sample_for(&pool_state).await;
            add_runtime_counts(&mut credentials, &sample.counts);
            if sample.readiness.needs_operator_input {
                operator_input_alerts += 1;
            }
            if sample.readiness.blocking {
                credential_set_blocking_alerts += 1;
            }
            if sample.counts.available < 2 {
                credential_sets_without_spare += 1;
            }
            break;
        }
    }

    let route_target_retry_enabled_channels = channel_topologies
        .iter()
        .filter(|topology| topology.route_target_retry_enabled)
        .count();
    let same_request_credential_retry_attempt_capacity = channel_topologies
        .iter()
        .filter(|topology| topology.retry_switched_key_in_same_request)
        .map(|topology| topology.max_same_request_retries)
        .sum();

    for topology in channel_topologies {
        let Some(pool_state) = state.channels.get(&topology.id) else {
            continue;
        };
        if !pool_state.configured_enabled || !pool_state.account_enabled {
            continue;
        }
        let health = pool_state
            .health
            .lock()
            .expect("channel health mutex poisoned")
            .clone();
        if matches!(
            health,
            ChannelHealth::CoolingDown { until, .. } if Instant::now() < until
        ) {
            channels_cooling_down += 1;
        }
        if !health.is_available_for_routing() && !matches!(health, ChannelHealth::Degraded { .. }) {
            continue;
        }
        let sample = snapshot_cache.runtime_sample_for(&pool_state).await;
        if sample.has_available_credentials() {
            serving_channels += 1;
        }
    }

    let active_registry_generation = state.channels.registry_generation();
    let active_registry_version = *state
        .active_registry_version
        .read()
        .expect("active registry version lock poisoned");
    let client_tokens = state
        .client_tokens
        .read()
        .expect("client token registry lock poisoned")
        .len();
    let (
        routing_telemetry_events,
        routing_telemetry_capacity,
        routing_telemetry_dropped_events,
        recent_retry_counters,
    ) = {
        let telemetry = state
            .routing_telemetry
            .lock()
            .expect("routing telemetry mutex poisoned");
        (
            telemetry.len(),
            telemetry.capacity(),
            total_dropped_events(
                telemetry.dropped_events(),
                &state.routing_telemetry_lock_contention_drops,
            ),
            telemetry.retry_pressure_snapshot(),
        )
    };
    let (response_filter_events, response_filter_event_capacity) = {
        let events = state
            .response_filter_events
            .lock()
            .expect("response filter events mutex poisoned");
        (events.len(), events.capacity())
    };

    RuntimeSnapshotSample {
        active_registry_generation,
        active_registry_version,
        channels: topology_summary.channels,
        client_tokens,
        credentials,
        retry_policy: runtime_retry_policy_from_topology_summary(topology_summary.clone()),
        retry_profile_summary: runtime_retry_profile_summary_from_topology(
            topology_summary,
            route_target_retry_enabled_channels,
        ),
        same_request_credential_retry_attempt_capacity,
        recent_retry_counters,
        management_events: state.events.len(),
        management_event_window_capacity: state.events.window_capacity(),
        routing_telemetry_events,
        routing_telemetry_capacity,
        routing_telemetry_dropped_events,
        response_filter_events,
        response_filter_event_capacity,
        serving_channels,
        credential_set_blocking_alerts,
        operator_input_alerts,
        credential_sets_without_spare,
        channels_cooling_down,
    }
}

fn add_runtime_counts(
    total: &mut RuntimeCredentialCounts,
    credential_set: &RuntimeCredentialCounts,
) {
    total.total += credential_set.total;
    total.available += credential_set.available;
    total.cooling_down += credential_set.cooling_down;
    total.expired += credential_set.expired;
    total.quota_exhausted += credential_set.quota_exhausted;
    total.disabled += credential_set.disabled;
}

fn collect_runtime_reload_sample(state: &AppState) -> RuntimeReloadSample {
    RuntimeReloadSample {
        active_registry_generation: state.channels.registry_generation(),
        active_registry_version: *state
            .active_registry_version
            .read()
            .expect("active registry version lock poisoned"),
        channels: state.runtime_catalogs.runtime_topology_summary().channels,
    }
}

#[derive(Debug, Serialize)]
pub struct ReadinessResponse {
    pub status: &'static str,
    pub channels: usize,
    pub serving_channels: usize,
    pub credentials: RuntimeCredentialCounts,
    pub blocking_alerts: usize,
    pub operator_input_alerts: usize,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReloadResponse {
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub runtime_reload_required: bool,
    pub channels: usize,
}

pub struct RuntimeReloadResponseParts {
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub channels: usize,
}

pub fn runtime_reload_response(parts: RuntimeReloadResponseParts) -> RuntimeReloadResponse {
    RuntimeReloadResponse {
        active_registry_generation: parts.active_registry_generation,
        active_registry_version: parts.active_registry_version,
        staged_registry_version: parts.staged_registry_version,
        runtime_reload_required: parts.staged_registry_version != parts.active_registry_version,
        channels: parts.channels,
    }
}

pub async fn reload_runtime(
    state: &AppState,
    actor: ManagementEventActor,
    expected_staged_registry_version: Option<u64>,
) -> Result<RuntimeReloadResponse, ManagementServiceError> {
    let result = apply_runtime_reload(state, actor, expected_staged_registry_version).await;
    match result {
        Ok(response) => {
            state.record_runtime_reload_success(current_unix_seconds());
            Ok(response)
        }
        Err(err) => {
            if err.records_runtime_reload_failure() {
                state
                    .record_runtime_reload_failure(current_unix_seconds(), "runtime_reload_failed");
            }
            Err(err)
        }
    }
}

async fn apply_runtime_reload(
    state: &AppState,
    actor: ManagementEventActor,
    expected_staged_registry_version: Option<u64>,
) -> Result<RuntimeReloadResponse, ManagementServiceError> {
    let Some(expected_staged_registry_version) = expected_staged_registry_version else {
        return Err(ManagementServiceError::PreconditionFailed(
            "runtime reload requires expected_staged_registry_version precondition".to_string(),
        ));
    };
    let _mutation_guard = state.registry_mutation_lock.lock().await;
    let staged_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?;
    if staged_registry_version != Some(expected_staged_registry_version) {
        return Err(ManagementServiceError::PreconditionFailed(format!(
            "runtime reload precondition failed: staged registry version changed from {expected_staged_registry_version} to {staged_registry_version:?}"
        )));
    }
    let Some(staged_registry_document) = state
        .registry_store
        .load_registry_for_validation()
        .await
        .map_err(registry_store_error)?
    else {
        return Err(ManagementServiceError::Conflict(
            "runtime reload requires a writable registry store".to_string(),
        ));
    };
    let validation_bootstrap = (*state.registry_validation_bootstrap).clone();
    let config = resolve_staged_registry_document(validation_bootstrap, staged_registry_document)
        .map_err(registry_store_error)?;
    state
        .events
        .record_audit_event(ManagementAuditEvent {
            kind: "runtime_reloaded".to_string(),
            action: "runtime_reloaded".to_string(),
            resource_type: "runtime".to_string(),
            resource_id: "runtime".to_string(),
            channel_id: String::new(),
            credential_id: String::new(),
            outcome: "applied".to_string(),
            request_id: None,
            generation: staged_registry_version,
            reason_code: "manual_runtime_reload".to_string(),
            actor: Some(actor),
        })
        .await
        .map_err(|err| ManagementServiceError::EventAppendFailed {
            message: err.to_string(),
            stale_history_id: None,
        })?;
    state
        .rebuild_runtime_from_resolved_config(config, staged_registry_version)
        .map_err(|err| {
            ManagementServiceError::Persistence(format!("runtime reload failed: {err}"))
        })?;
    let sample = collect_runtime_reload_sample(state);
    Ok(runtime_reload_response(RuntimeReloadResponseParts {
        active_registry_generation: sample.active_registry_generation,
        active_registry_version: sample.active_registry_version,
        staged_registry_version,
        channels: sample.channels,
    }))
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeRetryPolicy {
    pub same_request_retry_enabled_channels: usize,
    pub max_same_request_retries: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeRetryProfileSummary {
    pub channels: usize,
    pub same_request_credential_retry_enabled_channels: usize,
    pub route_target_retry_enabled_channels: usize,
    pub max_same_request_retries: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeRetryPressureCapacity {
    pub same_request_credential_retry_attempts: usize,
    pub route_target_fallback_candidates: usize,
    pub same_target_transient_retry_attempts: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeRecentRetryCounters {
    pub same_request_credential_retries: u64,
    pub route_target_fallbacks: u64,
    pub same_target_transient_retries: u64,
    pub terminal_retry_decisions: u64,
    pub window_capacity: usize,
    pub by_directive: RuntimeRetryPressureDirectiveCounters,
    pub by_denial_reason: BTreeMap<String, u64>,
    pub by_duplicate_charge_risk: RuntimeDuplicateChargeRiskCounters,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeRetryPressureDirectiveCounters {
    pub retry_credential: u64,
    pub retry_route_target: u64,
    pub retry_same_target: u64,
    pub return_error: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeDuplicateChargeRiskCounters {
    pub none: u64,
    pub known_no_charge: u64,
    pub unknown: u64,
}

impl RuntimeRecentRetryCounters {
    fn from_window(counters: RetryPressureCounters, window_capacity: usize) -> Self {
        Self {
            same_request_credential_retries: counters.by_directive.retry_credential,
            route_target_fallbacks: counters.by_directive.retry_route_target,
            same_target_transient_retries: counters.by_directive.retry_same_target,
            terminal_retry_decisions: counters.by_directive.return_error,
            window_capacity,
            by_directive: RuntimeRetryPressureDirectiveCounters {
                retry_credential: counters.by_directive.retry_credential,
                retry_route_target: counters.by_directive.retry_route_target,
                retry_same_target: counters.by_directive.retry_same_target,
                return_error: counters.by_directive.return_error,
            },
            by_denial_reason: counters.by_denial_reason,
            by_duplicate_charge_risk: RuntimeDuplicateChargeRiskCounters {
                none: counters.by_duplicate_charge_risk.none,
                known_no_charge: counters.by_duplicate_charge_risk.known_no_charge,
                unknown: counters.by_duplicate_charge_risk.unknown,
            },
        }
    }
}

fn runtime_retry_policy_from_topology_summary(
    summary: RuntimeTopologySummary,
) -> RuntimeRetryPolicy {
    RuntimeRetryPolicy {
        same_request_retry_enabled_channels: summary.same_request_retry_enabled_channels,
        max_same_request_retries: summary.max_same_request_retries,
    }
}

fn runtime_retry_profile_summary_from_topology(
    summary: RuntimeTopologySummary,
    route_target_retry_enabled_channels: usize,
) -> RuntimeRetryProfileSummary {
    RuntimeRetryProfileSummary {
        channels: summary.channels,
        same_request_credential_retry_enabled_channels: summary.same_request_retry_enabled_channels,
        route_target_retry_enabled_channels,
        max_same_request_retries: summary.max_same_request_retries,
    }
}

pub async fn readiness_response(state: &AppState) -> ReadinessResponse {
    let sample = collect_runtime_snapshot(state).await;
    let serving_channels = sample.serving_channels;
    let credential_set_blocking_alerts = sample.credential_set_blocking_alerts;
    let RuntimeReadinessProjection {
        status,
        blocking_alerts,
    } = runtime_readiness_projection(serving_channels, credential_set_blocking_alerts);
    ReadinessResponse {
        status,
        channels: sample.channels,
        serving_channels,
        credentials: sample.credentials,
        blocking_alerts,
        operator_input_alerts: sample.operator_input_alerts,
    }
}

#[derive(Debug, Serialize)]
pub struct ServingHealthResponse {
    pub status: &'static str,
    pub serving: bool,
    pub serving_channels: usize,
    pub blocking_alerts: usize,
    pub blocking_reasons: Vec<&'static str>,
    pub relay_suppression: RelaySuppressionHealthSummary,
    pub retry_pressure: RuntimeRecentRetryCounters,
    pub credential_spare_capacity: CredentialSpareCapacityHealthSummary,
    pub response_filter_alert_summary: ResponseFilterAlertHealthSummary,
}

pub async fn serving_health_response(state: &AppState) -> ServingHealthResponse {
    let sample = collect_runtime_snapshot(state).await;
    let serving_channels = sample.serving_channels;
    let credential_set_blocking_alerts = sample.credential_set_blocking_alerts;
    let RuntimeReadinessProjection {
        status,
        blocking_alerts,
    } = runtime_readiness_projection(serving_channels, credential_set_blocking_alerts);
    let mut blocking_reasons = Vec::new();
    if serving_channels == 0 {
        blocking_reasons.push("no_serving_channels");
    }
    if credential_set_blocking_alerts > 0 {
        blocking_reasons.push("credential_set_blocking_alerts");
    }
    let relay_suppression = relay_suppression_health_summary(state, sample.channels_cooling_down);
    let response_filter_alerts = response_filter_contamination_alerts_for_state(state);
    let credential_spare_capacity = credential_spare_capacity_summary(&sample);
    let retry_pressure = RuntimeRecentRetryCounters::from_window(
        sample.recent_retry_counters,
        sample.routing_telemetry_capacity,
    );
    ServingHealthResponse {
        status,
        serving: serving_channels > 0,
        serving_channels,
        blocking_alerts,
        blocking_reasons,
        relay_suppression,
        retry_pressure,
        credential_spare_capacity,
        response_filter_alert_summary: response_filter_alert_summary(response_filter_alerts),
    }
}

#[derive(Debug, Serialize)]
pub struct ResilienceHealthResponse {
    pub status: &'static str,
    pub operator_input_alerts: usize,
    pub credential_sets_without_spare: usize,
    pub channels_cooling_down: usize,
    pub response_filter_alerts: usize,
    pub relay_suppression: RelaySuppressionHealthSummary,
    pub retry_pressure: RuntimeRecentRetryCounters,
    pub credential_spare_capacity: CredentialSpareCapacityHealthSummary,
    pub response_filter_alert_summary: ResponseFilterAlertHealthSummary,
}

pub async fn resilience_health_response(state: &AppState) -> ResilienceHealthResponse {
    let sample = collect_runtime_snapshot(state).await;
    let response_filter_alerts = response_filter_contamination_alerts_for_state(state);
    let response_filter_alert_count = response_filter_alerts.len();
    let relay_suppression = relay_suppression_health_summary(state, sample.channels_cooling_down);
    let credential_spare_capacity = credential_spare_capacity_summary(&sample);
    let retry_pressure = RuntimeRecentRetryCounters::from_window(
        sample.recent_retry_counters,
        sample.routing_telemetry_capacity,
    );
    let status = if sample.serving_channels == 0 || sample.credential_set_blocking_alerts > 0 {
        "blocked"
    } else if sample.operator_input_alerts > 0
        || sample.credential_sets_without_spare > 0
        || sample.channels_cooling_down > 0
        || response_filter_alert_count > 0
    {
        "degraded"
    } else {
        "ok"
    };
    ResilienceHealthResponse {
        status,
        operator_input_alerts: sample.operator_input_alerts,
        credential_sets_without_spare: sample.credential_sets_without_spare,
        channels_cooling_down: sample.channels_cooling_down,
        response_filter_alerts: response_filter_alert_count,
        relay_suppression,
        retry_pressure,
        credential_spare_capacity,
        response_filter_alert_summary: response_filter_alert_summary(response_filter_alerts),
    }
}

#[derive(Debug, Serialize)]
pub struct RequestLimits {
    pub max_request_body_bytes: usize,
    pub max_model_catalog_body_bytes: usize,
    pub max_model_catalog_channels: usize,
    pub max_error_body_bytes: usize,
    pub max_route_candidates: usize,
}

#[derive(Debug, Serialize)]
pub struct TimeoutSeconds {
    pub connect: u64,
    pub non_streaming_total: u64,
    pub streaming_idle: u64,
}

pub fn duration_secs(duration: Duration) -> u64 {
    duration.as_secs()
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RelaySuppressionHealthSummary {
    pub channels_cooling_down: usize,
    pub suppressed_public_models: usize,
    pub suppressed_candidates: usize,
    pub affected_channels: Vec<String>,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CredentialSpareCapacityHealthSummary {
    pub credential_sets_without_spare: usize,
    pub total_credentials: usize,
    pub available_credentials: usize,
    pub cooling_down_credentials: usize,
    pub expired_credentials: usize,
    pub quota_exhausted_credentials: usize,
    pub disabled_credentials: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ResponseFilterAlertHealthSummary {
    pub alerts: usize,
    pub affected_channels: Vec<String>,
    pub rules: Vec<String>,
    pub redact_count: usize,
    pub reject_count: usize,
    pub window_seconds: Option<u64>,
}

fn relay_suppression_health_summary(
    state: &AppState,
    channels_cooling_down: usize,
) -> RelaySuppressionHealthSummary {
    let alerts = model_route_all_target_suppression_alerts(state);
    let mut affected_channels = BTreeSet::new();
    let mut reason_codes = BTreeSet::new();
    let mut suppressed_candidates = 0usize;
    for alert in &alerts {
        suppressed_candidates =
            suppressed_candidates.saturating_add(alert.suppressed_count.unwrap_or_default());
        affected_channels.extend(alert.channel_ids.iter().cloned());
        reason_codes.extend(alert.reason_codes.iter().cloned());
    }

    RelaySuppressionHealthSummary {
        channels_cooling_down,
        suppressed_public_models: alerts.len(),
        suppressed_candidates,
        affected_channels: affected_channels.into_iter().collect(),
        reason_codes: reason_codes.into_iter().collect(),
    }
}

fn credential_spare_capacity_summary(
    sample: &RuntimeSnapshotSample,
) -> CredentialSpareCapacityHealthSummary {
    CredentialSpareCapacityHealthSummary {
        credential_sets_without_spare: sample.credential_sets_without_spare,
        total_credentials: sample.credentials.total,
        available_credentials: sample.credentials.available,
        cooling_down_credentials: sample.credentials.cooling_down,
        expired_credentials: sample.credentials.expired,
        quota_exhausted_credentials: sample.credentials.quota_exhausted,
        disabled_credentials: sample.credentials.disabled,
    }
}

fn response_filter_alert_summary(
    alerts: Vec<ManagementAlertStatus>,
) -> ResponseFilterAlertHealthSummary {
    let mut affected_channels = BTreeSet::new();
    let mut rules = BTreeSet::new();
    let mut redact_count = 0usize;
    let mut reject_count = 0usize;
    let mut window_seconds = None;

    for alert in &alerts {
        affected_channels.extend(alert.channel_ids.iter().cloned());
        if let Some(rule_id) = &alert.rule_id {
            rules.insert(rule_id.clone());
        }
        redact_count = redact_count.saturating_add(alert.redact_count.unwrap_or_default());
        reject_count = reject_count.saturating_add(alert.reject_count.unwrap_or_default());
        window_seconds = window_seconds.or(alert.window_seconds);
    }

    ResponseFilterAlertHealthSummary {
        alerts: alerts.len(),
        affected_channels: affected_channels.into_iter().collect(),
        rules: rules.into_iter().collect(),
        redact_count,
        reject_count,
        window_seconds,
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_body<'a>(source: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
        let start = source
            .find(start_marker)
            .unwrap_or_else(|| panic!("{start_marker} exists"));
        let end = source[start..]
            .find(end_marker)
            .map(|offset| start + offset)
            .unwrap_or_else(|| panic!("{end_marker} follows {start_marker}"));
        &source[start..end]
    }

    #[test]
    fn response_filter_events_response_reports_capacity_and_dropped_events() {
        let response = response_filter_events_response(Vec::new(), 1024, 3, 0, 50);

        assert_eq!(response.buffered_events, 0);
        assert_eq!(response.capacity, 1024);
        assert_eq!(response.dropped_events, 3);
        assert_eq!(response.offset, 0);
        assert_eq!(response.limit, 50);
    }

    #[test]
    fn response_filter_events_response_projects_failure_events_for_cli_without_raw_text() {
        let response = response_filter_events_response(
            vec![
                ResponseFilterEvent {
                    event_id: 1,
                    created_at_unix_seconds: 10,
                    request_id: "req_filter".to_string(),
                    channel_id: "test".to_string(),
                    public_model: "gpt-test".to_string(),
                    rule_id: "rule-secret".to_string(),
                    action: "reject_and_expire_credential".to_string(),
                    content_kind: "json".to_string(),
                    reason_code: "response_filter_rejected".to_string(),
                    outcome: "rejected".to_string(),
                    body_committed: false,
                },
                ResponseFilterEvent {
                    event_id: 2,
                    created_at_unix_seconds: 11,
                    request_id: "req_allowed".to_string(),
                    channel_id: "test".to_string(),
                    public_model: "gpt-test".to_string(),
                    rule_id: "rule-allowed".to_string(),
                    action: "allow".to_string(),
                    content_kind: "json".to_string(),
                    reason_code: "none".to_string(),
                    outcome: "allowed".to_string(),
                    body_committed: false,
                },
            ],
            64,
            0,
            0,
            50,
        );
        let value = serde_json::to_value(response).unwrap();
        let body = value.to_string();
        let failures = value["failure_events"].as_array().unwrap();

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["request_id"], "req_filter");
        assert_eq!(failures[0]["source"], "response_filter_events");
        assert_eq!(failures[0]["stage"], "response_filter");
        assert_eq!(failures[0]["failure_class"], "response_filter_rejected");
        assert_eq!(failures[0]["router_action"], "marked_credential");
        assert_eq!(failures[0]["retry_eligibility"], "eligible_before_output");
        assert_eq!(failures[0]["client_visible_status"], "local_502");
        assert_eq!(failures[0]["reason_code"], "response_filter_rejected");
        assert!(failures[0].get("client_token_ref").is_none());
        assert!(!body.contains("client_token"));
        assert!(!body.contains("matched_text"));
        assert!(!body.contains("response_body"));
    }

    #[test]
    fn failure_transition_summary_projects_credential_expiration_without_raw_ids() {
        let response = routing_telemetry_response(
            vec![
                upstream_failure_event(
                    "req_expire",
                    "test",
                    failure_telemetry("auth_invalid", "credential", "upstream_transaction"),
                ),
                RoutingTelemetry::CredentialTransitionApplied {
                    request_id: "req_expire".to_string(),
                    channel_id: "test".to_string(),
                    credential_id: "internal/credential/id".to_string(),
                    state: "expired".to_string(),
                    reason: "upstream_auth_invalid".to_string(),
                },
                RoutingTelemetry::TransitionApplied {
                    request_id: "req_expire".to_string(),
                    channel_id: "test".to_string(),
                },
            ],
            64,
            0,
            0,
            50,
        );
        let value = serde_json::to_value(response).unwrap();
        let body = value.to_string();

        assert!(!body.contains("internal/credential/id"));
        assert_eq!(value["events"].as_array().unwrap().len(), 3);
        let summaries = value["failure_transition_summaries"].as_array().unwrap();
        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary["failure_kind"], "auth_invalid");
        assert_eq!(summary["failure_scope"], "credential");
        assert_eq!(summary["failure_source"], "upstream_transaction");
        assert_eq!(summary["transition_action"], "expire_credential");
        assert_eq!(summary["mutation_kind"], "credential_transition");
        assert_eq!(summary["affected_resource_kind"], "credential");
        assert_eq!(summary["resulting_state"], "expired");
        assert_eq!(
            summary["affected_credential_id_hash"],
            crate::credentials::short_hash("internal/credential/id")
        );
        assert_eq!(summary["request_id"], "req_expire");
        assert_eq!(summary["channel_id"], "test");
        assert_eq!(summary["public_model"], "gpt-test");
        assert_eq!(summary["retry_decision"], "return_current_error");
        assert_eq!(summary["retry_decision_reason"], "failure_not_retryable");

        let failures = value["failure_events"].as_array().unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["request_id"], "req_expire");
        assert_eq!(failures[0]["source"], "routing_telemetry");
        assert_eq!(failures[0]["stage"], "upstream_transport");
        assert_eq!(failures[0]["failure_class"], "credential_unavailable");
        assert_eq!(failures[0]["router_action"], "marked_credential");
        assert_eq!(
            failures[0]["retry_eligibility"],
            "blocked_failure_not_retryable"
        );
        assert_eq!(failures[0]["retry_blocked_reason"], "failure_not_retryable");
        assert_eq!(failures[0]["client_visible_status"], "upstream_4xx");
        assert_eq!(failures[0]["reason_code"], "credential_unavailable");
    }

    #[test]
    fn failure_transition_summary_projects_credential_cooldown_and_quota() {
        let response = routing_telemetry_response(
            vec![
                upstream_failure_event(
                    "req_cooldown",
                    "test",
                    failure_telemetry("rate_limited", "credential", "upstream_transaction"),
                ),
                RoutingTelemetry::CredentialTransitionApplied {
                    request_id: "req_cooldown".to_string(),
                    channel_id: "test".to_string(),
                    credential_id: "credential-a".to_string(),
                    state: "cooling_down".to_string(),
                    reason: "upstream_rate_limited".to_string(),
                },
                upstream_failure_event(
                    "req_quota",
                    "test",
                    failure_telemetry("quota_exhausted", "credential", "upstream_transaction"),
                ),
                RoutingTelemetry::CredentialTransitionApplied {
                    request_id: "req_quota".to_string(),
                    channel_id: "test".to_string(),
                    credential_id: "credential-b".to_string(),
                    state: "quota_exhausted".to_string(),
                    reason: "upstream_quota_exhausted".to_string(),
                },
            ],
            64,
            0,
            0,
            50,
        );
        let value = serde_json::to_value(response).unwrap();
        let summaries = value["failure_transition_summaries"].as_array().unwrap();

        assert_eq!(summaries.len(), 2);
        assert_eq!(
            summaries[0]["transition_action"],
            "mark_credential_cooling_down"
        );
        assert_eq!(summaries[0]["resulting_state"], "cooling_down");
        assert_eq!(
            summaries[1]["transition_action"],
            "mark_credential_quota_exhausted"
        );
        assert_eq!(summaries[1]["resulting_state"], "quota_exhausted");
    }

    #[test]
    fn failure_transition_summary_projects_channel_cooldown_and_degraded() {
        let response = routing_telemetry_response(
            vec![
                upstream_failure_event(
                    "req_channel_cooldown",
                    "test",
                    failure_telemetry(
                        "relay_balance_unavailable",
                        "channel",
                        "upstream_transaction",
                    ),
                ),
                RoutingTelemetry::ChannelHealthTransitionApplied {
                    request_id: "req_channel_cooldown".to_string(),
                    channel_id: "test".to_string(),
                    state: "cooling_down".to_string(),
                    reason: "relay_balance_unavailable".to_string(),
                },
                upstream_failure_event(
                    "req_degraded",
                    "test",
                    failure_telemetry("provider_unavailable", "channel", "upstream_transaction"),
                ),
                RoutingTelemetry::ChannelHealthTransitionApplied {
                    request_id: "req_degraded".to_string(),
                    channel_id: "test".to_string(),
                    state: "degraded".to_string(),
                    reason: "upstream_provider_unavailable".to_string(),
                },
            ],
            64,
            0,
            0,
            50,
        );
        let value = serde_json::to_value(response).unwrap();
        let summaries = value["failure_transition_summaries"].as_array().unwrap();

        assert_eq!(summaries.len(), 2);
        assert_eq!(
            summaries[0]["transition_action"],
            "mark_relay_balance_channel_cooling_down"
        );
        assert_eq!(summaries[0]["affected_resource_kind"], "channel");
        assert_eq!(summaries[0]["resulting_state"], "cooling_down");
        assert_eq!(
            summaries[1]["transition_action"],
            "mark_provider_account_channel_degraded"
        );
        assert_eq!(summaries[1]["resulting_state"], "degraded");
    }

    #[test]
    fn failure_transition_summary_projects_noop_for_request_only_failures() {
        let response = routing_telemetry_response(
            vec![upstream_failure_event(
                "req_noop",
                "test",
                failure_telemetry("client_error", "request_only", "upstream_transaction"),
            )],
            64,
            0,
            0,
            50,
        );
        let value = serde_json::to_value(response).unwrap();
        let summary = &value["failure_transition_summaries"].as_array().unwrap()[0];

        assert_eq!(summary["failure_kind"], "client_error");
        assert_eq!(summary["failure_scope"], "request_only");
        assert_eq!(summary["transition_action"], "noop");
        assert_eq!(summary["mutation_kind"], "noop");
        assert_eq!(summary["affected_resource_kind"], "none");
        assert_eq!(summary["resulting_state"], "noop");
    }

    #[test]
    fn routing_failure_events_distinguish_local_admission_denial_from_upstream_503() {
        let mut upstream_503 =
            failure_telemetry("provider_unavailable", "channel", "upstream_transaction");
        upstream_503.status = Some(503);

        let response = routing_telemetry_response(
            vec![
                RoutingTelemetry::RouteAdmissionDenied {
                    request_id: "req_local_503".to_string(),
                    registry_generation: 9,
                    endpoint_family: "chat_completions".to_string(),
                    public_model: Some("gpt-test".to_string()),
                    client_token_ref: Some("local-client".to_string()),
                    route_kind: "explicit_model_route".to_string(),
                    reason_code: "no_route_candidate".to_string(),
                    blocking_domain: "route".to_string(),
                    client_visible_status: 503,
                    upstream_status: None,
                    candidate_count: 2,
                    included_count: 0,
                    blocked_count: 2,
                    hard_blocked_count: 2,
                    soft_suppressed_count: 0,
                    last_resort_used: false,
                    hard_reason_codes: vec![
                        "channel_disabled".to_string(),
                        "no_available_credentials".to_string(),
                    ],
                    soft_reason_codes: Vec::new(),
                },
                upstream_failure_event("req_upstream_503", "test", upstream_503),
            ],
            64,
            0,
            0,
            50,
        );
        let value = serde_json::to_value(response).unwrap();
        let failures = value["failure_events"].as_array().unwrap();

        assert_eq!(failures.len(), 2);
        let local = failures
            .iter()
            .find(|failure| failure["request_id"] == "req_local_503")
            .expect("local admission denial should be projected");
        assert_eq!(local["event_kind"], "route_admission_denied");
        assert_eq!(local["stage"], "route_admission");
        assert_eq!(local["reason_code"], "no_route_candidate");
        assert_eq!(local["blocking_domain"], "route");
        assert_eq!(local["client_visible_status"], "local_503");
        assert!(local["upstream_status"].is_null());
        assert_eq!(local["admission"]["candidate_count"], 2);
        assert_eq!(local["admission"]["included_count"], 0);
        assert_eq!(
            local["admission"]["hard_reason_codes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            local["next_action"]["side_effect_class"],
            "runtime_readonly"
        );

        let upstream = failures
            .iter()
            .find(|failure| failure["request_id"] == "req_upstream_503")
            .expect("upstream failure should be projected");
        assert_eq!(upstream["event_kind"], "upstream_failure_observed");
        assert_eq!(upstream["stage"], "upstream_transport");
        assert_eq!(upstream["client_visible_status"], "upstream_5xx");
        assert_eq!(upstream["upstream_status"], 503);
        assert_eq!(upstream["reason_code"], "upstream_5xx");
    }

    #[test]
    fn routing_failure_events_preserve_telemetry_order_for_primary_tie_breaks() {
        let mut upstream_503 =
            failure_telemetry("provider_unavailable", "channel", "upstream_transaction");
        upstream_503.status = Some(503);

        let response = routing_telemetry_response(
            vec![
                RoutingTelemetry::RouteAdmissionDenied {
                    request_id: "req_local_earlier".to_string(),
                    registry_generation: 9,
                    endpoint_family: "chat_completions".to_string(),
                    public_model: Some("gpt-test".to_string()),
                    client_token_ref: Some("local-client".to_string()),
                    route_kind: "explicit_model_route".to_string(),
                    reason_code: "no_route_candidate".to_string(),
                    blocking_domain: "route".to_string(),
                    client_visible_status: 503,
                    upstream_status: None,
                    candidate_count: 1,
                    included_count: 0,
                    blocked_count: 1,
                    hard_blocked_count: 1,
                    soft_suppressed_count: 0,
                    last_resort_used: false,
                    hard_reason_codes: vec!["channel_cooling_down".to_string()],
                    soft_reason_codes: Vec::new(),
                },
                upstream_failure_event("req_upstream_later", "test", upstream_503),
            ],
            64,
            0,
            0,
            50,
        );
        let value = serde_json::to_value(response).unwrap();
        let failures = value["failure_events"].as_array().unwrap();

        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0]["request_id"], "req_local_earlier");
        assert_eq!(failures[0]["event_kind"], "route_admission_denied");
        assert_eq!(failures[1]["request_id"], "req_upstream_later");
        assert_eq!(failures[1]["event_kind"], "upstream_failure_observed");
    }

    #[test]
    fn failure_transition_summary_projects_response_filter_precommit_actions_without_raw_upstream_text(
    ) {
        let response = routing_telemetry_response(
            vec![
                upstream_failure_event(
                    "req_filter_credential",
                    "test",
                    failure_telemetry(
                        "response_filter_rejected",
                        "credential",
                        "response_filter_precommit",
                    ),
                ),
                RoutingTelemetry::CredentialTransitionApplied {
                    request_id: "req_filter_credential".to_string(),
                    channel_id: "test".to_string(),
                    credential_id: "credential-filter".to_string(),
                    state: "expired".to_string(),
                    reason: "response_filter_rejected".to_string(),
                },
                upstream_failure_event(
                    "req_filter_channel",
                    "test",
                    failure_telemetry(
                        "response_filter_rejected",
                        "channel",
                        "response_filter_precommit",
                    ),
                ),
                RoutingTelemetry::ChannelHealthTransitionApplied {
                    request_id: "req_filter_channel".to_string(),
                    channel_id: "test".to_string(),
                    state: "cooling_down".to_string(),
                    reason: "response_filter_rejected".to_string(),
                },
            ],
            64,
            0,
            0,
            50,
        );
        let value = serde_json::to_value(response).unwrap();
        let body = value.to_string();

        assert!(!body.contains("raw upstream text"));
        assert!(!body.contains("request_body"));
        assert!(!body.contains("response_body"));
        let summaries = value["failure_transition_summaries"].as_array().unwrap();
        assert_eq!(summaries[0]["transition_action"], "expire_credential");
        assert_eq!(summaries[0]["failure_source"], "response_filter_precommit");
        assert_eq!(
            summaries[1]["transition_action"],
            "mark_channel_cooling_down"
        );
        assert_eq!(summaries[1]["failure_source"], "response_filter_precommit");
    }

    #[test]
    fn failure_transition_summary_is_management_projection_only_and_not_routing_input() {
        let source = include_str!("management_runtime.rs");
        assert!(source.contains("failure_transition_summaries"));

        for hot_path in [
            include_str!("routing.rs"),
            include_str!("failure_state_executor.rs"),
            include_str!("proxy.rs"),
        ] {
            assert!(
                !hot_path.contains("failure_transition_summaries"),
                "failure transition summary must remain outside routing inputs and mutation execution"
            );
        }
    }

    fn upstream_failure_event(
        request_id: &str,
        channel_id: &str,
        failure: UpstreamFailureTelemetry,
    ) -> RoutingTelemetry {
        RoutingTelemetry::UpstreamFailureObserved {
            request_id: request_id.to_string(),
            channel_id: channel_id.to_string(),
            failure: Box::new(failure),
        }
    }

    fn failure_telemetry(
        failure_kind: &str,
        failure_scope: &str,
        failure_source: &str,
    ) -> UpstreamFailureTelemetry {
        UpstreamFailureTelemetry {
            public_model: Some("gpt-test".to_string()),
            credential_id_hash: "safe-credential-hash".to_string(),
            attempt: 0,
            failure_source: failure_source.to_string(),
            failure_kind: failure_kind.to_string(),
            failure_scope: failure_scope.to_string(),
            retryable: false,
            confidence: "high".to_string(),
            status: Some(401),
            classifier_id: "test-classifier".to_string(),
            classifier_version: "1".to_string(),
            adaptation_rule_id: None,
            retry_after_source: None,
            cooldown_seconds: None,
            directive: "return_error".to_string(),
            denial_reason: Some("failure_not_retryable".to_string()),
            duplicate_charge_risk: "none".to_string(),
            effective_deadline_remaining_ms: Some(500),
            retry_pressure_accounted: true,
            retry_decision: "return_current_error".to_string(),
            retry_decision_reason: Some("failure_not_retryable".to_string()),
        }
    }

    #[test]
    fn total_dropped_events_includes_external_lock_contention_drops() {
        let external = std::sync::atomic::AtomicU64::new(2);

        assert_eq!(total_dropped_events(3, &external), 5);
    }

    #[test]
    fn runtime_and_readiness_responses_consume_runtime_snapshot_sample() {
        let source = include_str!("management_runtime.rs");
        assert!(source.contains("struct RuntimeSnapshotSample"));
        assert!(source.contains("async fn collect_runtime_snapshot"));

        let runtime_body = item_body(
            source,
            "pub async fn runtime_response",
            "\npub async fn runtime_response_for_state",
        );
        let readiness_body = item_body(
            source,
            "pub async fn readiness_response",
            "\n#[derive(Debug, Serialize)]\npub struct ServingHealthResponse",
        );
        let serving_health_body = item_body(
            source,
            "pub async fn serving_health_response",
            "\n#[derive(Debug, Serialize)]\npub struct ResilienceHealthResponse",
        );
        let resilience_health_body = item_body(
            source,
            "pub async fn resilience_health_response",
            "\n#[derive(Debug, Serialize)]\npub struct RequestLimits",
        );

        for body in [
            runtime_body,
            readiness_body,
            serving_health_body,
            resilience_health_body,
        ] {
            assert!(body.contains("collect_runtime_snapshot(state).await"));
            for forbidden_token in [
                "CredentialSetSnapshotCache::default()",
                "BTreeSet::new()",
                "add_counts_once(",
                "snapshot_for(",
            ] {
                assert!(
                    !body.contains(forbidden_token),
                    "response functions should consume sampled runtime state, not {forbidden_token}"
                );
            }
        }

        let collector_body = item_body(
            source,
            "async fn collect_runtime_snapshot",
            "\nfn collect_runtime_reload_sample",
        );
        for forbidden_token in [
            "credential_set_readiness_projection_from_snapshot(",
            "snapshot_for(",
            "runtime_retry_policy(state)",
            "state.channels.len()",
            "state.channels.channel_values_snapshot()",
            "mark_credential_set_seen(&pool_state)",
        ] {
            assert!(
                !collector_body.contains(forbidden_token),
                "runtime snapshot collector should consume credential-set runtime samples, not {forbidden_token}"
            );
        }
    }
}
