use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::Ordering,
    time::Instant,
};

use crate::credential_probe::{
    credential_probe_result_status, execute_credential_probe, probe_apply_action,
    probe_filter_matches, probe_result_is_default_key_switch_cooldown,
    CredentialProbeApplyActionStatus, CredentialProbeCommand, CredentialProbeFilter,
    CredentialProbeResultStatus, ExecuteCredentialProbeContext,
};
use crate::credential_repository::KeyImport;
use crate::credential_repository::{
    CredentialIdPage, CredentialImportBatchRecord, CredentialImportSourceKind,
    CredentialLifecycleHistoryRecord, CredentialLifecycleHistorySource, CredentialLifecycleState,
    CredentialProbeResultRecordInput, CredentialResourceRecord, CredentialSetId,
    CredentialStoreError, CredentialStoreHandle, KeyImportReport,
};
use crate::credentials::{CredentialId, CredentialSnapshot, CredentialStateSnapshot};
use crate::events::{ManagementAuditEvent, ManagementEventActor};
use crate::management_commands::{
    credential_mutation_response, credential_set_probe_apply_command,
    execute_credential_command_for_state, CredentialMutationResponse,
};
use crate::management_credential_refs::credential_ref_for_position;
use crate::management_credential_sources::{key_import_source_id_for_pool, source_id};
use crate::management_errors::{
    credential_resource_store_error, credential_store_error, ManagementServiceError,
};
use crate::management_operations::{
    credential_set_operations_for_set, CredentialSetOperationsStatus,
};
use crate::management_resource_lookup::{
    channel_pool_for_id, credential_set_runtime_scope, ensure_credential_set_exists,
};
use crate::management_status::redact_management_reason;
use crate::pool::{CredentialSnapshotFilter, PoolCredentialInput, SelectedKey};
use crate::state::{AppState, PoolState};

const CREDENTIAL_PROBE_FILTER_SCAN_PAGE_SIZE: usize = 256;
const BULK_PROBE_APPLY_CANDIDATE_OFFSET: usize = 0;

async fn record_credential_management_audit(
    state: &AppState,
    actor: ManagementEventActor,
    kind: &'static str,
    resource_type: &'static str,
    resource_id: impl Into<String>,
    reason_code: &'static str,
) -> Result<(), ManagementServiceError> {
    state
        .events
        .record_audit_event(ManagementAuditEvent::applied(
            kind,
            resource_type,
            resource_id,
            reason_code,
            Some(actor),
        ))
        .await
        .map_err(|err| ManagementServiceError::EventAppendFailed {
            message: err.to_string(),
            stale_history_id: None,
        })
}

#[derive(Debug, Serialize)]
pub struct CredentialImportsResponse {
    pub credential_set_id: String,
    pub offset: usize,
    pub limit: usize,
    pub imports: Vec<CredentialImportBatchStatus>,
}

#[derive(Debug, Serialize)]
pub struct CredentialImportDetailResponse {
    pub credential_set_id: String,
    pub import: CredentialImportBatchStatus,
}

