use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

use crate::{
    config::ResolvedClientToken,
    management_errors::ManagementServiceError,
    management_status::{
        add_key_pool_snapshot_counts, channel_health_status, channel_health_status_from_health,
        ChannelHealthStatus, RuntimeCredentialCounts,
    },
    provider::ProviderKind,
    route_plan::{
        preview_route, ModelRoute, RoutePreviewCandidate, RoutePreviewInput, RoutePreviewReason,
        RouteStrategy, RouteTarget,
    },
    state::{AppState, ChannelHealth, ChannelId, ChannelRoutePlanContext},
};

#[derive(Debug, Serialize)]
pub struct ModelRoutesResponse {
    pub default_channel: Option<String>,
    pub unmapped_model_policy: &'static str,
    pub routes: Vec<ModelRouteStatus>,
}

#[derive(Debug, Serialize)]
pub struct ModelRouteStatus {
    pub model: String,
    pub strategy: String,
    pub targets: Vec<ModelRouteTargetStatus>,
}

#[derive(Debug, Serialize)]
pub struct ModelRouteTargetStatus {
    pub channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_model: Option<String>,
    pub provider_kind: ProviderKind,
    pub priority: u16,
    pub weight: u16,
    pub enabled: bool,
    pub health: ChannelHealthStatus,
}

pub async fn model_routes_response(state: &AppState) -> ModelRoutesResponse {
    let context = state.channels.model_routes_context();
    let routes = context
        .routes
        .into_iter()
        .map(|route_context| ModelRouteStatus {
            model: route_context.route.public_model,
            strategy: route_context.route.strategy.as_str().to_string(),
            targets: route_context
                .route
                .targets
                .into_iter()
                .map(|target| {
                    let health = route_context_health_status(
                        route_context.target_health[&target.channel_id].clone(),
                    );
                    ModelRouteTargetStatus {
                        channel_id: target.channel_id.0,
                        upstream_model: target.upstream_model,
                        provider_kind: target.provider_kind,
                        priority: target.priority,
                        weight: target.weight,
                        enabled: target.enabled,
                        health,
                    }
                })
                .collect(),
        })
        .collect();
    ModelRoutesResponse {
        default_channel: context.default_channel,
        unmapped_model_policy: "default_channel",
        routes,
    }
}

fn route_context_health_status(health: Option<ChannelHealth>) -> ChannelHealthStatus {
    match health {
        Some(health) => channel_health_status_from_health(health),
        None => ChannelHealthStatus {
            kind: "degraded",
            reason: Some("unknown channel".to_string()),
            reason_code: Some("unknown_channel".to_string()),
            source: Some("runtime"),
            remaining_seconds: None,
            suppression_count: 0,
            generation: 0,
        },
    }
}

#[derive(Debug, Serialize)]
pub struct RoutingPreviewResponse {
    pub request_id: String,
    pub model: String,
    pub route_kind: &'static str,
    pub registry_generation: u64,
    pub candidate_limit: usize,
    pub client_token: RoutingPreviewClientStatus,
    pub selected_target: Option<RoutingPreviewSelectedTarget>,
    pub candidates: Vec<RoutingPreviewCandidateStatus>,
}

pub fn routing_preview_response(
    request_id: String,
    model: String,
    route_kind: &'static str,
    registry_generation: u64,
    candidate_limit: usize,
    client_token: RoutingPreviewClientStatus,
    candidates: Vec<RoutingPreviewCandidateStatus>,
) -> RoutingPreviewResponse {
    let selected_target = candidates
        .iter()
        .find(|candidate| candidate.selected)
        .and_then(|candidate| {
            candidate
                .plan_position
                .map(|position| RoutingPreviewSelectedTarget {
                    channel_id: candidate.channel_id.clone(),
                    plan_position: position,
                })
        });

    RoutingPreviewResponse {
        request_id,
        model,
        route_kind,
        registry_generation,
        candidate_limit,
        client_token,
        selected_target,
        candidates,
    }
}

#[derive(Debug, Serialize)]
pub struct RoutingPreviewClientStatus {
    pub id: String,
    pub name: String,
    pub unrestricted_model_groups: bool,
    pub unrestricted_channels: bool,
    pub allowed_model_groups: Vec<String>,
    pub allowed_channels: Vec<String>,
}

