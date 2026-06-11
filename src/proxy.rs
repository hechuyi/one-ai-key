use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::Response,
};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::Body as ReqwestBody;

use crate::{
    auth::{authorize_client, json_error, json_error_with_code},
    credentials::CredentialId,
    error::{ClassifiedFailure, FailureConfidence, FailureKind, FailureScope},
    events::RoutingTelemetry,
    failure_observer::{
        record_routing_telemetry, transition_observed_failure, transition_observed_upstream_failure,
    },
    model_catalog,
    provider::{
        bytes_from_body_for_context, EndpointKind, InboundProtocol, NamedPoolRequestMode,
        ProviderAdapter,
    },
    response_filter::{ResponseFilterDecision, ResponseFilterMatch},
    route_plan::{
        plan_route, preview_route, route_preview_reason_is_hard_blocker,
        route_preview_reason_is_soft_suppression, ChannelRouteState, RouteCandidate, RoutePlan,
        RoutePlanInput, RoutePreview, RoutePreviewInput, RoutePreviewReason,
    },
    routing::{
        apply_retry_directive_to_attempt_state, FailureSource, FrozenRetryCandidates,
        RequestSelectionSnapshot, RetryAttemptContinuation, SelectionReason,
        CONSERVATIVE_RETRY_BUDGET,
    },
    state::{AppState, ChannelId, PoolState},
    success_guard::{
        guard_success_response, should_guard_success_status, GuardOutcome, GuardResult,
    },
    upstream_response::{
        prefixed_body_stream, read_limited_body, response_with_headers,
        stream_response_from_byte_stream_with_filter_events, stream_response_with_filter_events,
        BodyStreamFailureObserver, ResponseFilterEventContext, ResponseFilterEventOptions,
    },
};

use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, TryLockError},
    time::{Duration, Instant},
};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
const SUCCESS_GUARD_MAX_BYTES: usize = 8192;
const SUCCESS_GUARD_MAX_DURATION: Duration = Duration::from_millis(200);

fn next_request_id() -> String {
    format!("req_{}", REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn body_stream_failure_observer(
    state: AppState,
    pool_state: PoolState,
    snapshot: RequestSelectionSnapshot,
) -> BodyStreamFailureObserver {
    std::sync::Arc::new(move |partial_output_started| {
        let state = state.clone();
        let pool_state = pool_state.clone();
        let mut snapshot = snapshot.clone();
        Box::pin(async move {
            snapshot.partial_output_started = partial_output_started;
            let failure = pool_state.error_classifier.classify_transport_failure();
            let _ = transition_observed_failure(
                &state,
                &pool_state,
                &snapshot,
                failure,
                FailureSource::LocalTransport,
            )
            .await;
        })
    })
}

fn response_filter_event_options(
    state: &AppState,
    route_plan: &FrozenRoutePlan,
    snapshot: &RequestSelectionSnapshot,
) -> ResponseFilterEventOptions {
    let events = state.response_filter_events.clone();
    let lock_contention_drops = state.response_filter_event_lock_contention_drops.clone();
    ResponseFilterEventOptions {
        context: ResponseFilterEventContext {
            request_id: snapshot.request_id.clone(),
            channel_id: snapshot.channel_id.0.clone(),
            public_model: route_plan
                .public_model
                .clone()
                .or_else(|| snapshot.requested_model.clone())
                .unwrap_or_else(|| "unknown".to_string()),
        },
        sink: std::sync::Arc::new(move |event| {
            record_response_filter_event_to_buffer(&events, &lock_contention_drops, event);
        }),
    }
}

fn record_response_filter_event_to_buffer(
    events: &Arc<Mutex<crate::events::ResponseFilterEventBuffer>>,
    lock_contention_drops: &Arc<AtomicU64>,
    event: crate::events::ResponseFilterEventInput,
) {
    match events.try_lock() {
        Ok(mut events) => {
            let _ = events.push(event);
        }
        Err(TryLockError::WouldBlock) => {
            lock_contention_drops.fetch_add(1, Ordering::Relaxed);
        }
        Err(TryLockError::Poisoned(err)) => {
            let _ = err.into_inner().push(event);
        }
    }
}

fn guarded_success_envelope_response() -> Response {
    json_error_with_code(
        StatusCode::BAD_GATEWAY,
        "guarded_success_envelope",
        "upstream returned a structured error envelope with success status",
    )
}

struct PrecommitFilterRejection {
    failure: Option<ClassifiedFailure>,
    response: Response,
}

enum BufferedBodyFilterDecision {
    Redacted(Bytes),
    Rejected(Box<PrecommitFilterRejection>),
}

fn inspect_response_filter_before_commit(
    state: &AppState,
    route_plan: &FrozenRoutePlan,
    snapshot: &RequestSelectionSnapshot,
    headers: &HeaderMap,
    guard_outcome: GuardOutcome,
    prefix: &Bytes,
) -> Option<PrecommitFilterRejection> {
    if has_non_identity_content_encoding(headers) {
        return None;
    }
    let Ok(text) = std::str::from_utf8(prefix) else {
        return None;
    };
    let policy = state
        .response_filter
        .read()
        .expect("response filter lock poisoned")
        .clone();
    let ResponseFilterDecision::Rejected { matches } =
        policy.inspect_text_for_precommit(text, matches!(guard_outcome, GuardOutcome::Pass))
    else {
        return None;
    };
    emit_precommit_response_filter_events(
        state, route_plan, snapshot, headers, &matches, "rejected",
    );
    let failure =
        (snapshot.body_replayable && !snapshot.streaming && !snapshot.partial_output_started)
            .then(|| {
                matches
                    .iter()
                    .find_map(|matched| {
                        matched
                            .action
                            .lifecycle_failure_scope()
                            .map(|scope| (matched, scope))
                    })
                    .map(|(matched, scope)| response_filter_failure(snapshot, matched, scope, 200))
            })
            .flatten();
    Some(PrecommitFilterRejection {
        failure,
        response: response_filter_precommit_rejected_response(),
    })
}

fn inspect_response_filter_buffered_body_before_commit(
    state: &AppState,
    route_plan: &FrozenRoutePlan,
    snapshot: &RequestSelectionSnapshot,
    headers: &HeaderMap,
    upstream_status: StatusCode,
    body: &Bytes,
) -> Option<BufferedBodyFilterDecision> {
    if has_non_identity_content_encoding(headers) {
        return None;
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return None;
    };
    let policy = state
        .response_filter
        .read()
        .expect("response filter lock poisoned")
        .clone();
    match policy.inspect_text_for_precommit(text, false) {
        ResponseFilterDecision::Unchanged => None,
        ResponseFilterDecision::Redacted { text, matches } => {
            emit_precommit_response_filter_events(
                state, route_plan, snapshot, headers, &matches, "redacted",
            );
            Some(BufferedBodyFilterDecision::Redacted(Bytes::from(text)))
        }
        ResponseFilterDecision::Rejected { matches } => {
            emit_precommit_response_filter_events(
                state, route_plan, snapshot, headers, &matches, "rejected",
            );
            let failure = (snapshot.body_replayable
                && !snapshot.streaming
                && !snapshot.partial_output_started)
                .then(|| {
                    matches
                        .iter()
                        .find_map(|matched| {
                            matched
                                .action
                                .lifecycle_failure_scope()
                                .map(|scope| (matched, scope))
                        })
                        .map(|(matched, scope)| {
                            response_filter_failure(
                                snapshot,
                                matched,
                                scope,
                                upstream_status.as_u16(),
                            )
                        })
                })
                .flatten();
            Some(BufferedBodyFilterDecision::Rejected(Box::new(
                PrecommitFilterRejection {
                    failure,
                    response: response_filter_precommit_rejected_response(),
                },
            )))
        }
    }
}

fn emit_precommit_response_filter_events(
    state: &AppState,
    route_plan: &FrozenRoutePlan,
    snapshot: &RequestSelectionSnapshot,
    headers: &HeaderMap,
    matches: &[ResponseFilterMatch],
    outcome: &str,
) {
    let context = response_filter_event_options(state, route_plan, snapshot).context;
    let content_kind = response_filter_content_kind(headers);
    for matched_rule in matches {
        record_response_filter_event_to_buffer(
            &state.response_filter_events,
            &state.response_filter_event_lock_contention_drops,
            crate::events::ResponseFilterEventInput {
                request_id: context.request_id.clone(),
                channel_id: context.channel_id.clone(),
                public_model: context.public_model.clone(),
                rule_id: matched_rule.rule_id.clone(),
                action: matched_rule.action.as_str().to_string(),
                content_kind: content_kind.to_string(),
                reason_code: matched_rule.reason_code.to_string(),
                outcome: outcome.to_string(),
                body_committed: false,
            },
        );
    }
}

fn response_filter_content_kind(headers: &HeaderMap) -> &'static str {
    let media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if media_type == "text/event-stream" {
        "sse"
    } else if media_type == "application/json" || media_type.ends_with("+json") {
        "json"
    } else {
        "unknown"
    }
}

fn has_non_identity_content_encoding(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .any(|value| !value.eq_ignore_ascii_case("identity"))
        })
        .unwrap_or(false)
}