#[derive(Debug, Serialize)]
pub struct CredentialImportBatchStatus {
    pub batch_id: String,
    pub source_kind: CredentialImportSourceKindStatus,
    pub source_ref: Option<String>,
    pub physical_line_count: usize,
    pub non_empty_count: usize,
    pub unique_count: usize,
    pub duplicate_occurrence_count: usize,
    pub ignored_empty_count: usize,
    pub invalid_line_count: usize,
    pub created_at_unix_seconds: i64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialImportSourceKindStatus {
    FileBootstrap,
    ManagementApi,
}

#[derive(Debug, Serialize)]
pub struct CredentialResourceStatus {
    pub credential_set_id: String,
    pub credential_id: String,
    pub credential_ref: String,
    pub fingerprint: String,
    pub label: Option<String>,
    pub note: Option<String>,
    pub source_ref: Option<String>,
    pub source_line: Option<usize>,
    pub batch_id: Option<String>,
    pub position: usize,
    pub first_imported_at_unix_seconds: i64,
    pub last_seen_at_unix_seconds: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialStatus {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    pub fingerprint: String,
    pub state: CredentialStateSnapshot,
    pub source: CredentialSourceStatus,
    pub latest_probe: Option<CredentialProbeResultStatus>,
}

#[derive(Debug, Serialize)]
pub struct CredentialImportResponse {
    pub credential_set_id: String,
    pub channel_ids: Vec<String>,
    pub selector_generation: u64,
    pub requested_credentials: usize,
    pub imported_credentials: usize,
    pub duplicate_credentials: usize,
    pub ignored_empty_credentials: usize,
    pub credentials: Vec<CredentialStatus>,
    pub operations: CredentialSetOperationsStatus,
}

pub struct CredentialImportResponseInput {
    pub credential_set_id: String,
    pub channel_ids: Vec<String>,
    pub selector_generation: u64,
    pub requested_credentials: usize,
    pub imported_credentials: usize,
    pub duplicate_credentials: usize,
    pub ignored_empty_credentials: usize,
    pub credentials: Vec<CredentialStatus>,
    pub operations: CredentialSetOperationsStatus,
}

pub fn credential_import_response(
    input: CredentialImportResponseInput,
) -> CredentialImportResponse {
    CredentialImportResponse {
        credential_set_id: input.credential_set_id,
        channel_ids: input.channel_ids,
        selector_generation: input.selector_generation,
        requested_credentials: input.requested_credentials,
        imported_credentials: input.imported_credentials,
        duplicate_credentials: input.duplicate_credentials,
        ignored_empty_credentials: input.ignored_empty_credentials,
        credentials: input.credentials,
        operations: input.operations,
    }
}

pub fn credential_import_response_from_apply(
    credential_set_id: &str,
    channel_ids: Vec<String>,
    requested_credentials: usize,
    runtime_apply: CredentialImportRuntimeApply,
    operations: CredentialSetOperationsStatus,
) -> CredentialImportResponse {
    let KeyImportReport {
        duplicate_occurrence_count,
        ignored_empty_count,
        ..
    } = runtime_apply.import_report;
    credential_import_response(CredentialImportResponseInput {
        credential_set_id: credential_set_id.to_string(),
        channel_ids,
        selector_generation: runtime_apply.selector_generation,
        requested_credentials,
        imported_credentials: runtime_apply.credentials.len(),
        duplicate_credentials: duplicate_occurrence_count,
        ignored_empty_credentials: ignored_empty_count,
        credentials: runtime_apply.credentials,
        operations,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialSourceStatus {
    pub source_id: Option<String>,
    pub source_line: Option<usize>,
    pub batch_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CredentialsResponse {
    pub channel_id: String,
    pub total_credentials: usize,
    pub filtered_credentials: usize,
    pub offset: usize,
    pub limit: usize,
    pub state_filter: CredentialStateFilter,
    pub source_id: String,
    pub credentials: Vec<CredentialStatus>,
}

#[derive(Debug, Clone, Copy)]
pub struct CredentialsPageStatus {
    pub total_credentials: usize,
    pub filtered_credentials: usize,
    pub offset: usize,
    pub limit: usize,
    pub state_filter: CredentialStateFilter,
}

pub fn credentials_page_status(
    total_credentials: usize,
    filtered_credentials: usize,
    offset: usize,
    limit: usize,
    state_filter: CredentialStateFilter,
) -> CredentialsPageStatus {
    CredentialsPageStatus {
        total_credentials,
        filtered_credentials,
        offset,
        limit,
        state_filter,
    }
}

pub fn credentials_response(
    channel_id: &str,
    source_id: String,
    page: CredentialsPageStatus,
    credentials: Vec<CredentialStatus>,
) -> CredentialsResponse {
    CredentialsResponse {
        channel_id: channel_id.to_string(),
        total_credentials: page.total_credentials,
        filtered_credentials: page.filtered_credentials,
        offset: page.offset,
        limit: page.limit,
        state_filter: page.state_filter,
        source_id,
        credentials,
    }
}

pub async fn credentials_response_for_snapshot_page(
    credential_store: &CredentialStoreHandle,
    channel_id: &str,
    pool_state: &PoolState,
    page: CredentialsPageStatus,
) -> Result<CredentialsResponse, ManagementServiceError> {
    let snapshot_page = {
        let pool = pool_state.pool.lock().await;
        pool.credential_snapshot_page(page.state_filter.into(), page.offset, page.limit)
    };
    let credentials = credential_statuses_with_latest_probe(
        credential_store,
        &pool_state.credential_set_id,
        snapshot_page.credentials,
    )
    .await?;
    Ok(credentials_response(
        channel_id,
        key_import_source_id_for_pool(pool_state),
        CredentialsPageStatus {
            total_credentials: snapshot_page.total_credentials,
            filtered_credentials: snapshot_page.filtered_credentials,
            ..page
        },
        credentials,
    ))
}

pub async fn credentials_response_for_channel_page(
    state: &AppState,
    channel_id: &str,
    offset: usize,
    limit: usize,
    state_filter: CredentialStateFilter,
) -> Result<CredentialsResponse, ManagementServiceError> {
    let pool_state = channel_pool_for_id(state, channel_id)?;
    credentials_response_for_snapshot_page(
        &state.credential_store,
        channel_id,
        &pool_state,
        credentials_page_status(0, 0, offset, limit, state_filter),
    )
    .await
}

#[derive(Debug, Serialize)]
pub struct CredentialSetCredentialsResponse {
    pub credential_set_id: String,
    pub channel_ids: Vec<String>,
    pub total_credentials: usize,
    pub filtered_credentials: usize,
    pub offset: usize,
    pub limit: usize,
    pub state_filter: CredentialStateFilter,
    pub probe_filter: CredentialProbeFilter,
    pub source_id: String,
    pub credentials: Vec<CredentialStatus>,
}

#[derive(Debug, Clone, Copy)]
pub struct CredentialSetCredentialsPageStatus {
    pub total_credentials: usize,
    pub filtered_credentials: usize,
    pub offset: usize,
    pub limit: usize,
    pub state_filter: CredentialStateFilter,
    pub probe_filter: CredentialProbeFilter,
}

pub fn credential_set_credentials_page_status(
    total_credentials: usize,
    filtered_credentials: usize,
    offset: usize,
    limit: usize,
    state_filter: CredentialStateFilter,
    probe_filter: CredentialProbeFilter,
) -> CredentialSetCredentialsPageStatus {
    CredentialSetCredentialsPageStatus {
        total_credentials,
        filtered_credentials,
        offset,
        limit,
        state_filter,
        probe_filter,
    }
}

pub fn credential_set_credentials_response(
    credential_set_id: &str,
    channel_ids: Vec<String>,
    source_id: String,
    page: CredentialSetCredentialsPageStatus,
    credentials: Vec<CredentialStatus>,
) -> CredentialSetCredentialsResponse {
    CredentialSetCredentialsResponse {
        credential_set_id: credential_set_id.to_string(),
        channel_ids,
        total_credentials: page.total_credentials,
        filtered_credentials: page.filtered_credentials,
        offset: page.offset,
        limit: page.limit,
        state_filter: page.state_filter,
        probe_filter: page.probe_filter,
        source_id,
        credentials,
    }
}

pub struct FilteredCredentialStatuses {
    pub total_credentials: usize,
    pub filtered_credentials: usize,
    pub credentials: Vec<CredentialStatus>,
}

pub async fn credential_probe_outcome_credential_ids_for_set(
    credential_store: &CredentialStoreHandle,
    credential_set_id: CredentialSetId,
    outcome: crate::credential_repository::CredentialProbeOutcome,
    offset: usize,
    limit: usize,
) -> Result<CredentialIdPage, ManagementServiceError> {
    match credential_store
        .load_latest_probe_outcome_credential_ids(credential_set_id, outcome, offset, limit)
        .await
    {
        Ok(page) => Ok(CredentialIdPage {
            total: page.total,
            credential_ids: page.credential_ids,
        }),
        Err(CredentialStoreError::NotWritable) => Ok(CredentialIdPage {
            total: 0,
            credential_ids: Vec::new(),
        }),
        Err(err) => Err(credential_resource_store_error(err)),
    }
}

pub async fn unprobed_credential_ids_for_set(
    credential_store: &CredentialStoreHandle,
    credential_set_id: CredentialSetId,
    offset: usize,
    limit: usize,
) -> Result<Option<CredentialIdPage>, ManagementServiceError> {
    match credential_store
        .load_unprobed_credential_ids(credential_set_id, offset, limit)
        .await
    {
        Ok(page) => Ok(Some(page)),
        Err(CredentialStoreError::NotWritable) => Ok(None),
        Err(err) => Err(credential_resource_store_error(err)),
    }
}

pub async fn credential_set_credentials_response_for_id_page(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    channel_ids: Vec<String>,
    pool_state: &PoolState,
    id_page: CredentialIdPage,
    page: CredentialSetCredentialsPageStatus,
) -> Result<CredentialSetCredentialsResponse, ManagementServiceError> {
    let (total_credentials, snapshots) = {
        let pool = pool_state.pool.lock().await;
        let snapshots = id_page
            .credential_ids
            .iter()
            .filter_map(|credential_id| pool.credential_snapshot_by_id(credential_id))
            .collect();
        (pool.snapshot().total_credentials, snapshots)
    };
    let credentials = credential_statuses_with_latest_probe(
        credential_store,
        &pool_state.credential_set_id,
        snapshots,
    )
    .await?;
    Ok(credential_set_credentials_response(
        credential_set_id,
        channel_ids,
        key_import_source_id_for_pool(pool_state),
        CredentialSetCredentialsPageStatus {
            total_credentials,
            filtered_credentials: id_page.total,
            ..page
        },
        credentials,
    ))
}

pub async fn credential_set_credentials_response_for_snapshot_page(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    channel_ids: Vec<String>,
    pool_state: &PoolState,
    page: CredentialSetCredentialsPageStatus,
) -> Result<CredentialSetCredentialsResponse, ManagementServiceError> {
    let snapshot_page = {
        let pool = pool_state.pool.lock().await;
        pool.credential_snapshot_page(page.state_filter.into(), page.offset, page.limit)
    };
    let credentials = credential_statuses_with_latest_probe(
        credential_store,
        &pool_state.credential_set_id,
        snapshot_page.credentials,
    )
    .await?;
    Ok(credential_set_credentials_response(
        credential_set_id,
        channel_ids,
        key_import_source_id_for_pool(pool_state),
        CredentialSetCredentialsPageStatus {
            total_credentials: snapshot_page.total_credentials,
            filtered_credentials: snapshot_page.filtered_credentials,
            ..page
        },
        credentials,
    ))
}

pub async fn credential_set_credentials_response_for_probe_filter_page(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    channel_ids: Vec<String>,
    pool_state: &PoolState,
    page: CredentialSetCredentialsPageStatus,
) -> Result<CredentialSetCredentialsResponse, ManagementServiceError> {
    let filtered_page = credential_statuses_matching_probe_filter(
        credential_store,
        pool_state,
        page.state_filter,
        page.probe_filter,
        page.offset,
        page.limit,
    )
    .await?;
    Ok(credential_set_credentials_response(
        credential_set_id,
        channel_ids,
        key_import_source_id_for_pool(pool_state),
        CredentialSetCredentialsPageStatus {
            total_credentials: filtered_page.total_credentials,
            filtered_credentials: filtered_page.filtered_credentials,
            ..page
        },
        filtered_page.credentials,
    ))
}

pub async fn credential_set_credentials_response_for_set_page(
    state: &AppState,
    credential_set_id: &str,
    offset: usize,
    limit: usize,
    state_filter: CredentialStateFilter,
    probe_filter: CredentialProbeFilter,
) -> Result<CredentialSetCredentialsResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;

    if state_filter == CredentialStateFilter::All {
        if let Some(outcome) = crate::credential_probe::probe_filter_outcome(probe_filter) {
            let indexed_page = credential_probe_outcome_credential_ids_for_set(
                &state.credential_store,
                scope.pool_state.credential_set_id.clone(),
                outcome,
                offset,
                limit,
            )
            .await?;
            return credential_set_credentials_response_for_id_page(
                &state.credential_store,
                credential_set_id,
                scope.channel_ids,
                &scope.pool_state,
                indexed_page,
                credential_set_credentials_page_status(
                    0,
                    0,
                    offset,
                    limit,
                    state_filter,
                    probe_filter,
                ),
            )
            .await;
        }
    }

    if state_filter == CredentialStateFilter::All && probe_filter == CredentialProbeFilter::Unprobed
    {
        if let Some(unprobed_page) = unprobed_credential_ids_for_set(
            &state.credential_store,
            scope.pool_state.credential_set_id.clone(),
            offset,
            limit,
        )
        .await?
        {
            return credential_set_credentials_response_for_id_page(
                &state.credential_store,
                credential_set_id,
                scope.channel_ids,
                &scope.pool_state,
                unprobed_page,
                credential_set_credentials_page_status(
                    0,
                    0,
                    offset,
                    limit,
                    state_filter,
                    probe_filter,
                ),
            )
            .await;
        }
    }

    if probe_filter == CredentialProbeFilter::All {
        return credential_set_credentials_response_for_snapshot_page(
            &state.credential_store,
            credential_set_id,
            scope.channel_ids,
            &scope.pool_state,
            credential_set_credentials_page_status(0, 0, offset, limit, state_filter, probe_filter),
        )
        .await;
    }

    credential_set_credentials_response_for_probe_filter_page(
        &state.credential_store,
        credential_set_id,
        scope.channel_ids,
        &scope.pool_state,
        credential_set_credentials_page_status(0, 0, offset, limit, state_filter, probe_filter),
    )
    .await
}

#[derive(Debug, Serialize)]
pub struct CredentialResourceResponse {
    pub credential_set_id: String,
    pub channel_ids: Vec<String>,
    pub credential: CredentialStatus,
    pub resource: CredentialResourceStatus,
    pub latest_probe: Option<CredentialProbeResultStatus>,
}

pub fn credential_resource_response(
    credential_set_id: &str,
    channel_ids: Vec<String>,
    credential: CredentialStatus,
    resource: CredentialResourceStatus,
    latest_probe: Option<CredentialProbeResultStatus>,
) -> CredentialResourceResponse {
    CredentialResourceResponse {
        credential_set_id: credential_set_id.to_string(),
        channel_ids,
        credential,
        resource,
        latest_probe,
    }
}

pub async fn credential_resource_for_set(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    channel_ids: Vec<String>,
    pool_state: &PoolState,
    credential_id: CredentialId,
) -> Result<CredentialResourceResponse, ManagementServiceError> {
    let snapshot = {
        let pool = pool_state.pool.lock().await;
        pool.credential_snapshot_by_id(&credential_id)
    }
    .ok_or_else(|| {
        ManagementServiceError::NotFound(format!(
            "unknown credential {} in credential_set {credential_set_id}",
            credential_id.0
        ))
    })?;
    let resource = credential_store
        .load_credential_resource(
            CredentialSetId(credential_set_id.to_string()),
            credential_id.clone(),
        )
        .await
        .map_err(credential_resource_store_error)?
        .ok_or_else(|| {
            ManagementServiceError::NotFound(format!(
                "unknown credential {} in credential_set {credential_set_id}",
                credential_id.0
            ))
        })?;
    let latest_probe = credential_store
        .load_latest_probe_result(
            CredentialSetId(credential_set_id.to_string()),
            credential_id.clone(),
        )
        .await
        .map_err(credential_resource_store_error)?
        .map(credential_probe_result_status);
    let credential_ref = credential_ref_for_position(resource.position);
    Ok(credential_resource_response(
        credential_set_id,
        channel_ids,
        credential_status_with_latest_probe_and_ref(
            snapshot,
            latest_probe.clone(),
            Some(credential_ref),
        ),
        credential_resource_status(resource),
        latest_probe,
    ))
}

pub async fn credential_resource_response_for_set(
    state: &AppState,
    credential_set_id: &str,
    credential_id: CredentialId,
) -> Result<CredentialResourceResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    credential_resource_for_set(
        &state.credential_store,
        credential_set_id,
        scope.channel_ids,
        &scope.pool_state,
        credential_id,
    )
    .await
}

pub async fn ensure_credential_exists_in_runtime_set(
    credential_set_id: &str,
    pool_state: &PoolState,
    credential_id: &CredentialId,
) -> Result<(), ManagementServiceError> {
    let exists = {
        let pool = pool_state.pool.lock().await;
        pool.credential_snapshot_by_id(credential_id).is_some()
    };
    if exists {
        Ok(())
    } else {
        Err(ManagementServiceError::NotFound(format!(
            "unknown credential {} in credential_set {credential_set_id}",
            credential_id.0
        )))
    }
}

#[derive(Debug, Serialize)]
pub struct CredentialProbeResponse {
    pub credential_set_id: String,
    pub channel_id: String,
    pub credential_id: String,
    pub result: CredentialProbeResultStatus,
    pub credential: CredentialStatus,
}

pub fn credential_probe_response(
    credential_set_id: String,
    channel_id: String,
    credential_id: CredentialId,
    result: CredentialProbeResultStatus,
    credential: CredentialStatus,
) -> CredentialProbeResponse {
    CredentialProbeResponse {
        credential_set_id,
        channel_id,
        credential_id: credential_id.0,
        result,
        credential,
    }
}

pub async fn record_credential_probe_response(
    credential_store: &CredentialStoreHandle,
    pool_state: &PoolState,
    input: CredentialProbeResultRecordInput,
) -> Result<CredentialProbeResponse, ManagementServiceError> {
    let credential_set_id = input.credential_set_id.0.clone();
    let channel_id = input.channel_id.clone();
    let credential_id = input.credential_id.clone();
    let result = credential_store
        .record_probe_result(input)
        .await
        .map_err(credential_resource_store_error)?;
    let snapshot = {
        let pool = pool_state.pool.lock().await;
        pool.credential_snapshot_by_id(&credential_id)
    }
    .ok_or_else(|| {
        ManagementServiceError::NotFound(format!(
            "unknown credential {} in credential_set {credential_set_id}",
            credential_id.0
        ))
    })?;
    Ok(credential_probe_response(
        credential_set_id,
        channel_id,
        credential_id,
        credential_probe_result_status(result),
        credential_status(snapshot),
    ))
}

pub async fn runtime_existing_secrets_for_import(
    pool_state: &PoolState,
    keys: &[String],
) -> HashSet<String> {
    let pool = pool_state.pool.lock().await;
    keys.iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
        .filter(|key| pool.contains_secret(key))
        .map(ToString::to_string)
        .collect()
}

pub fn ensure_import_has_non_empty_key(keys: &[String]) -> Result<(), ManagementServiceError> {
    if keys.iter().any(|key| !key.trim().is_empty()) {
        Ok(())
    } else {
        Err(ManagementServiceError::Conflict(
            "credential import requires at least one non-empty key".to_string(),
        ))
    }
}

pub fn ensure_probe_model_is_non_empty(model: &str) -> Result<(), ManagementServiceError> {
    if model.trim().is_empty() {
        Err(ManagementServiceError::Conflict(
            "credential probe model must not be empty".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub async fn selected_credential_key_for_probe(
    pool_state: &PoolState,
    credential_set_id: &str,
    credential_id: &CredentialId,
) -> Result<SelectedKey, ManagementServiceError> {
    let pool = pool_state.pool.lock().await;
    pool.credential_key_by_id(credential_id).ok_or_else(|| {
        ManagementServiceError::NotFound(format!(
            "unknown credential {} in credential_set {credential_set_id}",
            credential_id.0
        ))
    })
}

pub async fn probe_credential_response_for_command(
    state: &AppState,
    actor: ManagementEventActor,
    command: CredentialProbeCommand,
) -> Result<CredentialProbeResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, &command.credential_set_id)?;
    ensure_probe_model_is_non_empty(&command.model)?;
    let selected = selected_credential_key_for_probe(
        &scope.pool_state,
        &command.credential_set_id,
        &command.credential_id,
    )
    .await?;

    let input = execute_credential_probe(ExecuteCredentialProbeContext {
        http_client: &state.http_client,
        command: &command,
        pool_state: &scope.pool_state,
        channel_id: &scope.canonical_channel_id,
        selected: &selected,
        max_error_body_bytes: state.max_error_body_bytes,
    })
    .await;
    record_credential_management_audit(
        state,
        actor,
        "credential_probe_recorded",
        "credential",
        input.credential_id.0.clone(),
        "manual_credential_probe",
    )
    .await?;
    record_credential_probe_response(&state.credential_store, &scope.pool_state, input).await
}

pub async fn append_credential_import_for_set(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    keys: Vec<String>,
    batch_id: String,
    runtime_existing_secrets: HashSet<String>,
) -> Result<KeyImport, ManagementServiceError> {
    credential_store
        .append_credentials_excluding_runtime_secrets(
            CredentialSetId(credential_set_id.to_string()),
            keys,
            batch_id,
            runtime_existing_secrets,
        )
        .await
        .map_err(credential_store_error)
}

pub struct CredentialImportRuntimeApply {
    pub credentials: Vec<CredentialStatus>,
    pub import_report: KeyImportReport,
    pub selector_generation: u64,
}

pub async fn apply_credential_import_to_runtime(
    pool_state: &PoolState,
    import: KeyImport,
) -> Result<CredentialImportRuntimeApply, ManagementServiceError> {
    let import_report = import.report;
    let persisted_imported_count = import_report.unique_count;
    let credentials = import
        .credentials
        .into_iter()
        .map(|credential| PoolCredentialInput {
            secret: credential.secret,
            source: credential.source,
        })
        .collect();
    let _mutation_guard = pool_state.mutation_gate.lock().await;
    let mut pool = pool_state.pool.lock().await;
    let snapshots = pool
        .add_credentials(credentials)
        .map_err(|err| ManagementServiceError::Persistence(err.to_string()))?;
    let credentials: Vec<_> = snapshots.into_iter().map(credential_status).collect();
    if credentials.len() != persisted_imported_count {
        return Err(ManagementServiceError::Persistence(format!(
            "credential import persisted {persisted_imported_count} credentials but applied {} to runtime pool",
            credentials.len()
        )));
    }
    let selector_generation = if credentials.is_empty() {
        pool_state.selector_generation.load(Ordering::Acquire)
    } else {
        pool_state
            .selector_generation
            .fetch_add(1, Ordering::AcqRel)
            + 1
    };
    {
        let mut report = pool_state
            .key_import_report
            .lock()
            .expect("key import report mutex poisoned");
        report.physical_line_count += import_report.physical_line_count;
        report.non_empty_count += import_report.non_empty_count;
        report.unique_count += credentials.len();
        report.duplicate_occurrence_count += import_report.duplicate_occurrence_count;
        report.ignored_empty_count += import_report.ignored_empty_count;
        report.import_generation = import_report.import_generation;
        report.last_imported_at_unix_seconds = import_report.last_imported_at_unix_seconds;
        report
            .duplicate_fingerprints
            .extend(import_report.duplicate_fingerprints.clone());
    }

    Ok(CredentialImportRuntimeApply {
        credentials,
        import_report,
        selector_generation,
    })
}

pub async fn credential_import_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    keys: Vec<String>,
    batch_id: Option<String>,
) -> Result<CredentialImportResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    let requested_credentials = keys.len();
    ensure_import_has_non_empty_key(&keys)?;
    let runtime_existing_secrets =
        runtime_existing_secrets_for_import(&scope.pool_state, &keys).await;
    let batch_id = batch_id.unwrap_or_else(|| "management-api".to_string());
    record_credential_management_audit(
        state,
        actor,
        "credential_set_credentials_imported",
        "credential_set",
        scope.credential_set_id.0.clone(),
        "manual_credential_import",
    )
    .await?;
    let import = append_credential_import_for_set(
        &state.credential_store,
        credential_set_id,
        keys,
        batch_id,
        runtime_existing_secrets,
    )
    .await?;
    let runtime_apply = apply_credential_import_to_runtime(&scope.pool_state, import).await?;
    let operations = credential_set_operations_for_set(state, credential_set_id).await?;
    Ok(credential_import_response_from_apply(
        credential_set_id,
        scope.channel_ids,
        requested_credentials,
        runtime_apply,
        operations,
    ))
}

#[derive(Debug, Serialize)]
pub struct CredentialProbeResultsResponse {
    pub credential_set_id: String,
    pub credential_id: String,
    pub offset: usize,
    pub limit: usize,
    pub probes: Vec<CredentialProbeResultStatus>,
}

pub fn credential_probe_results_response(
    credential_set_id: &str,
    credential_id: CredentialId,
    offset: usize,
    limit: usize,
    probes: Vec<CredentialProbeResultStatus>,
) -> CredentialProbeResultsResponse {
    CredentialProbeResultsResponse {
        credential_set_id: credential_set_id.to_string(),
        credential_id: credential_id.0,
        offset,
        limit,
        probes,
    }
}

pub async fn credential_probe_results_for_credential(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    pool_state: &PoolState,
    credential_id: CredentialId,
    offset: usize,
    limit: usize,
) -> Result<CredentialProbeResultsResponse, ManagementServiceError> {
    {
        let pool = pool_state.pool.lock().await;
        if pool.credential_snapshot_by_id(&credential_id).is_none() {
            return Err(ManagementServiceError::NotFound(format!(
                "unknown credential {} in credential_set {credential_set_id}",
                credential_id.0
            )));
        }
    }
    let probes = credential_store
        .load_probe_results(
            CredentialSetId(credential_set_id.to_string()),
            credential_id.clone(),
            offset,
            limit,
        )
        .await
        .map_err(credential_resource_store_error)?
        .into_iter()
        .map(credential_probe_result_status)
        .collect();
    Ok(credential_probe_results_response(
        credential_set_id,
        credential_id,
        offset,
        limit,
        probes,
    ))
}

pub async fn credential_probe_results_response_for_set(
    state: &AppState,
    credential_set_id: &str,
    credential_id: CredentialId,
    offset: usize,
    limit: usize,
) -> Result<CredentialProbeResultsResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    credential_probe_results_for_credential(
        &state.credential_store,
        &scope.credential_set_id.0,
        &scope.pool_state,
        credential_id,
        offset,
        limit,
    )
    .await
}

#[derive(Debug, Serialize)]
pub struct CredentialProbeApplyResponse {
    pub credential_set_id: String,
    pub credential_id: String,
    pub action: CredentialProbeApplyActionStatus,
    pub probe: CredentialProbeResultStatus,
    pub mutation: Option<CredentialMutationResponse>,
}

pub struct CredentialProbeApplyPlan {
    pub probe_result_ref: String,
    pub action: CredentialProbeApplyActionStatus,
    pub probe: CredentialProbeResultStatus,
}

#[derive(Debug, Serialize)]
pub struct CredentialProbeApplyPlanResponse {
    pub credential_set_id: String,
    pub credential_ref: String,
    pub probe_result_ref: String,
    pub action: CredentialProbeApplyActionStatus,
    pub probe: CredentialProbeResultStatus,
}

pub fn credential_probe_result_ref_for_id(id: i64) -> String {
    format!("pr:v1:id:{id}")
}

pub async fn credential_probe_apply_plan_for_latest_result(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    pool_state: &PoolState,
    credential_id: CredentialId,
) -> Result<CredentialProbeApplyPlan, ManagementServiceError> {
    let probe = credential_store
        .load_latest_probe_result(
            CredentialSetId(credential_set_id.to_string()),
            credential_id.clone(),
        )
        .await
        .map_err(credential_resource_store_error)?
        .ok_or_else(|| {
            ManagementServiceError::NotFound(format!(
                "no probe result for selected credential in credential_set {credential_set_id}"
            ))
        })?;
    let action = if probe_result_is_default_key_switch_cooldown(
        &probe,
        pool_state.error_classifier.classifier_id(),
    ) {
        CredentialProbeApplyActionStatus::Noop
    } else {
        probe_apply_action(probe.outcome, &pool_state.probe_result_policy)
    };
    let probe_result_ref = credential_probe_result_ref_for_id(probe.id);
    Ok(CredentialProbeApplyPlan {
        probe_result_ref,
        action,
        probe: credential_probe_result_status(probe),
    })
}

pub async fn credential_probe_apply_plan_response_for_set(
    state: &AppState,
    credential_set_id: &str,
    credential_id: CredentialId,
) -> Result<CredentialProbeApplyPlanResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    let resource = credential_resource_for_set(
        &state.credential_store,
        &scope.credential_set_id.0,
        scope.channel_ids,
        &scope.pool_state,
        credential_id.clone(),
    )
    .await?;
    let plan = credential_probe_apply_plan_for_latest_result(
        &state.credential_store,
        &scope.credential_set_id.0,
        &scope.pool_state,
        credential_id,
    )
    .await?;
    Ok(CredentialProbeApplyPlanResponse {
        credential_set_id: scope.credential_set_id.0,
        credential_ref: resource.resource.credential_ref,
        probe_result_ref: plan.probe_result_ref,
        action: plan.action,
        probe: plan.probe,
    })
}

pub fn credential_probe_apply_response(
    credential_set_id: &str,
    credential_id: CredentialId,
    action: CredentialProbeApplyActionStatus,
    probe: CredentialProbeResultStatus,
    mutation: Option<CredentialMutationResponse>,
) -> CredentialProbeApplyResponse {
    CredentialProbeApplyResponse {
        credential_set_id: credential_set_id.to_string(),
        credential_id: credential_id.0,
        action,
        probe,
        mutation,
    }
}

pub async fn apply_latest_credential_probe_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    credential_id: CredentialId,
    expected_probe_result_ref: Option<String>,
    require_probe_result_ref: bool,
    reason: String,
) -> Result<CredentialProbeApplyResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    let plan = credential_probe_apply_plan_for_latest_result(
        &state.credential_store,
        credential_set_id,
        &scope.pool_state,
        credential_id.clone(),
    )
    .await?;
    if require_probe_result_ref {
        validate_probe_apply_precondition(expected_probe_result_ref.as_deref(), &plan)?;
    }
    let mutation = match credential_set_probe_apply_command(
        plan.action,
        actor.clone(),
        scope.credential_set_id.0.clone(),
        scope.canonical_channel_id.clone(),
        credential_id.clone(),
        reason.clone(),
    ) {
        Some(command) => Some(execute_credential_command_for_state(state, command).await?),
        None if matches!(plan.action, CredentialProbeApplyActionStatus::Cooldown) => {
            record_credential_management_audit(
                state,
                actor,
                "credential_cooldown_applied",
                "credential",
                credential_id.0.clone(),
                "manual_apply_latest_probe_cooldown",
            )
            .await?;
            Some(
                apply_probe_credential_cooldown(
                    &scope.pool_state,
                    scope.canonical_channel_id.clone(),
                    scope.credential_set_id.clone(),
                    credential_id.clone(),
                    reason,
                )
                .await?,
            )
        }
        None => None,
    };
    Ok(credential_probe_apply_response(
        credential_set_id,
        credential_id,
        plan.action,
        plan.probe,
        mutation,
    ))
}