pub fn routing_preview_client_status(client: &ResolvedClientToken) -> RoutingPreviewClientStatus {
    RoutingPreviewClientStatus {
        id: client.id.clone(),
        name: client.name.clone(),
        unrestricted_model_groups: client.allowed_model_groups.is_empty(),
        unrestricted_channels: client.allowed_channels.is_empty(),
        allowed_model_groups: client.allowed_model_groups.clone(),
        allowed_channels: client.allowed_channels.clone(),
    }
}

pub fn routing_preview_client(
    state: &AppState,
    client_token_ref: Option<&str>,
) -> Result<ResolvedClientToken, ManagementServiceError> {
    let client_tokens = state
        .client_tokens
        .read()
        .expect("client token registry lock poisoned")
        .clone();
    let client = match client_token_ref {
        Some(reference) => client_tokens
            .into_iter()
            .find(|token| token.enabled && (token.id == reference || token.name == reference))
            .ok_or_else(|| {
                ManagementServiceError::NotFound(format!("unknown client token {reference}"))
            })?,
        None => client_tokens
            .into_iter()
            .find(|token| token.enabled)
            .ok_or_else(|| {
                ManagementServiceError::NotFound("no enabled client token".to_string())
            })?,
    };
    Ok(client)
}

pub struct RoutingPreviewRoute {
    pub route_kind: &'static str,
    pub route: Option<ModelRoute>,
}

pub fn routing_preview_route(
    route_context: &ChannelRoutePlanContext,
    model: &str,
) -> Result<RoutingPreviewRoute, ManagementServiceError> {
    if let Some(route) = route_context.model_route.clone() {
        return Ok(RoutingPreviewRoute {
            route_kind: "explicit_model_route",
            route: Some(route),
        });
    }

    let Some(default_channel) = route_context.default_channel.clone() else {
        return Ok(RoutingPreviewRoute {
            route_kind: "no_route",
            route: None,
        });
    };
    let Some(provider_kind) = route_context.default_channel_provider_kind else {
        return Err(ManagementServiceError::NotFound(format!(
            "unknown default channel {default_channel}"
        )));
    };

    Ok(RoutingPreviewRoute {
        route_kind: "default_channel",
        route: Some(ModelRoute {
            public_model: model.to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![RouteTarget {
                channel_id: ChannelId(default_channel),
                provider_kind,
                upstream_model: None,
                priority: 0,
                weight: 1,
                enabled: true,
            }],
        }),
    })
}

pub async fn routing_preview_for_model(
    state: &AppState,
    model: &str,
    client_token_ref: Option<&str>,
) -> Result<RoutingPreviewResponse, ManagementServiceError> {
    let client = routing_preview_client(state, client_token_ref)?;
    let client_status = routing_preview_client_status(&client);
    let route_context = state.channels.route_plan_context(Some(model));
    if !client.allowed_model_groups.is_empty()
        && !state
            .runtime_catalogs
            .client_model_allowed(&client.allowed_model_groups, model)
    {
        return Ok(routing_preview_response(
            format!("preview:{model}:{}", client.id),
            model.to_string(),
            "client_model_denied",
            route_context.registry_generation,
            state.routing.max_route_candidates,
            client_status,
            Vec::new(),
        ));
    }

    let routing_preview_route = routing_preview_route(&route_context, model)?;
    let route_ref = routing_preview_route.route.as_ref();
    if route_ref.is_none() {
        return Ok(routing_preview_response(
            format!("preview:{model}:{}", client.id),
            model.to_string(),
            routing_preview_route.route_kind,
            route_context.registry_generation,
            state.routing.max_route_candidates,
            client_status,
            Vec::new(),
        ));
    }

    let channel_states = if route_context.model_route.is_some() {
        route_context.model_route_channel_states.clone()
    } else {
        route_context
            .default_channel
            .as_ref()
            .zip(route_context.default_channel_route_state)
            .map(|(channel_id, state)| HashMap::from([(ChannelId(channel_id.clone()), state)]))
            .unwrap_or_default()
    };
    let request_id = format!("preview:{model}:{}", client.id);
    let preview = preview_route(RoutePreviewInput {
        request_id: request_id.clone(),
        registry_generation: route_context.registry_generation,
        public_model: Some(model.to_string()),
        route: route_ref,
        channel_states: &channel_states,
        allowed_channels: &client.allowed_channels,
        candidate_limit: state.routing.max_route_candidates,
    });

    let channel_statuses =
        routing_preview_channel_statuses_for_candidates(state, &preview.candidates).await;
    let candidates = routing_preview_candidate_statuses(&channel_statuses, preview.candidates);
    Ok(routing_preview_response(
        request_id,
        model.to_string(),
        routing_preview_route.route_kind,
        preview.registry_generation,
        preview.candidate_limit,
        client_status,
        candidates,
    ))
}

