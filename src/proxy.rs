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
    events::RoutingTelemetry,
    failure_observer::{
        record_routing_telemetry, transition_observed_failure, transition_observed_upstream_failure,
    },
    model_catalog,
    provider::{
        bytes_from_body_for_context, EndpointKind, InboundProtocol, NamedPoolRequestMode,
        ProviderAdapter,
    },
    route_plan::{
        plan_route, preview_route, ChannelRouteState, RouteCandidate, RoutePlan, RoutePlanInput,
        RoutePreview, RoutePreviewInput, RoutePreviewReason,
    },
    routing::{
        apply_retry_directive_to_attempt_state, FailureSource, FrozenRetryCandidates,
        RequestSelectionSnapshot, RetryAttemptContinuation, RetryDirective, SelectionReason,
    },
    state::{AppState, ChannelId},
    upstream_response::{read_limited_body, response_with_headers, stream_response},
};

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> String {
    format!("req_{}", REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed))
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
            return Err(Box::new(no_route_candidate_response(&[
                "channel_cooling_down",
            ])));
        }
        let reason_codes = relevant_route_candidate_reason_codes(&preview);
        if !reason_codes.is_empty() {
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
    if let Some(response) = route_state_unavailable_response(
        &route_plan.targets[0].channel_id,
        route_context.route_state,
    ) {
        return response;
    }
    let adapter = ProviderAdapter::new(pool_state.provider_kind);
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
        ChannelRouteState::Available | ChannelRouteState::Degraded => return None,
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
        if let Some(response) = route_state_unavailable_response(
            &pool_name,
            target
                .route_state
                .most_restrictive(pool_state.route_state()),
        ) {
            return response;
        }
        let mut pool = pool_state.pool.lock().await;
        let selected = match pool.select() {
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
        pool_state
            .failure_domains
            .record_success(&pool_state.provider_id, &pool_state.account_id);
        pool_state.record_selected_channel_success();
        return stream_response(
            status,
            response_headers,
            upstream_resp,
            state.response_filter.clone(),
        );
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
        match forward_with_pool(&req, target.clone()).await {
            PoolForwardResult::Response(response) => return response,
            PoolForwardResult::RouteFallback(response) if route_attempt + 1 < total_pools => {
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

async fn forward_with_pool(req: &ForwardRequest, target: ForwardTarget) -> PoolForwardResult {
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
    let mut attempt = 0usize;
    let mut retry_credential_id: Option<CredentialId> = None;
    let mut frozen_retry_candidates: Option<FrozenRetryCandidates> = None;

    loop {
        let send_guard = pool_state.send_gate.read().await;
        let (selected, auth_header, auth_prefix, snapshot) = {
            let _mutation_guard = pool_state.mutation_gate.lock().await;
            if let Some(response) = route_state_unavailable_response(
                &pool_name,
                target
                    .route_state
                    .most_restrictive(pool_state.route_state()),
            ) {
                return PoolForwardResult::RouteFallback(response);
            }
            let mut pool = pool_state.pool.lock().await;
            let selected = match retry_credential_id.take() {
                Some(credential_id) => pool.select_credential_by_id(&credential_id),
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
            upstream = upstream.timeout(state.timeout_profile.non_streaming_total);
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
                if matches!(directive, RetryDirective::RetryRouteTarget) {
                    return PoolForwardResult::RouteFallback(response);
                }
                return PoolForwardResult::Response(response);
            }
        };

        let status = upstream_resp.status();
        let response_headers = upstream_resp.headers().clone();
        if status.is_success() {
            pool_state
                .failure_domains
                .record_success(&pool_state.provider_id, &pool_state.account_id);
            pool_state.record_selected_channel_success();
            return PoolForwardResult::Response(stream_response(
                status,
                response_headers,
                upstream_resp,
                state.response_filter.clone(),
            ));
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
            RetryAttemptContinuation::ReturnCurrentError { .. }
            | RetryAttemptContinuation::FrozenCandidateDrift { .. } => {}
        }

        let response = response_with_headers(status, response_headers, Body::from(bytes));
        return PoolForwardResult::Response(response);
    }
}
