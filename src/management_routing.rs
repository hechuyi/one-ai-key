use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

use crate::{
    config::ResolvedClientToken,
    endpoint_capabilities::{EndpointCapabilitiesStatus, EndpointSupport},
    events::{RoutingTelemetry, UpstreamFailureTelemetry},
    management_errors::ManagementServiceError,
    management_status::{
        add_key_pool_snapshot_counts, channel_health_status, channel_health_status_from_health,
        ChannelHealthStatus, RuntimeCredentialCounts,
    },
    provider::ProviderKind,
    route_plan::{
        preview_route, route_admission_summary, ChannelRouteState, ModelRoute,
        RouteAdmissionSelectedTarget, RouteAdmissionSummary, RoutePreview, RoutePreviewCandidate,
        RoutePreviewInput, RoutePreviewReason, RouteStrategy, RouteTarget,
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
    pub admission_summary: RoutingPreviewAdmissionSummary,
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
    pub admission_summary: RouteAdmissionSummary,
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
        admission_summary,
        candidates,
    } = input;
    let selected_target =
        routing_preview_selected_target_from_route_summary(&admission_summary.selected_target);
    let admission_summary = RoutingPreviewAdmissionSummary::from_route_summary(&admission_summary);

    RoutingPreviewResponse {
        request_id,
        model,
        route_kind,
        registry_generation,
        candidate_limit,
        policy_summary,
        client_token,
        admission_summary,
        selected_target,
        candidates,
    }
}