fn response_filter_failure(
    snapshot: &RequestSelectionSnapshot,
    matched: &ResponseFilterMatch,
    scope: FailureScope,
    upstream_status: u16,
) -> ClassifiedFailure {
    ClassifiedFailure {
        kind: FailureKind::ResponseFilterRejected,
        primary_scope: scope,
        retryable: true,
        cooldown: None,
        retry_after_source: None,
        confidence: FailureConfidence::High,
        upstream_status: Some(upstream_status),
        upstream_code: None,
        upstream_limit_type: None,
        classifier_id: snapshot.classifier_id.clone(),
        classifier_version: snapshot.classifier_version.clone(),
        adaptation_rule_id: Some(matched.rule_id.clone()),
    }
}

fn response_filter_precommit_rejected_response() -> Response {
    json_error_with_code(
        StatusCode::BAD_GATEWAY,
        "response_filter_rejected",
        "upstream response content was blocked by response filter",
    )
}

fn response_with_filtered_buffered_body(
    status: StatusCode,
    mut headers: HeaderMap,
    body: Bytes,
) -> Response {
    headers.remove(header::CONTENT_LENGTH);
    if !has_non_identity_content_encoding(&headers) {
        headers.remove(header::CONTENT_ENCODING);
    }
    response_with_headers(status, headers, Body::from(body))
}

pub async fn proxy_openai_compatible(State(state): State<AppState>, req: Request) -> Response {
    let client = match authorize_client(&state, req.headers()) {
        Ok(client) => client,
        Err(resp) => return *resp,
    };
    let _client_context = (&client.id, &client.name);

    let max_request_body_bytes = state.max_request_body_bytes;
    let (parts, raw_body) = req.into_parts();
    let path = parts.uri.path().to_string();
    let preliminary_context = InboundProtocol::OpenAiCompatible.request_context(
        &parts.method,
        &path,
        bytes_from_body_for_context(&Bytes::new()),
    );
    let request_id = next_request_id();
    if preliminary_context.endpoint == EndpointKind::Models {
        let public_catalog = state.channels.public_model_catalog();
        return model_catalog::compiled_openai_models(
            public_catalog,
            &state.runtime_catalogs,
            &client.allowed_model_groups,
            &client.allowed_channels,
        );
    }

    let body = match axum::body::to_bytes(raw_body, max_request_body_bytes).await {
        Ok(body) => body,
        Err(err) => {
            return json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("request body exceeded limit or is invalid: {err}"),
            )
        }
    };
    let context = InboundProtocol::OpenAiCompatible.request_context(
        &parts.method,
        &path,
        bytes_from_body_for_context(&body),
    );
    let model = context.requested_model.as_deref();
    let route_context = state.channels.route_plan_context(model);
    if model.is_some()
        && route_context.model_route.is_none()
        && route_context.has_claimed_upstream_model(&client.allowed_channels)
    {
        return json_error(
            StatusCode::FORBIDDEN,
            "model must be requested through its public route",
        );
    }
    let route_plan = match route_plan_for_openai_request(
        &state,
        model,
        &client.allowed_channels,
        &request_id,
        context.endpoint,
        &client.id,
        &route_context,
    ) {
        Ok(targets) => targets,
        Err(response) => return *response,
    };
    if route_plan.targets.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "no matching pool".to_string());
    }
    let selection_reason = route_plan.selection_reason;
    if !client.allowed_model_groups.is_empty() {
        match model {
            Some(model)
                if state
                    .runtime_catalogs
                    .client_model_allowed(&client.allowed_model_groups, model) => {}
            _ => return json_error(StatusCode::FORBIDDEN, "model not allowed for client token"),
        }
    }
    let query = parts.uri.query().map(ToOwned::to_owned);
    let effective_deadline = effective_deadline_for_request(
        context.streaming,
        state.timeout_profile.non_streaming_total,
    );

    forward_with_pools(ForwardRequest {
        state,
        route_plan,
        method: parts.method,
        headers: parts.headers,
        path,
        query,
        body,
        client_token_id: client.id,
        request_context: context,
        selection_reason,
        effective_deadline,
    })
    .await
}

fn requested_model_is_claimed_upstream_model(
    state: &AppState,
    requested_model: &str,
    allowed_channels: &[String],
) -> bool {
    state
        .channels
        .has_claimed_upstream_model(requested_model, allowed_channels)
}

fn route_plan_for_openai_request(
    state: &AppState,
    model: Option<&str>,
    allowed_channels: &[String],
    request_id: &str,
    endpoint: EndpointKind,
    client_token_ref: &str,
    route_context: &crate::state::ChannelRoutePlanContext,
) -> Result<FrozenRoutePlan, Box<Response>> {
    if let Some(route) = route_context.model_route.as_ref() {
        let preview = preview_route(RoutePreviewInput {
            request_id: request_id.to_string(),
            registry_generation: route_context.registry_generation,
            public_model: model.map(ToOwned::to_owned),
            route: Some(route),
            channel_states: &route_context.model_route_channel_states,
            allowed_channels,
            candidate_limit: state.routing.max_route_candidates,
        });
        if let Ok(plan) = plan_route(RoutePlanInput {
            request_id: request_id.to_string(),
            registry_generation: route_context.registry_generation,
            public_model: model.map(ToOwned::to_owned),
            route: Some(route),
            channel_states: &route_context.model_route_channel_states,
            allowed_channels,
            candidate_limit: state.routing.max_route_candidates,
        }) {
            return Ok(FrozenRoutePlan::from_plan(
                plan,
                &route_context.model_route_channel_states,
            ));
        }
        if all_relevant_route_candidates_are_cooling_down(&preview) {
            record_route_admission_denied(
                state,
                &preview,
                no_route_candidate_denial(endpoint, Some(client_token_ref), "explicit_model_route"),
            );
            return Err(Box::new(no_route_candidate_response(&[
                "channel_cooling_down",
            ])));
        }
        let reason_codes = relevant_route_candidate_reason_codes(&preview);
        if !reason_codes.is_empty() {
            record_route_admission_denied(
                state,
                &preview,
                no_route_candidate_denial(endpoint, Some(client_token_ref), "explicit_model_route"),
            );
            return Err(Box::new(no_route_candidate_response(&reason_codes)));
        }
        return Ok(FrozenRoutePlan {
            request_id: request_id.to_string(),
            registry_generation: route_context.registry_generation,
            public_model: model.map(ToOwned::to_owned),
            selection_reason: SelectionReason::ModelMapping,
            targets: Vec::new(),
        });
    }
    let Some(pool_name) = route_context.default_channel.as_deref() else {
        return Err(Box::new(json_error(
            StatusCode::BAD_REQUEST,
            "no matching pool".to_string(),
        )));
    };
    if !allowed_channels.is_empty() && !allowed_channels.iter().any(|allowed| allowed == pool_name)
    {
        return Err(Box::new(json_error(
            StatusCode::BAD_REQUEST,
            "no matching pool".to_string(),
        )));
    }
    let route_state = route_context
        .default_channel_route_state
        .unwrap_or(ChannelRouteState::UnknownChannel);
    if matches!(route_state, ChannelRouteState::CoolingDown) {
        record_single_route_admission_denied(
            state,
            request_id,
            route_context.registry_generation,
            no_route_candidate_denial(endpoint, Some(client_token_ref), "default_channel"),
            model,
            &["channel_cooling_down"],
        );
        return Err(Box::new(no_route_candidate_response(&[
            "channel_cooling_down",
        ])));
    }
    if let Some(response) = route_state_unavailable_response(pool_name, route_state) {
        return Err(Box::new(response));
    }
    Ok(FrozenRoutePlan::single_default(
        request_id.to_string(),
        route_context.registry_generation,
        pool_name.to_string(),
        route_state,
    ))
}