fn validate_probe_apply_precondition(
    expected_probe_result_ref: Option<&str>,
    plan: &CredentialProbeApplyPlan,
) -> Result<(), ManagementServiceError> {
    let Some(expected_probe_result_ref) = expected_probe_result_ref else {
        return Err(ManagementServiceError::Conflict(
            "probe_result_ref is required before applying latest probe evidence".to_string(),
        ));
    };
    let Some(expected_probe_result_ref) = normalize_probe_result_ref(expected_probe_result_ref)
    else {
        return Err(ManagementServiceError::BadRequest(
            "probe_result_ref must use pr:v1:id:<number>".to_string(),
        ));
    };
    if expected_probe_result_ref != plan.probe_result_ref {
        return Err(ManagementServiceError::Conflict(
            "probe_result_ref does not match the latest probe result".to_string(),
        ));
    }
    Ok(())
}

fn normalize_probe_result_ref(value: &str) -> Option<String> {
    let id = value.strip_prefix("pr:v1:id:")?;
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id = id.parse::<i64>().ok()?;
    if id < 0 {
        return None;
    }
    Some(format!("pr:v1:id:{id}"))
}

#[derive(Debug, Serialize)]
pub struct CredentialProbeBatchApplyResponse {
    pub credential_set_id: String,
    pub probe_filter: CredentialProbeFilter,
    pub matched_credentials: usize,
    pub applied_credentials: usize,
    pub failed_credentials: usize,
    pub results: Vec<CredentialProbeBatchApplyItem>,
}

