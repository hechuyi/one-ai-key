use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

use crate::{
    config::ResolvedClientToken,
    endpoint_capabilities::{EndpointCapabilitiesStatus, EndpointSupport},
    management_errors::ManagementServiceError,
    management_status::{
        add_key_pool_snapshot_counts, channel_health_status, channel_health_status_from_health,
        ChannelHealthStatus, RuntimeCredentialCounts,
    },
    provider::ProviderKind,
    route_plan::{
        preview_route, ChannelRouteState, ModelRoute, RoutePreviewCandidate, RoutePreviewInput,
        RoutePreviewReason, RouteStrategy, RouteTarget,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_capabilities: Option<EndpointCapabilitiesStatus>,
    pub priority: u16,
    pub weight: u16,
    pub enabled: bool,
    pub health: ChannelHealthStatus,
}

pub async fn model_routes_response(state: &AppState) -> ModelRoutesResponse {
    let context = state.channels.model_routes_context();
    let endpoint_capabilities =
        endpoint_capabilities_by_channel(state, context.registry_generation);
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
                    let channel_id = target.channel_id.0;
                    let health = route_context_health_status(
                        route_context
                            .target_health
                            .get(&ChannelId(channel_id.clone()))
                            .cloned()
                            .flatten(),
                    );
                    ModelRouteTargetStatus {
                        endpoint_capabilities: endpoint_capabilities.get(&channel_id).cloned(),
                        channel_id,
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
    pub policy_summary: RoutingPreviewPolicySummary,
    pub client_token: RoutingPreviewClientStatus,
    pub selected_target: Option<RoutingPreviewSelectedTarget>,
    pub candidates: Vec<RoutingPreviewCandidateStatus>,
}

pub struct RoutingPreviewResponseInput {
    pub request_id: String,
    pub model: String,
    pub route_kind: &'static str,
    pub registry_generation: u64,
    pub candidate_limit: usize,
    pub policy_summary: RoutingPreviewPolicySummary,
    pub client_token: RoutingPreviewClientStatus,
    pub candidates: Vec<RoutingPreviewCandidateStatus>,
}

pub fn routing_preview_response(input: RoutingPreviewResponseInput) -> RoutingPreviewResponse {
    let RoutingPreviewResponseInput {
        request_id,
        model,
        route_kind,
        registry_generation,
        candidate_limit,
        policy_summary,
        client_token,
        candidates,
    } = input;
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
        policy_summary,
        client_token,
        selected_target,
        candidates,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct RoutingPreviewPolicySummary {
    pub route_target_retry_enabled: bool,
    pub same_request_credential_retry_enabled: bool,
    pub max_same_request_retries: usize,
    pub candidate_limit: usize,
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
        return Ok(routing_preview_response(RoutingPreviewResponseInput {
            request_id: format!("preview:{model}:{}", client.id),
            model: model.to_string(),
            route_kind: "client_model_denied",
            registry_generation: route_context.registry_generation,
            candidate_limit: state.routing.max_route_candidates,
            policy_summary: RoutingPreviewPolicySummary {
                candidate_limit: state.routing.max_route_candidates,
                ..RoutingPreviewPolicySummary::default()
            },
            client_token: client_status,
            candidates: Vec::new(),
        }));
    }

    let routing_preview_route = routing_preview_route(&route_context, model)?;
    let route_ref = routing_preview_route.route.as_ref();
    if route_ref.is_none() {
        return Ok(routing_preview_response(RoutingPreviewResponseInput {
            request_id: format!("preview:{model}:{}", client.id),
            model: model.to_string(),
            route_kind: routing_preview_route.route_kind,
            registry_generation: route_context.registry_generation,
            candidate_limit: state.routing.max_route_candidates,
            policy_summary: RoutingPreviewPolicySummary {
                candidate_limit: state.routing.max_route_candidates,
                ..RoutingPreviewPolicySummary::default()
            },
            client_token: client_status,
            candidates: Vec::new(),
        }));
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

    let channel_statuses = routing_preview_channel_statuses_for_candidates(
        state,
        preview.registry_generation,
        &preview.candidates,
    )
    .await;
    let policy_summary = routing_preview_policy_summary(
        &channel_statuses,
        &preview.candidates,
        preview.candidate_limit,
    );
    let candidates = routing_preview_candidate_statuses(&channel_statuses, preview.candidates);
    Ok(routing_preview_response(RoutingPreviewResponseInput {
        request_id,
        model: model.to_string(),
        route_kind: routing_preview_route.route_kind,
        registry_generation: preview.registry_generation,
        candidate_limit: preview.candidate_limit,
        policy_summary,
        client_token: client_status,
        candidates,
    }))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_capabilities: Option<EndpointCapabilitiesStatus>,
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
    let channel_id = candidate.channel_id.0;
    RoutingPreviewCandidateStatus {
        target_index: candidate.target_index,
        channel_id,
        upstream_model: candidate.upstream_model,
        provider_kind: candidate.provider_kind,
        endpoint_capabilities: status.endpoint_capabilities,
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
    pub endpoint_capabilities: Option<EndpointCapabilitiesStatus>,
    pub credential_set_id: String,
    pub selector_generation: u64,
    pub credentials: RuntimeCredentialCounts,
    pub route_target_retry_enabled: bool,
    pub same_request_credential_retry_enabled: bool,
    pub max_same_request_retries: usize,
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
            endpoint_capabilities: None,
            credential_set_id: String::new(),
            selector_generation: 0,
            credentials: RuntimeCredentialCounts::default(),
            route_target_retry_enabled: false,
            same_request_credential_retry_enabled: false,
            max_same_request_retries: 0,
        }
    }
}

pub async fn routing_preview_channel_statuses_for_candidates(
    state: &AppState,
    expected_registry_generation: u64,
    candidates: &[RoutePreviewCandidate],
) -> HashMap<String, RoutingPreviewChannelStatus> {
    let mut channel_statuses: HashMap<String, RoutingPreviewChannelStatus> = HashMap::new();
    for candidate in candidates {
        if channel_statuses.contains_key(&candidate.channel_id.0) {
            continue;
        }
        let status = routing_preview_channel_status(
            state,
            expected_registry_generation,
            &candidate.channel_id.0,
        )
        .await;
        channel_statuses.insert(candidate.channel_id.0.clone(), status);
    }
    channel_statuses
}

pub async fn routing_preview_channel_status(
    state: &AppState,
    expected_registry_generation: u64,
    channel_id: &str,
) -> RoutingPreviewChannelStatus {
    let endpoint_capabilities =
        endpoint_capability_status_for_channel(state, channel_id, expected_registry_generation);
    match state.channels.get(channel_id) {
        Some(pool_state) => {
            let snapshot = pool_state.pool.lock().await.snapshot();
            let mut credentials = RuntimeCredentialCounts::default();
            add_key_pool_snapshot_counts(&mut credentials, snapshot);
            RoutingPreviewChannelStatus {
                health: channel_health_status(&pool_state),
                endpoint_capabilities,
                credential_set_id: pool_state.credential_set_id.0.clone(),
                selector_generation: pool_state.selector_generation.load(Ordering::Acquire),
                credentials,
                route_target_retry_enabled: pool_state.route_target_retry_enabled,
                same_request_credential_retry_enabled: pool_state
                    .retry_switched_key_in_same_request,
                max_same_request_retries: pool_state.max_same_request_retries,
            }
        }
        None => RoutingPreviewChannelStatus::unknown_channel(),
    }
}

fn endpoint_capabilities_by_channel(
    state: &AppState,
    expected_registry_generation: u64,
) -> HashMap<String, EndpointCapabilitiesStatus> {
    let (registry_generation, capabilities) =
        state.runtime_catalogs.channel_endpoint_capabilities();
    if registry_generation != expected_registry_generation {
        return HashMap::new();
    }
    capabilities
        .into_iter()
        .map(|(channel_id, capabilities)| {
            (channel_id, EndpointCapabilitiesStatus::from(&capabilities))
        })
        .collect()
}

fn endpoint_capability_status_for_channel(
    state: &AppState,
    channel_id: &str,
    expected_registry_generation: u64,
) -> Option<EndpointCapabilitiesStatus> {
    let (registry_generation, capabilities) =
        state.runtime_catalogs.channel_endpoint_capabilities();
    if registry_generation != expected_registry_generation {
        return None;
    }
    capabilities
        .get(channel_id)
        .map(EndpointCapabilitiesStatus::from)
}

pub fn routing_preview_policy_summary(
    channel_statuses: &HashMap<String, RoutingPreviewChannelStatus>,
    candidates: &[RoutePreviewCandidate],
    candidate_limit: usize,
) -> RoutingPreviewPolicySummary {
    let mut summary = RoutingPreviewPolicySummary {
        candidate_limit,
        ..RoutingPreviewPolicySummary::default()
    };
    for candidate in candidates {
        let Some(status) = channel_statuses.get(&candidate.channel_id.0) else {
            continue;
        };
        summary.route_target_retry_enabled |= status.route_target_retry_enabled;
        summary.same_request_credential_retry_enabled |=
            status.same_request_credential_retry_enabled;
        summary.max_same_request_retries = summary
            .max_same_request_retries
            .max(status.max_same_request_retries);
    }
    summary
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

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct EndpointFamilyAvailabilityExplain {
    pub status: &'static str,
    pub reason_code: &'static str,
    pub next_action: &'static str,
    pub endpoint_family: String,
    pub public_model: String,
    pub route_kind: &'static str,
    pub registry_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_token: Option<EndpointFamilyAvailabilityClient>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct EndpointFamilyAvailabilityClient {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

pub struct EndpointFamilyAvailabilityExplainInput<'a> {
    pub client_tokens: &'a [ResolvedClientToken],
    pub client_token_ref: Option<&'a str>,
    pub endpoint_family: &'a str,
    pub public_model: &'a str,
    pub registry_generation: u64,
    pub model_allowed: bool,
    pub model_visible: bool,
    pub route_kind: &'static str,
    pub route: Option<&'a ModelRoute>,
    pub channel_states: &'a HashMap<ChannelId, ChannelRouteState>,
    pub endpoint_capabilities: &'a HashMap<String, EndpointCapabilitiesStatus>,
    pub candidate_limit: usize,
}

pub async fn endpoint_family_availability_explain(
    state: &AppState,
    client_token_ref: Option<&str>,
    endpoint_family: &str,
    public_model: &str,
) -> EndpointFamilyAvailabilityExplain {
    let client_tokens = state
        .client_tokens
        .read()
        .expect("client token registry lock poisoned")
        .clone();
    let client = client_token_ref.and_then(|reference| {
        client_tokens
            .iter()
            .find(|token| token.id == reference || token.name == reference)
    });
    let enabled_client = client.filter(|token| token.enabled);
    let route_context = state.channels.route_plan_context(Some(public_model));
    let routing_preview_route = routing_preview_route(&route_context, public_model).ok();
    let route_kind = routing_preview_route
        .as_ref()
        .map(|route| route.route_kind)
        .unwrap_or("no_route");
    let route = routing_preview_route
        .as_ref()
        .and_then(|route| route.route.as_ref());
    let model_allowed = enabled_client.is_some_and(|client| {
        state
            .runtime_catalogs
            .client_model_allowed(&client.allowed_model_groups, public_model)
    });
    let model_visible = enabled_client.is_some_and(|client| {
        model_allowed
            && (route_context.model_route.is_some()
                || route_kind == "default_channel"
                || route_context.has_claimed_upstream_model(&client.allowed_channels))
    });
    let channel_states = route_context_channel_states(&route_context);
    let endpoint_capabilities =
        endpoint_capabilities_by_channel(state, route_context.registry_generation);

    endpoint_family_availability_explain_from_parts(EndpointFamilyAvailabilityExplainInput {
        client_tokens: &client_tokens,
        client_token_ref,
        endpoint_family,
        public_model,
        registry_generation: route_context.registry_generation,
        model_allowed,
        model_visible,
        route_kind,
        route,
        channel_states: &channel_states,
        endpoint_capabilities: &endpoint_capabilities,
        candidate_limit: state.routing.max_route_candidates,
    })
}

pub fn endpoint_family_availability_explain_from_parts(
    input: EndpointFamilyAvailabilityExplainInput<'_>,
) -> EndpointFamilyAvailabilityExplain {
    let Some(client_token_ref) = input.client_token_ref else {
        return endpoint_family_availability_explain_result(
            input,
            None,
            "unavailable",
            "token_missing",
            "provide_client_token_ref",
        );
    };
    let Some(client) = input
        .client_tokens
        .iter()
        .find(|token| token.id == client_token_ref || token.name == client_token_ref)
    else {
        return endpoint_family_availability_explain_result(
            input,
            None,
            "unavailable",
            "token_unknown",
            "check_client_token_ref",
        );
    };
    let client_status = EndpointFamilyAvailabilityClient {
        id: client.id.clone(),
        name: client.name.clone(),
        enabled: client.enabled,
    };
    if !client.enabled {
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            "unavailable",
            "token_disabled",
            "enable_client_token",
        );
    }

    let Some(endpoint_family) = EndpointFamily::parse(input.endpoint_family) else {
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            "unavailable",
            "unsupported_endpoint_family",
            "use_supported_endpoint_family",
        );
    };
    if !input.model_allowed || !input.model_visible {
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            "unavailable",
            "model_missing",
            "publish_or_route_model",
        );
    }
    let Some(route) = input.route else {
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            "unavailable",
            "no_route",
            "configure_route_or_default_channel",
        );
    };

    let family_targets =
        route_with_endpoint_family_targets(route, endpoint_family, input.endpoint_capabilities);
    let Some(family_route) = family_targets.route.as_ref() else {
        let reason_code = if family_targets.has_unsupported_target {
            "endpoint_family_unsupported"
        } else {
            "endpoint_family_mismatch"
        };
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            "unavailable",
            reason_code,
            "configure_endpoint_capabilities_or_route",
        );
    };
    let preview = preview_route(RoutePreviewInput {
        request_id: format!(
            "endpoint-family-explain:{}:{}",
            input.public_model,
            endpoint_family.as_str()
        ),
        registry_generation: input.registry_generation,
        public_model: Some(input.public_model.to_string()),
        route: Some(family_route),
        channel_states: input.channel_states,
        allowed_channels: &client.allowed_channels,
        candidate_limit: input.candidate_limit,
    });
    if preview.selected_target_index.is_none() {
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            "unavailable",
            "no_usable_key_or_target",
            "enable_target_or_key",
        );
    }

    endpoint_family_availability_explain_result(
        input,
        Some(client_status),
        "available",
        "available",
        "none",
    )
}

