use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use crate::{
    credential_probe::{
        credential_probe_result_status, credential_probe_summary_status,
        CredentialProbeResultStatus, CredentialProbeSummaryStatus,
    },
    credential_repository::{
        CredentialProbeSummaryRecord, CredentialSetId, CredentialStoreError, CredentialStoreHandle,
        KeyImportReport,
    },
    events::{ManagementAuditEvent, ManagementEventActor},
    management_credential_sources::{
        key_import_status, key_import_status_for_pool, KeyImportStatus,
    },
    management_errors::ManagementServiceError,
    management_resource_lookup::channel_pool_for_id,
    management_status::{
        add_runtime_channel_health_count, channel_health_status, redacted_api_base,
        ChannelHealthStatus, CredentialSetRuntimeSampler, RuntimeChannelHealthCounts,
        RuntimeCredentialCounts,
    },
    pool::KeyPoolSnapshot,
    provider::ProviderKind,
    state::{AppState, ChannelHealth, ChannelTopologyProjection, PoolState},
};

#[derive(Debug, Serialize)]
pub struct PoolsResponse {
    pub pools: Vec<PoolStatus>,
}

#[derive(Debug, Serialize)]
pub struct PoolStatus {
    pub name: String,
    pub configured_enabled: bool,
    pub provider_id: String,
    pub account_id: String,
    pub credential_set_id: String,
    pub provider_kind: ProviderKind,
    pub api_base: String,
    pub auth_header: String,
    pub auth_prefix_configured: bool,
    pub routing_profile_id: String,
    pub retry_switched_key_in_same_request: bool,
    pub max_same_request_retries: usize,
    pub route_target_retry_enabled: bool,
    pub default_credential_cooldown_seconds: u64,
    pub config_generation: u64,
    pub selector_generation: u64,
    pub current_index: usize,
    pub total_credentials: usize,
    pub available_credentials: usize,
    pub cooling_down_credentials: usize,
    pub expired_credentials: usize,
    pub quota_exhausted_credentials: usize,
    pub disabled_credentials: usize,
    pub health: ChannelHealthStatus,
    pub key_import: KeyImportStatus,
}

pub async fn pools_response(state: &AppState) -> PoolsResponse {
    let pools = pool_statuses_from_channel_projection(state).await;
    PoolsResponse { pools }
}

async fn pool_statuses_from_channel_projection(state: &AppState) -> Vec<PoolStatus> {
    let mut statuses = Vec::new();
    for topology in state.runtime_catalogs.channel_topologies() {
        let Some(pool_state) = state.channels.get(&topology.id) else {
            continue;
        };
        statuses.push(pool_status_for_topology(&topology, &pool_state).await);
    }
    statuses
}

pub async fn pool_status_for_topology(
    topology: &ChannelTopologyProjection,
    pool_state: &PoolState,
) -> PoolStatus {
    let snapshot = {
        let pool = pool_state.pool.lock().await;
        pool.snapshot()
    };
    pool_status_from_snapshot(topology, pool_state, snapshot)
}

pub async fn pool_status_for_channel(
    state: &AppState,
    channel_id: &str,
) -> Result<PoolStatus, ManagementServiceError> {
    let pool_state = channel_pool_for_id(state, channel_id)?;
    let topology = channel_topology_for_id(state, channel_id)
        .ok_or_else(|| ManagementServiceError::NotFound(format!("unknown channel {channel_id}")))?;
    Ok(pool_status_for_topology(&topology, &pool_state).await)
}