struct RouteAdmissionDenial<'a> {
    endpoint: EndpointKind,
    client_token_ref: Option<&'a str>,
    route_kind: &'a str,
    reason_code: &'a str,
    blocking_domain: &'a str,
    client_visible_status: StatusCode,
}

fn no_route_candidate_denial<'a>(
    endpoint: EndpointKind,
    client_token_ref: Option<&'a str>,
    route_kind: &'a str,
) -> RouteAdmissionDenial<'a> {
    RouteAdmissionDenial {
        endpoint,
        client_token_ref,
        route_kind,
        reason_code: "no_route_candidate",
        blocking_domain: "route",
        client_visible_status: StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn record_route_admission_denied(
    state: &AppState,
    preview: &RoutePreview,
    denial: RouteAdmissionDenial<'_>,
) {
    let (hard_reason_codes, soft_reason_codes) = admission_reason_codes(preview);
    record_routing_telemetry(
        state,
        RoutingTelemetry::RouteAdmissionDenied {
            request_id: preview.request_id.clone(),
            registry_generation: preview.registry_generation,
            endpoint_family: denial.endpoint.family_code().to_string(),
            public_model: preview.public_model.clone(),
            client_token_ref: denial.client_token_ref.map(ToOwned::to_owned),
            route_kind: denial.route_kind.to_string(),
            reason_code: denial.reason_code.to_string(),
            blocking_domain: denial.blocking_domain.to_string(),
            client_visible_status: denial.client_visible_status.as_u16(),
            upstream_status: None,
            candidate_count: preview.candidates.len(),
            included_count: preview
                .candidates
                .iter()
                .filter(|candidate| candidate.included)
                .count(),
            blocked_count: preview
                .candidates
                .iter()
                .filter(|candidate| !candidate.included)
                .count(),
            hard_blocked_count: hard_blocked_candidate_count(preview),
            soft_suppressed_count: soft_suppressed_candidate_count(preview),
            last_resort_used: preview.candidates.iter().any(|candidate| {
                candidate.included
                    && candidate.reasons.iter().any(|reason| {
                        matches!(
                            reason,
                            RoutePreviewReason::DegradedLastResort
                                | RoutePreviewReason::ProviderCoolingDownLastResort
                                | RoutePreviewReason::CredentialCoolingDownLastResort
                        )
                    })
            }),
            hard_reason_codes,
            soft_reason_codes,
        },
    );
}

fn record_single_route_admission_denied(
    state: &AppState,
    request_id: &str,
    registry_generation: u64,
    denial: RouteAdmissionDenial<'_>,
    public_model: Option<&str>,
    hard_reason_codes: &[&str],
) {
    record_routing_telemetry(
        state,
        RoutingTelemetry::RouteAdmissionDenied {
            request_id: request_id.to_string(),
            registry_generation,
            endpoint_family: denial.endpoint.family_code().to_string(),
            public_model: public_model.map(ToOwned::to_owned),
            client_token_ref: denial.client_token_ref.map(ToOwned::to_owned),
            route_kind: denial.route_kind.to_string(),
            reason_code: denial.reason_code.to_string(),
            blocking_domain: denial.blocking_domain.to_string(),
            client_visible_status: denial.client_visible_status.as_u16(),
            upstream_status: None,
            candidate_count: 1,
            included_count: 0,
            blocked_count: 1,
            hard_blocked_count: 1,
            soft_suppressed_count: 0,
            last_resort_used: false,
            hard_reason_codes: hard_reason_codes
                .iter()
                .map(|code| (*code).to_string())
                .collect(),
            soft_reason_codes: Vec::new(),
        },
    );
}

fn admission_reason_codes(preview: &RoutePreview) -> (Vec<String>, Vec<String>) {
    let mut hard = BTreeSet::new();
    let mut soft = BTreeSet::new();
    for candidate in &preview.candidates {
        for reason in &candidate.reasons {
            if route_preview_reason_is_soft_suppression(*reason) {
                soft.insert(reason.as_str().to_string());
            } else if route_preview_reason_is_hard_blocker(*reason) {
                hard.insert(reason.as_str().to_string());
            }
        }
    }
    (
        hard.into_iter().take(8).collect(),
        soft.into_iter().take(8).collect(),
    )
}

fn hard_blocked_candidate_count(preview: &RoutePreview) -> usize {
    preview
        .candidates
        .iter()
        .filter(|candidate| {
            !candidate.included
                && candidate
                    .reasons
                    .iter()
                    .any(|reason| route_preview_reason_is_hard_blocker(*reason))
        })
        .count()
}

fn soft_suppressed_candidate_count(preview: &RoutePreview) -> usize {
    preview
        .candidates
        .iter()
        .filter(|candidate| {
            !candidate.included
                && !candidate.reasons.is_empty()
                && candidate
                    .reasons
                    .iter()
                    .copied()
                    .all(route_preview_reason_is_soft_suppression)
        })
        .count()
}

fn route_preview_reason_excludes_candidate_from_cooling_denominator(
    reason: &RoutePreviewReason,
) -> bool {
    matches!(
        reason,
        RoutePreviewReason::TargetDisabled
            | RoutePreviewReason::ClientChannelScope
            | RoutePreviewReason::ChannelDisabled
            | RoutePreviewReason::UnknownChannel
            | RoutePreviewReason::CandidateLimit
    )
}

fn all_relevant_route_candidates_are_cooling_down(preview: &RoutePreview) -> bool {
    let mut relevant_candidates = preview.candidates.iter().filter(|candidate| {
        !candidate
            .reasons
            .iter()
            .any(route_preview_reason_excludes_candidate_from_cooling_denominator)
    });
    let Some(first) = relevant_candidates.next() else {
        return false;
    };
    first
        .reasons
        .contains(&RoutePreviewReason::ChannelCoolingDown)
        && relevant_candidates.all(|candidate| {
            candidate
                .reasons
                .contains(&RoutePreviewReason::ChannelCoolingDown)
        })
}

fn relevant_route_candidate_reason_codes(preview: &RoutePreview) -> Vec<&'static str> {
    preview
        .candidates
        .iter()
        .filter(|candidate| {
            !candidate.reasons.iter().any(|reason| {
                route_preview_reason_excludes_candidate_from_cooling_denominator(reason)
            })
        })
        .flat_map(|candidate| candidate.reasons.iter().map(|reason| reason.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub async fn proxy_named_pool(
    State(state): State<AppState>,
    Path((pool_name, tail)): Path<(String, String)>,
    req: Request,
) -> Response {
    let client = match authorize_client(&state, req.headers()) {
        Ok(client) => client,
        Err(resp) => return *resp,
    };
    let _client_context = (&client.id, &client.name);
    if !client.allowed_channels.is_empty()
        && !client
            .allowed_channels
            .iter()
            .any(|allowed| allowed == &pool_name)
    {
        return json_error(
            StatusCode::FORBIDDEN,
            "channel not allowed for client token",
        );
    }

    let (parts, raw_body) = req.into_parts();
    let query = parts.uri.query().map(ToOwned::to_owned);
    let path = format!("/{tail}");
    let request_id = next_request_id();
    let route_context = state.channels.named_route_plan_context(&pool_name);
    let route_plan = FrozenRoutePlan::single_named(
        request_id.clone(),
        route_context.registry_generation,
        pool_name,
        route_context.route_state,
    );
    let Some(pool_state) = state.channels.get(&route_plan.targets[0].channel_id) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            format!("unknown pool {}", route_plan.targets[0].channel_id),
        );
    };
    let adapter = ProviderAdapter::new(pool_state.provider_kind);
    if matches!(route_context.route_state, ChannelRouteState::CoolingDown) {
        let preliminary_context = adapter.request_context(&parts.method, &path, &[]);
        record_single_route_admission_denied(
            &state,
            &request_id,
            route_context.registry_generation,
            no_route_candidate_denial(
                preliminary_context.endpoint,
                Some(&client.id),
                "named_channel",
            ),
            preliminary_context.requested_model.as_deref(),
            &["channel_cooling_down"],
        );
        return no_route_candidate_response(&["channel_cooling_down"]);
    }
    if let Some(response) = route_state_unavailable_response(
        &route_plan.targets[0].channel_id,
        route_context.route_state,
    ) {
        return response;
    }
    if adapter.named_pool_request_mode() == NamedPoolRequestMode::ReplayableWithModelContext {
        let body = match axum::body::to_bytes(raw_body, state.max_request_body_bytes).await {
            Ok(body) => body,
            Err(err) => {
                return json_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("request body exceeded limit or is invalid: {err}"),
                )
            }
        };
        let request_context =
            adapter.request_context(&parts.method, &path, bytes_from_body_for_context(&body));
        if let Some(model) = request_context.requested_model.as_deref() {
            let named_channel_scope = [route_plan.targets[0].channel_id.clone()];
            if requested_model_is_claimed_upstream_model(&state, model, &named_channel_scope) {
                return json_error(
                    StatusCode::FORBIDDEN,
                    "model must be requested through its public route",
                );
            }
            if !client.allowed_model_groups.is_empty()
                && !state
                    .runtime_catalogs
                    .client_model_allowed(&client.allowed_model_groups, model)
            {
                return json_error(StatusCode::FORBIDDEN, "model not allowed for client token");
            }
        }
        let effective_deadline = effective_deadline_for_request(
            request_context.streaming,
            state.timeout_profile.non_streaming_total,
        );
        return forward_with_pools(ForwardRequest {
            state,
            route_plan,
            method: parts.method,
            headers: parts.headers,
            path,
            query,
            body,
            client_token_id: client.id,
            request_context,
            selection_reason: SelectionReason::NamedChannel,
            effective_deadline,
        })
        .await;
    }
    let request_context = adapter.request_context(&parts.method, &path, &[]);

    forward_streaming_named_pool(StreamingForwardRequest {
        state,
        route_plan,
        method: parts.method,
        headers: parts.headers,
        path,
        query,
        body: raw_body,
        client_token_id: client.id,
        request_context,
    })
    .await
}

