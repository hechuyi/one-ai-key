use std::collections::BTreeMap;

use serde::Serialize;

use crate::{
    config::{AccountConfig, ModelRouteConfig, PoolConfig, ProviderConfig, RoutingProfileConfig},
    management_errors::{registry_store_error, ManagementServiceError},
    management_runtime::runtime_explain_response,
    registry::RegistryDocument,
    route_plan::ModelRoute,
    state::AppState,
};

const RELOAD_DIFF_BUDGET: usize = 64;

#[derive(Debug, Serialize)]
pub struct RuntimeReloadDiffResponse {
    pub status: &'static str,
    pub reason: &'static str,
    pub reason_code: &'static str,
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub runtime_reload_required: bool,
    pub mutating_reload_sent: bool,
    pub reload_apply_status: &'static str,
    pub next_action: RuntimeReloadDiffNextAction,
    pub budget: RuntimeReloadDiffBudget,
    pub resource_changes: Vec<RuntimeReloadResourceDiff>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReloadDiffNextAction {
    pub summary: &'static str,
    pub template_id: &'static str,
    pub safe_argv: Vec<&'static str>,
    pub side_effect_class: &'static str,
    pub requires_confirmation: bool,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReloadDiffBudget {
    pub max_resource_changes: usize,
    pub total_resource_changes: usize,
    pub omitted_resource_changes: usize,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReloadResourceDiff {
    pub resource_type: &'static str,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<RuntimeReloadResourceChange>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReloadResourceChange {
    pub id: String,
    pub changed_fields: Vec<&'static str>,
}

pub async fn runtime_reload_diff_response_for_state(
    state: &AppState,
) -> Result<RuntimeReloadDiffResponse, ManagementServiceError> {
    let staged_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?;
    let explain = runtime_explain_response(state, staged_registry_version).await;
    let staged_registry_document = state
        .registry_store
        .load_registry_for_validation()
        .await
        .map_err(registry_store_error)?;
    let Some(staged_registry_document) = staged_registry_document else {
        return Ok(unavailable_response(explain));
    };

    let mut resource_changes = runtime_reload_resource_diff(state, &staged_registry_document);
    let total_resource_changes = resource_changes.iter().map(section_change_count).sum();
    let omitted_resource_changes =
        apply_resource_change_budget(&mut resource_changes, RELOAD_DIFF_BUDGET);
    let truncated = omitted_resource_changes > 0;
    let reason_code = if truncated {
        "reload_diff_truncated"
    } else if total_resource_changes == 0 {
        "reload_diff_empty"
    } else {
        "reload_diff_available"
    };
    let reason = if truncated {
        "A typed reload diff is available but was truncated to the configured budget."
    } else if total_resource_changes == 0 {
        "A typed reload diff is available and no resource-level changes were found."
    } else {
        "A typed reload diff is available for the current staged registry document."
    };

    Ok(RuntimeReloadDiffResponse {
        status: "ok",
        reason,
        reason_code,
        active_registry_generation: explain.active_registry_generation,
        active_registry_version: explain.active_registry_version,
        staged_registry_version: explain.staged_registry_version,
        runtime_reload_required: explain.runtime_reload_required,
        mutating_reload_sent: false,
        reload_apply_status: "dry_run_available",
        next_action: RuntimeReloadDiffNextAction {
            summary: "Reload diff is read-only and complete for this request. No further diagnostic action is required.",
            template_id: "no_action_required",
            safe_argv: vec![],
            side_effect_class: "runtime_readonly",
            requires_confirmation: false,
        },
        budget: RuntimeReloadDiffBudget {
            max_resource_changes: RELOAD_DIFF_BUDGET,
            total_resource_changes,
            omitted_resource_changes,
            truncated,
        },
        resource_changes,
    })
}

fn unavailable_response(
    explain: crate::management_runtime::RuntimeExplainResponse,
) -> RuntimeReloadDiffResponse {
    RuntimeReloadDiffResponse {
        status: "unavailable",
        reason: "A typed reload diff projection is not available yet.",
        reason_code: "unavailable_without_staged_projection",
        active_registry_generation: explain.active_registry_generation,
        active_registry_version: explain.active_registry_version,
        staged_registry_version: explain.staged_registry_version,
        runtime_reload_required: explain.runtime_reload_required,
        mutating_reload_sent: false,
        reload_apply_status: "unavailable_without_staged_projection",
        next_action: RuntimeReloadDiffNextAction {
            summary: "Reload diff is read-only and currently unavailable; inspect reload status or prepare explicit staged registry changes.",
            template_id: "reload_diff_unavailable",
            safe_argv: Vec::new(),
            side_effect_class: "runtime_readonly",
            requires_confirmation: false,
        },
        budget: RuntimeReloadDiffBudget {
            max_resource_changes: RELOAD_DIFF_BUDGET,
            total_resource_changes: 0,
            omitted_resource_changes: 0,
            truncated: false,
        },
        resource_changes: Vec::new(),
    }
}

fn runtime_reload_resource_diff(
    state: &AppState,
    staged: &RegistryDocument,
) -> Vec<RuntimeReloadResourceDiff> {
    vec![
        diff_maps(
            "providers",
            active_provider_summaries(state),
            staged_provider_summaries(staged),
            provider_changed_fields,
        ),
        diff_maps(
            "accounts",
            active_account_summaries(state),
            staged_account_summaries(staged),
            account_changed_fields,
        ),
        diff_maps(
            "credential_sets",
            active_credential_set_summaries(state),
            staged_credential_set_summaries(staged),
            credential_set_changed_fields,
        ),
        diff_maps(
            "channels",
            active_channel_summaries(state),
            staged_channel_summaries(staged),
            channel_changed_fields,
        ),
        diff_model_route_maps(
            "model_route",
            active_model_route_summaries(state),
            staged_model_route_summaries(staged),
            model_route_changed_fields,
        ),
        diff_maps(
            "policy_profiles",
            active_policy_profile_summaries(state),
            staged_policy_profile_summaries(staged),
            policy_profile_changed_fields,
        ),
        diff_maps(
            "routing_profiles",
            active_routing_profile_summaries(state),
            staged_routing_profile_summaries(staged),
            routing_profile_changed_fields,
        ),
    ]
}

fn diff_maps<T>(
    resource_type: &'static str,
    active: BTreeMap<String, T>,
    staged: BTreeMap<String, T>,
    changed_fields: fn(&T, &T) -> Vec<&'static str>,
) -> RuntimeReloadResourceDiff {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for id in active.keys().chain(staged.keys()) {
        if active.contains_key(id) && staged.contains_key(id) {
            continue;
        }
        if staged.contains_key(id) {
            added.push(id.clone());
        } else {
            removed.push(id.clone());
        }
    }
    added.sort();
    added.dedup();
    removed.sort();
    removed.dedup();

    for (id, active_summary) in &active {
        let Some(staged_summary) = staged.get(id) else {
            continue;
        };
        let fields = changed_fields(active_summary, staged_summary);
        if !fields.is_empty() {
            changed.push(RuntimeReloadResourceChange {
                id: id.clone(),
                changed_fields: fields,
            });
        }
    }
    changed.sort_by(|left, right| left.id.cmp(&right.id));

    RuntimeReloadResourceDiff {
        resource_type,
        added,
        removed,
        changed,
    }
}

fn diff_model_route_maps<T>(
    resource_type: &'static str,
    active: BTreeMap<String, T>,
    staged: BTreeMap<String, T>,
    changed_fields: fn(&T, &T) -> Vec<&'static str>,
) -> RuntimeReloadResourceDiff {
    let mut diff = diff_maps(resource_type, active, staged, changed_fields);
    redact_resource_diff_ids(&mut diff, "model_route");
    diff
}

fn redact_resource_diff_ids(diff: &mut RuntimeReloadResourceDiff, prefix: &str) {
    let mut ids: Vec<String> = diff
        .added
        .iter()
        .chain(diff.removed.iter())
        .chain(diff.changed.iter().map(|change| &change.id))
        .cloned()
        .collect();
    ids.sort();
    ids.dedup();
    let refs: BTreeMap<String, String> = ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, format!("{prefix}:{index}")))
        .collect();
    for id in &mut diff.added {
        if let Some(redacted) = refs.get(id) {
            *id = redacted.clone();
        }
    }
    for id in &mut diff.removed {
        if let Some(redacted) = refs.get(id) {
            *id = redacted.clone();
        }
    }
    for change in &mut diff.changed {
        if let Some(redacted) = refs.get(&change.id) {
            change.id = redacted.clone();
        }
    }
}

fn section_change_count(section: &RuntimeReloadResourceDiff) -> usize {
    section.added.len() + section.removed.len() + section.changed.len()
}

fn apply_resource_change_budget(
    sections: &mut [RuntimeReloadResourceDiff],
    budget: usize,
) -> usize {
    let mut remaining = budget;
    let mut omitted = 0;
    for section in sections {
        let before = section_change_count(section);
        truncate_vec(&mut section.added, &mut remaining);
        truncate_vec(&mut section.removed, &mut remaining);
        truncate_vec(&mut section.changed, &mut remaining);
        let after = section_change_count(section);
        omitted += before.saturating_sub(after);
    }
    omitted
}

fn truncate_vec<T>(values: &mut Vec<T>, remaining: &mut usize) {
    if *remaining >= values.len() {
        *remaining -= values.len();
        return;
    }
    values.truncate(*remaining);
    *remaining = 0;
}

#[derive(Clone, PartialEq, Eq)]
struct ProviderSummary {
    provider_kind: &'static str,
    enabled: bool,
}

fn active_provider_summaries(state: &AppState) -> BTreeMap<String, ProviderSummary> {
    state
        .runtime_catalogs
        .provider_topologies()
        .into_iter()
        .map(|provider| {
            (
                provider.id,
                ProviderSummary {
                    provider_kind: provider.provider_kind.stable_id_fragment(),
                    enabled: provider.enabled,
                },
            )
        })
        .collect()
}

fn staged_provider_summaries(staged: &RegistryDocument) -> BTreeMap<String, ProviderSummary> {
    staged
        .providers
        .iter()
        .map(|(id, provider)| (format!("provider:{id}"), provider_summary(provider)))
        .collect()
}

fn provider_summary(provider: &ProviderConfig) -> ProviderSummary {
    ProviderSummary {
        provider_kind: provider.provider_kind.stable_id_fragment(),
        enabled: provider.enabled,
    }
}

fn provider_changed_fields(
    active: &ProviderSummary,
    staged: &ProviderSummary,
) -> Vec<&'static str> {
    changed_field_pairs(&[
        (
            "provider_kind",
            active.provider_kind != staged.provider_kind,
        ),
        ("enabled", active.enabled != staged.enabled),
    ])
}