fn pool_status_from_snapshot(
    topology: &ChannelTopologyProjection,
    pool_state: &PoolState,
    snapshot: KeyPoolSnapshot,
) -> PoolStatus {
    PoolStatus {
        name: topology.id.clone(),
        configured_enabled: topology.configured_enabled,
        provider_id: topology.provider_id.clone(),
        account_id: topology.account_id.clone(),
        credential_set_id: topology.credential_set_id.0.clone(),
        provider_kind: topology.provider_kind,
        api_base: redacted_api_base(&topology.api_base),
        auth_header: topology.auth_header.clone(),
        auth_prefix_configured: topology.auth_prefix_configured,
        routing_profile_id: topology.routing_profile_id.clone(),
        retry_switched_key_in_same_request: topology.retry_switched_key_in_same_request,
        max_same_request_retries: topology.max_same_request_retries,
        route_target_retry_enabled: topology.route_target_retry_enabled,
        default_credential_cooldown_seconds: topology.default_credential_cooldown.as_secs(),
        config_generation: topology.config_generation,
        selector_generation: pool_state.selector_generation.load(Ordering::Acquire),
        current_index: snapshot.current_index,
        total_credentials: snapshot.total_credentials,
        available_credentials: snapshot.available_credentials,
        cooling_down_credentials: snapshot.cooling_down_credentials,
        expired_credentials: snapshot.expired_credentials,
        quota_exhausted_credentials: snapshot.quota_exhausted_credentials,
        disabled_credentials: snapshot.disabled_credentials,
        health: channel_health_status(pool_state),
        key_import: key_import_status_for_pool(pool_state),
    }
}

fn channel_topology_for_id(
    state: &AppState,
    channel_id: &str,
) -> Option<ChannelTopologyProjection> {
    state
        .runtime_catalogs
        .channel_topologies()
        .into_iter()
        .find(|topology| topology.id == channel_id)
}

pub async fn reset_channel_health_status(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
) -> Result<PoolStatus, ManagementServiceError> {
    let pool_state = channel_pool_for_id(state, channel_id)?;
    {
        let _send_guard = pool_state.send_gate.write().await;
        let _mutation_guard = pool_state.mutation_gate.lock().await;
        let health = pool_state
            .health
            .lock()
            .expect("channel health mutex poisoned");
        if matches!(*health, ChannelHealth::Disabled { .. }) {
            return Err(ManagementServiceError::Conflict(format!(
                "channel {channel_id} is disabled; enable it explicitly"
            )));
        }
    }
    state
        .events
        .record_audit_event(ManagementAuditEvent {
            kind: "channel_health_reset".to_string(),
            action: "channel_health_reset".to_string(),
            resource_type: "channel".to_string(),
            resource_id: channel_id.to_string(),
            channel_id: channel_id.to_string(),
            credential_id: String::new(),
            outcome: "applied".to_string(),
            request_id: None,
            generation: None,
            reason_code: "manual_channel_health_reset".to_string(),
            actor: Some(actor),
        })
        .await
        .map_err(|err| ManagementServiceError::EventAppendFailed {
            message: err.to_string(),
            stale_history_id: None,
        })?;
    let _send_guard = pool_state.send_gate.write().await;
    let _mutation_guard = pool_state.mutation_gate.lock().await;
    {
        let mut health = pool_state
            .health
            .lock()
            .expect("channel health mutex poisoned");
        if matches!(*health, ChannelHealth::Disabled { .. }) {
            return Err(ManagementServiceError::Conflict(format!(
                "channel {channel_id} is disabled; enable it explicitly"
            )));
        }
        *health = ChannelHealth::Available;
    }
    pool_state.reset_relay_suppression_count();
    pool_state
        .channel_health_generation
        .fetch_add(1, Ordering::AcqRel);
    pool_status_for_channel(state, channel_id).await
}

pub async fn disable_channel_resource(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    reason: String,
) -> Result<PoolStatus, ManagementServiceError> {
    let pool_state = channel_pool_for_id(state, channel_id)?;
    let _send_guard = pool_state.send_gate.write().await;
    let _mutation_guard = pool_state.mutation_gate.lock().await;
    let health = pool_state.health.clone();
    let channel_health_generation = pool_state.channel_health_generation.clone();
    let reason_for_state = reason.clone();
    state
        .events
        .record_channel_disabled_transaction(
            actor,
            channel_id.to_string(),
            reason,
            move |pending| {
                pending.append()?;
                *health.lock().expect("channel health mutex poisoned") = ChannelHealth::Disabled {
                    reason: reason_for_state,
                };
                channel_health_generation.fetch_add(1, Ordering::AcqRel);
                Ok(())
            },
        )
        .await
        .map_err(|err| {
            ManagementServiceError::Persistence(format!("failed to record management event: {err}"))
        })?;
    pool_status_for_channel(state, channel_id).await
}

