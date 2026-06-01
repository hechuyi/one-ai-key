use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    config::{ModelRouteConfig, ModelRouteTargetConfig},
    error::{ClassifiedFailure, FailureKind, FailureScope},
    management_errors::ManagementServiceError,
    management_registry::{
        apply_staged_model_route_batch_for_state, existing_staged_registry_mutation_status,
        registry_mutation_status_fields, staged_model_routes_for_state,
        staged_registry_mutation_status, RegistryMutationStatusFields,
    },
    provider::ProviderAdapter,
    route_plan::ModelRoute,
    state::AppState,
    upstream_response::read_limited_body,
};

#[derive(Debug, Serialize)]
pub struct ModelDiscoveryResponse {
    pub channel_id: String,
    pub provider_id: String,
    pub account_id: String,
    pub credential_set_id: String,
    pub credential_id: String,
    pub credential_fingerprint: String,
    pub upstream_status: u16,
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ModelDiscoveryErrorStatus>,
}

#[derive(Debug, Serialize)]
pub struct ModelDiscoveryErrorStatus {
    pub kind: FailureKind,
    pub scope: FailureScope,
    pub retryable: bool,
    pub classifier_id: String,
    pub upstream_code: Option<String>,
    pub upstream_limit_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ModelDiscoverySyncPlanResponse {
    pub channels: Vec<ModelDiscoveryResponse>,
    pub actions: Vec<ModelDiscoverySyncPlanActionStatus>,
}

#[derive(Debug, Serialize)]
pub struct ModelDiscoverySyncPlanActionStatus {
    pub model: String,
    pub channel_id: String,
    pub action: ModelDiscoverySyncPlanActionKind,
    pub existing_target_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDiscoverySyncPlanActionKind {
    WouldCreateRoute,
    WouldAddTarget,
    Unchanged,
}

#[derive(Debug, Serialize)]
pub struct ModelDiscoverySyncApplyResponse {
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
    pub channels: Vec<ModelDiscoveryResponse>,
    pub actions: Vec<ModelDiscoverySyncApplyActionStatus>,
}

#[derive(Debug, Serialize)]
pub struct ModelDiscoverySyncApplyActionStatus {
    pub model: String,
    pub channel_id: String,
    pub action: ModelDiscoverySyncPlanActionKind,
    pub existing_target_count: usize,
    pub applied: bool,
}

pub async fn discover_channel_models(
    state: &AppState,
    channel_id: &str,
) -> Result<ModelDiscoveryResponse, ManagementServiceError> {
    let Some(pool_state) = state.channels.get(channel_id) else {
        return Err(ManagementServiceError::NotFound(format!(
            "unknown channel {channel_id}"
        )));
    };
    let adapter = ProviderAdapter::new(pool_state.provider_kind);
    let Some(catalog_capability) = adapter.model_catalog_capability() else {
        return Err(ManagementServiceError::Conflict(
            "model discovery requires a provider with model catalog capability".to_string(),
        ));
    };
    let selected = {
        let mut pool = pool_state.pool.lock().await;
        pool.select().map_err(|_| {
            ManagementServiceError::Conflict(format!(
                "credential pool for channel {channel_id} has no available credentials"
            ))
        })?
    };
    let url = adapter.upstream_url(&selected.api_base, catalog_capability.target.path(), None);
    let response = adapter
        .apply_upstream_auth_and_headers(
            state.http_client.get(url),
            &axum::http::HeaderMap::new(),
            pool_state.auth_header.as_str(),
            pool_state.auth_prefix.as_str(),
            &selected.key,
        )
        .send()
        .await
        .map_err(|err| ManagementServiceError::Persistence(format!("upstream error: {err}")))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body_limit = if status.is_success() {
        state.max_model_catalog_body_bytes
    } else {
        state.max_error_body_bytes
    };
    let body = read_limited_body(response, body_limit)
        .await
        .map_err(|err| {
            ManagementServiceError::Persistence(format!("upstream body error: {err}"))
        })?;
    if status.is_success() {
        let catalog = match catalog_capability.parse(&body) {
            Ok(catalog) => catalog,
            Err(_) => {
                return Ok(ModelDiscoveryResponse {
                    channel_id: channel_id.to_string(),
                    provider_id: pool_state.provider_id.clone(),
                    account_id: pool_state.account_id.clone(),
                    credential_set_id: pool_state.credential_set_id.0.clone(),
                    credential_id: selected.credential_id.0,
                    credential_fingerprint: selected.credential_fingerprint.0,
                    upstream_status: status.as_u16(),
                    models: Vec::new(),
                    error: Some(malformed_model_catalog_error_status()),
                });
            }
        };
        return Ok(ModelDiscoveryResponse {
            channel_id: channel_id.to_string(),
            provider_id: pool_state.provider_id.clone(),
            account_id: pool_state.account_id.clone(),
            credential_set_id: pool_state.credential_set_id.0.clone(),
            credential_id: selected.credential_id.0,
            credential_fingerprint: selected.credential_fingerprint.0,
            upstream_status: status.as_u16(),
            models: catalog.model_ids,
            error: None,
        });
    }

    let failure = adapter.classify_failure(
        &pool_state.error_classifier,
        status.as_u16(),
        &headers,
        &body,
    );
    Ok(ModelDiscoveryResponse {
        channel_id: channel_id.to_string(),
        provider_id: pool_state.provider_id.clone(),
        account_id: pool_state.account_id.clone(),
        credential_set_id: pool_state.credential_set_id.0.clone(),
        credential_id: selected.credential_id.0,
        credential_fingerprint: selected.credential_fingerprint.0,
        upstream_status: status.as_u16(),
        models: Vec::new(),
        error: Some(model_discovery_error_status(failure)),
    })
}

pub async fn model_discovery_sync_plan(
    state: &AppState,
    channel_ids: Vec<String>,
) -> Result<ModelDiscoverySyncPlanResponse, ManagementServiceError> {
    let channel_ids = normalize_model_discovery_channel_ids(channel_ids)?;
    let (_, staged_model_routes) = staged_model_routes_for_state(state).await?;
    let (channels, actions) =
        model_discovery_sync_actions(state, channel_ids, staged_model_routes).await?;

    Ok(ModelDiscoverySyncPlanResponse { channels, actions })
}

pub async fn model_discovery_sync_apply(
    state: &AppState,
    channel_ids: Vec<String>,
) -> Result<ModelDiscoverySyncApplyResponse, ManagementServiceError> {
    let (expected_registry_version, staged_model_routes) =
        staged_model_routes_for_state(state).await?;
    let mut channels = Vec::new();
    for channel_id in normalize_model_discovery_channel_ids(channel_ids)? {
        channels.push(discover_channel_models(state, &channel_id).await?);
    }

    let changes = staged_registry_model_discovery_sync_changes(staged_model_routes, &channels)
        .map_err(ManagementServiceError::Conflict)?;

    if changes.routes.is_empty() {
        let active_registry_version = *state
            .active_registry_version
            .read()
            .expect("active registry version lock poisoned");
        return Ok(ModelDiscoverySyncApplyResponse {
            status: registry_mutation_status_fields(existing_staged_registry_mutation_status(
                expected_registry_version,
                active_registry_version,
                state.channels.registry_generation(),
            )),
            channels,
            actions: changes.actions,
        });
    }

    let commit =
        apply_staged_model_route_batch_for_state(state, expected_registry_version, changes.routes)
            .await?;

    Ok(ModelDiscoverySyncApplyResponse {
        status: registry_mutation_status_fields(staged_registry_mutation_status(
            commit.registry_version,
            state.channels.registry_generation(),
        )),
        channels,
        actions: changes.actions,
    })
}

struct StagedModelDiscoverySyncChanges {
    routes: Vec<(String, ModelRouteConfig)>,
    actions: Vec<ModelDiscoverySyncApplyActionStatus>,
}

fn staged_registry_model_discovery_sync_changes(
    mut routes_by_model: HashMap<String, ModelRouteConfig>,
    discoveries: &[ModelDiscoveryResponse],
) -> Result<StagedModelDiscoverySyncChanges, String> {
    let mut changed_routes = BTreeMap::new();
    let mut actions = Vec::new();
    for discovery in discoveries {
        if discovery.error.is_some() {
            continue;
        }
        for model in &discovery.models {
            let (action, existing_target_count) =
                model_discovery_sync_plan_action(routes_by_model.get(model), &discovery.channel_id);
            let applied = apply_discovered_model_route_change(
                &mut routes_by_model,
                &mut changed_routes,
                model,
                &discovery.channel_id,
                action,
            )?;
            actions.push(ModelDiscoverySyncApplyActionStatus {
                model: model.clone(),
                channel_id: discovery.channel_id.clone(),
                action,
                existing_target_count,
                applied,
            });
        }
    }
    sort_discovery_actions(&mut actions);
    Ok(StagedModelDiscoverySyncChanges {
        routes: changed_routes.into_iter().collect(),
        actions,
    })
}

fn staged_registry_model_discovery_plan_actions(
    routes_by_model: HashMap<String, ModelRouteConfig>,
    discoveries: &[ModelDiscoveryResponse],
) -> Vec<ModelDiscoverySyncPlanActionStatus> {
    let mut actions = Vec::new();
    for discovery in discoveries {
        if discovery.error.is_some() {
            continue;
        }
        for model in &discovery.models {
            let (action, existing_target_count) =
                model_discovery_sync_plan_action(routes_by_model.get(model), &discovery.channel_id);
            actions.push(ModelDiscoverySyncPlanActionStatus {
                model: model.clone(),
                channel_id: discovery.channel_id.clone(),
                action,
                existing_target_count,
            });
        }
    }
    sort_discovery_actions(&mut actions);
    actions
}

async fn model_discovery_sync_actions(
    state: &AppState,
    channel_ids: Vec<String>,
    staged_model_routes: HashMap<String, ModelRouteConfig>,
) -> Result<
    (
        Vec<ModelDiscoveryResponse>,
        Vec<ModelDiscoverySyncPlanActionStatus>,
    ),
    ManagementServiceError,
> {
    let mut channels = Vec::new();
    for channel_id in channel_ids {
        let discovery = discover_channel_models(state, &channel_id).await?;
        channels.push(discovery);
    }
    let actions = staged_registry_model_discovery_plan_actions(staged_model_routes, &channels);
    Ok((channels, actions))
}

fn sort_discovery_actions<T: DiscoveryActionSortKey>(actions: &mut [T]) {
    actions.sort_by(|a, b| {
        a.model()
            .cmp(b.model())
            .then_with(|| a.channel_id().cmp(b.channel_id()))
    });
}

trait DiscoveryActionSortKey {
    fn model(&self) -> &str;
    fn channel_id(&self) -> &str;
}

impl DiscoveryActionSortKey for ModelDiscoverySyncPlanActionStatus {
    fn model(&self) -> &str {
        &self.model
    }