fn route_context_channel_states(
    route_context: &ChannelRoutePlanContext,
) -> HashMap<ChannelId, ChannelRouteState> {
    if route_context.model_route.is_some() {
        route_context.model_route_channel_states.clone()
    } else {
        route_context
            .default_channel
            .as_ref()
            .zip(route_context.default_channel_route_state)
            .map(|(channel_id, state)| HashMap::from([(ChannelId(channel_id.clone()), state)]))
            .unwrap_or_default()
    }
}

fn endpoint_family_availability_explain_result(
    input: EndpointFamilyAvailabilityExplainInput<'_>,
    client_token: Option<EndpointFamilyAvailabilityClient>,
    status: &'static str,
    reason_code: &'static str,
    next_action: &'static str,
) -> EndpointFamilyAvailabilityExplain {
    EndpointFamilyAvailabilityExplain {
        status,
        reason_code,
        next_action,
        endpoint_family: input.endpoint_family.to_string(),
        public_model: safe_public_model_label(input.public_model),
        route_kind: input.route_kind,
        registry_generation: input.registry_generation,
        client_token,
    }
}

fn safe_public_model_label(public_model: &str) -> String {
    let trimmed = public_model.trim();
    if is_safe_public_model_label(trimmed) {
        trimmed.to_string()
    } else {
        "<redacted-public-model>".to_string()
    }
}