fn route_state_unavailable_response(
    channel_id: &str,
    state: ChannelRouteState,
) -> Option<Response> {
    if matches!(state, ChannelRouteState::CoolingDown) {
        return Some(no_route_candidate_response(&["channel_cooling_down"]));
    }
    let (status, code, message) = match state {
        ChannelRouteState::Available
        | ChannelRouteState::Degraded
        | ChannelRouteState::ProviderCoolingDown
        | ChannelRouteState::CredentialCoolingDown => return None,
        ChannelRouteState::CoolingDown => unreachable!("cooling down handled above"),
        ChannelRouteState::Disabled => (
            StatusCode::SERVICE_UNAVAILABLE,
            "channel_disabled",
            format!("channel {channel_id} is disabled"),
        ),
        ChannelRouteState::NoAvailableCredentials => (
            StatusCode::SERVICE_UNAVAILABLE,
            "credential_pool_exhausted",
            format!("credential pool for channel {channel_id} has no available credentials"),
        ),
        ChannelRouteState::RuntimeUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "channel_runtime_unavailable",
            format!("channel {channel_id} runtime state is temporarily unavailable"),
        ),
        ChannelRouteState::UnknownChannel => (
            StatusCode::BAD_REQUEST,
            "unknown_channel",
            format!("unknown channel {channel_id}"),
        ),
    };
    Some(json_error_with_code(status, code, message))
}

fn route_state_unavailable_response_for_attempt(
    channel_id: &str,
    state: ChannelRouteState,
    frozen_state: ChannelRouteState,
    route_target_available: bool,
) -> Option<Response> {
    if matches!(state, ChannelRouteState::ProviderCoolingDown)
        && !matches!(frozen_state, ChannelRouteState::ProviderCoolingDown)
        && route_target_available
    {
        return Some(no_route_candidate_response(&["provider_cooling_down"]));
    }
    if matches!(state, ChannelRouteState::CredentialCoolingDown)
        && !matches!(frozen_state, ChannelRouteState::CredentialCoolingDown)
        && route_target_available
    {
        return Some(no_route_candidate_response(&["credential_cooling_down"]));
    }
    route_state_unavailable_response(channel_id, state)
}

fn credential_pool_exhausted_response(message: impl Into<String>) -> Response {
    json_error_with_code(
        StatusCode::SERVICE_UNAVAILABLE,
        "credential_pool_exhausted",
        message.into(),
    )
}