    fn channel_id(&self) -> &str {
        &self.channel_id
    }
}

impl DiscoveryActionSortKey for ModelDiscoverySyncApplyActionStatus {
    fn model(&self) -> &str {
        &self.model
    }

    fn channel_id(&self) -> &str {
        &self.channel_id
    }
}

pub fn normalize_model_discovery_channel_ids(
    channel_ids: Vec<String>,
) -> Result<Vec<String>, ManagementServiceError> {
    let mut normalized_channel_ids = Vec::new();
    let mut seen_channel_ids = HashSet::new();
    for channel_id in channel_ids {
        let channel_id = channel_id.trim();
        if channel_id.is_empty() {
            continue;
        }
        if seen_channel_ids.insert(channel_id.to_string()) {
            normalized_channel_ids.push(channel_id.to_string());
        }
    }
    if normalized_channel_ids.is_empty() {
        return Err(ManagementServiceError::Conflict(
            "model discovery sync plan requires at least one channel".to_string(),
        ));
    }
    Ok(normalized_channel_ids)
}

pub fn model_discovery_error_status(failure: ClassifiedFailure) -> ModelDiscoveryErrorStatus {
    ModelDiscoveryErrorStatus {
        kind: failure.kind,
        scope: failure.primary_scope,
        retryable: failure.retryable,
        classifier_id: failure.classifier_id,
        upstream_code: failure.upstream_code,
        upstream_limit_type: failure.upstream_limit_type,
    }
}

pub fn malformed_model_catalog_error_status() -> ModelDiscoveryErrorStatus {
    ModelDiscoveryErrorStatus {
        kind: FailureKind::ClientError,
        scope: FailureScope::ProviderAdapter,
        retryable: false,
        classifier_id: "model_catalog".to_string(),
        upstream_code: Some("catalog_unsupported_or_malformed".to_string()),
        upstream_limit_type: None,
    }
}

pub fn apply_discovered_model_route_change(
    routes_by_model: &mut HashMap<String, ModelRouteConfig>,
    changed_routes: &mut BTreeMap<String, ModelRouteConfig>,
    model: &str,
    channel_id: &str,
    action: ModelDiscoverySyncPlanActionKind,
) -> Result<bool, String> {
    match action {
        ModelDiscoverySyncPlanActionKind::WouldCreateRoute => {
            let route = discovered_model_route(channel_id);
            routes_by_model.insert(model.to_string(), route.clone());
            changed_routes.insert(model.to_string(), route);
            Ok(true)
        }
        ModelDiscoverySyncPlanActionKind::WouldAddTarget => {
            let mut route = routes_by_model.get(model).cloned().ok_or_else(|| {
                format!("route {model} disappeared while applying discovery sync")
            })?;
            append_discovered_route_target(&mut route, channel_id);
            routes_by_model.insert(model.to_string(), route.clone());
            changed_routes.insert(model.to_string(), route);
            Ok(true)
        }
        ModelDiscoverySyncPlanActionKind::Unchanged => Ok(false),
    }
}

pub trait RouteTargetChannelIds {
    fn route_target_count(&self) -> usize;
    fn contains_route_target_channel(&self, channel_id: &str) -> bool;
}

pub fn model_discovery_sync_plan_action<R: RouteTargetChannelIds>(
    route: Option<&R>,
    channel_id: &str,
) -> (ModelDiscoverySyncPlanActionKind, usize) {
    let Some(route) = route else {
        return (ModelDiscoverySyncPlanActionKind::WouldCreateRoute, 0);
    };
    let target_count = route.route_target_count();
    if route.contains_route_target_channel(channel_id) {
        (ModelDiscoverySyncPlanActionKind::Unchanged, target_count)
    } else {
        (
            ModelDiscoverySyncPlanActionKind::WouldAddTarget,
            target_count,
        )
    }
}

fn discovered_model_route(channel_id: &str) -> ModelRouteConfig {
    ModelRouteConfig {
        strategy: Some("priority".to_string()),
        targets: vec![discovered_route_target(channel_id, 10)],
    }
}

fn append_discovered_route_target(route: &mut ModelRouteConfig, channel_id: &str) {
    if route
        .targets
        .iter()
        .any(|target| target.channel == channel_id)
    {
        return;
    }
    let priority = route
        .targets
        .iter()
        .map(|target| target.priority)
        .max()
        .unwrap_or(0)
        .saturating_add(10);
    route
        .targets
        .push(discovered_route_target(channel_id, priority));
}

fn discovered_route_target(channel_id: &str, priority: u16) -> ModelRouteTargetConfig {
    ModelRouteTargetConfig {
        channel: channel_id.to_string(),
        upstream_model: None,
        priority,
        weight: 1,
        enabled: true,
    }
}

impl RouteTargetChannelIds for ModelRoute {
    fn route_target_count(&self) -> usize {
        self.targets.len()
    }