#[derive(Debug, Serialize)]
pub struct RoutingPreviewSelectedTarget {
    pub channel_id: String,
    pub plan_position: usize,
}

#[derive(Debug, Serialize)]
pub struct RoutingPreviewCandidateStatus {
    pub target_index: usize,
    pub channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_model: Option<String>,
    pub provider_kind: ProviderKind,
    pub priority: u16,
    pub weight: u16,
    pub target_enabled: bool,
    pub included: bool,
    pub selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_position: Option<usize>,
    pub reasons: Vec<&'static str>,
    pub health: ChannelHealthStatus,
    pub credential_set_id: String,
    pub selector_generation: u64,
    pub credentials: RuntimeCredentialCounts,
}

pub fn routing_preview_candidate_status(
    candidate: RoutePreviewCandidate,
    status: RoutingPreviewChannelStatus,
) -> RoutingPreviewCandidateStatus {
    RoutingPreviewCandidateStatus {
        target_index: candidate.target_index,
        channel_id: candidate.channel_id.0,
        upstream_model: candidate.upstream_model,
        provider_kind: candidate.provider_kind,
        priority: candidate.priority,
        weight: candidate.weight,
        target_enabled: candidate.target_enabled,
        included: candidate.included,
        selected: candidate.selected,
        plan_position: candidate.plan_position,
        reasons: candidate
            .reasons
            .into_iter()
            .map(RoutePreviewReason::as_str)
            .collect(),
        health: status.health,
        credential_set_id: status.credential_set_id,
        selector_generation: status.selector_generation,
        credentials: status.credentials,
    }
}

#[derive(Debug, Clone)]
pub struct RoutingPreviewChannelStatus {
    pub health: ChannelHealthStatus,
    pub credential_set_id: String,
    pub selector_generation: u64,
    pub credentials: RuntimeCredentialCounts,
}

impl RoutingPreviewChannelStatus {
    fn unknown_channel() -> Self {
        Self {
            health: ChannelHealthStatus {
                kind: "degraded",
                reason: Some("unknown channel".to_string()),
                reason_code: Some("unknown_channel".to_string()),
                source: Some("runtime"),
                remaining_seconds: None,
                suppression_count: 0,
                generation: 0,
            },
            credential_set_id: String::new(),
            selector_generation: 0,
            credentials: RuntimeCredentialCounts::default(),
        }
    }
}

pub async fn routing_preview_channel_statuses_for_candidates(
    state: &AppState,
    candidates: &[RoutePreviewCandidate],
) -> HashMap<String, RoutingPreviewChannelStatus> {
    let mut channel_statuses: HashMap<String, RoutingPreviewChannelStatus> = HashMap::new();
    for candidate in candidates {
        if channel_statuses.contains_key(&candidate.channel_id.0) {
            continue;
        }
        let status = routing_preview_channel_status(state, &candidate.channel_id.0).await;
        channel_statuses.insert(candidate.channel_id.0.clone(), status);
    }
    channel_statuses
}

pub async fn routing_preview_channel_status(
    state: &AppState,
    channel_id: &str,
) -> RoutingPreviewChannelStatus {
    match state.channels.get(channel_id) {
        Some(pool_state) => {
            let snapshot = pool_state.pool.lock().await.snapshot();
            let mut credentials = RuntimeCredentialCounts::default();
            add_key_pool_snapshot_counts(&mut credentials, snapshot);
            RoutingPreviewChannelStatus {
                health: channel_health_status(&pool_state),
                credential_set_id: pool_state.credential_set_id.0.clone(),
                selector_generation: pool_state.selector_generation.load(Ordering::Acquire),
                credentials,
            }
        }
        None => RoutingPreviewChannelStatus::unknown_channel(),
    }
}

pub fn routing_preview_candidate_statuses(
    channel_statuses: &HashMap<String, RoutingPreviewChannelStatus>,
    candidates: Vec<RoutePreviewCandidate>,
) -> Vec<RoutingPreviewCandidateStatus> {
    let mut statuses = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let status = channel_statuses
            .get(&candidate.channel_id.0)
            .cloned()
            .unwrap_or_else(RoutingPreviewChannelStatus::unknown_channel);
        statuses.push(routing_preview_candidate_status(candidate, status));
    }
    statuses
}