fn no_route_candidate_response(reason_classes: &[&str]) -> Response {
    let body = serde_json::json!({
        "error": {
            "message": "no route candidate",
            "type": "router_error",
            "code": "no_route_candidate",
            "reasons": reason_classes,
        }
    });
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("valid json error response")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ForwardTarget {
    route_target_index: Option<usize>,
    channel_id: String,
    upstream_model: Option<String>,
    route_state: ChannelRouteState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrozenRoutePlan {
    request_id: String,
    registry_generation: u64,
    public_model: Option<String>,
    selection_reason: SelectionReason,
    targets: Vec<ForwardTarget>,
}

impl FrozenRoutePlan {
    fn single_default(
        request_id: String,
        registry_generation: u64,
        channel_id: String,
        route_state: ChannelRouteState,
    ) -> Self {
        Self {
            request_id,
            registry_generation,
            public_model: None,
            selection_reason: SelectionReason::DefaultPool,
            targets: vec![ForwardTarget {
                route_target_index: None,
                channel_id,
                upstream_model: None,
                route_state,
            }],
        }
    }

    fn single_named(
        request_id: String,
        registry_generation: u64,
        channel_id: String,
        route_state: ChannelRouteState,
    ) -> Self {
        Self {
            request_id,
            registry_generation,
            public_model: None,
            selection_reason: SelectionReason::NamedChannel,
            targets: vec![ForwardTarget {
                route_target_index: None,
                channel_id,
                upstream_model: None,
                route_state,
            }],
        }
    }
}

impl FrozenRoutePlan {
    fn from_plan(
        plan: RoutePlan,
        channel_states: &std::collections::HashMap<ChannelId, ChannelRouteState>,
    ) -> Self {
        Self {
            request_id: plan.request_id,
            registry_generation: plan.registry_generation,
            public_model: plan.public_model,
            selection_reason: SelectionReason::ModelMapping,
            targets: plan
                .targets
                .into_iter()
                .map(|target| {
                    let route_state = channel_states
                        .get(&target.channel_id)
                        .copied()
                        .unwrap_or(ChannelRouteState::UnknownChannel);
                    ForwardTarget::from_candidate(target, route_state)
                })
                .collect(),
        }
    }
}

impl ForwardTarget {
    fn from_candidate(candidate: RouteCandidate, route_state: ChannelRouteState) -> Self {
        Self {
            route_target_index: Some(candidate.target_index),
            channel_id: candidate.channel_id.0,
            upstream_model: candidate.upstream_model,
            route_state,
        }
    }
}

struct ForwardRequest {
    state: AppState,
    route_plan: FrozenRoutePlan,
    method: Method,
    headers: HeaderMap,
    path: String,
    query: Option<String>,
    body: Bytes,
    client_token_id: String,
    request_context: crate::provider::RequestContext,
    selection_reason: SelectionReason,
    effective_deadline: Option<Instant>,
}

struct StreamingForwardRequest {
    state: AppState,
    route_plan: FrozenRoutePlan,
    method: Method,
    headers: HeaderMap,
    path: String,
    query: Option<String>,
    body: Body,
    client_token_id: String,
    request_context: crate::provider::RequestContext,
}

fn effective_deadline_for_request(
    streaming: bool,
    non_streaming_total: Duration,
) -> Option<Instant> {
    (!streaming).then(|| Instant::now() + non_streaming_total)
}

fn upstream_timeout_for_effective_deadline(
    effective_deadline: Option<Instant>,
    non_streaming_total: Duration,
    retry_attempt: bool,
) -> Duration {
    let base_timeout = if retry_attempt {
        non_streaming_total.min(CONSERVATIVE_RETRY_BUDGET)
    } else {
        non_streaming_total
    };
    effective_deadline
        .map(|deadline| {
            deadline
                .saturating_duration_since(Instant::now())
                .min(base_timeout)
        })
        .unwrap_or(base_timeout)
}

async fn forward_streaming_named_pool(req: StreamingForwardRequest) -> Response {
    let StreamingForwardRequest {
        state,
        route_plan,
        method,
        headers,
        path,
        query,
        body,
        client_token_id,
        request_context,
    } = req;
    let Some(target) = route_plan.targets.first() else {
        return json_error(StatusCode::BAD_REQUEST, "no matching pool".to_string());
    };
    let pool_name = target.channel_id.clone();
    let pool_state = match state.channels.get(&pool_name) {
        Some(pool) => pool.clone(),
        None => return json_error(StatusCode::BAD_REQUEST, format!("unknown pool {pool_name}")),
    };

    if let Some(content_length) = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    {
        if content_length > state.max_request_body_bytes {
            return json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "request body exceeded limit of {} bytes",
                    state.max_request_body_bytes
                ),
            );
        }
    }

    let send_guard = pool_state.send_gate.read().await;
    let (selected, auth_header, auth_prefix, snapshot) = {
        let _mutation_guard = pool_state.mutation_gate.lock().await;
        let route_state = target
            .route_state
            .most_restrictive(pool_state.route_state());
        if let Some(response) = route_state_unavailable_response(&pool_name, route_state) {
            return response;
        }
        let mut pool = pool_state.pool.lock().await;
        let selected = if matches!(route_state, ChannelRouteState::CredentialCoolingDown) {
            pool.select_cooling_down_last_resort()
        } else {
            pool.select()
        };
        let selected = match selected {
            Ok(selected) => selected,
            Err(err) => {
                return credential_pool_exhausted_response(err.to_string());
            }
        };
        let snapshot = RequestSelectionSnapshot {
            request_id: route_plan.request_id.clone(),
            config_generation: pool_state.config_generation,
            channel_health_generation: pool_state.channel_health_generation.load(Ordering::Acquire),
            client_token_id,
            requested_model: request_context.requested_model.clone(),
            endpoint: request_context.endpoint,
            route_target_index: target.route_target_index,
            channel_id: ChannelId(pool_name.clone()),
            provider_id: pool_state.provider_id.clone(),
            account_id: pool_state.account_id.clone(),
            provider_kind: pool_state.provider_kind,
            credential_id: selected.credential_id.clone(),
            credential_fingerprint: selected.credential_fingerprint.clone(),
            classifier_id: pool_state.error_classifier.classifier_id().to_string(),
            classifier_version: pool_state.error_classifier.classifier_version().to_string(),
            retry_candidates: Vec::new(),
            body_replayable: false,
            streaming: false,
            partial_output_started: false,
            route_target_available: false,
            effective_deadline: None,
            attempt: 0,
            selection_reason: SelectionReason::NamedChannel,
        };
        (
            selected,
            pool_state.auth_header.clone(),
            pool_state.auth_prefix.clone(),
            snapshot,
        )
    };
    record_routing_telemetry(
        &state,
        RoutingTelemetry::RouteSelected {
            request_id: snapshot.request_id.clone(),
            registry_generation: route_plan.registry_generation,
            channel_id: pool_name.clone(),
        },
    );

    let adapter = ProviderAdapter::new(pool_state.provider_kind);
    let query = query.as_deref().unwrap_or_default();
    let url = adapter.upstream_url(&selected.api_base, &path, Some(query));
    let mut upstream = state.http_client.request(method, url);
    upstream = adapter.apply_upstream_auth_and_headers(
        upstream,
        &headers,
        &auth_header,
        &auth_prefix,
        &selected.key,
    );
    upstream = upstream.timeout(state.timeout_profile.non_streaming_total);
    upstream = upstream.body(ReqwestBody::wrap_stream(bounded_request_body_stream(
        body,
        state.max_request_body_bytes,
    )));

    let upstream_result = upstream.send().await;
    drop(send_guard);
    let upstream_resp = match upstream_result {
        Ok(resp) => resp,
        Err(err) => {
            let failure = pool_state.error_classifier.classify_transport_failure();
            let _ = transition_observed_failure(
                &state,
                &pool_state,
                &snapshot,
                failure,
                FailureSource::LocalTransport,
            )
            .await;
            return json_error(StatusCode::BAD_GATEWAY, format!("upstream error: {err}"));
        }
    };

    let status = upstream_resp.status();
    let response_headers = upstream_resp.headers().clone();
    if status.is_success() {
        if !should_guard_success_status(status.as_u16()) {
            pool_state
                .failure_domains
                .record_success(&pool_state.provider_id, &pool_state.account_id);
            pool_state.record_selected_channel_success();
            return stream_response_with_filter_events(
                status,
                response_headers,
                upstream_resp,
                state.response_filter.clone(),
                Some(body_stream_failure_observer(
                    state.clone(),
                    pool_state.clone(),
                    snapshot.clone(),
                )),
                Some(response_filter_event_options(
                    &state,
                    &route_plan,
                    &snapshot,
                )),
            );
        }
        match guard_success_response(
            upstream_resp,
            request_context.endpoint,
            &pool_state.error_classifier,
            SUCCESS_GUARD_MAX_BYTES,
            SUCCESS_GUARD_MAX_DURATION,
        )
        .await
        {
            Ok(GuardResult::Classified { failure, .. }) => {
                let _ = transition_observed_failure(
                    &state,
                    &pool_state,
                    &snapshot,
                    failure,
                    FailureSource::GuardedSuccessEnvelope,
                )
                .await;
                return guarded_success_envelope_response();
            }
            Ok(GuardResult::PassThrough {
                outcome,
                prefix,
                stream,
            }) => {
                if let Some(rejection) = inspect_response_filter_before_commit(
                    &state,
                    &route_plan,
                    &snapshot,
                    &response_headers,
                    outcome,
                    &prefix,
                ) {
                    if let Some(failure) = rejection.failure {
                        let _ = transition_observed_failure(
                            &state,
                            &pool_state,
                            &snapshot,
                            failure,
                            FailureSource::ResponseFilterPrecommit,
                        )
                        .await;
                    }
                    return rejection.response;
                }
                pool_state
                    .failure_domains
                    .record_success(&pool_state.provider_id, &pool_state.account_id);
                pool_state.record_selected_channel_success();
                let stream = Box::pin(prefixed_body_stream(prefix, stream));
                return stream_response_from_byte_stream_with_filter_events(
                    status,
                    response_headers,
                    stream,
                    state.response_filter.clone(),
                    Some(body_stream_failure_observer(
                        state.clone(),
                        pool_state.clone(),
                        snapshot.clone(),
                    )),
                    false,
                    Some(response_filter_event_options(
                        &state,
                        &route_plan,
                        &snapshot,
                    )),
                );
            }
            Err(err) => {
                let failure = pool_state.error_classifier.classify_transport_failure();
                let _ = transition_observed_failure(
                    &state,
                    &pool_state,
                    &snapshot,
                    failure,
                    FailureSource::LocalTransport,
                )
                .await;
                return json_error(
                    StatusCode::BAD_GATEWAY,
                    format!("upstream body error: {err}"),
                );
            }
        }
    }

    let bytes = match read_limited_body(upstream_resp, state.max_error_body_bytes).await {
        Ok(bytes) => bytes,
        Err(err) => {
            let failure = adapter.classify_failure(
                &pool_state.error_classifier,
                status.as_u16(),
                &response_headers,
                b"",
            );
            let _ =
                transition_observed_upstream_failure(&state, &pool_state, &snapshot, failure).await;
            return json_error(
                StatusCode::BAD_GATEWAY,
                format!("upstream body error: {err}"),
            );
        }
    };

    let failure = adapter.classify_failure(
        &pool_state.error_classifier,
        status.as_u16(),
        &response_headers,
        &bytes,
    );
    let _ = transition_observed_upstream_failure(&state, &pool_state, &snapshot, failure).await;
    if let Some(decision) = inspect_response_filter_buffered_body_before_commit(
        &state,
        &route_plan,
        &snapshot,
        &response_headers,
        status,
        &bytes,
    ) {
        match decision {
            BufferedBodyFilterDecision::Redacted(bytes) => {
                return response_with_filtered_buffered_body(status, response_headers, bytes);
            }
            BufferedBodyFilterDecision::Rejected(rejection) => {
                if let Some(failure) = rejection.failure {
                    let _ = transition_observed_failure(
                        &state,
                        &pool_state,
                        &snapshot,
                        failure,
                        FailureSource::ResponseFilterPrecommit,
                    )
                    .await;
                }
                return rejection.response;
            }
        }
    }
    response_with_headers(status, response_headers, Body::from(bytes))
}