#[derive(Clone, PartialEq, Eq)]
struct AccountSummary {
    provider_id: String,
    enabled: bool,
}

fn active_account_summaries(state: &AppState) -> BTreeMap<String, AccountSummary> {
    state
        .runtime_catalogs
        .account_topologies()
        .into_iter()
        .map(|account| {
            (
                account.id,
                AccountSummary {
                    provider_id: account.provider_id,
                    enabled: account.enabled,
                },
            )
        })
        .collect()
}

fn staged_account_summaries(staged: &RegistryDocument) -> BTreeMap<String, AccountSummary> {
    staged
        .accounts
        .iter()
        .map(|(id, account)| (format!("account:{id}"), account_summary(account)))
        .collect()
}

fn account_summary(account: &AccountConfig) -> AccountSummary {
    AccountSummary {
        provider_id: format!("provider:{}", account.provider),
        enabled: account.enabled,
    }
}

fn account_changed_fields(active: &AccountSummary, staged: &AccountSummary) -> Vec<&'static str> {
    changed_field_pairs(&[
        ("provider_id", active.provider_id != staged.provider_id),
        ("enabled", active.enabled != staged.enabled),
    ])
}

#[derive(Clone, PartialEq, Eq)]
struct CredentialSetSummary;

fn active_credential_set_summaries(state: &AppState) -> BTreeMap<String, CredentialSetSummary> {
    state
        .runtime_catalogs
        .credential_set_topologies()
        .into_iter()
        .map(|credential_set| (credential_set.id.0, CredentialSetSummary))
        .collect()
}

