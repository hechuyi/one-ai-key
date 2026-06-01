use serde::Serialize;

use crate::{
    management_errors::ManagementServiceError,
    management_resource_lookup::credential_set_runtime_scope,
    management_status::{
        credential_set_operational_alerts, credential_set_operational_state,
        CredentialSetOperationalAlert, CredentialSetRuntimeSampler, RuntimeCredentialCounts,
    },
    state::{AppState, CredentialSetTopologyProjection, PoolState},
};

#[derive(Debug, Serialize)]
pub struct CredentialSetOperationsStatus {
    pub credential_set_id: String,
    pub channels: usize,
    pub channel_ids: Vec<String>,
    pub account_ids: Vec<String>,
    pub provider_ids: Vec<String>,
    pub status: &'static str,
    pub serving_mode: &'static str,
    pub accepting_requests: bool,
    pub needs_operator_input: bool,
    pub required_action: &'static str,
    pub credentials: RuntimeCredentialCounts,
    pub alerts: Vec<CredentialSetOperationalAlert>,
}

pub async fn credential_set_operations_response(
    credential_set_id: &str,
    topology: &CredentialSetTopologyProjection,
    canonical_pool_state: &PoolState,
) -> CredentialSetOperationsStatus {
    let mut runtime_sampler = CredentialSetRuntimeSampler::default();
    let credentials = runtime_sampler
        .sample_pool(canonical_pool_state)
        .await
        .counts;
    let operational_state = credential_set_operational_state(&credentials);
    CredentialSetOperationsStatus {
        credential_set_id: credential_set_id.to_string(),
        channels: topology.channel_ids.len(),
        channel_ids: topology.channel_ids.clone(),
        account_ids: topology.account_ids.clone(),
        provider_ids: topology.provider_ids.clone(),
        status: operational_state.status,
        serving_mode: operational_state.serving_mode,
        accepting_requests: operational_state.accepting_requests,
        needs_operator_input: operational_state.needs_operator_input,
        required_action: operational_state.required_action,
        alerts: credential_set_operational_alerts(&credentials),
        credentials,
    }
}

pub async fn credential_set_operations_for_set(
    state: &AppState,
    credential_set_id: &str,
) -> Result<CredentialSetOperationsStatus, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    let topology = credential_set_topology_for_id(state, credential_set_id).ok_or_else(|| {
        ManagementServiceError::NotFound(format!("unknown credential_set {credential_set_id}"))
    })?;
    Ok(credential_set_operations_response(credential_set_id, &topology, &scope.pool_state).await)
}

pub async fn credential_set_operations_for_all_sets(
    state: &AppState,
) -> Result<Vec<CredentialSetOperationsStatus>, ManagementServiceError> {
    let mut operations_by_credential_set = Vec::new();
    for topology in credential_set_topologies_for_all_sets(state) {
        let scope = credential_set_runtime_scope(state, &topology.id.0)?;
        let operations =
            credential_set_operations_response(&topology.id.0, &topology, &scope.pool_state).await;
        operations_by_credential_set.push(operations);
    }
    Ok(operations_by_credential_set)
}

fn credential_set_topology_for_id(
    state: &AppState,
    credential_set_id: &str,
) -> Option<CredentialSetTopologyProjection> {
    state
        .runtime_catalogs
        .credential_set_topologies()
        .into_iter()
        .find(|topology| topology.id.0 == credential_set_id)
}

fn credential_set_topologies_for_all_sets(
    state: &AppState,
) -> Vec<CredentialSetTopologyProjection> {
    state.runtime_catalogs.credential_set_topologies()
}