fn bounded_request_body_stream(
    body: Body,
    limit: usize,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let mut sent = 0usize;
    body.into_data_stream().map(move |chunk| {
        let chunk = chunk.map_err(std::io::Error::other)?;
        sent = sent.saturating_add(chunk.len());
        if sent > limit {
            Err(std::io::Error::other(format!(
                "request body exceeded limit of {limit} bytes"
            )))
        } else {
            Ok(chunk)
        }
    })
}

async fn forward_with_pools(req: ForwardRequest) -> Response {
    let mut last_response = None;
    let total_pools = req.route_plan.targets.len();
    for (route_attempt, target) in req.route_plan.targets.iter().enumerate() {
        let route_target_available = route_attempt + 1 < total_pools;
        let initial_attempt = usize::from(route_attempt > 0);
        match forward_with_pool(
            &req,
            target.clone(),
            route_target_available,
            initial_attempt,
        )
        .await
        {
            PoolForwardResult::Response(response) => return response,
            PoolForwardResult::RouteFallback(response) if route_target_available => {
                last_response = Some(response);
            }
            PoolForwardResult::RouteFallback(response) => return response,
        }
    }
    last_response
        .unwrap_or_else(|| json_error(StatusCode::BAD_REQUEST, "no matching pool".to_string()))
}

enum PoolForwardResult {
    Response(Response),
    RouteFallback(Response),
}