fn staged_credential_set_summaries(
    staged: &RegistryDocument,
) -> BTreeMap<String, CredentialSetSummary> {
    staged
        .credential_sets
        .keys()
        .map(|id| (id.clone(), CredentialSetSummary))
        .collect()
}

fn credential_set_changed_fields(
    _active: &CredentialSetSummary,
    _staged: &CredentialSetSummary,
) -> Vec<&'static str> {
    Vec::new()
}

#[derive(Clone, PartialEq, Eq)]
struct ChannelSummary {
    enabled: bool,
    provider_kind: &'static str,
    endpoint_capabilities: crate::endpoint_capabilities::ResolvedEndpointCapabilities,
    account_id: String,
    credential_set_id: String,
    routing_profile_id: String,
}

fn active_channel_summaries(state: &AppState) -> BTreeMap<String, ChannelSummary> {
    state
        .runtime_catalogs
        .channel_topologies()
        .into_iter()
        .map(|channel| {
            (
                channel.id,
                ChannelSummary {
                    enabled: channel.configured_enabled,
                    provider_kind: channel.provider_kind.stable_id_fragment(),
                    endpoint_capabilities: channel.endpoint_capabilities,
                    account_id: channel.account_id,
                    credential_set_id: channel.credential_set_id.0,
                    routing_profile_id: channel.routing_profile_id,
                },
            )
        })
        .collect()
}