pub struct CredentialProbeApplyCandidates {
    pub matched_credentials: usize,
    pub credential_ids: Vec<String>,
}

pub async fn credential_probe_apply_candidates_for_latest_outcome(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    probe_filter: CredentialProbeFilter,
    offset: usize,
    limit: usize,
) -> Result<CredentialProbeApplyCandidates, ManagementServiceError> {
    let outcome = crate::credential_probe::probe_filter_outcome(probe_filter).ok_or_else(|| {
        ManagementServiceError::Conflict(
            "bulk apply latest probe requires a concrete probed outcome filter".to_string(),
        )
    })?;
    let candidates = credential_store
        .load_latest_probe_outcome_credential_ids(
            CredentialSetId(credential_set_id.to_string()),
            outcome,
            offset,
            limit,
        )
        .await
        .map_err(credential_resource_store_error)?;
    Ok(CredentialProbeApplyCandidates {
        matched_credentials: candidates.total,
        credential_ids: candidates
            .credential_ids
            .into_iter()
            .map(|credential_id| credential_id.0)
            .collect(),
    })
}

pub async fn credential_probe_apply_candidates_for_existing_set(
    state: &AppState,
    credential_set_id: &str,
    probe_filter: CredentialProbeFilter,
    offset: usize,
    limit: usize,
) -> Result<CredentialProbeApplyCandidates, ManagementServiceError> {
    ensure_credential_set_exists(state, credential_set_id)?;
    credential_probe_apply_candidates_for_latest_outcome(
        &state.credential_store,
        credential_set_id,
        probe_filter,
        offset,
        limit,
    )
    .await
}