pub async fn enable_channel_resource(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    reason: String,
) -> Result<PoolStatus, ManagementServiceError> {
    let pool_state = channel_pool_for_id(state, channel_id)?;
    let _mutation_guard = pool_state.mutation_gate.lock().await;
    let health = pool_state.health.clone();
    let channel_health_generation = pool_state.channel_health_generation.clone();
    state
        .events
        .record_channel_enabled_transaction(actor, channel_id.to_string(), reason, move |pending| {
            pending.append()?;
            *health.lock().expect("channel health mutex poisoned") = ChannelHealth::Available;
            channel_health_generation.fetch_add(1, Ordering::AcqRel);
            Ok(())
        })
        .await
        .map_err(|err| {
            ManagementServiceError::Persistence(format!("failed to record management event: {err}"))
        })?;
    pool_status_for_channel(state, channel_id).await
}

#[derive(Debug, Serialize)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderStatus>,
}

#[derive(Debug, Serialize)]
pub struct ProviderStatus {
    pub id: String,
    pub provider_kind: ProviderKind,
    pub enabled: bool,
    pub channels: usize,
    pub channel_health: RuntimeChannelHealthCounts,
    pub accounts: usize,
    pub credentials: RuntimeCredentialCounts,
}

pub async fn providers_response(state: &AppState) -> ProvidersResponse {
    ProvidersResponse {
        providers: provider_statuses_from_runtime_projection(
            provider_runtime_projection_from_channels(state).await,
        ),
    }
}

#[derive(Debug)]
struct ProviderRuntimeProjection {
    id: String,
    provider_kind: ProviderKind,
    enabled: bool,
    channels: usize,
    channel_health: RuntimeChannelHealthCounts,
    account_ids: BTreeSet<String>,
    credentials: RuntimeCredentialCounts,
}

async fn provider_runtime_projection_from_channels(
    state: &AppState,
) -> Vec<ProviderRuntimeProjection> {
    let mut providers = Vec::new();
    let mut runtime_sampler = CredentialSetRuntimeSampler::default();
    for topology in state.runtime_catalogs.provider_topologies() {
        let topology_credential_set_ids: BTreeSet<String> = topology
            .credential_set_ids
            .into_iter()
            .map(|id| id.0)
            .collect();
        let mut accumulator = runtime_sampler.accumulator();
        let mut projection = ProviderRuntimeProjection {
            id: topology.id,
            provider_kind: topology.provider_kind,
            enabled: topology.enabled,
            channels: topology.channel_ids.len(),
            channel_health: RuntimeChannelHealthCounts::default(),
            account_ids: topology.account_ids.into_iter().collect(),
            credentials: RuntimeCredentialCounts::default(),
        };
        for channel_id in topology.channel_ids {
            let Some(pool_state) = state.channels.get(&channel_id) else {
                continue;
            };
            add_runtime_channel_health_count(&mut projection.channel_health, &pool_state);
            accumulator
                .add_pool_counts(&pool_state, &mut projection.credentials)
                .await;
        }
        debug_assert!(accumulator
            .seen_credential_set_ids()
            .is_subset(&topology_credential_set_ids));
        providers.push(projection);
    }
    providers
}