fn staged_channel_summaries(staged: &RegistryDocument) -> BTreeMap<String, ChannelSummary> {
    staged
        .pools
        .iter()
        .map(|(id, pool)| (id.clone(), channel_summary(staged, pool)))
        .collect()
}

fn channel_summary(staged: &RegistryDocument, pool: &PoolConfig) -> ChannelSummary {
    let provider_kind = staged_channel_provider_kind(staged, pool);
    let endpoint_capabilities = pool
        .endpoint_capabilities
        .resolve_with_base(provider_kind.default_endpoint_capabilities())
        .unwrap_or_else(|_| provider_kind.default_endpoint_capabilities());
    ChannelSummary {
        enabled: pool.enabled,
        provider_kind: provider_kind.stable_id_fragment(),
        endpoint_capabilities,
        account_id: pool
            .account
            .as_ref()
            .map(|account| format!("account:{account}"))
            .unwrap_or_default(),
        credential_set_id: pool.credential_set.clone(),
        routing_profile_id: pool
            .routing_profile
            .clone()
            .or_else(|| staged.default_routing_profile.clone())
            .unwrap_or_default(),
    }
}

fn staged_channel_provider_kind(
    staged: &RegistryDocument,
    pool: &PoolConfig,
) -> crate::provider::ProviderKind {
    pool.account
        .as_ref()
        .and_then(|account_id| staged.accounts.get(account_id))
        .and_then(|account| staged.providers.get(&account.provider))
        .map(|provider| provider.provider_kind)
        .unwrap_or(pool.provider_kind)
}

fn channel_changed_fields(active: &ChannelSummary, staged: &ChannelSummary) -> Vec<&'static str> {
    changed_field_pairs(&[
        ("enabled", active.enabled != staged.enabled),
        (
            "provider_kind",
            active.provider_kind != staged.provider_kind,
        ),
        (
            "endpoint_capabilities",
            active.endpoint_capabilities != staged.endpoint_capabilities,
        ),
        ("account_id", active.account_id != staged.account_id),
        (
            "credential_set_id",
            active.credential_set_id != staged.credential_set_id,
        ),
        (
            "routing_profile_id",
            active.routing_profile_id != staged.routing_profile_id,
        ),
    ])
}

#[derive(Clone, PartialEq, Eq)]
struct ModelRouteSummary {
    strategy: String,
    targets: Vec<String>,
}

fn active_model_route_summaries(state: &AppState) -> BTreeMap<String, ModelRouteSummary> {
    state
        .channels
        .model_routes_context()
        .routes
        .into_iter()
        .map(|route| {
            let summary = active_model_route_summary(&route.route);
            (route.route.public_model, summary)
        })
        .collect()
}

fn active_model_route_summary(route: &ModelRoute) -> ModelRouteSummary {
    let mut targets: Vec<String> = route
        .targets
        .iter()
        .map(|target| {
            format!(
                "{}|{}|{}|{}|{}",
                target.channel_id.0,
                target.upstream_model.as_deref().unwrap_or(""),
                target.priority,
                target.weight,
                target.enabled
            )
        })
        .collect();
    targets.sort();
    ModelRouteSummary {
        strategy: route.strategy.as_str().to_string(),
        targets,
    }
}

fn staged_model_route_summaries(staged: &RegistryDocument) -> BTreeMap<String, ModelRouteSummary> {
    staged
        .model_routes
        .iter()
        .map(|(id, route)| (id.clone(), staged_model_route_summary(route)))
        .collect()
}