pub fn credential_probe_batch_apply_response(
    credential_set_id: &str,
    probe_filter: CredentialProbeFilter,
    matched_credentials: usize,
    results: Vec<CredentialProbeBatchApplyItem>,
) -> CredentialProbeBatchApplyResponse {
    let applied_credentials = results
        .iter()
        .filter(|result| matches!(result.status, CredentialProbeBatchApplyItemStatus::Applied))
        .count();
    CredentialProbeBatchApplyResponse {
        credential_set_id: credential_set_id.to_string(),
        probe_filter,
        matched_credentials,
        applied_credentials,
        failed_credentials: results.len() - applied_credentials,
        results,
    }
}

pub async fn apply_latest_credential_probes_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    probe_filter: CredentialProbeFilter,
    limit: usize,
    reason: String,
) -> Result<CredentialProbeBatchApplyResponse, ManagementServiceError> {
    if matches!(
        probe_filter,
        CredentialProbeFilter::All | CredentialProbeFilter::Unprobed
    ) {
        return Err(ManagementServiceError::Conflict(
            "bulk apply latest probe requires a concrete probed outcome filter".to_string(),
        ));
    }
    let candidates = credential_probe_apply_candidates_for_existing_set(
        state,
        credential_set_id,
        probe_filter,
        BULK_PROBE_APPLY_CANDIDATE_OFFSET,
        limit,
    )
    .await?;
    let mut results = Vec::new();
    for credential_id in candidates.credential_ids {
        let result = apply_latest_credential_probe_response_for_set(
            state,
            actor.clone(),
            credential_set_id,
            CredentialId(credential_id.clone()),
            None,
            false,
            reason.clone(),
        )
        .await;
        results.push(credential_probe_batch_apply_item(credential_id, result));
    }
    Ok(credential_probe_batch_apply_response(
        credential_set_id,
        probe_filter,
        candidates.matched_credentials,
        results,
    ))
}

