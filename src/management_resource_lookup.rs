use crate::{
    config::{ResolvedErrorPolicySources, ResolvedPolicyProfile, ResolvedRoutingProfile},
    credential_repository::CredentialSetId,
    error::ErrorClassifierSnapshot,
    management_errors::ManagementServiceError,
    state::{AppState, PoolState},
};

pub fn channel_pool_for_id(
    state: &AppState,
    channel_id: &str,
) -> Result<PoolState, ManagementServiceError> {
    state
        .channels
        .get(channel_id)
        .ok_or_else(|| ManagementServiceError::NotFound(format!("unknown channel {channel_id}")))
}

pub fn channel_ids_for_credential_set(
    state: &AppState,
    credential_set_id: &str,
) -> Result<Vec<String>, ManagementServiceError> {
    let channel_ids = state
        .channels
        .channel_ids_for_credential_set(&CredentialSetId(credential_set_id.to_string()));
    if channel_ids.is_empty() {
        Err(ManagementServiceError::NotFound(format!(
            "unknown credential_set {credential_set_id}"
        )))
    } else {
        Ok(channel_ids)
    }
}

pub struct CredentialSetRuntimeScope {
    pub credential_set_id: CredentialSetId,
    pub channel_ids: Vec<String>,
    pub canonical_channel_id: String,
    pub pool_state: PoolState,
}

pub fn credential_set_runtime_scope(
    state: &AppState,
    credential_set_id: &str,
) -> Result<CredentialSetRuntimeScope, ManagementServiceError> {
    let channel_ids = channel_ids_for_credential_set(state, credential_set_id)?;
    let canonical_channel_id = channel_ids[0].clone();
    let pool_state = channel_pool_for_id(state, &canonical_channel_id)?;
    Ok(CredentialSetRuntimeScope {
        credential_set_id: CredentialSetId(credential_set_id.to_string()),
        channel_ids,
        canonical_channel_id,
        pool_state,
    })
}

pub fn ensure_credential_set_exists(
    state: &AppState,
    credential_set_id: &str,
) -> Result<(), ManagementServiceError> {
    channel_ids_for_credential_set(state, credential_set_id).map(|_| ())
}

pub struct ChannelErrorRulesLookup {
    pub snapshot: ErrorClassifierSnapshot,
    pub error_policy_sources: Option<ResolvedErrorPolicySources>,
}

pub fn channel_error_rules_lookup(
    state: &AppState,
    channel_id: &str,
) -> Result<ChannelErrorRulesLookup, ManagementServiceError> {
    let pool_state = channel_pool_for_id(state, channel_id)?;
    Ok(ChannelErrorRulesLookup {
        snapshot: pool_state.error_classifier.snapshot(),
        error_policy_sources: state.runtime_catalogs.channel_error_policy(channel_id),
    })
}

pub fn policy_profiles(state: &AppState) -> Vec<ResolvedPolicyProfile> {
    state.runtime_catalogs.policy_profiles()
}

pub fn policy_profile(state: &AppState, profile_id: &str) -> Option<ResolvedPolicyProfile> {
    state.runtime_catalogs.policy_profile(profile_id)
}

pub fn policy_profile_channel_ids(state: &AppState, profile_id: &str) -> Vec<String> {
    state
        .runtime_catalogs
        .policy_profile_channel_ids(profile_id)
}

pub fn routing_profiles(state: &AppState) -> Vec<ResolvedRoutingProfile> {
    state.runtime_catalogs.routing_profiles()
}

pub fn routing_profile(state: &AppState, profile_id: &str) -> Option<ResolvedRoutingProfile> {
    state.runtime_catalogs.routing_profile(profile_id)
}

pub fn routing_profile_channel_ids(state: &AppState, profile_id: &str) -> Vec<String> {
    state
        .runtime_catalogs
        .routing_profile_channel_ids(profile_id)
}