fn is_safe_public_model_label(public_model: &str) -> bool {
    if public_model.is_empty() || public_model.len() > 128 {
        return false;
    }
    if public_model.starts_with('/')
        || public_model.starts_with("~/")
        || public_model.starts_with("./")
        || public_model.starts_with("../")
        || public_model.contains("/../")
        || public_model.contains('\\')
        || public_model.contains("://")
        || public_model.contains('?')
        || public_model.contains('&')
        || public_model.contains('=')
        || public_model.chars().any(char::is_control)
    {
        return false;
    }
    let lower = public_model.to_ascii_lowercase();
    if looks_like_windows_absolute_path(public_model)
        || looks_like_url_scheme(&lower)
        || lower.contains("token")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("secret")
        || lower.contains("authorization")
        || lower.contains("bearer")
        || lower.contains("sk-")
        || lower.contains("sk_")
    {
        return false;
    }
    public_model.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
    })
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn looks_like_url_scheme(lower: &str) -> bool {
    ["http:", "https:", "file:", "ftp:", "s3:", "gs:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
}

struct EndpointFamilyRouteTargets {
    route: Option<ModelRoute>,
    has_unsupported_target: bool,
}

fn route_with_endpoint_family_targets(
    route: &ModelRoute,
    endpoint_family: EndpointFamily,
    endpoint_capabilities: &HashMap<String, EndpointCapabilitiesStatus>,
) -> EndpointFamilyRouteTargets {
    let mut has_unsupported_target = false;
    let targets: Vec<RouteTarget> = route
        .targets
        .iter()
        .filter(|target| {
            let Some(capabilities) = endpoint_capabilities.get(&target.channel_id.0) else {
                return false;
            };
            match endpoint_family.support(capabilities) {
                EndpointSupport::Supported => true,
                EndpointSupport::Unsupported => {
                    has_unsupported_target = true;
                    false
                }
                EndpointSupport::Unknown => false,
            }
        })
        .cloned()
        .collect();
    if targets.is_empty() {
        return EndpointFamilyRouteTargets {
            route: None,
            has_unsupported_target,
        };
    }
    EndpointFamilyRouteTargets {
        route: Some(ModelRoute {
            public_model: route.public_model.clone(),
            strategy: route.strategy,
            targets,
        }),
        has_unsupported_target,
    }
}

#[derive(Debug, Clone, Copy)]
enum EndpointFamily {
    ChatCompletions,
    Responses,
    Embeddings,
}

impl EndpointFamily {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "chat_completions" => Some(Self::ChatCompletions),
            "responses" => Some(Self::Responses),
            "embeddings" => Some(Self::Embeddings),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
            Self::Embeddings => "embeddings",
        }
    }

    fn support(self, capabilities: &EndpointCapabilitiesStatus) -> EndpointSupport {
        match self {
            Self::ChatCompletions => capabilities.chat_completions,
            Self::Responses => capabilities.responses,
            Self::Embeddings => capabilities.embeddings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint_capabilities::EndpointSupport;

    fn client(id: &str, enabled: bool) -> ResolvedClientToken {
        ResolvedClientToken {
            id: id.to_string(),
            name: format!("{id}-name"),
            token_hash: format!("{id}-hash"),
            enabled,
            allowed_model_groups: Vec::new(),
            allowed_channels: Vec::new(),
        }
    }

    fn route() -> ModelRoute {
        ModelRoute {
            public_model: "gpt-public".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![RouteTarget {
                channel_id: ChannelId("ch1".to_string()),
                provider_kind: ProviderKind::OpenAiCompatible,
                upstream_model: Some("upstream-private".to_string()),
                priority: 0,
                weight: 1,
                enabled: true,
            }],
        }
    }

    fn capabilities(
        chat_completions: EndpointSupport,
    ) -> HashMap<String, EndpointCapabilitiesStatus> {
        HashMap::from([(
            "ch1".to_string(),
            EndpointCapabilitiesStatus {
                chat_completions,
                responses: EndpointSupport::Unsupported,
                embeddings: EndpointSupport::Unknown,
                models: Default::default(),
                diagnostic_labels: Vec::new(),
            },
        )])
    }

    #[test]
    fn endpoint_family_explain_reports_available_for_supported_usable_route() {
        let route = route();
        let clients = vec![client("client-a", true)];

        let explain = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: Some("client-a"),
                endpoint_family: "chat_completions",
                public_model: "gpt-public",
                registry_generation: 7,
                model_allowed: true,
                model_visible: true,
                route_kind: "explicit_model_route",
                route: Some(&route),
                channel_states: &HashMap::from([(
                    ChannelId("ch1".to_string()),
                    ChannelRouteState::Available,
                )]),
                endpoint_capabilities: &capabilities(EndpointSupport::Supported),
                candidate_limit: 16,
            },
        );

        assert_eq!(explain.status, "available");
        assert_eq!(explain.reason_code, "available");
        assert_eq!(explain.next_action, "none");
        assert_eq!(explain.client_token.as_ref().unwrap().id, "client-a");
    }

    #[test]
    fn endpoint_family_explain_distinguishes_missing_unknown_and_disabled_token() {
        let clients = vec![client("disabled", false)];

        let missing = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: None,
                endpoint_family: "chat_completions",
                public_model: "gpt-public",
                registry_generation: 7,
                model_allowed: true,
                model_visible: true,
                route_kind: "explicit_model_route",
                route: Some(&route()),
                channel_states: &HashMap::new(),
                endpoint_capabilities: &HashMap::new(),
                candidate_limit: 16,
            },
        );
        assert_eq!(missing.reason_code, "token_missing");

        let unknown = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: Some("unknown"),
                endpoint_family: "chat_completions",
                public_model: "gpt-public",
                registry_generation: 7,
                model_allowed: true,
                model_visible: true,
                route_kind: "explicit_model_route",
                route: Some(&route()),
                channel_states: &HashMap::new(),
                endpoint_capabilities: &HashMap::new(),
                candidate_limit: 16,
            },
        );
        assert_eq!(unknown.reason_code, "token_unknown");

        let disabled = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: Some("disabled"),
                endpoint_family: "chat_completions",
                public_model: "gpt-public",
                registry_generation: 7,
                model_allowed: true,
                model_visible: true,
                route_kind: "explicit_model_route",
                route: Some(&route()),
                channel_states: &HashMap::new(),
                endpoint_capabilities: &HashMap::new(),
                candidate_limit: 16,
            },
        );
        assert_eq!(disabled.reason_code, "token_disabled");
    }

    #[test]
    fn endpoint_family_explain_reports_model_and_family_failures() {
        let clients = vec![client("client-a", true)];
        let route = route();

        let model_missing = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: Some("client-a"),
                endpoint_family: "chat_completions",
                public_model: "missing-model",
                registry_generation: 7,
                model_allowed: true,
                model_visible: false,
                route_kind: "no_route",
                route: None,
                channel_states: &HashMap::new(),
                endpoint_capabilities: &HashMap::new(),
                candidate_limit: 16,
            },
        );
        assert_eq!(model_missing.reason_code, "model_missing");

        let unsupported_family = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: Some("client-a"),
                endpoint_family: "responses",
                public_model: "gpt-public",
                registry_generation: 7,
                model_allowed: true,
                model_visible: true,
                route_kind: "explicit_model_route",
                route: Some(&route),
                channel_states: &HashMap::from([(
                    ChannelId("ch1".to_string()),
                    ChannelRouteState::Available,
                )]),
                endpoint_capabilities: &capabilities(EndpointSupport::Supported),
                candidate_limit: 16,
            },
        );
        assert_eq!(
            unsupported_family.reason_code,
            "endpoint_family_unsupported"
        );

        let family_mismatch = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: Some("client-a"),
                endpoint_family: "embeddings",
                public_model: "gpt-public",
                registry_generation: 7,
                model_allowed: true,
                model_visible: true,
                route_kind: "explicit_model_route",
                route: Some(&route),
                channel_states: &HashMap::from([(
                    ChannelId("ch1".to_string()),
                    ChannelRouteState::Available,
                )]),
                endpoint_capabilities: &capabilities(EndpointSupport::Supported),
                candidate_limit: 16,
            },
        );
        assert_eq!(family_mismatch.reason_code, "endpoint_family_mismatch");
    }

    #[test]
    fn endpoint_family_explain_reports_no_route_and_no_usable_key_or_target() {
        let clients = vec![client("client-a", true)];
        let route = route();

        let no_route = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: Some("client-a"),
                endpoint_family: "chat_completions",
                public_model: "gpt-public",
                registry_generation: 7,
                model_allowed: true,
                model_visible: true,
                route_kind: "no_route",
                route: None,
                channel_states: &HashMap::new(),
                endpoint_capabilities: &HashMap::new(),
                candidate_limit: 16,
            },
        );
        assert_eq!(no_route.reason_code, "no_route");

        let no_usable = endpoint_family_availability_explain_from_parts(
            EndpointFamilyAvailabilityExplainInput {
                client_tokens: &clients,
                client_token_ref: Some("client-a"),
                endpoint_family: "chat_completions",
                public_model: "gpt-public",
                registry_generation: 7,
                model_allowed: true,
                model_visible: true,
                route_kind: "explicit_model_route",
                route: Some(&route),
                channel_states: &HashMap::from([(
                    ChannelId("ch1".to_string()),
                    ChannelRouteState::NoAvailableCredentials,
                )]),
                endpoint_capabilities: &capabilities(EndpointSupport::Supported),
                candidate_limit: 16,
            },
        );
        assert_eq!(no_usable.reason_code, "no_usable_key_or_target");
    }
}