#[derive(Debug, Serialize)]
pub struct CredentialProbeBatchApplyItem {
    pub credential_id: String,
    pub status: CredentialProbeBatchApplyItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<CredentialProbeApplyResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CredentialProbeBatchApplyError>,
}

pub fn credential_probe_batch_apply_item(
    credential_id: String,
    result: Result<CredentialProbeApplyResponse, ManagementServiceError>,
) -> CredentialProbeBatchApplyItem {
    match result {
        Ok(result) => CredentialProbeBatchApplyItem {
            credential_id,
            status: CredentialProbeBatchApplyItemStatus::Applied,
            result: Some(result),
            error: None,
        },
        Err(err) => CredentialProbeBatchApplyItem {
            credential_id,
            status: CredentialProbeBatchApplyItemStatus::Failed,
            result: None,
            error: Some(credential_probe_batch_apply_error(err)),
        },
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProbeBatchApplyItemStatus {
    Applied,
    Failed,
}

#[derive(Debug, Serialize)]
pub struct CredentialProbeBatchApplyError {
    pub kind: &'static str,
    pub message: String,
}

pub fn credential_probe_batch_apply_error(
    err: ManagementServiceError,
) -> CredentialProbeBatchApplyError {
    match err.into_public_error() {
        ManagementServiceError::BadRequest(message) => CredentialProbeBatchApplyError {
            kind: "bad_request",
            message,
        },
        ManagementServiceError::NotFound(message) => CredentialProbeBatchApplyError {
            kind: "not_found",
            message,
        },
        ManagementServiceError::Conflict(message) => CredentialProbeBatchApplyError {
            kind: "conflict",
            message,
        },
        ManagementServiceError::PreconditionFailed(message) => CredentialProbeBatchApplyError {
            kind: "precondition_failed",
            message,
        },
        ManagementServiceError::Persistence(message) => CredentialProbeBatchApplyError {
            kind: "persistence",
            message,
        },
        ManagementServiceError::EventAppendFailed { .. } => CredentialProbeBatchApplyError {
            kind: "event_append_failed",
            message: "failed to record management event".to_string(),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStateFilter {
    All,
    Available,
    CoolingDown,
    Expired,
    QuotaExhausted,
    Disabled,
}

impl From<CredentialStateFilter> for CredentialSnapshotFilter {
    fn from(value: CredentialStateFilter) -> Self {
        match value {
            CredentialStateFilter::All => CredentialSnapshotFilter::All,
            CredentialStateFilter::Available => CredentialSnapshotFilter::Available,
            CredentialStateFilter::CoolingDown => CredentialSnapshotFilter::CoolingDown,
            CredentialStateFilter::Expired => CredentialSnapshotFilter::Expired,
            CredentialStateFilter::QuotaExhausted => CredentialSnapshotFilter::QuotaExhausted,
            CredentialStateFilter::Disabled => CredentialSnapshotFilter::Disabled,
        }
    }
}

pub async fn apply_probe_credential_cooldown(
    pool_state: &PoolState,
    channel_id: String,
    credential_set_id: CredentialSetId,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    let _send_guard = pool_state.send_gate.write().await;
    let _mutation_guard = pool_state.mutation_gate.lock().await;
    let mut pool = pool_state.pool.lock().await;
    let before = pool
        .credential_snapshot_by_id(&credential_id)
        .ok_or_else(|| {
            ManagementServiceError::NotFound(format!("unknown credential {}", credential_id.0))
        })?;
    let until = Instant::now() + pool_state.probe_result_policy.cooldown;
    pool.apply_credential_cooldown_until(&credential_id, until, reason);
    let after = pool
        .credential_snapshot_by_id(&credential_id)
        .ok_or_else(|| {
            ManagementServiceError::NotFound(format!("unknown credential {}", credential_id.0))
        })?;
    let selector_generation = pool_state
        .advance_selector_generation_if_state_kind_changed(Some(&before.state), &after.state);
    Ok(credential_mutation_response(
        channel_id,
        credential_set_id.0,
        selector_generation,
        after,
    ))
}

#[derive(Debug, Serialize)]
pub struct CredentialLifecycleHistoryResponse {
    pub credential_set_id: String,
    pub credential_id: String,
    pub offset: usize,
    pub limit: usize,
    pub history: Vec<CredentialLifecycleHistoryStatus>,
}

pub fn credential_lifecycle_history_response(
    credential_set_id: &str,
    credential_id: CredentialId,
    offset: usize,
    limit: usize,
    history: Vec<CredentialLifecycleHistoryStatus>,
) -> CredentialLifecycleHistoryResponse {
    CredentialLifecycleHistoryResponse {
        credential_set_id: credential_set_id.to_string(),
        credential_id: credential_id.0,
        offset,
        limit,
        history,
    }
}

pub async fn credential_lifecycle_history_for_credential(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    pool_state: &PoolState,
    credential_id: CredentialId,
    offset: usize,
    limit: usize,
) -> Result<CredentialLifecycleHistoryResponse, ManagementServiceError> {
    {
        let pool = pool_state.pool.lock().await;
        if pool.credential_snapshot_by_id(&credential_id).is_none() {
            return Err(ManagementServiceError::NotFound(format!(
                "unknown credential {} in credential_set {credential_set_id}",
                credential_id.0
            )));
        }
    }
    let history = credential_store
        .load_lifecycle_history(
            CredentialSetId(credential_set_id.to_string()),
            credential_id.clone(),
            offset,
            limit,
        )
        .await
        .map_err(credential_resource_store_error)?
        .into_iter()
        .map(credential_lifecycle_history_status)
        .collect();
    Ok(credential_lifecycle_history_response(
        credential_set_id,
        credential_id,
        offset,
        limit,
        history,
    ))
}

pub async fn credential_lifecycle_history_response_for_set(
    state: &AppState,
    credential_set_id: &str,
    credential_id: CredentialId,
    offset: usize,
    limit: usize,
) -> Result<CredentialLifecycleHistoryResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    credential_lifecycle_history_for_credential(
        &state.credential_store,
        &scope.credential_set_id.0,
        &scope.pool_state,
        credential_id,
        offset,
        limit,
    )
    .await
}

#[derive(Debug, Serialize)]
pub struct CredentialLifecycleHistoryStatus {
    pub state: CredentialLifecycleStateStatus,
    pub source: CredentialLifecycleHistorySourceStatus,
    pub reason_class: Option<String>,
    pub actor: Option<CredentialLifecycleActorStatus>,
    pub channel_id: Option<String>,
    pub created_at_unix_seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct CredentialLifecycleActorStatus {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialLifecycleStateStatus {
    Available,
    Expired { reason: String },
    QuotaExhausted { reason: String },
    Disabled { reason: String },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialLifecycleHistorySourceStatus {
    ManagementCommand,
    Compensation,
    AutomaticFailure,
}

pub fn credential_lifecycle_history_status(
    record: CredentialLifecycleHistoryRecord,
) -> CredentialLifecycleHistoryStatus {
    let actor = match (record.actor_id, record.actor_name, record.actor_role) {
        (Some(id), Some(name), Some(role)) => {
            Some(CredentialLifecycleActorStatus { id, name, role })
        }
        _ => None,
    };
    CredentialLifecycleHistoryStatus {
        state: credential_lifecycle_state_status(record.state),
        source: credential_lifecycle_history_source_status(record.source),
        reason_class: record.reason_class,
        actor,
        channel_id: record.channel_id,
        created_at_unix_seconds: record.created_at_unix_seconds,
    }
}

fn credential_lifecycle_state_status(
    state: CredentialLifecycleState,
) -> CredentialLifecycleStateStatus {
    match state {
        CredentialLifecycleState::Available => CredentialLifecycleStateStatus::Available,
        CredentialLifecycleState::Expired { reason } => {
            CredentialLifecycleStateStatus::Expired { reason }
        }
        CredentialLifecycleState::QuotaExhausted { reason } => {
            CredentialLifecycleStateStatus::QuotaExhausted { reason }
        }
        CredentialLifecycleState::Disabled { reason } => {
            CredentialLifecycleStateStatus::Disabled { reason }
        }
    }
}

fn credential_lifecycle_history_source_status(
    source: CredentialLifecycleHistorySource,
) -> CredentialLifecycleHistorySourceStatus {
    match source {
        CredentialLifecycleHistorySource::ManagementCommand => {
            CredentialLifecycleHistorySourceStatus::ManagementCommand
        }
        CredentialLifecycleHistorySource::Compensation => {
            CredentialLifecycleHistorySourceStatus::Compensation
        }
        CredentialLifecycleHistorySource::AutomaticFailure => {
            CredentialLifecycleHistorySourceStatus::AutomaticFailure
        }
    }
}

pub fn credential_import_batch_status(
    record: CredentialImportBatchRecord,
) -> CredentialImportBatchStatus {
    CredentialImportBatchStatus {
        batch_id: record.batch_id,
        source_kind: credential_import_source_kind_status(record.source_kind),
        source_ref: record.source_ref,
        physical_line_count: record.physical_line_count,
        non_empty_count: record.non_empty_count,
        unique_count: record.unique_count,
        duplicate_occurrence_count: record.duplicate_occurrence_count,
        ignored_empty_count: record.ignored_empty_count,
        invalid_line_count: record.invalid_line_count,
        created_at_unix_seconds: record.created_at_unix_seconds,
    }
}

pub async fn credential_imports_for_set(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    offset: usize,
    limit: usize,
) -> Result<CredentialImportsResponse, ManagementServiceError> {
    let imports = credential_store
        .load_import_batches(
            CredentialSetId(credential_set_id.to_string()),
            offset,
            limit,
        )
        .await
        .map_err(credential_resource_store_error)?;
    Ok(credential_imports_response(
        credential_set_id,
        offset,
        limit,
        imports,
    ))
}

pub async fn credential_imports_for_existing_set(
    state: &AppState,
    credential_set_id: &str,
    offset: usize,
    limit: usize,
) -> Result<CredentialImportsResponse, ManagementServiceError> {
    ensure_credential_set_exists(state, credential_set_id)?;
    credential_imports_for_set(&state.credential_store, credential_set_id, offset, limit).await
}

pub fn credential_imports_response(
    credential_set_id: &str,
    offset: usize,
    limit: usize,
    imports: Vec<CredentialImportBatchRecord>,
) -> CredentialImportsResponse {
    CredentialImportsResponse {
        credential_set_id: credential_set_id.to_string(),
        offset,
        limit,
        imports: imports
            .into_iter()
            .map(credential_import_batch_status)
            .collect(),
    }
}

pub async fn credential_import_for_set(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    batch_id: &str,
) -> Result<CredentialImportDetailResponse, ManagementServiceError> {
    let import = credential_store
        .load_import_batch(
            CredentialSetId(credential_set_id.to_string()),
            batch_id.to_string(),
        )
        .await
        .map_err(credential_resource_store_error)?
        .ok_or_else(|| {
            ManagementServiceError::NotFound(format!(
                "unknown credential import batch {batch_id} in credential_set {credential_set_id}"
            ))
        })?;
    Ok(credential_import_detail_response(credential_set_id, import))
}

pub async fn credential_import_for_existing_set(
    state: &AppState,
    credential_set_id: &str,
    batch_id: &str,
) -> Result<CredentialImportDetailResponse, ManagementServiceError> {
    ensure_credential_set_exists(state, credential_set_id)?;
    credential_import_for_set(&state.credential_store, credential_set_id, batch_id).await
}

pub fn credential_import_detail_response(
    credential_set_id: &str,
    import: CredentialImportBatchRecord,
) -> CredentialImportDetailResponse {
    CredentialImportDetailResponse {
        credential_set_id: credential_set_id.to_string(),
        import: credential_import_batch_status(import),
    }
}

pub fn credential_status(snapshot: CredentialSnapshot) -> CredentialStatus {
    credential_status_with_latest_probe_and_ref(snapshot, None, None)
}

pub fn credential_status_with_latest_probe_and_ref(
    snapshot: CredentialSnapshot,
    latest_probe: Option<CredentialProbeResultStatus>,
    credential_ref: Option<String>,
) -> CredentialStatus {
    CredentialStatus {
        id: snapshot.id,
        credential_ref,
        fingerprint: snapshot.fingerprint,
        state: redact_credential_state(snapshot.state),
        source: CredentialSourceStatus {
            source_id: snapshot.source.source_path.as_deref().map(source_id),
            source_line: snapshot.source.source_line,
            batch_id: snapshot.source.batch_id,
        },
        latest_probe,
    }
}

pub async fn credential_statuses_with_latest_probe(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &CredentialSetId,
    snapshots: Vec<CredentialSnapshot>,
) -> Result<Vec<CredentialStatus>, ManagementServiceError> {
    let credential_ids: Vec<_> = snapshots
        .iter()
        .map(|snapshot| CredentialId(snapshot.id.clone()))
        .collect();
    let latest_probes = match credential_store
        .load_latest_probe_results_for_credentials(credential_set_id.clone(), credential_ids)
        .await
    {
        Ok(latest_probes) => latest_probes,
        Err(CredentialStoreError::NotWritable) => HashMap::new(),
        Err(err) => return Err(credential_resource_store_error(err)),
    };
    let credential_ids: Vec<_> = snapshots
        .iter()
        .map(|snapshot| CredentialId(snapshot.id.clone()))
        .collect();
    let credential_positions = match credential_store
        .load_credential_positions_for_credentials(credential_set_id.clone(), credential_ids)
        .await
    {
        Ok(positions) => positions,
        Err(CredentialStoreError::NotWritable) => HashMap::new(),
        Err(err) => return Err(credential_resource_store_error(err)),
    };
    Ok(snapshots
        .into_iter()
        .map(|snapshot| {
            let credential_id = CredentialId(snapshot.id.clone());
            let latest_probe = latest_probes
                .get(&credential_id)
                .cloned()
                .map(credential_probe_result_status);
            let credential_ref = credential_positions
                .get(&credential_id)
                .copied()
                .map(credential_ref_for_position);
            credential_status_with_latest_probe_and_ref(snapshot, latest_probe, credential_ref)
        })
        .collect())
}

pub async fn credential_statuses_matching_probe_filter(
    credential_store: &CredentialStoreHandle,
    pool_state: &PoolState,
    state_filter: CredentialStateFilter,
    probe_filter: CredentialProbeFilter,
    offset: usize,
    limit: usize,
) -> Result<FilteredCredentialStatuses, ManagementServiceError> {
    let mut scan_offset = 0;
    let mut total_credentials;
    let mut probe_filtered_total = 0;
    let mut credentials = Vec::new();

    loop {
        let page = {
            let pool = pool_state.pool.lock().await;
            pool.credential_snapshot_page(
                state_filter.into(),
                scan_offset,
                CREDENTIAL_PROBE_FILTER_SCAN_PAGE_SIZE,
            )
        };
        total_credentials = page.total_credentials;

        if page.credentials.is_empty() {
            break;
        }

        let state_filtered_total = page.filtered_credentials;
        let statuses = credential_statuses_with_latest_probe(
            credential_store,
            &pool_state.credential_set_id,
            page.credentials,
        )
        .await?;
        for status in statuses {
            if !probe_filter_matches(probe_filter, status.latest_probe.as_ref()) {
                continue;
            }
            if probe_filtered_total >= offset && credentials.len() < limit {
                credentials.push(status);
            }
            probe_filtered_total += 1;
        }

        scan_offset += CREDENTIAL_PROBE_FILTER_SCAN_PAGE_SIZE;
        if scan_offset >= state_filtered_total {
            break;
        }
    }

    Ok(FilteredCredentialStatuses {
        total_credentials,
        filtered_credentials: probe_filtered_total,
        credentials,
    })
}

pub fn credential_resource_status(record: CredentialResourceRecord) -> CredentialResourceStatus {
    CredentialResourceStatus {
        credential_set_id: record.credential_set_id.0,
        credential_id: record.credential_id.0,
        credential_ref: credential_ref_for_position(record.position),
        fingerprint: record.fingerprint,
        label: record.label,
        note: record.note,
        source_ref: record.source_ref,
        source_line: record.source_line,
        batch_id: record.batch_id,
        position: record.position,
        first_imported_at_unix_seconds: record.first_imported_at_unix_seconds,
        last_seen_at_unix_seconds: record.last_seen_at_unix_seconds,
    }
}

pub fn normalize_operator_metadata_field(
    value: Option<String>,
    max_chars: usize,
    field: &str,
) -> Result<Option<String>, ManagementServiceError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().to_string();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > max_chars {
        return Err(ManagementServiceError::Conflict(format!(
            "credential {field} must be at most {max_chars} characters"
        )));
    }
    Ok(Some(value))
}

pub async fn update_credential_operator_metadata_for_set(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    credential_id: CredentialId,
    label: Option<String>,
    note: Option<String>,
) -> Result<(), ManagementServiceError> {
    credential_store
        .update_credential_operator_metadata(
            CredentialSetId(credential_set_id.to_string()),
            credential_id,
            normalize_operator_metadata_field(label, 120, "label")?,
            normalize_operator_metadata_field(note, 1000, "note")?,
        )
        .await
        .map_err(credential_resource_store_error)?;
    Ok(())
}

pub async fn credential_operator_metadata_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    credential_id: CredentialId,
    label: Option<String>,
    note: Option<String>,
) -> Result<CredentialResourceResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    ensure_credential_exists_in_runtime_set(
        &scope.credential_set_id.0,
        &scope.pool_state,
        &credential_id,
    )
    .await?;
    let label = normalize_operator_metadata_field(label, 120, "label")?;
    let note = normalize_operator_metadata_field(note, 1000, "note")?;
    record_credential_management_audit(
        state,
        actor,
        "credential_metadata_updated",
        "credential",
        credential_id.0.clone(),
        "manual_credential_metadata_update",
    )
    .await?;
    update_credential_operator_metadata_for_set(
        &state.credential_store,
        &scope.credential_set_id.0,
        credential_id.clone(),
        label,
        note,
    )
    .await?;
    credential_resource_response_for_set(state, &scope.credential_set_id.0, credential_id).await
}

pub fn redact_credential_state(state: CredentialStateSnapshot) -> CredentialStateSnapshot {
    match state {
        CredentialStateSnapshot::Available => CredentialStateSnapshot::Available,
        CredentialStateSnapshot::CoolingDown {
            reason,
            remaining_seconds,
        } => CredentialStateSnapshot::CoolingDown {
            reason: redact_management_reason(&reason),
            remaining_seconds,
        },
        CredentialStateSnapshot::Expired { reason } => CredentialStateSnapshot::Expired {
            reason: redact_management_reason(&reason),
        },
        CredentialStateSnapshot::QuotaExhausted { reason } => {
            CredentialStateSnapshot::QuotaExhausted {
                reason: redact_management_reason(&reason),
            }
        }
        CredentialStateSnapshot::Disabled { reason } => CredentialStateSnapshot::Disabled {
            reason: redact_management_reason(&reason),
        },
    }
}

fn credential_import_source_kind_status(
    kind: CredentialImportSourceKind,
) -> CredentialImportSourceKindStatus {
    match kind {
        CredentialImportSourceKind::FileBootstrap => {
            CredentialImportSourceKindStatus::FileBootstrap
        }
        CredentialImportSourceKind::ManagementApi => {
            CredentialImportSourceKindStatus::ManagementApi
        }
    }
}