async fn forward_with_pool(
    req: &ForwardRequest,
    target: ForwardTarget,
    route_target_available: bool,
    initial_attempt: usize,
) -> PoolForwardResult {
    let ForwardRequest {
        state,
        method,
        headers,
        path,
        query,
        body,
        client_token_id,
        request_context,
        selection_reason,
        effective_deadline,
        route_plan,
        ..
    } = req;
    let request_id = &route_plan.request_id;
    let pool_name = target.channel_id.clone();
    let pool_state = match state.channels.get(&pool_name) {
        Some(pool) => pool.clone(),
        None => {
            return PoolForwardResult::Response(json_error(
                StatusCode::BAD_REQUEST,
                format!("unknown pool {pool_name}"),
            ))
        }
    };

    let query = query.as_deref().unwrap_or_default();
    let mut attempt = initial_attempt;
    let mut retry_credential_id: Option<CredentialId> = None;
    let mut frozen_retry_candidates: Option<FrozenRetryCandidates> = None;

    loop {
        let send_guard = pool_state.send_gate.read().await;
        let (selected, auth_header, auth_prefix, snapshot) = {
            let _mutation_guard = pool_state.mutation_gate.lock().await;
            let route_state = target
                .route_state
                .most_restrictive(pool_state.route_state());
            if let Some(response) = route_state_unavailable_response_for_attempt(
                &pool_name,
                route_state,
                target.route_state,
                route_target_available,
            ) {
                return PoolForwardResult::RouteFallback(response);
            }
            let mut pool = pool_state.pool.lock().await;
            let select_cooling_last_resort =
                matches!(route_state, ChannelRouteState::CredentialCoolingDown);
            let selected = match retry_credential_id.take() {
                Some(credential_id) => pool.select_credential_by_id(&credential_id),
                None if select_cooling_last_resort => pool.select_cooling_down_last_resort(),
                None => pool.select(),
            };
            let selected = match selected {
                Ok(selected) => selected,
                Err(err) => {
                    return PoolForwardResult::RouteFallback(credential_pool_exhausted_response(
                        err.to_string(),
                    ));
                }
            };
            if frozen_retry_candidates.is_none() {
                let candidates = if pool_state.max_same_request_retries > 0 {
                    pool.retry_candidates_from_current(pool_state.max_same_request_retries)
                        .into_iter()
                        .map(|candidate| candidate.credential_id)
                        .collect()
                } else {
                    Vec::new()
                };
                frozen_retry_candidates = Some(FrozenRetryCandidates::new(candidates));
            }
            let retry_candidates = frozen_retry_candidates
                .as_ref()
                .map(FrozenRetryCandidates::remaining)
                .unwrap_or_default();
            let snapshot = RequestSelectionSnapshot {
                request_id: request_id.clone(),
                config_generation: pool_state.config_generation,
                channel_health_generation: pool_state
                    .channel_health_generation
                    .load(Ordering::Acquire),
                client_token_id: client_token_id.clone(),
                requested_model: request_context.requested_model.clone(),
                endpoint: request_context.endpoint,
                route_target_index: target.route_target_index,
                channel_id: ChannelId(pool_name.clone()),
                provider_id: pool_state.provider_id.clone(),
                account_id: pool_state.account_id.clone(),
                provider_kind: pool_state.provider_kind,
                credential_id: selected.credential_id.clone(),
                credential_fingerprint: selected.credential_fingerprint.clone(),
                classifier_id: pool_state.error_classifier.classifier_id().to_string(),
                classifier_version: pool_state.error_classifier.classifier_version().to_string(),
                retry_candidates: retry_candidates.clone(),
                body_replayable: request_context.body_replayable,
                streaming: request_context.streaming,
                partial_output_started: false,
                route_target_available,
                effective_deadline: *effective_deadline,
                attempt,
                selection_reason: *selection_reason,
            };
            (
                selected,
                pool_state.auth_header.clone(),
                pool_state.auth_prefix.clone(),
                snapshot,
            )
        };
        let _credential_context = (&selected.credential_id, &selected.credential_fingerprint);
        record_routing_telemetry(
            state,
            RoutingTelemetry::RouteSelected {
                request_id: snapshot.request_id.clone(),
                registry_generation: route_plan.registry_generation,
                channel_id: pool_name.clone(),
            },
        );
        let adapter = ProviderAdapter::new(pool_state.provider_kind);
        let outbound = adapter.transform_request_for_target(
            method,
            path,
            body,
            target.upstream_model.as_deref(),
        );
        let url = adapter.upstream_url(&selected.api_base, &outbound.path, Some(query));
        let mut upstream = state.http_client.request(method.clone(), url);
        upstream = adapter.apply_upstream_auth_and_headers(
            upstream,
            headers,
            &auth_header,
            &auth_prefix,
            &selected.key,
        );
        if !request_context.streaming {
            upstream = upstream.timeout(upstream_timeout_for_effective_deadline(
                *effective_deadline,
                state.timeout_profile.non_streaming_total,
                attempt > 0,
            ));
        }
        if !outbound.body.is_empty() {
            upstream = upstream.body(outbound.body);
        }

        let upstream_result = upstream.send().await;
        drop(send_guard);
        let upstream_resp = match upstream_result {
            Ok(resp) => resp,
            Err(err) => {
                let failure = pool_state.error_classifier.classify_transport_failure();
                let directive = transition_observed_failure(
                    state,
                    &pool_state,
                    &snapshot,
                    failure,
                    FailureSource::LocalTransport,
                )
                .await;
                let response =
                    json_error(StatusCode::BAD_GATEWAY, format!("upstream error: {err}"));
                match apply_retry_directive_to_attempt_state(
                    directive,
                    &mut frozen_retry_candidates,
                    &mut attempt,
                ) {
                    RetryAttemptContinuation::RetrySameTarget => {
                        continue;
                    }
                    RetryAttemptContinuation::RetryRouteTarget => {
                        return PoolForwardResult::RouteFallback(response);
                    }
                    RetryAttemptContinuation::RetryCredential { credential_id } => {
                        retry_credential_id = Some(credential_id);
                        continue;
                    }
                    RetryAttemptContinuation::ReturnCurrentError { .. } => {}
                    RetryAttemptContinuation::FrozenCandidateDrift { .. } => {
                        return PoolForwardResult::Response(json_error(
                            StatusCode::BAD_GATEWAY,
                            "retry candidate drifted from frozen request selection",
                        ));
                    }
                }
                return PoolForwardResult::Response(response);
            }
        };

        let status = upstream_resp.status();
        let response_headers = upstream_resp.headers().clone();
        if status.is_success() {
            if !should_guard_success_status(status.as_u16()) {
                pool_state
                    .failure_domains
                    .record_success(&pool_state.provider_id, &pool_state.account_id);
                pool_state.record_selected_channel_success();
                return PoolForwardResult::Response(stream_response_with_filter_events(
                    status,
                    response_headers,
                    upstream_resp,
                    state.response_filter.clone(),
                    Some(body_stream_failure_observer(
                        state.clone(),
                        pool_state.clone(),
                        snapshot.clone(),
                    )),
                    Some(response_filter_event_options(state, route_plan, &snapshot)),
                ));
            }
            match guard_success_response(
                upstream_resp,
                request_context.endpoint,
                &pool_state.error_classifier,
                SUCCESS_GUARD_MAX_BYTES,
                SUCCESS_GUARD_MAX_DURATION,
            )
            .await
            {
                Ok(GuardResult::Classified { failure, .. }) => {
                    let directive = transition_observed_failure(
                        state,
                        &pool_state,
                        &snapshot,
                        failure,
                        FailureSource::GuardedSuccessEnvelope,
                    )
                    .await;
                    let response = guarded_success_envelope_response();
                    match apply_retry_directive_to_attempt_state(
                        directive,
                        &mut frozen_retry_candidates,
                        &mut attempt,
                    ) {
                        RetryAttemptContinuation::RetryCredential { credential_id } => {
                            retry_credential_id = Some(credential_id);
                            continue;
                        }
                        RetryAttemptContinuation::RetryRouteTarget => {
                            return PoolForwardResult::RouteFallback(response);
                        }
                        RetryAttemptContinuation::RetrySameTarget => {
                            continue;
                        }
                        RetryAttemptContinuation::ReturnCurrentError { .. } => {}
                        RetryAttemptContinuation::FrozenCandidateDrift { .. } => {
                            return PoolForwardResult::Response(json_error(
                                StatusCode::BAD_GATEWAY,
                                "retry candidate drifted from frozen request selection",
                            ));
                        }
                    }
                    return PoolForwardResult::Response(response);
                }
                Ok(GuardResult::PassThrough {
                    outcome,
                    prefix,
                    stream,
                }) => {
                    if let Some(rejection) = inspect_response_filter_before_commit(
                        state,
                        route_plan,
                        &snapshot,
                        &response_headers,
                        outcome,
                        &prefix,
                    ) {
                        if let Some(failure) = rejection.failure {
                            let _ = transition_observed_failure(
                                state,
                                &pool_state,
                                &snapshot,
                                failure,
                                FailureSource::ResponseFilterPrecommit,
                            )
                            .await;
                        }
                        return PoolForwardResult::Response(rejection.response);
                    }
                    pool_state
                        .failure_domains
                        .record_success(&pool_state.provider_id, &pool_state.account_id);
                    pool_state.record_selected_channel_success();
                    let stream = Box::pin(prefixed_body_stream(prefix, stream));
                    return PoolForwardResult::Response(
                        stream_response_from_byte_stream_with_filter_events(
                            status,
                            response_headers,
                            stream,
                            state.response_filter.clone(),
                            Some(body_stream_failure_observer(
                                state.clone(),
                                pool_state.clone(),
                                snapshot.clone(),
                            )),
                            false,
                            Some(response_filter_event_options(state, route_plan, &snapshot)),
                        ),
                    );
                }
                Err(err) => {
                    let failure = pool_state.error_classifier.classify_transport_failure();
                    let directive = transition_observed_failure(
                        state,
                        &pool_state,
                        &snapshot,
                        failure,
                        FailureSource::LocalTransport,
                    )
                    .await;
                    match apply_retry_directive_to_attempt_state(
                        directive,
                        &mut frozen_retry_candidates,
                        &mut attempt,
                    ) {
                        RetryAttemptContinuation::RetryCredential { credential_id } => {
                            retry_credential_id = Some(credential_id);
                            continue;
                        }
                        RetryAttemptContinuation::RetryRouteTarget => {
                            let response = json_error(
                                StatusCode::BAD_GATEWAY,
                                format!("upstream body error: {err}"),
                            );
                            return PoolForwardResult::RouteFallback(response);
                        }
                        RetryAttemptContinuation::RetrySameTarget => {
                            continue;
                        }
                        RetryAttemptContinuation::ReturnCurrentError { .. } => {}
                        RetryAttemptContinuation::FrozenCandidateDrift { .. } => {
                            return PoolForwardResult::Response(json_error(
                                StatusCode::BAD_GATEWAY,
                                "retry candidate drifted from frozen request selection",
                            ));
                        }
                    }
                    let response = json_error(
                        StatusCode::BAD_GATEWAY,
                        format!("upstream body error: {err}"),
                    );
                    return PoolForwardResult::Response(response);
                }
            }
        }

        let bytes = match read_limited_body(upstream_resp, state.max_error_body_bytes).await {
            Ok(bytes) => bytes,
            Err(err) => {
                let failure = adapter.classify_failure(
                    &pool_state.error_classifier,
                    status.as_u16(),
                    &response_headers,
                    b"",
                );
                let directive =
                    transition_observed_upstream_failure(state, &pool_state, &snapshot, failure)
                        .await;
                match apply_retry_directive_to_attempt_state(
                    directive,
                    &mut frozen_retry_candidates,
                    &mut attempt,
                ) {
                    RetryAttemptContinuation::RetryCredential { credential_id } => {
                        retry_credential_id = Some(credential_id);
                        continue;
                    }
                    RetryAttemptContinuation::RetryRouteTarget => {
                        let response = json_error(
                            StatusCode::BAD_GATEWAY,
                            format!("upstream body error: {err}"),
                        );
                        return PoolForwardResult::RouteFallback(response);
                    }
                    RetryAttemptContinuation::RetrySameTarget => {
                        continue;
                    }
                    RetryAttemptContinuation::ReturnCurrentError { .. } => {}
                    RetryAttemptContinuation::FrozenCandidateDrift { .. } => {
                        return PoolForwardResult::Response(json_error(
                            StatusCode::BAD_GATEWAY,
                            "retry candidate drifted from frozen request selection",
                        ));
                    }
                }
                let response = json_error(
                    StatusCode::BAD_GATEWAY,
                    format!("upstream body error: {err}"),
                );
                return PoolForwardResult::Response(response);
            }
        };

        let failure = adapter.classify_failure(
            &pool_state.error_classifier,
            status.as_u16(),
            &response_headers,
            &bytes,
        );
        let directive =
            transition_observed_upstream_failure(state, &pool_state, &snapshot, failure).await;
        match apply_retry_directive_to_attempt_state(
            directive,
            &mut frozen_retry_candidates,
            &mut attempt,
        ) {
            RetryAttemptContinuation::RetryCredential { credential_id } => {
                retry_credential_id = Some(credential_id);
                continue;
            }
            RetryAttemptContinuation::RetryRouteTarget => {
                let response = response_with_headers(status, response_headers, Body::from(bytes));
                return PoolForwardResult::RouteFallback(response);
            }
            RetryAttemptContinuation::RetrySameTarget => {
                continue;
            }
            RetryAttemptContinuation::ReturnCurrentError { .. }
            | RetryAttemptContinuation::FrozenCandidateDrift { .. } => {}
        }

        if let Some(decision) = inspect_response_filter_buffered_body_before_commit(
            state,
            route_plan,
            &snapshot,
            &response_headers,
            status,
            &bytes,
        ) {
            match decision {
                BufferedBodyFilterDecision::Redacted(bytes) => {
                    let response =
                        response_with_filtered_buffered_body(status, response_headers, bytes);
                    return PoolForwardResult::Response(response);
                }
                BufferedBodyFilterDecision::Rejected(rejection) => {
                    if let Some(failure) = rejection.failure {
                        let _ = transition_observed_failure(
                            state,
                            &pool_state,
                            &snapshot,
                            failure,
                            FailureSource::ResponseFilterPrecommit,
                        )
                        .await;
                    }
                    return PoolForwardResult::Response(rejection.response);
                }
            }
        }

        let response = response_with_headers(status, response_headers, Body::from(bytes));
        return PoolForwardResult::Response(response);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ResponseFilterEventBuffer, ResponseFilterEventInput};
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    };

    #[test]
    fn attempt_gate_allows_frozen_provider_cooling_last_resort() {
        assert!(
            route_state_unavailable_response_for_attempt(
                "soft-last-resort",
                ChannelRouteState::ProviderCoolingDown,
                ChannelRouteState::ProviderCoolingDown,
                true,
            )
            .is_none(),
            "a provider/account soft-cooling target selected by the frozen route plan is a legitimate last-resort first attempt"
        );
    }

    #[test]
    fn attempt_gate_skips_new_provider_cooling_when_better_frozen_target_remains() {
        let response = route_state_unavailable_response_for_attempt(
            "soft-now",
            ChannelRouteState::ProviderCoolingDown,
            ChannelRouteState::Available,
            true,
        )
        .expect("new attempt-time provider cooling should fall through to later frozen targets");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn attempt_gate_allows_new_provider_cooling_when_no_better_frozen_target_remains() {
        assert!(
            route_state_unavailable_response_for_attempt(
                "final-soft-target",
                ChannelRouteState::ProviderCoolingDown,
                ChannelRouteState::Available,
                false,
            )
            .is_none(),
            "attempt-time provider cooling is not a local admission denial when no later frozen route target remains"
        );
    }

    #[test]
    fn record_response_filter_event_to_buffer_records_when_lock_available() {
        let events = Arc::new(Mutex::new(ResponseFilterEventBuffer::new(2)));
        let external_dropped = Arc::new(AtomicU64::new(0));

        record_response_filter_event_to_buffer(
            &events,
            &external_dropped,
            response_filter_event_input("req-1"),
        );

        let events = events
            .lock()
            .expect("response filter events mutex poisoned");
        assert_eq!(events.len(), 1);
        assert_eq!(events.dropped_events(), 0);
        assert_eq!(external_dropped.load(Ordering::Relaxed), 0);
        assert_eq!(events.snapshot()[0].request_id, "req-1");
    }

    #[test]
    fn record_response_filter_event_to_buffer_accounts_contended_external_drop_without_blocking() {
        let events = Arc::new(Mutex::new(ResponseFilterEventBuffer::new(2)));
        let external_dropped = Arc::new(AtomicU64::new(0));
        let guard = events
            .lock()
            .expect("response filter events mutex poisoned");
        let recorder_events = events.clone();
        let recorder_external_dropped = external_dropped.clone();
        let (recorded_tx, recorded_rx) = std::sync::mpsc::channel();
        let recorder = std::thread::spawn(move || {
            record_response_filter_event_to_buffer(
                &recorder_events,
                &recorder_external_dropped,
                response_filter_event_input("req-contended"),
            );
            recorded_tx.send(()).unwrap();
        });

        recorded_rx
            .recv_timeout(Duration::from_millis(50))
            .expect("recording should not wait for a contended response-filter event mutex");
        drop(guard);
        recorder.join().unwrap();

        let events = events
            .lock()
            .expect("response filter events mutex poisoned");
        assert_eq!(events.len(), 0);
        assert_eq!(events.dropped_events(), 0);
        assert_eq!(external_dropped.load(Ordering::Relaxed), 1);
        assert_eq!(events.snapshot(), Vec::new());
    }

    fn response_filter_event_input(request_id: &str) -> ResponseFilterEventInput {
        ResponseFilterEventInput {
            request_id: request_id.to_string(),
            channel_id: "channel-a".to_string(),
            public_model: "gpt-test".to_string(),
            rule_id: "rule-a".to_string(),
            action: "reject".to_string(),
            content_kind: "json".to_string(),
            reason_code: "rule_matched".to_string(),
            outcome: "rejected".to_string(),
            body_committed: false,
        }
    }
}