fn no_route_admission_summary(
    request_id: String,
    registry_generation: u64,
    model: &str,
    candidate_limit: usize,
) -> RouteAdmissionSummary {
    let channel_states = HashMap::new();
    let preview = preview_route(RoutePreviewInput {
        request_id,
        registry_generation,
        public_model: Some(model.to_string()),
        route: None,
        channel_states: &channel_states,
        allowed_channels: &[],
        candidate_limit,
    });
    route_admission_summary(&preview)
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
            admission_summary: no_route_admission_summary(
                format!("preview:{model}:{}", client.id),
                route_context.registry_generation,
                model,
                state.routing.max_route_candidates,
            ),
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
            admission_summary: no_route_admission_summary(
                format!("preview:{model}:{}", client.id),
                route_context.registry_generation,
                model,
                state.routing.max_route_candidates,
            ),
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
    let admission_summary = route_admission_summary(&preview);
    let candidates = routing_preview_candidate_statuses(&channel_statuses, preview.candidates);
    Ok(routing_preview_response(RoutingPreviewResponseInput {
        request_id,
        model: model.to_string(),
        route_kind: routing_preview_route.route_kind,
        registry_generation: preview.registry_generation,
        candidate_limit: preview.candidate_limit,
        policy_summary,
        client_token: client_status,
        admission_summary,
        candidates,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutingPreviewSelectedTarget {
    pub channel_id: String,
    pub plan_position: usize,
}

fn routing_preview_selected_target_from_route_summary(
    target: &Option<RouteAdmissionSelectedTarget>,
) -> Option<RoutingPreviewSelectedTarget> {
    target.as_ref().map(|target| RoutingPreviewSelectedTarget {
        channel_id: target.channel_id.0.clone(),
        plan_position: target.plan_position,
    })
}

#[derive(Debug, Serialize)]
pub struct RoutingPreviewAdmissionSummary {
    pub status: &'static str,
    pub reason_code: &'static str,
    pub primary_reason_code: &'static str,
    pub selected_target: Option<RoutingPreviewSelectedTarget>,
    pub candidate_count: usize,
    pub included_count: usize,
    pub blocked_count: usize,
    pub soft_suppressed_count: usize,
    pub hard_blocked_count: usize,
    pub last_resort_used: bool,
    pub last_resort_reason: Option<&'static str>,
}

impl RoutingPreviewAdmissionSummary {
    fn from_route_summary(summary: &RouteAdmissionSummary) -> Self {
        Self {
            status: summary.status.as_str(),
            reason_code: summary.reason_code,
            primary_reason_code: summary.primary_reason_code,
            selected_target: routing_preview_selected_target_from_route_summary(
                &summary.selected_target,
            ),
            candidate_count: summary.candidate_count,
            included_count: summary.included_count,
            blocked_count: summary.blocked_count,
            soft_suppressed_count: summary.soft_suppressed_count,
            hard_blocked_count: summary.hard_blocked_count,
            last_resort_used: summary.last_resort_used,
            last_resort_reason: summary.last_resort_reason,
        }
    }
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
    pub can_use: bool,
    pub blocking_domain: &'static str,
    pub reason_code: &'static str,
    pub next_action: &'static str,
    pub endpoint_family: String,
    pub model: String,
    pub public_model: String,
    pub client_token_ref: Option<String>,
    pub route_kind: &'static str,
    pub registry_generation: u64,
    pub reload_drift: EndpointFamilyAvailabilityReloadDrift,
    pub recent_failure_hint: EndpointFamilyAvailabilityRecentFailureHint,
    pub evidence: EndpointFamilyAvailabilityEvidence,
    pub next_step: EndpointFamilyAvailabilityNextStep,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_token: Option<EndpointFamilyAvailabilityClient>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct EndpointFamilyAvailabilityClient {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct EndpointFamilyAvailabilityEvidence {
    pub client_token_ref_supplied: bool,
    pub client_token_known: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_token_enabled: Option<bool>,
    pub model_allowed: bool,
    pub model_visible: bool,
    pub route_present: bool,
    pub route_target_count: usize,
    pub endpoint_family_target_count: usize,
    pub unsupported_target_count: usize,
    pub unknown_or_missing_target_count: usize,
    pub preview_candidate_count: usize,
    pub selected_target_present: bool,
    pub candidate_limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admission_primary_reason_code: Option<&'static str>,
    pub candidate_reason_codes: Vec<&'static str>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct EndpointFamilyAvailabilityReloadDrift {
    pub status: &'static str,
    pub reason_code: &'static str,
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub runtime_reload_required: Option<bool>,
    pub last_reload_at_unix_seconds: Option<u64>,
    pub last_reload_error_reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EndpointFamilyAvailabilityRecentFailureHint {
    pub status: &'static str,
    pub source: &'static str,
    pub window_event_count: usize,
    pub matched_event_count: usize,
    pub reason_codes: Vec<String>,
    pub channel_ids: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct EndpointFamilyAvailabilityNextStep {
    pub summary: String,
    pub template_id: String,
    pub safe_argv: Vec<String>,
    pub side_effect_class: String,
    pub requires_confirmation: bool,
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

    let mut explain =
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
        });
    let staged_registry_version = state.registry_store.current_version().await.ok().flatten();
    explain.reload_drift = endpoint_family_reload_drift_from_state(
        state,
        route_context.registry_generation,
        staged_registry_version,
    );
    explain.recent_failure_hint =
        endpoint_family_recent_failure_hint_from_state(state, public_model, route);
    explain
}

pub fn endpoint_family_availability_explain_from_parts(
    input: EndpointFamilyAvailabilityExplainInput<'_>,
) -> EndpointFamilyAvailabilityExplain {
    let Some(client_token_ref) = input.client_token_ref else {
        let evidence = endpoint_family_availability_evidence(&input, None, None, None);
        return endpoint_family_availability_explain_result(
            input,
            None,
            EndpointFamilyAvailabilityOutcome {
                status: "unavailable",
                can_use: false,
                blocking_domain: "client_token",
                reason_code: "token_missing",
                next_action: "provide_client_token_ref",
            },
            evidence,
        );
    };
    let Some(client) = input
        .client_tokens
        .iter()
        .find(|token| token.id == client_token_ref || token.name == client_token_ref)
    else {
        let evidence = endpoint_family_availability_evidence(&input, None, None, None);
        return endpoint_family_availability_explain_result(
            input,
            None,
            EndpointFamilyAvailabilityOutcome {
                status: "unavailable",
                can_use: false,
                blocking_domain: "client_token",
                reason_code: "token_unknown",
                next_action: "check_client_token_ref",
            },
            evidence,
        );
    };
    let client_status = EndpointFamilyAvailabilityClient {
        id: safe_client_token_id_label(&client.id),
        name: safe_client_token_name_label(&client.name),
        enabled: client.enabled,
    };
    if !client.enabled {
        let evidence = endpoint_family_availability_evidence(&input, Some(client), None, None);
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            EndpointFamilyAvailabilityOutcome {
                status: "unavailable",
                can_use: false,
                blocking_domain: "client_token",
                reason_code: "token_disabled",
                next_action: "enable_client_token",
            },
            evidence,
        );
    }

    let Some(endpoint_family) = EndpointFamily::parse(input.endpoint_family) else {
        let evidence = endpoint_family_availability_evidence(&input, Some(client), None, None);
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            EndpointFamilyAvailabilityOutcome {
                status: "unavailable",
                can_use: false,
                blocking_domain: "endpoint_family",
                reason_code: "unsupported_endpoint_family",
                next_action: "use_supported_endpoint_family",
            },
            evidence,
        );
    };
    if !input.model_allowed || !input.model_visible {
        let evidence = endpoint_family_availability_evidence(&input, Some(client), None, None);
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            EndpointFamilyAvailabilityOutcome {
                status: "unavailable",
                can_use: false,
                blocking_domain: "model",
                reason_code: "model_missing",
                next_action: "publish_or_route_model",
            },
            evidence,
        );
    }
    let Some(route) = input.route else {
        let evidence = endpoint_family_availability_evidence(&input, Some(client), None, None);
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            EndpointFamilyAvailabilityOutcome {
                status: "unavailable",
                can_use: false,
                blocking_domain: "route",
                reason_code: "no_route",
                next_action: "configure_route_or_default_channel",
            },
            evidence,
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
        let evidence = endpoint_family_availability_evidence(
            &input,
            Some(client),
            Some(&family_targets),
            None,
        );
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            EndpointFamilyAvailabilityOutcome {
                status: "unavailable",
                can_use: false,
                blocking_domain: "endpoint_family",
                reason_code,
                next_action: "configure_endpoint_capabilities_or_route",
            },
            evidence,
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
        let evidence = endpoint_family_availability_evidence(
            &input,
            Some(client),
            Some(&family_targets),
            Some(&preview),
        );
        return endpoint_family_availability_explain_result(
            input,
            Some(client_status),
            EndpointFamilyAvailabilityOutcome {
                status: "unavailable",
                can_use: false,
                blocking_domain: "target",
                reason_code: "no_usable_key_or_target",
                next_action: "enable_target_or_key",
            },
            evidence,
        );
    }

    let evidence = endpoint_family_availability_evidence(
        &input,
        Some(client),
        Some(&family_targets),
        Some(&preview),
    );
    endpoint_family_availability_explain_result(
        input,
        Some(client_status),
        EndpointFamilyAvailabilityOutcome {
            status: "available",
            can_use: true,
            blocking_domain: "none",
            reason_code: "available",
            next_action: "none",
        },
        evidence,
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

struct EndpointFamilyAvailabilityOutcome {
    status: &'static str,
    can_use: bool,
    blocking_domain: &'static str,
    reason_code: &'static str,
    next_action: &'static str,
}

fn endpoint_family_availability_explain_result(
    input: EndpointFamilyAvailabilityExplainInput<'_>,
    client_token: Option<EndpointFamilyAvailabilityClient>,
    outcome: EndpointFamilyAvailabilityOutcome,
    evidence: EndpointFamilyAvailabilityEvidence,
) -> EndpointFamilyAvailabilityExplain {
    let model = safe_public_model_label(input.public_model);
    EndpointFamilyAvailabilityExplain {
        status: outcome.status,
        can_use: outcome.can_use,
        blocking_domain: outcome.blocking_domain,
        reason_code: outcome.reason_code,
        next_action: outcome.next_action,
        endpoint_family: input.endpoint_family.to_string(),
        model: model.clone(),
        public_model: model,
        client_token_ref: input.client_token_ref.map(safe_reference_label),
        route_kind: input.route_kind,
        registry_generation: input.registry_generation,
        reload_drift: endpoint_family_unknown_reload_drift(input.registry_generation),
        recent_failure_hint: endpoint_family_no_recent_failure_hint(),
        evidence,
        next_step: endpoint_family_next_step(outcome.reason_code),
        client_token,
    }
}

fn endpoint_family_reload_drift_from_state(
    state: &AppState,
    active_registry_generation: u64,
    staged_registry_version: Option<u64>,
) -> EndpointFamilyAvailabilityReloadDrift {
    let active_registry_version = *state
        .active_registry_version
        .read()
        .expect("active registry version lock poisoned");
    let last_reload = state.runtime_reload_status_snapshot();
    let runtime_reload_required =
        staged_registry_version.map(|staged| Some(staged) != active_registry_version);
    let (status, reason_code) = match runtime_reload_required {
        Some(true) => ("drift", "staged_registry_differs"),
        Some(false) => ("current", "active_registry_matches_staged"),
        None => ("unknown", "staged_registry_version_unavailable"),
    };
    EndpointFamilyAvailabilityReloadDrift {
        status,
        reason_code,
        active_registry_generation,
        active_registry_version,
        staged_registry_version,
        runtime_reload_required,
        last_reload_at_unix_seconds: last_reload.last_reload_at_unix_seconds,
        last_reload_error_reason_code: last_reload
            .last_reload_error_reason_code
            .and_then(safe_reload_reason_code_label),
    }
}

fn endpoint_family_unknown_reload_drift(
    active_registry_generation: u64,
) -> EndpointFamilyAvailabilityReloadDrift {
    EndpointFamilyAvailabilityReloadDrift {
        status: "unknown",
        reason_code: "staged_registry_version_unavailable",
        active_registry_generation,
        active_registry_version: None,
        staged_registry_version: None,
        runtime_reload_required: None,
        last_reload_at_unix_seconds: None,
        last_reload_error_reason_code: None,
    }
}

fn endpoint_family_no_recent_failure_hint() -> EndpointFamilyAvailabilityRecentFailureHint {
    EndpointFamilyAvailabilityRecentFailureHint {
        status: "none",
        source: "routing_telemetry_bounded_window",
        window_event_count: 0,
        matched_event_count: 0,
        reason_codes: Vec::new(),
        channel_ids: Vec::new(),
    }
}

fn endpoint_family_recent_failure_hint_from_state(
    state: &AppState,
    public_model: &str,
    route: Option<&ModelRoute>,
) -> EndpointFamilyAvailabilityRecentFailureHint {
    let snapshot = state
        .routing_telemetry
        .lock()
        .expect("routing telemetry mutex poisoned")
        .snapshot();
    endpoint_family_recent_failure_hint(&snapshot, public_model, route)
}

fn endpoint_family_recent_failure_hint(
    snapshot: &[RoutingTelemetry],
    public_model: &str,
    route: Option<&ModelRoute>,
) -> EndpointFamilyAvailabilityRecentFailureHint {
    let window_event_count = snapshot.len();
    let route_channel_ids = route
        .map(|route| {
            route
                .targets
                .iter()
                .map(|target| target.channel_id.0.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut matched_event_count = 0usize;
    let mut reason_codes = Vec::new();
    let mut channel_ids = Vec::new();
    for event in snapshot.iter().rev() {
        let RoutingTelemetry::UpstreamFailureObserved {
            channel_id,
            failure,
            ..
        } = event
        else {
            continue;
        };
        if failure.public_model.as_deref() != Some(public_model) {
            continue;
        }
        if !route_channel_ids.is_empty()
            && !route_channel_ids
                .iter()
                .any(|route_channel_id| route_channel_id == channel_id)
        {
            continue;
        }
        matched_event_count = matched_event_count.saturating_add(1);
        let reason_code = endpoint_family_failure_reason_code(failure);
        if reason_codes.len() < 8 && !reason_codes.iter().any(|value| value == reason_code) {
            reason_codes.push(reason_code.to_string());
        }
        let channel_label = safe_channel_id_label(channel_id);
        if channel_ids.len() < 8 && !channel_ids.iter().any(|value| value == &channel_label) {
            channel_ids.push(channel_label);
        }
    }
    EndpointFamilyAvailabilityRecentFailureHint {
        status: if matched_event_count == 0 {
            "none"
        } else {
            "present"
        },
        source: "routing_telemetry_bounded_window",
        window_event_count,
        matched_event_count,
        reason_codes,
        channel_ids,
    }
}

fn endpoint_family_failure_reason_code(failure: &UpstreamFailureTelemetry) -> &'static str {
    match failure.failure_kind.as_str() {
        "response_filter_rejected" => "response_filter_rejected",
        "rate_limited" => "upstream_rate_limited",
        "auth_invalid" => "upstream_auth_invalid",
        "quota_exhausted" => "upstream_quota_exhausted",
        "provider_unavailable" => "upstream_provider_unavailable",
        "key_switch_cooldown" => "key_switch_cooldown",
        _ => match failure.status {
            Some(500..=599) => "upstream_5xx",
            Some(408) => "upstream_timeout",
            Some(400..=499) => "upstream_4xx",
            _ => "unknown_failure_class",
        },
    }
}

fn endpoint_family_availability_evidence(
    input: &EndpointFamilyAvailabilityExplainInput<'_>,
    client: Option<&ResolvedClientToken>,
    family_targets: Option<&EndpointFamilyRouteTargets>,
    preview: Option<&RoutePreview>,
) -> EndpointFamilyAvailabilityEvidence {
    EndpointFamilyAvailabilityEvidence {
        client_token_ref_supplied: input.client_token_ref.is_some(),
        client_token_known: client.is_some(),
        client_token_enabled: client.map(|client| client.enabled),
        model_allowed: input.model_allowed,
        model_visible: input.model_visible,
        route_present: input.route.is_some(),
        route_target_count: input
            .route
            .map(|route| route.targets.len())
            .unwrap_or_default(),
        endpoint_family_target_count: family_targets
            .map(|targets| targets.supported_target_count)
            .unwrap_or_default(),
        unsupported_target_count: family_targets
            .map(|targets| targets.unsupported_target_count)
            .unwrap_or_default(),
        unknown_or_missing_target_count: family_targets
            .map(|targets| targets.unknown_or_missing_target_count)
            .unwrap_or_default(),
        preview_candidate_count: preview
            .map(|preview| preview.candidates.len())
            .unwrap_or_default(),
        selected_target_present: preview
            .and_then(|preview| preview.selected_target_index)
            .is_some(),
        candidate_limit: input.candidate_limit,
        admission_primary_reason_code: preview
            .map(route_admission_summary)
            .map(|summary| summary.primary_reason_code),
        candidate_reason_codes: preview
            .map(endpoint_family_candidate_reason_codes)
            .unwrap_or_default(),
    }
}

fn endpoint_family_candidate_reason_codes(preview: &RoutePreview) -> Vec<&'static str> {
    let mut reason_codes = Vec::new();
    for reason in preview
        .candidates
        .iter()
        .flat_map(|candidate| candidate.reasons.iter())
    {
        let reason_code = reason.as_str();
        if !reason_codes.contains(&reason_code) {
            reason_codes.push(reason_code);
        }
        if reason_codes.len() >= 8 {
            break;
        }
    }
    reason_codes
}

fn endpoint_family_next_step(reason_code: &str) -> EndpointFamilyAvailabilityNextStep {
    let contract = crate::diagnostic_contract::contract_for_reason(reason_code)
        .unwrap_or_else(crate::diagnostic_contract::fallback_contract);
    endpoint_family_next_step_from_contract(&contract.next_action)
}

fn endpoint_family_next_step_from_contract(
    next_action: &serde_json::Value,
) -> EndpointFamilyAvailabilityNextStep {
    EndpointFamilyAvailabilityNextStep {
        summary: next_action
            .get("summary")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Inspect the read-only runtime doctor projection.")
            .to_string(),
        template_id: next_action
            .get("template_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("doctor")
            .to_string(),
        safe_argv: next_action
            .get("safe_argv")
            .and_then(serde_json::Value::as_array)
            .map(|argv| {
                argv.iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        side_effect_class: next_action
            .get("side_effect_class")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("runtime_readonly")
            .to_string(),
        requires_confirmation: next_action
            .get("requires_confirmation")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    }
}

fn safe_reference_label(reference: &str) -> String {
    safe_reference_label_value(reference).unwrap_or_else(|| "<redacted-reference>".to_string())
}

fn safe_reference_label_value(reference: &str) -> Option<String> {
    if reference.chars().any(char::is_control) {
        return None;
    }
    let trimmed = reference.trim();
    if is_safe_reference_label(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn is_safe_reference_label(reference: &str) -> bool {
    !reference.is_empty()
        && reference.len() <= 128
        && reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !reference.to_ascii_lowercase().contains("secret")
        && !reference.to_ascii_lowercase().contains("authorization")
        && !reference.to_ascii_lowercase().contains("api_key")
        && !reference.to_ascii_lowercase().contains("apikey")
        && !reference.to_ascii_lowercase().contains("bearer")
        && !reference.to_ascii_lowercase().contains("sk-")
        && !reference.to_ascii_lowercase().contains("sk_")
}

fn safe_client_token_id_label(id: &str) -> String {
    safe_reference_label_value(id).unwrap_or_else(|| "<redacted-client-token-id>".to_string())
}

fn safe_client_token_name_label(name: &str) -> String {
    safe_reference_label_value(name).unwrap_or_else(|| "<redacted-client-token-name>".to_string())
}

fn safe_channel_id_label(channel_id: &str) -> String {
    safe_reference_label_value(channel_id).unwrap_or_else(|| "<redacted-channel-id>".to_string())
}

fn safe_reload_reason_code_label(reason_code: String) -> Option<String> {
    if is_safe_reference_label(&reason_code) {
        Some(reason_code)
    } else {
        None
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
    supported_target_count: usize,
    unsupported_target_count: usize,
    unknown_or_missing_target_count: usize,
}

fn route_with_endpoint_family_targets(
    route: &ModelRoute,
    endpoint_family: EndpointFamily,
    endpoint_capabilities: &HashMap<String, EndpointCapabilitiesStatus>,
) -> EndpointFamilyRouteTargets {
    let mut has_unsupported_target = false;
    let mut unsupported_target_count = 0;
    let mut unknown_or_missing_target_count = 0;
    let mut targets = Vec::new();
    for target in &route.targets {
        let Some(capabilities) = endpoint_capabilities.get(&target.channel_id.0) else {
            unknown_or_missing_target_count += 1;
            continue;
        };
        match endpoint_family.support(capabilities) {
            EndpointSupport::Supported => targets.push(target.clone()),
            EndpointSupport::Unsupported => {
                has_unsupported_target = true;
                unsupported_target_count += 1;
            }
            EndpointSupport::Unknown => {
                unknown_or_missing_target_count += 1;
            }
        }
    }
    let supported_target_count = targets.len();
    if targets.is_empty() {
        return EndpointFamilyRouteTargets {
            route: None,
            has_unsupported_target,
            supported_target_count,
            unsupported_target_count,
            unknown_or_missing_target_count,
        };
    }
    EndpointFamilyRouteTargets {
        route: Some(ModelRoute {
            public_model: route.public_model.clone(),
            strategy: route.strategy,
            targets,
        }),
        has_unsupported_target,
        supported_target_count,
        unsupported_target_count,
        unknown_or_missing_target_count,
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
    use crate::route_plan::RouteAdmissionStatus;
    use crate::{
        config::{
            AccountConfig, ClientTokenConfig, CredentialSetConfig, ErrorRulesConfig,
            KeySelectionStrategyConfig, ManagementConfig, ModelRouteConfig, ModelRouteTargetConfig,
            PoolConfig, ProviderConfig, RouteTargetRetryConfig, RoutingProfileConfig,
            SameRequestCredentialRetryConfig,
        },
        endpoint_capabilities::{EndpointCapabilitiesConfig, EndpointSupport},
        provider::ProviderKind,
        registry::RegistryDocument,
        registry_store::{ProviderRegistryCommand, RegistryCommand, RegistryStoreHandle},
    };
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEMP_SQLITE_COUNTER: AtomicU64 = AtomicU64::new(0);

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

    fn temp_sqlite_registry_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let sequence = TEMP_SQLITE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        std::env::temp_dir().join(format!(
            "{name}-{}-{suffix}-{sequence}.sqlite",
            std::process::id()
        ))
    }

    fn availability_registry_document(keys_file: PathBuf) -> RegistryDocument {
        RegistryDocument {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "client-a".to_string(),
                token: "test-client-token".to_string(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: "test-admin-token".to_string(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: Default::default(),
            routing: Default::default(),
            response_filter: Default::default(),
            default_pool: Some("ch1".to_string()),
            providers: HashMap::from([(
                "openai".to_string(),
                ProviderConfig {
                    provider_kind: ProviderKind::OpenAiCompatible,
                    enabled: true,
                },
            )]),
            accounts: HashMap::from([(
                "primary".to_string(),
                AccountConfig {
                    provider: "openai".to_string(),
                    api_base: "https://relay.example.test/v1".to_string(),
                    auth_header: "Authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    enabled: true,
                },
            )]),
            credential_sets: HashMap::from([(
                "primary-keys".to_string(),
                CredentialSetConfig { keys_file },
            )]),
            model_groups: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: HashMap::from([(
                "default-routing".to_string(),
                RoutingProfileConfig {
                    key_selection: KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry: SameRequestCredentialRetryConfig {
                        enabled: false,
                        max_retries: 0,
                    },
                    route_target_retry: RouteTargetRetryConfig { enabled: true },
                },
            )]),
            model_routes: HashMap::from([(
                "gpt-public".to_string(),
                ModelRouteConfig {
                    strategy: Some("priority".to_string()),
                    targets: vec![ModelRouteTargetConfig {
                        channel: "ch1".to_string(),
                        upstream_model: Some("upstream-private".to_string()),
                        priority: 0,
                        weight: 1,
                        enabled: true,
                    }],
                },
            )]),
            pools: HashMap::from([(
                "ch1".to_string(),
                PoolConfig {
                    endpoint_capabilities: EndpointCapabilitiesConfig {
                        chat_completions: Some(EndpointSupport::Supported),
                        responses: Some(EndpointSupport::Unsupported),
                        embeddings: Some(EndpointSupport::Unknown),
                        models: Default::default(),
                        diagnostic_labels: Vec::new(),
                    },
                    enabled: true,
                    account: Some("primary".to_string()),
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: "https://relay.example.test/v1".to_string(),
                    credential_set: "primary-keys".to_string(),
                    auth_header: "Authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            )]),
        }
    }

    async fn availability_state_with_registry_store() -> (AppState, PathBuf) {
        let path = temp_sqlite_registry_path("one-ai-key-availability-registry");
        let keys_path = temp_sqlite_registry_path("one-ai-key-availability-keys");
        fs::write(&keys_path, "k1\n").unwrap();
        let document = availability_registry_document(keys_path);
        crate::registry_store::SqliteRegistryStore::bootstrap_from_document(&path, {
            let mut persisted = document.clone();
            persisted.client_tokens.clear();
            persisted
        })
        .unwrap();
        let state = AppState::new_with_registry_store_and_validation_bootstrap(
            document.clone().resolve().unwrap(),
            RegistryStoreHandle::sqlite(&path).unwrap(),
            Some(document),
        )
        .unwrap();
        (state, path)
    }

    fn candidate_status(
        channel_id: &str,
        included: bool,
        selected: bool,
        plan_position: Option<usize>,
        reasons: Vec<&'static str>,
    ) -> RoutingPreviewCandidateStatus {
        RoutingPreviewCandidateStatus {
            target_index: 0,
            channel_id: channel_id.to_string(),
            upstream_model: None,
            provider_kind: ProviderKind::OpenAiCompatible,
            endpoint_capabilities: None,
            priority: 0,
            weight: 1,
            target_enabled: true,
            included,
            selected,
            plan_position,
            reasons,
            health: ChannelHealthStatus {
                kind: "available",
                reason: None,
                reason_code: None,
                source: Some("runtime"),
                remaining_seconds: None,
                suppression_count: 0,
                generation: 0,
            },
            credential_set_id: "set-a".to_string(),
            selector_generation: 0,
            credentials: RuntimeCredentialCounts::default(),
        }
    }

    fn preview_response_with_candidates(
        candidates: Vec<RoutingPreviewCandidateStatus>,
        admission_summary: RouteAdmissionSummary,
    ) -> serde_json::Value {
        serde_json::to_value(routing_preview_response(RoutingPreviewResponseInput {
            request_id: "preview:gpt-public:client-a".to_string(),
            model: "gpt-public".to_string(),
            route_kind: "explicit_model_route",
            registry_generation: 7,
            candidate_limit: 16,
            policy_summary: RoutingPreviewPolicySummary {
                candidate_limit: 16,
                ..RoutingPreviewPolicySummary::default()
            },
            client_token: routing_preview_client_status(&client("client-a", true)),
            admission_summary,
            candidates,
        }))
        .unwrap()
    }

    fn route_admission_selected_target(
        channel_id: &str,
        plan_position: usize,
    ) -> RouteAdmissionSelectedTarget {
        RouteAdmissionSelectedTarget {
            channel_id: ChannelId(channel_id.to_string()),
            plan_position,
        }
    }

    fn route_admission_summary_for_test(
        status: RouteAdmissionStatus,
        reason_code: &'static str,
        selected_target: Option<RouteAdmissionSelectedTarget>,
        candidate_count: usize,
        included_count: usize,
        soft_suppressed_count: usize,
        hard_blocked_count: usize,
        last_resort_reason: Option<&'static str>,
    ) -> RouteAdmissionSummary {
        RouteAdmissionSummary {
            status,
            reason_code,
            primary_reason_code: reason_code,
            selected_target,
            candidate_count,
            included_count,
            blocked_count: candidate_count.saturating_sub(included_count),
            soft_suppressed_count,
            hard_blocked_count,
            last_resort_used: last_resort_reason.is_some(),
            last_resort_reason,
        }
    }

    #[test]
    fn routing_preview_exports_admission_summary_for_selected_route() {
        let value = preview_response_with_candidates(
            vec![candidate_status("ch1", true, true, Some(0), Vec::new())],
            route_admission_summary_for_test(
                RouteAdmissionStatus::Available,
                "available",
                Some(route_admission_selected_target("ch1", 0)),
                1,
                1,
                0,
                0,
                None,
            ),
        );

        let summary = &value["admission_summary"];
        assert_eq!(summary["status"], "available");
        assert_eq!(summary["reason_code"], "available");
        assert_eq!(summary["primary_reason_code"], "available");
        assert_eq!(summary["selected_target"]["channel_id"], "ch1");
        assert_eq!(summary["selected_target"]["plan_position"], 0);
        assert_eq!(summary["candidate_count"], 1);
        assert_eq!(summary["included_count"], 1);
        assert_eq!(summary["blocked_count"], 0);
        assert_eq!(summary["soft_suppressed_count"], 0);
        assert_eq!(summary["hard_blocked_count"], 0);
        assert_eq!(summary["last_resort_used"], false);
        assert!(summary["last_resort_reason"].is_null());
    }

    #[test]
    fn routing_preview_admission_summary_reports_unavailable_hard_blocker_without_selected_target()
    {
        let value = preview_response_with_candidates(
            vec![candidate_status(
                "cooling",
                false,
                false,
                None,
                vec!["channel_cooling_down"],
            )],
            route_admission_summary_for_test(
                RouteAdmissionStatus::Unavailable,
                "channel_cooling_down",
                None,
                1,
                0,
                0,
                1,
                None,
            ),
        );

        assert!(value["selected_target"].is_null());
        let summary = &value["admission_summary"];
        assert_eq!(summary["status"], "unavailable");
        assert_eq!(summary["reason_code"], "channel_cooling_down");
        assert_eq!(summary["primary_reason_code"], "channel_cooling_down");
        assert!(summary["selected_target"].is_null());
        assert_eq!(summary["candidate_count"], 1);
        assert_eq!(summary["included_count"], 0);
        assert_eq!(summary["blocked_count"], 1);
        assert_eq!(summary["soft_suppressed_count"], 0);
        assert_eq!(summary["hard_blocked_count"], 1);
        assert_eq!(summary["last_resort_used"], false);
        assert!(summary["last_resort_reason"].is_null());
    }

    #[test]
    fn routing_preview_admission_summary_counts_hard_soft_and_last_resort_reasons() {
        let mut candidates = vec![
            candidate_status(
                "target-disabled",
                false,
                false,
                None,
                vec!["target_disabled"],
            ),
            candidate_status(
                "client-scope",
                false,
                false,
                None,
                vec!["client_channel_scope"],
            ),
            candidate_status(
                "channel-disabled",
                false,
                false,
                None,
                vec!["channel_disabled"],
            ),
            candidate_status(
                "cooling-down",
                false,
                false,
                None,
                vec!["channel_cooling_down"],
            ),
            candidate_status(
                "no-credentials",
                false,
                false,
                None,
                vec!["no_available_credentials"],
            ),
            candidate_status(
                "runtime-unavailable",
                false,
                false,
                None,
                vec!["runtime_unavailable"],
            ),
            candidate_status(
                "unknown-channel",
                false,
                false,
                None,
                vec!["unknown_channel"],
            ),
            candidate_status("degraded", false, false, None, vec!["channel_degraded"]),
            candidate_status(
                "provider-cooling",
                false,
                false,
                None,
                vec!["provider_cooling_down"],
            ),
        ];
        candidates.push(candidate_status(
            "last-resort",
            true,
            true,
            Some(0),
            vec!["provider_cooling_down_last_resort"],
        ));
        let value = preview_response_with_candidates(
            candidates,
            route_admission_summary_for_test(
                RouteAdmissionStatus::LastResort,
                "provider_cooling_down_last_resort",
                Some(route_admission_selected_target("last-resort", 0)),
                10,
                1,
                2,
                7,
                Some("provider_cooling_down_last_resort"),
            ),
        );

        let summary = &value["admission_summary"];
        assert_eq!(summary["status"], "last_resort");
        assert_eq!(summary["reason_code"], "provider_cooling_down_last_resort");
        assert_eq!(summary["selected_target"]["channel_id"], "last-resort");
        assert_eq!(summary["candidate_count"], 10);
        assert_eq!(summary["included_count"], 1);
        assert_eq!(summary["blocked_count"], 9);
        assert_eq!(summary["soft_suppressed_count"], 2);
        assert_eq!(summary["hard_blocked_count"], 7);
        assert_eq!(summary["last_resort_used"], true);
        assert_eq!(
            summary["last_resort_reason"],
            "provider_cooling_down_last_resort"
        );
    }

    #[test]
    fn routing_preview_admission_summary_preserves_candidate_reasons() {
        let value = preview_response_with_candidates(
            vec![
                candidate_status(
                    "blocked",
                    false,
                    false,
                    None,
                    vec!["client_channel_scope", "channel_degraded"],
                ),
                candidate_status("selected", true, true, Some(0), Vec::new()),
            ],
            route_admission_summary_for_test(
                RouteAdmissionStatus::Available,
                "available",
                Some(route_admission_selected_target("selected", 0)),
                2,
                1,
                1,
                1,
                None,
            ),
        );

        assert_eq!(value["candidates"][0]["reasons"][0], "client_channel_scope");
        assert_eq!(value["candidates"][0]["reasons"][1], "channel_degraded");
        assert_eq!(
            value["candidates"][1]["reasons"].as_array().unwrap().len(),
            0
        );
        assert_eq!(value["admission_summary"]["hard_blocked_count"], 1);
        assert_eq!(value["admission_summary"]["soft_suppressed_count"], 1);
    }

    #[test]
    fn endpoint_family_availability_reports_reload_drift_for_supported_usable_route() {
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

        let value = serde_json::to_value(&explain).unwrap();
        assert_eq!(value["can_use"], true);
        assert_eq!(value["blocking_domain"], "none");
        assert_eq!(value["model"], "gpt-public");
        assert_eq!(value["client_token_ref"], "client-a");
        assert_eq!(value["evidence"]["route_target_count"], 1);
        assert_eq!(value["evidence"]["endpoint_family_target_count"], 1);
        assert_eq!(value["evidence"]["selected_target_present"], true);
        assert_eq!(value["next_step"]["template_id"], "no_action_required");
        assert_eq!(value["next_step"]["safe_argv"], serde_json::json!([]));
        assert_eq!(value["next_step"]["side_effect_class"], "runtime_readonly");
        assert_eq!(value["next_step"]["requires_confirmation"], false);
        assert_eq!(value["reload_drift"]["status"], "unknown");
        assert_eq!(
            value["reload_drift"]["reason_code"],
            "staged_registry_version_unavailable"
        );
        assert_eq!(value["reload_drift"]["active_registry_generation"], 7);
        assert_eq!(value["recent_failure_hint"]["status"], "none");
        assert_eq!(
            value["recent_failure_hint"]["source"],
            "routing_telemetry_bounded_window"
        );
        assert_eq!(value["recent_failure_hint"]["matched_event_count"], 0);
        assert_eq!(
            value["recent_failure_hint"]["reason_codes"],
            serde_json::json!([])
        );
    }

    #[tokio::test]
    async fn endpoint_family_availability_reload_drift_reports_staged_registry_drift_from_state() {
        let (state, _path) = availability_state_with_registry_store().await;
        state
            .registry_store
            .apply_command(
                RegistryCommand::Provider(ProviderRegistryCommand::SetEnabled {
                    provider_id: "openai".to_string(),
                    enabled: false,
                }),
                |_| Ok(()),
            )
            .await
            .unwrap();

        let explain = endpoint_family_availability_explain(
            &state,
            Some("client-a"),
            "chat_completions",
            "gpt-public",
        )
        .await;

        let drift = serde_json::to_value(explain.reload_drift).unwrap();
        assert_eq!(drift["status"], "drift");
        assert_eq!(drift["reason_code"], "staged_registry_differs");
        assert_eq!(drift["active_registry_version"], 1);
        assert_eq!(drift["staged_registry_version"], 2);
        assert_eq!(drift["runtime_reload_required"], true);
        assert!(drift["active_registry_generation"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn endpoint_family_availability_reload_drift_reports_current_from_state() {
        let (state, _path) = availability_state_with_registry_store().await;

        let explain = endpoint_family_availability_explain(
            &state,
            Some("client-a"),
            "chat_completions",
            "gpt-public",
        )
        .await;

        let drift = serde_json::to_value(explain.reload_drift).unwrap();
        assert_eq!(drift["status"], "current");
        assert_eq!(drift["reason_code"], "active_registry_matches_staged");
        assert_eq!(drift["active_registry_version"], 1);
        assert_eq!(drift["staged_registry_version"], 1);
        assert_eq!(drift["runtime_reload_required"], false);
        assert!(drift["active_registry_generation"].as_u64().unwrap() > 0);
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
        let missing_value = serde_json::to_value(&missing).unwrap();
        assert_eq!(missing_value["can_use"], false);
        assert_eq!(missing_value["blocking_domain"], "client_token");
        assert_eq!(missing_value["client_token_ref"], serde_json::Value::Null);
        assert_eq!(missing_value["evidence"]["client_token_known"], false);
        assert_eq!(
            missing_value["next_step"]["template_id"],
            "models_explain_visibility"
        );
        assert_eq!(
            missing_value["next_step"]["side_effect_class"],
            "runtime_readonly"
        );

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
        let unknown_value = serde_json::to_value(&unknown).unwrap();
        assert_eq!(unknown_value["can_use"], false);
        assert_eq!(unknown_value["blocking_domain"], "client_token");
        assert_eq!(unknown_value["client_token_ref"], "unknown");
        assert_eq!(unknown_value["evidence"]["client_token_known"], false);

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
        let disabled_value = serde_json::to_value(&disabled).unwrap();
        assert_eq!(disabled_value["can_use"], false);
        assert_eq!(disabled_value["blocking_domain"], "client_token");
        assert_eq!(disabled_value["client_token_ref"], "disabled");
        assert_eq!(disabled_value["evidence"]["client_token_known"], true);
        assert_eq!(disabled_value["evidence"]["client_token_enabled"], false);
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
        let model_missing_value = serde_json::to_value(&model_missing).unwrap();
        assert_eq!(model_missing_value["can_use"], false);
        assert_eq!(model_missing_value["blocking_domain"], "model");
        assert_eq!(model_missing_value["model"], "missing-model");
        assert_eq!(model_missing_value["evidence"]["model_allowed"], true);
        assert_eq!(model_missing_value["evidence"]["model_visible"], false);

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
        let unsupported_value = serde_json::to_value(&unsupported_family).unwrap();
        assert_eq!(unsupported_value["can_use"], false);
        assert_eq!(unsupported_value["blocking_domain"], "endpoint_family");
        assert_eq!(unsupported_value["evidence"]["unsupported_target_count"], 1);
        assert_eq!(
            unsupported_value["evidence"]["unknown_or_missing_target_count"],
            0
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
        let mismatch_value = serde_json::to_value(&family_mismatch).unwrap();
        assert_eq!(mismatch_value["can_use"], false);
        assert_eq!(mismatch_value["blocking_domain"], "endpoint_family");
        assert_eq!(
            mismatch_value["evidence"]["endpoint_family_target_count"],
            0
        );
        assert_eq!(
            mismatch_value["evidence"]["unknown_or_missing_target_count"],
            1
        );
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
        let no_route_value = serde_json::to_value(&no_route).unwrap();
        assert_eq!(no_route_value["can_use"], false);
        assert_eq!(no_route_value["blocking_domain"], "route");
        assert_eq!(no_route_value["evidence"]["route_present"], false);

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
        let no_usable_value = serde_json::to_value(&no_usable).unwrap();
        assert_eq!(no_usable_value["can_use"], false);
        assert_eq!(no_usable_value["blocking_domain"], "target");
        assert_eq!(
            no_usable_value["evidence"]["candidate_reason_codes"],
            serde_json::json!(["no_available_credentials"])
        );
        assert_eq!(no_usable_value["next_step"]["template_id"], "route_explain");
    }

    #[test]
    fn endpoint_family_explain_uses_only_stage2_safe_readonly_next_steps() {
        let clients = vec![client("disabled", false), client("client-a", true)];

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
        let unknown_value = serde_json::to_value(&unknown).unwrap();
        assert_ne!(
            unknown_value["next_step"]["template_id"],
            "client_tokens_list"
        );
        assert_eq!(unknown_value["next_step"]["template_id"], "doctor");

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
        let model_missing_value = serde_json::to_value(&model_missing).unwrap();
        assert_ne!(
            model_missing_value["next_step"]["template_id"],
            "models_list"
        );
        assert_eq!(
            model_missing_value["next_step"]["template_id"],
            "models_explain"
        );
    }

    #[test]
    fn endpoint_family_reason_codes_align_next_steps_with_diagnostic_contract() {
        let clients = vec![client("disabled", false), client("client-a", true)];
        let route = route();

        let cases = [
            endpoint_family_availability_explain_from_parts(
                EndpointFamilyAvailabilityExplainInput {
                    client_tokens: &clients,
                    client_token_ref: None,
                    endpoint_family: "chat_completions",
                    public_model: "gpt-public",
                    registry_generation: 7,
                    model_allowed: true,
                    model_visible: true,
                    route_kind: "explicit_model_route",
                    route: Some(&route),
                    channel_states: &HashMap::new(),
                    endpoint_capabilities: &HashMap::new(),
                    candidate_limit: 16,
                },
            ),
            endpoint_family_availability_explain_from_parts(
                EndpointFamilyAvailabilityExplainInput {
                    client_tokens: &clients,
                    client_token_ref: Some("unknown"),
                    endpoint_family: "chat_completions",
                    public_model: "gpt-public",
                    registry_generation: 7,
                    model_allowed: true,
                    model_visible: true,
                    route_kind: "explicit_model_route",
                    route: Some(&route),
                    channel_states: &HashMap::new(),
                    endpoint_capabilities: &HashMap::new(),
                    candidate_limit: 16,
                },
            ),
            endpoint_family_availability_explain_from_parts(
                EndpointFamilyAvailabilityExplainInput {
                    client_tokens: &clients,
                    client_token_ref: Some("disabled"),
                    endpoint_family: "chat_completions",
                    public_model: "gpt-public",
                    registry_generation: 7,
                    model_allowed: true,
                    model_visible: true,
                    route_kind: "explicit_model_route",
                    route: Some(&route),
                    channel_states: &HashMap::new(),
                    endpoint_capabilities: &HashMap::new(),
                    candidate_limit: 16,
                },
            ),
            endpoint_family_availability_explain_from_parts(
                EndpointFamilyAvailabilityExplainInput {
                    client_tokens: &clients,
                    client_token_ref: Some("client-a"),
                    endpoint_family: "invalid_family",
                    public_model: "gpt-public",
                    registry_generation: 7,
                    model_allowed: true,
                    model_visible: true,
                    route_kind: "explicit_model_route",
                    route: Some(&route),
                    channel_states: &HashMap::new(),
                    endpoint_capabilities: &HashMap::new(),
                    candidate_limit: 16,
                },
            ),
            endpoint_family_availability_explain_from_parts(
                EndpointFamilyAvailabilityExplainInput {
                    client_tokens: &clients,
                    client_token_ref: Some("client-a"),
                    endpoint_family: "chat_completions",
                    public_model: "missing-model",
                    registry_generation: 7,
                    model_allowed: true,
                    model_visible: false,
                    route_kind: "explicit_model_route",
                    route: Some(&route),
                    channel_states: &HashMap::new(),
                    endpoint_capabilities: &HashMap::new(),
                    candidate_limit: 16,
                },
            ),
            endpoint_family_availability_explain_from_parts(
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
            ),
            endpoint_family_availability_explain_from_parts(
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
            ),
            endpoint_family_availability_explain_from_parts(
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
            ),
            endpoint_family_availability_explain_from_parts(
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
            ),
        ];

        for explain in cases {
            let value = serde_json::to_value(&explain).unwrap();
            let contract =
                crate::diagnostic_contract::contract_for_reason(explain.reason_code).unwrap();

            assert_eq!(
                value["blocking_domain"], contract.blocking_domain,
                "{} should use the diagnostic contract blocking domain",
                explain.reason_code
            );
            assert_eq!(
                value["next_step"], contract.next_action,
                "{} should use the diagnostic contract next action",
                explain.reason_code
            );
        }
    }

    #[test]
    fn endpoint_family_availability_recent_failure_hint_is_bounded_and_redacted() {
        let snapshot = vec![
            RoutingTelemetry::RouteSelected {
                request_id: "req-ignored".to_string(),
                registry_generation: 1,
                channel_id: "safe-channel".to_string(),
            },
            RoutingTelemetry::UpstreamFailureObserved {
                request_id: "req-secret".to_string(),
                channel_id: "https://relay.example/private?secret=sk-SHOULD_NOT_RENDER".to_string(),
                failure: Box::new(UpstreamFailureTelemetry {
                    endpoint_family: "chat_completions".to_string(),
                    public_model: Some("gpt-public".to_string()),
                    credential_id_hash: "credential-hash-should-not-render".to_string(),
                    attempt: 0,
                    failure_source: "upstream_transaction".to_string(),
                    failure_kind: "provider_unavailable".to_string(),
                    failure_scope: "channel".to_string(),
                    retryable: true,
                    confidence: "high".to_string(),
                    status: Some(503),
                    classifier_id: "classifier-should-not-render".to_string(),
                    classifier_version: "1".to_string(),
                    adaptation_rule_id: None,
                    retry_after_source: None,
                    cooldown_seconds: None,
                    directive: "return_error".to_string(),
                    denial_reason: Some("upstream body SHOULD_NOT_RENDER".to_string()),
                    duplicate_charge_risk: "unknown".to_string(),
                    effective_deadline_remaining_ms: None,
                    retry_pressure_accounted: true,
                    retry_decision: "return_current_error".to_string(),
                    retry_decision_reason: Some("attempt_limit_reached".to_string()),
                }),
            },
        ];

        let hint = endpoint_family_recent_failure_hint(&snapshot, "gpt-public", None);
        let rendered = serde_json::to_string(&hint).unwrap();

        assert_eq!(hint.status, "present");
        assert_eq!(hint.window_event_count, 2);
        assert_eq!(hint.matched_event_count, 1);
        assert_eq!(hint.reason_codes, vec!["upstream_provider_unavailable"]);
        assert_eq!(hint.channel_ids, vec!["<redacted-channel-id>"]);
        assert!(!rendered.contains("SHOULD_NOT_RENDER"));
        assert!(!rendered.contains("secret="));
        assert!(!rendered.contains("https://relay.example"));
        assert!(!rendered.contains("credential-hash-should-not-render"));
        assert!(!rendered.contains("classifier-should-not-render"));
    }
}