fn staged_model_route_summary(route: &ModelRouteConfig) -> ModelRouteSummary {
    let mut targets: Vec<String> = route
        .targets
        .iter()
        .map(|target| {
            format!(
                "{}|{}|{}|{}|{}",
                target.channel,
                target.upstream_model.as_deref().unwrap_or(""),
                target.priority,
                target.weight,
                target.enabled
            )
        })
        .collect();
    targets.sort();
    ModelRouteSummary {
        strategy: route
            .strategy
            .clone()
            .unwrap_or_else(|| "priority_weighted_sticky".to_string()),
        targets,
    }
}

fn model_route_changed_fields(
    active: &ModelRouteSummary,
    staged: &ModelRouteSummary,
) -> Vec<&'static str> {
    changed_field_pairs(&[
        ("strategy", active.strategy != staged.strategy),
        ("targets", active.targets != staged.targets),
    ])
}

#[derive(Clone, PartialEq, Eq)]
struct PolicyProfileSummary;

fn active_policy_profile_summaries(state: &AppState) -> BTreeMap<String, PolicyProfileSummary> {
    state
        .runtime_catalogs
        .policy_profiles()
        .into_iter()
        .map(|profile| (profile.id, PolicyProfileSummary))
        .collect()
}

fn staged_policy_profile_summaries(
    staged: &RegistryDocument,
) -> BTreeMap<String, PolicyProfileSummary> {
    staged
        .policy_profiles
        .keys()
        .map(|id| (id.clone(), PolicyProfileSummary))
        .collect()
}

fn policy_profile_changed_fields(
    _active: &PolicyProfileSummary,
    _staged: &PolicyProfileSummary,
) -> Vec<&'static str> {
    Vec::new()
}

#[derive(Clone, PartialEq, Eq)]
struct RoutingProfileSummary {
    key_selection: &'static str,
    default_credential_cooldown_seconds: u64,
    same_request_credential_retry_enabled: bool,
    max_same_request_retries: usize,
    route_target_retry_enabled: bool,
}

fn active_routing_profile_summaries(state: &AppState) -> BTreeMap<String, RoutingProfileSummary> {
    state
        .runtime_catalogs
        .routing_profiles()
        .into_iter()
        .map(|profile| {
            (
                profile.id,
                RoutingProfileSummary {
                    key_selection: "sticky_until_failure",
                    default_credential_cooldown_seconds: profile
                        .default_credential_cooldown
                        .as_secs(),
                    same_request_credential_retry_enabled: profile
                        .same_request_credential_retry_enabled,
                    max_same_request_retries: profile.max_same_request_retries,
                    route_target_retry_enabled: profile.route_target_retry_enabled,
                },
            )
        })
        .collect()
}

fn staged_routing_profile_summaries(
    staged: &RegistryDocument,
) -> BTreeMap<String, RoutingProfileSummary> {
    staged
        .routing_profiles
        .iter()
        .map(|(id, profile)| (id.clone(), routing_profile_summary(profile)))
        .collect()
}

fn routing_profile_summary(profile: &RoutingProfileConfig) -> RoutingProfileSummary {
    RoutingProfileSummary {
        key_selection: "sticky_until_failure",
        default_credential_cooldown_seconds: profile.default_credential_cooldown_seconds,
        same_request_credential_retry_enabled: profile.same_request_credential_retry.enabled,
        max_same_request_retries: profile.same_request_credential_retry.max_retries,
        route_target_retry_enabled: profile.route_target_retry.enabled,
    }
}

fn routing_profile_changed_fields(
    active: &RoutingProfileSummary,
    staged: &RoutingProfileSummary,
) -> Vec<&'static str> {
    changed_field_pairs(&[
        (
            "key_selection",
            active.key_selection != staged.key_selection,
        ),
        (
            "default_credential_cooldown_seconds",
            active.default_credential_cooldown_seconds
                != staged.default_credential_cooldown_seconds,
        ),
        (
            "same_request_credential_retry_enabled",
            active.same_request_credential_retry_enabled
                != staged.same_request_credential_retry_enabled,
        ),
        (
            "max_same_request_retries",
            active.max_same_request_retries != staged.max_same_request_retries,
        ),
        (
            "route_target_retry_enabled",
            active.route_target_retry_enabled != staged.route_target_retry_enabled,
        ),
    ])
}

fn changed_field_pairs(fields: &[(&'static str, bool)]) -> Vec<&'static str> {
    fields
        .iter()
        .filter_map(|(field, changed)| changed.then_some(*field))
        .collect()
}