    fn contains_route_target_channel(&self, channel_id: &str) -> bool {
        self.targets
            .iter()
            .any(|target| target.channel_id.0 == channel_id)
    }
}

impl RouteTargetChannelIds for ModelRouteConfig {
    fn route_target_count(&self) -> usize {
        self.targets.len()
    }

    fn contains_route_target_channel(&self, channel_id: &str) -> bool {
        self.targets
            .iter()
            .any(|target| target.channel == channel_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_registry_model_discovery_plan_actions_uses_staged_route_targets() {
        let mut routes_by_model = HashMap::new();
        routes_by_model.insert(
            "gpt-staged".to_string(),
            ModelRouteConfig {
                strategy: Some("priority".to_string()),
                targets: vec![ModelRouteTargetConfig {
                    channel: "channel-b".to_string(),
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                }],
            },
        );
        let discoveries = vec![ModelDiscoveryResponse {
            channel_id: "channel-b".to_string(),
            provider_id: "provider".to_string(),
            account_id: "account".to_string(),
            credential_set_id: "set".to_string(),
            credential_id: "credential".to_string(),
            credential_fingerprint: "fingerprint".to_string(),
            upstream_status: 200,
            models: vec!["gpt-staged".to_string()],
            error: None,
        }];

        let actions = staged_registry_model_discovery_plan_actions(routes_by_model, &discoveries);

        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].action,
            ModelDiscoverySyncPlanActionKind::Unchanged
        );
        assert_eq!(actions[0].existing_target_count, 1);
    }
}