fn provider_statuses_from_runtime_projection(
    projections: Vec<ProviderRuntimeProjection>,
) -> Vec<ProviderStatus> {
    projections
        .into_iter()
        .map(|projection| ProviderStatus {
            id: projection.id,
            provider_kind: projection.provider_kind,
            enabled: projection.enabled,
            channels: projection.channels,
            channel_health: projection.channel_health,
            accounts: projection.account_ids.len(),
            credentials: projection.credentials,
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct AccountsResponse {
    pub accounts: Vec<AccountStatus>,
}

#[derive(Debug, Serialize)]
pub struct AccountStatus {
    pub id: String,
    pub provider_id: String,
    pub channels: usize,
    pub channel_ids: Vec<String>,
    pub credential_set_ids: Vec<String>,
    pub provider_kind: ProviderKind,
    pub enabled: bool,
    pub provider_enabled: bool,
    pub effective_enabled: bool,
    pub channel_health: RuntimeChannelHealthCounts,
    pub credentials: RuntimeCredentialCounts,
    pub credential_sources: Vec<KeyImportStatus>,
}

pub async fn accounts_response(state: &AppState) -> AccountsResponse {
    AccountsResponse {
        accounts: account_statuses_from_runtime_projection(
            account_runtime_projection_from_channels(state).await,
        ),
    }
}

#[derive(Debug)]
struct AccountRuntimeProjection {
    id: String,
    provider_id: String,
    channels: usize,
    channel_ids: Vec<String>,
    credential_set_ids: BTreeSet<String>,
    provider_kind: ProviderKind,
    enabled: bool,
    provider_enabled: bool,
    effective_enabled: bool,
    channel_health: RuntimeChannelHealthCounts,
    credentials: RuntimeCredentialCounts,
    credential_sources: Vec<KeyImportStatus>,
}

async fn account_runtime_projection_from_channels(
    state: &AppState,
) -> Vec<AccountRuntimeProjection> {
    let mut accounts = Vec::new();
    let mut runtime_sampler = CredentialSetRuntimeSampler::default();
    for topology in state.runtime_catalogs.account_topologies() {
        let topology_credential_set_ids: BTreeSet<String> = topology
            .credential_set_ids
            .iter()
            .map(|id| id.0.clone())
            .collect();
        let mut accumulator = runtime_sampler.accumulator();
        let mut projection = AccountRuntimeProjection {
            id: topology.id,
            provider_id: topology.provider_id,
            channels: topology.channel_ids.len(),
            channel_ids: topology.channel_ids.clone(),
            credential_set_ids: topology_credential_set_ids.clone(),
            provider_kind: topology.provider_kind,
            enabled: topology.enabled,
            provider_enabled: topology.provider_enabled,
            effective_enabled: topology.effective_enabled,
            channel_health: RuntimeChannelHealthCounts::default(),
            credentials: RuntimeCredentialCounts::default(),
            credential_sources: Vec::new(),
        };
        for channel_id in topology.channel_ids {
            let Some(pool_state) = state.channels.get(&channel_id) else {
                continue;
            };
            add_runtime_channel_health_count(&mut projection.channel_health, &pool_state);
            if accumulator
                .add_pool_counts(&pool_state, &mut projection.credentials)
                .await
            {
                projection
                    .credential_sources
                    .push(key_import_status_for_pool(&pool_state));
            }
        }
        debug_assert!(accumulator
            .seen_credential_set_ids()
            .is_subset(&topology_credential_set_ids));
        accounts.push(projection);
    }
    accounts
}

fn account_statuses_from_runtime_projection(
    projections: Vec<AccountRuntimeProjection>,
) -> Vec<AccountStatus> {
    projections
        .into_iter()
        .map(|mut projection| {
            projection.channel_ids.sort();
            AccountStatus {
                id: projection.id,
                provider_id: projection.provider_id,
                channels: projection.channels,
                channel_ids: projection.channel_ids,
                credential_set_ids: projection.credential_set_ids.into_iter().collect(),
                provider_kind: projection.provider_kind,
                enabled: projection.enabled,
                provider_enabled: projection.provider_enabled,
                effective_enabled: projection.effective_enabled,
                channel_health: projection.channel_health,
                credentials: projection.credentials,
                credential_sources: projection.credential_sources,
            }
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct CredentialSetsResponse {
    pub credential_sets: Vec<CredentialSetStatus>,
}

#[derive(Debug, Serialize)]
pub struct CredentialSetStatus {
    pub id: String,
    pub channels: usize,
    pub channel_ids: Vec<String>,
    pub account_ids: Vec<String>,
    pub provider_ids: Vec<String>,
    pub credentials: RuntimeCredentialCounts,
    pub key_import: KeyImportStatus,
    pub latest_probe: Option<CredentialProbeResultStatus>,
    pub probe_summary: CredentialProbeSummaryStatus,
}

pub async fn credential_sets_response(
    state: &AppState,
    credential_store: &CredentialStoreHandle,
) -> CredentialSetsResponse {
    let mut projected_sets = credential_set_statuses_from_runtime_projection(
        credential_set_runtime_projection_from_channels(state).await,
    );
    enrich_credential_set_store_statuses(credential_store, &mut projected_sets).await;
    projected_sets.sort_by(|a, b| a.id.cmp(&b.id));
    CredentialSetsResponse {
        credential_sets: projected_sets,
    }
}

async fn enrich_credential_set_store_statuses(
    credential_store: &CredentialStoreHandle,
    projected_sets: &mut [CredentialSetStatus],
) {
    for set in projected_sets {
        set.latest_probe = credential_set_latest_probe_status(credential_store, &set.id).await;
        set.probe_summary = credential_set_probe_summary_status_from_store(
            credential_store,
            &set.id,
            set.credentials.total,
        )
        .await;
    }
}

#[derive(Debug)]
struct CredentialSetRuntimeProjection {
    id: String,
    channels: usize,
    channel_ids: Vec<String>,
    account_ids: Vec<String>,
    provider_ids: Vec<String>,
    credentials: RuntimeCredentialCounts,
    key_import: KeyImportStatus,
}

async fn credential_set_runtime_projection_from_channels(
    state: &AppState,
) -> Vec<CredentialSetRuntimeProjection> {
    let mut credential_sets = Vec::new();
    let mut runtime_sampler = CredentialSetRuntimeSampler::default();
    for topology in state.runtime_catalogs.credential_set_topologies() {
        let mut accumulator = runtime_sampler.accumulator();
        let mut projection = CredentialSetRuntimeProjection {
            id: topology.id.0,
            channels: topology.channel_ids.len(),
            channel_ids: topology.channel_ids.clone(),
            account_ids: topology.account_ids,
            provider_ids: topology.provider_ids,
            credentials: RuntimeCredentialCounts::default(),
            key_import: empty_key_import_status(),
        };
        for channel_id in topology.channel_ids {
            let Some(pool_state) = state.channels.get(&channel_id) else {
                continue;
            };
            if accumulator
                .add_pool_counts(&pool_state, &mut projection.credentials)
                .await
            {
                projection.key_import = key_import_status_for_pool(&pool_state);
            }
        }
        credential_sets.push(projection);
    }
    credential_sets
}

fn empty_key_import_status() -> KeyImportStatus {
    key_import_status(&KeyImportReport {
        source_path: std::path::PathBuf::new(),
        physical_line_count: 0,
        non_empty_count: 0,
        unique_count: 0,
        duplicate_occurrence_count: 0,
        ignored_empty_count: 0,
        invalid_line_count: 0,
        claimed_count: None,
        claim_source: None,
        import_generation: 0,
        last_imported_at_unix_seconds: 0,
        duplicate_fingerprints: Vec::new(),
    })
}

fn credential_set_statuses_from_runtime_projection(
    projections: Vec<CredentialSetRuntimeProjection>,
) -> Vec<CredentialSetStatus> {
    projections
        .into_iter()
        .map(|projection| CredentialSetStatus {
            id: projection.id,
            channels: projection.channels,
            channel_ids: projection.channel_ids,
            account_ids: projection.account_ids,
            provider_ids: projection.provider_ids,
            credentials: projection.credentials,
            key_import: projection.key_import,
            latest_probe: None,
            probe_summary: CredentialProbeSummaryStatus::default(),
        })
        .collect()
}

async fn credential_set_latest_probe_status(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
) -> Option<CredentialProbeResultStatus> {
    match credential_store
        .load_latest_probe_result_for_credential_set(CredentialSetId(credential_set_id.to_string()))
        .await
    {
        Ok(probe) => probe.map(credential_probe_result_status),
        Err(CredentialStoreError::NotWritable) => None,
        Err(CredentialStoreError::Persistence(_)) => None,
    }
}

async fn credential_set_probe_summary_status_from_store(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    total_credentials: usize,
) -> CredentialProbeSummaryStatus {
    let summary = match credential_store
        .load_probe_summary_for_credential_set(CredentialSetId(credential_set_id.to_string()))
        .await
    {
        Ok(summary) => summary,
        Err(CredentialStoreError::NotWritable) => CredentialProbeSummaryRecord::default(),
        Err(CredentialStoreError::Persistence(_)) => CredentialProbeSummaryRecord::default(),
    };
    credential_probe_summary_status(total_credentials, summary)
}

pub async fn credential_sets_response_for_state(state: &AppState) -> CredentialSetsResponse {
    credential_sets_response(state, &state.credential_store).await
}
