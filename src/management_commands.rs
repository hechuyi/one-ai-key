use std::{path::Path, sync::atomic::Ordering};

use serde::Serialize;

use crate::{
    credential_probe::{CredentialProbeApplyActionStatus, CredentialProbeResultStatus},
    credential_repository::{
        CredentialLifecycleEvidence, CredentialLifecycleState, CredentialLifecycleUpdate,
        CredentialSetId, CredentialStoreHandle,
    },
    credentials::{CredentialId, CredentialSnapshot, CredentialStateSnapshot},
    events::{EventLog, ManagementEventActor, PendingManagementEvent},
    management_errors::credential_lifecycle_store_error,
    management_errors::ManagementServiceError,
    management_resource_lookup::credential_set_runtime_scope,
    management_status::redact_management_reason,
    state::{AppState, PoolState},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialCommandKind {
    Expire,
    QuotaExhaust,
    Restore,
    Disable,
    Enable,
    ClearCooldown,
}

#[derive(Debug, Clone)]
pub enum CredentialCommandScope {
    Channel {
        channel_id: String,
    },
    CredentialSet {
        credential_set_id: String,
        canonical_channel_id: String,
    },
}

#[derive(Debug, Clone)]
pub struct CredentialCommand {
    pub kind: CredentialCommandKind,
    pub actor: ManagementEventActor,
    pub scope: CredentialCommandScope,
    pub credential_id: CredentialId,
    pub reason: String,
    pub reason_class: Option<String>,
}

struct CredentialCommandRuntimeScope {
    channel_id: String,
    requested_credential_set_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CredentialMutationResponse {
    pub channel_id: String,
    pub credential_set_id: String,
    pub credential_id: String,
    pub selector_generation: u64,
    pub state: CredentialStateSnapshot,
    pub credential: CredentialMutationCredentialStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialMutationCredentialStatus {
    pub id: String,
    pub fingerprint: String,
    pub state: CredentialStateSnapshot,
    pub source: CredentialMutationCredentialSourceStatus,
    pub latest_probe: Option<CredentialProbeResultStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialMutationCredentialSourceStatus {
    pub source_id: Option<String>,
    pub source_line: Option<usize>,
    pub batch_id: Option<String>,
}

pub fn credential_mutation_response(
    channel_id: String,
    credential_set_id: String,
    selector_generation: u64,
    snapshot: CredentialSnapshot,
) -> CredentialMutationResponse {
    CredentialMutationResponse {
        channel_id,
        credential_set_id,
        credential_id: snapshot.id.clone(),
        selector_generation,
        state: redact_credential_state(snapshot.state.clone()),
        credential: credential_mutation_credential_status(snapshot),
    }
}

fn credential_mutation_credential_status(
    snapshot: CredentialSnapshot,
) -> CredentialMutationCredentialStatus {
    CredentialMutationCredentialStatus {
        id: snapshot.id,
        fingerprint: snapshot.fingerprint,
        state: redact_credential_state(snapshot.state),
        source: CredentialMutationCredentialSourceStatus {
            source_id: snapshot
                .source
                .source_path
                .as_deref()
                .map(credential_mutation_source_id),
            source_line: snapshot.source.source_line,
            batch_id: snapshot.source.batch_id,
        },
        latest_probe: None,
    }
}

fn credential_mutation_source_id(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown-source".to_string())
}

fn redact_credential_state(state: CredentialStateSnapshot) -> CredentialStateSnapshot {
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

pub fn credential_command(
    kind: CredentialCommandKind,
    actor: ManagementEventActor,
    channel_id: impl Into<String>,
    credential_id: CredentialId,
    reason: String,
    reason_class: Option<&str>,
) -> CredentialCommand {
    CredentialCommand {
        kind,
        actor,
        scope: CredentialCommandScope::Channel {
            channel_id: channel_id.into(),
        },
        credential_id,
        reason,
        reason_class: reason_class.map(str::to_string),
    }
}

pub fn credential_set_command(
    kind: CredentialCommandKind,
    actor: ManagementEventActor,
    credential_set_id: impl Into<String>,
    canonical_channel_id: impl Into<String>,
    credential_id: CredentialId,
    reason: String,
    reason_class: Option<&str>,
) -> CredentialCommand {
    CredentialCommand {
        kind,
        actor,
        scope: CredentialCommandScope::CredentialSet {
            credential_set_id: credential_set_id.into(),
            canonical_channel_id: canonical_channel_id.into(),
        },
        credential_id,
        reason,
        reason_class: reason_class.map(str::to_string),
    }
}

pub fn credential_set_probe_apply_command(
    action: CredentialProbeApplyActionStatus,
    actor: ManagementEventActor,
    credential_set_id: String,
    canonical_channel_id: String,
    credential_id: CredentialId,
    reason: String,
) -> Option<CredentialCommand> {
    match action {
        CredentialProbeApplyActionStatus::Expire => Some(credential_set_command(
            CredentialCommandKind::Expire,
            actor,
            credential_set_id,
            canonical_channel_id,
            credential_id,
            reason,
            Some("probe_invalid"),
        )),
        CredentialProbeApplyActionStatus::QuotaExhaust => Some(credential_set_command(
            CredentialCommandKind::QuotaExhaust,
            actor,
            credential_set_id,
            canonical_channel_id,
            credential_id,
            reason,
            Some("probe_quota_exhausted"),
        )),
        CredentialProbeApplyActionStatus::Restore => Some(credential_set_command(
            CredentialCommandKind::Restore,
            actor,
            credential_set_id,
            canonical_channel_id,
            credential_id,
            reason,
            Some("probe_success"),
        )),
        CredentialProbeApplyActionStatus::Cooldown | CredentialProbeApplyActionStatus::Noop => None,
    }
}

pub async fn expire_credential_response_for_channel(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    execute_credential_command_for_state(
        state,
        credential_command(
            CredentialCommandKind::Expire,
            actor,
            channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn restore_credential_response_for_channel(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    execute_credential_command_for_state(
        state,
        credential_command(
            CredentialCommandKind::Restore,
            actor,
            channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn quota_exhaust_credential_response_for_channel(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    execute_credential_command_for_state(
        state,
        credential_command(
            CredentialCommandKind::QuotaExhaust,
            actor,
            channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn disable_credential_response_for_channel(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    execute_credential_command_for_state(
        state,
        credential_command(
            CredentialCommandKind::Disable,
            actor,
            channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn enable_credential_response_for_channel(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    execute_credential_command_for_state(
        state,
        credential_command(
            CredentialCommandKind::Enable,
            actor,
            channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn clear_credential_cooldown_response_for_channel(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    execute_credential_command_for_state(
        state,
        credential_command(
            CredentialCommandKind::ClearCooldown,
            actor,
            channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn expire_credential_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    execute_credential_command_for_state(
        state,
        credential_set_command(
            CredentialCommandKind::Expire,
            actor,
            scope.credential_set_id.0,
            scope.canonical_channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn restore_credential_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    execute_credential_command_for_state(
        state,
        credential_set_command(
            CredentialCommandKind::Restore,
            actor,
            scope.credential_set_id.0,
            scope.canonical_channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn quota_exhaust_credential_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    execute_credential_command_for_state(
        state,
        credential_set_command(
            CredentialCommandKind::QuotaExhaust,
            actor,
            scope.credential_set_id.0,
            scope.canonical_channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn disable_credential_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    execute_credential_command_for_state(
        state,
        credential_set_command(
            CredentialCommandKind::Disable,
            actor,
            scope.credential_set_id.0,
            scope.canonical_channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn enable_credential_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    execute_credential_command_for_state(
        state,
        credential_set_command(
            CredentialCommandKind::Enable,
            actor,
            scope.credential_set_id.0,
            scope.canonical_channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn clear_credential_cooldown_response_for_set(
    state: &AppState,
    actor: ManagementEventActor,
    credential_set_id: &str,
    credential_id: CredentialId,
    reason: String,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    let scope = credential_set_runtime_scope(state, credential_set_id)?;
    execute_credential_command_for_state(
        state,
        credential_set_command(
            CredentialCommandKind::ClearCooldown,
            actor,
            scope.credential_set_id.0,
            scope.canonical_channel_id,
            credential_id,
            reason,
            None,
        ),
    )
    .await
}

pub async fn execute_credential_command_for_state(
    state: &AppState,
    command: CredentialCommand,
) -> Result<CredentialMutationResponse, ManagementServiceError> {
    let CredentialCommand {
        kind,
        actor,
        scope,
        credential_id,
        reason,
        reason_class,
    } = command;
    let runtime_scope = command_scope_runtime_scope(scope);
    let channel_id = runtime_scope.channel_id;
    let Some(pool_state) = state.channels.get(&channel_id) else {
        return Err(ManagementServiceError::NotFound(format!(
            "unknown channel {channel_id}"
        )));
    };
    if let Some(requested_credential_set_id) = runtime_scope.requested_credential_set_id.as_deref()
    {
        if requested_credential_set_id != pool_state.credential_set_id.0 {
            return Err(ManagementServiceError::NotFound(format!(
                "unknown credential_set {requested_credential_set_id}"
            )));
        }
    }
    let (precondition_generation, precondition_state) = {
        let pool = pool_state.pool.lock().await;
        let Some(snapshot) = pool.credential_snapshot_by_id(&credential_id) else {
            return Err(ManagementServiceError::NotFound(format!(
                "unknown credential {}",
                credential_id.0
            )));
        };
        match credential_command_state_decision(kind, &snapshot.state) {
            CredentialCommandStateDecision::AlreadyApplied => {
                return Ok(credential_mutation_response(
                    channel_id,
                    pool_state.credential_set_id.0.clone(),
                    pool_state.selector_generation.load(Ordering::Acquire),
                    snapshot,
                ));
            }
            CredentialCommandStateDecision::Rejected => {
                return Err(ManagementServiceError::Conflict(format!(
                    "credential {} is not in a state accepted by {kind:?}",
                    credential_id.0
                )));
            }
            CredentialCommandStateDecision::Ready => {}
        }
        (
            pool_state.selector_generation.load(Ordering::Acquire),
            snapshot.state,
        )
    };
    let durable_update = lifecycle_update_for_command(
        kind,
        pool_state.credential_set_id.clone(),
        credential_id.clone(),
        reason.clone(),
        &precondition_state,
    );
    let durable_update_exists = durable_update.is_some();
    let lifecycle_evidence =
        lifecycle_evidence_for_command(kind, reason_class.as_deref(), &actor, &channel_id);
    let transaction = credential_command_transaction(CredentialCommandTransaction {
        kind,
        pool_state: pool_state.clone(),
        credential_store: state.credential_store.clone(),
        channel_id: channel_id.clone(),
        credential_id: credential_id.clone(),
        reason: reason.clone(),
        precondition_generation,
        precondition_state,
        durable_update: durable_update.clone(),
        lifecycle_evidence,
    });
    let result = record_credential_command_transaction(
        &state.events,
        kind,
        actor,
        channel_id,
        credential_id.clone(),
        reason,
        transaction,
    )
    .await;
    match result {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(err)) => {
            if durable_update_exists {
                let stale_history_id = match &err {
                    ManagementServiceError::EventAppendFailed {
                        stale_history_id, ..
                    } => *stale_history_id,
                    _ => None,
                };
                compensate_credential_lifecycle(
                    &state.credential_store,
                    &pool_state,
                    &credential_id,
                    stale_history_id,
                )
                .await?;
            }
            Err(err.into_public_error())
        }
        Err(err) => {
            if durable_update_exists {
                compensate_credential_lifecycle(
                    &state.credential_store,
                    &pool_state,
                    &credential_id,
                    None,
                )
                .await?;
            }
            Err(ManagementServiceError::Persistence(format!(
                "failed to record management event: {err}"
            )))
        }
    }
}

fn command_scope_runtime_scope(scope: CredentialCommandScope) -> CredentialCommandRuntimeScope {
    match scope {
        CredentialCommandScope::Channel { channel_id } => CredentialCommandRuntimeScope {
            channel_id,
            requested_credential_set_id: None,
        },
        CredentialCommandScope::CredentialSet {
            credential_set_id,
            canonical_channel_id,
        } => CredentialCommandRuntimeScope {
            channel_id: canonical_channel_id,
            requested_credential_set_id: Some(credential_set_id),
        },
    }
}

async fn compensate_credential_lifecycle(
    credential_store: &CredentialStoreHandle,
    pool_state: &PoolState,
    credential_id: &CredentialId,
    stale_history_id: Option<i64>,
) -> Result<(), ManagementServiceError> {
    let snapshot = {
        let pool = pool_state.pool.lock().await;
        pool.credential_snapshot_by_id(credential_id)
    };
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    let update = lifecycle_update_for_snapshot(
        pool_state.credential_set_id.clone(),
        credential_id.clone(),
        &snapshot.state,
    );
    credential_store
        .persist_lifecycle_snapshot(update, stale_history_id)
        .await
        .map_err(credential_lifecycle_store_error)?;
    Ok(())
}

pub async fn record_credential_command_transaction<T, F>(
    events: &EventLog,
    kind: CredentialCommandKind,
    actor: ManagementEventActor,
    channel_id: String,
    credential_id: CredentialId,
    reason: String,
    transaction: F,
) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
{
    match kind {
        CredentialCommandKind::Expire => {
            events
                .record_credential_expired_transaction(
                    actor,
                    channel_id,
                    credential_id.0.clone(),
                    reason,
                    transaction,
                )
                .await
        }
        CredentialCommandKind::QuotaExhaust => {
            events
                .record_credential_quota_exhausted_transaction(
                    actor,
                    channel_id,
                    credential_id.0.clone(),
                    reason,
                    transaction,
                )
                .await
        }
        CredentialCommandKind::Restore => {
            events
                .record_credential_restored_transaction(
                    actor,
                    channel_id,
                    credential_id.0.clone(),
                    reason,
                    transaction,
                )
                .await
        }
        CredentialCommandKind::Disable => {
            events
                .record_credential_disabled_transaction(
                    actor,
                    channel_id,
                    credential_id.0.clone(),
                    reason,
                    transaction,
                )
                .await
        }
        CredentialCommandKind::Enable => {
            events
                .record_credential_enabled_transaction(
                    actor,
                    channel_id,
                    credential_id.0.clone(),
                    reason,
                    transaction,
                )
                .await
        }
        CredentialCommandKind::ClearCooldown => {
            events
                .record_credential_cooldown_cleared_transaction(
                    actor,
                    channel_id,
                    credential_id.0.clone(),
                    reason,
                    transaction,
                )
                .await
        }
    }
}

pub struct CredentialCommandTransaction {
    pub kind: CredentialCommandKind,
    pub pool_state: PoolState,
    pub credential_store: CredentialStoreHandle,
    pub channel_id: String,
    pub credential_id: CredentialId,
    pub reason: String,
    pub precondition_generation: u64,
    pub precondition_state: CredentialStateSnapshot,
    pub durable_update: Option<CredentialLifecycleUpdate>,
    pub lifecycle_evidence: CredentialLifecycleEvidence,
}

pub fn credential_command_transaction(
    transaction: CredentialCommandTransaction,
) -> impl FnOnce(
    PendingManagementEvent,
) -> anyhow::Result<Result<CredentialMutationResponse, ManagementServiceError>>
       + Send
       + 'static {
    let CredentialCommandTransaction {
        kind,
        pool_state,
        credential_store,
        channel_id,
        credential_id,
        reason,
        precondition_generation,
        precondition_state,
        durable_update,
        lifecycle_evidence,
    } = transaction;
    move |pending: PendingManagementEvent| {
        let mut persisted_history_id = None;
        {
            let _mutation_guard = pool_state.mutation_gate.blocking_lock();
            let pool = pool_state.pool.blocking_lock();
            let current_generation = pool_state.selector_generation.load(Ordering::Acquire);
            let Some(current_snapshot) = pool.credential_snapshot_by_id(&credential_id) else {
                return Ok(Err(ManagementServiceError::NotFound(format!(
                    "unknown credential {}",
                    credential_id.0
                ))));
            };
            match check_command_precondition(
                CommandPrecondition {
                    kind,
                    channel_id: &channel_id,
                    credential_set_id: &pool_state.credential_set_id.0,
                    credential_id: &credential_id,
                    precondition_generation,
                    precondition_state: &precondition_state,
                    current_generation,
                },
                current_snapshot,
            ) {
                Ok(CommandReadiness::Ready(_)) => {}
                Ok(CommandReadiness::AlreadyApplied(response)) => return Ok(Ok(*response)),
                Err(err) => return Ok(Err(err)),
            }
        }

        if let Some(update) = durable_update.clone() {
            match credential_store
                .persist_lifecycle_update_with_evidence_blocking(update, lifecycle_evidence.clone())
                .map_err(credential_lifecycle_store_error)
            {
                Ok(persisted) => persisted_history_id = Some(persisted.history_id),
                Err(err) => return Ok(Err(err)),
            }
        }

        if let Err(err) = pending.append() {
            return Ok(Err(ManagementServiceError::EventAppendFailed {
                message: err.to_string(),
                stale_history_id: persisted_history_id,
            }));
        }

        let _send_guard = pool_state.send_gate.blocking_write();
        let _mutation_guard = pool_state.mutation_gate.blocking_lock();
        let mut pool = pool_state.pool.blocking_lock();
        let current_generation = pool_state.selector_generation.load(Ordering::Acquire);
        let Some(current_snapshot) = pool.credential_snapshot_by_id(&credential_id) else {
            return Ok(Err(ManagementServiceError::NotFound(format!(
                "unknown credential {}",
                credential_id.0
            ))));
        };
        let current_snapshot = match check_command_precondition(
            CommandPrecondition {
                kind,
                channel_id: &channel_id,
                credential_set_id: &pool_state.credential_set_id.0,
                credential_id: &credential_id,
                precondition_generation,
                precondition_state: &precondition_state,
                current_generation,
            },
            current_snapshot,
        ) {
            Ok(CommandReadiness::Ready(snapshot)) => snapshot,
            Ok(CommandReadiness::AlreadyApplied(response)) => return Ok(Ok(*response)),
            Err(err) => return Ok(Err(err)),
        };
        let snapshot = match kind {
            CredentialCommandKind::Expire => pool.expire_credential_by_id(&credential_id, reason),
            CredentialCommandKind::QuotaExhaust => {
                pool.apply_credential_quota_exhausted(&credential_id, reason);
                pool.credential_snapshot_by_id(&credential_id)
            }
            CredentialCommandKind::Restore => pool.restore_credential_by_id(&credential_id),
            CredentialCommandKind::Disable => pool.disable_credential_by_id(&credential_id, reason),
            CredentialCommandKind::Enable => pool.enable_credential_by_id(&credential_id),
            CredentialCommandKind::ClearCooldown => {
                pool.clear_credential_cooldown_by_id(&credential_id)
            }
        };
        let Some(snapshot) = snapshot else {
            return Ok(Err(ManagementServiceError::NotFound(format!(
                "unknown credential {}",
                credential_id.0
            ))));
        };
        let selector_generation = pool_state.advance_selector_generation_if_state_kind_changed(
            Some(&current_snapshot.state),
            &snapshot.state,
        );
        Ok(Ok(credential_mutation_response(
            channel_id,
            pool_state.credential_set_id.0.clone(),
            selector_generation,
            snapshot,
        )))
    }
}

pub enum CommandReadiness {
    Ready(CredentialSnapshot),
    AlreadyApplied(Box<CredentialMutationResponse>),
}

pub struct CommandPrecondition<'a> {
    pub kind: CredentialCommandKind,
    pub channel_id: &'a str,
    pub credential_set_id: &'a str,
    pub credential_id: &'a CredentialId,
    pub precondition_generation: u64,
    pub precondition_state: &'a CredentialStateSnapshot,
    pub current_generation: u64,
}

pub fn check_command_precondition(
    precondition: CommandPrecondition<'_>,
    current_snapshot: CredentialSnapshot,
) -> Result<CommandReadiness, ManagementServiceError> {
    match credential_command_state_decision(precondition.kind, &current_snapshot.state) {
        CredentialCommandStateDecision::AlreadyApplied => {
            return Ok(CommandReadiness::AlreadyApplied(Box::new(
                credential_mutation_response(
                    precondition.channel_id.to_string(),
                    precondition.credential_set_id.to_string(),
                    precondition.current_generation,
                    current_snapshot,
                ),
            )));
        }
        CredentialCommandStateDecision::Rejected => {
            return Err(ManagementServiceError::Conflict(format!(
                "credential {} is not in a state accepted by {:?}",
                precondition.credential_id.0, precondition.kind
            )));
        }
        CredentialCommandStateDecision::Ready => {}
    }
    if precondition.current_generation != precondition.precondition_generation
        || !command_precondition_state_matches(
            precondition.precondition_state,
            &current_snapshot.state,
        )
    {
        return Err(ManagementServiceError::Conflict(format!(
            "credential {} changed while management command was being persisted",
            precondition.credential_id.0
        )));
    }
    Ok(CommandReadiness::Ready(current_snapshot))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialCommandStateDecision {
    AlreadyApplied,
    Ready,
    Rejected,
}

pub fn lifecycle_update_for_command(
    kind: CredentialCommandKind,
    credential_set_id: CredentialSetId,
    credential_id: CredentialId,
    reason: String,
    precondition_state: &CredentialStateSnapshot,
) -> Option<CredentialLifecycleUpdate> {
    let state = match kind {
        CredentialCommandKind::Expire => CredentialLifecycleState::Expired { reason },
        CredentialCommandKind::QuotaExhaust => CredentialLifecycleState::QuotaExhausted { reason },
        CredentialCommandKind::Restore | CredentialCommandKind::Enable => {
            CredentialLifecycleState::Available
        }
        CredentialCommandKind::Disable => CredentialLifecycleState::Disabled { reason },
        CredentialCommandKind::ClearCooldown => {
            if matches!(
                precondition_state,
                CredentialStateSnapshot::CoolingDown { .. }
            ) {
                CredentialLifecycleState::Available
            } else {
                return None;
            }
        }
    };
    Some(CredentialLifecycleUpdate {
        credential_set_id,
        credential_id,
        state,
    })
}

pub fn lifecycle_evidence_for_command(
    kind: CredentialCommandKind,
    reason_class: Option<&str>,
    actor: &ManagementEventActor,
    channel_id: &str,
) -> CredentialLifecycleEvidence {
    CredentialLifecycleEvidence::management_command(
        reason_class.unwrap_or_else(|| lifecycle_reason_class_for_command(kind)),
        actor.id.clone(),
        actor.name.clone(),
        actor.role.clone(),
        channel_id.to_string(),
    )
}

pub fn lifecycle_reason_class_for_command(kind: CredentialCommandKind) -> &'static str {
    match kind {
        CredentialCommandKind::Expire => "manual_expire",
        CredentialCommandKind::QuotaExhaust => "manual_quota_exhaust",
        CredentialCommandKind::Restore => "manual_restore",
        CredentialCommandKind::Disable => "manual_disable",
        CredentialCommandKind::Enable => "manual_enable",
        CredentialCommandKind::ClearCooldown => "manual_clear_cooldown",
    }
}

pub fn lifecycle_update_for_snapshot(
    credential_set_id: CredentialSetId,
    credential_id: CredentialId,
    state: &CredentialStateSnapshot,
) -> CredentialLifecycleUpdate {
    let state = match state {
        CredentialStateSnapshot::Available | CredentialStateSnapshot::CoolingDown { .. } => {
            CredentialLifecycleState::Available
        }
        CredentialStateSnapshot::Expired { reason } => CredentialLifecycleState::Expired {
            reason: reason.clone(),
        },
        CredentialStateSnapshot::QuotaExhausted { reason } => {
            CredentialLifecycleState::QuotaExhausted {
                reason: reason.clone(),
            }
        }
        CredentialStateSnapshot::Disabled { reason } => CredentialLifecycleState::Disabled {
            reason: reason.clone(),
        },
    };
    CredentialLifecycleUpdate {
        credential_set_id,
        credential_id,
        state,
    }
}

pub fn credential_command_state_decision(
    kind: CredentialCommandKind,
    state: &CredentialStateSnapshot,
) -> CredentialCommandStateDecision {
    use CredentialCommandKind::{ClearCooldown, Disable, Enable, Expire, QuotaExhaust, Restore};
    use CredentialCommandStateDecision::{AlreadyApplied, Ready, Rejected};
    use CredentialStateSnapshot::{Available, CoolingDown, Disabled, Expired, QuotaExhausted};

    match (kind, state) {
        (Expire, Expired { .. })
        | (QuotaExhaust, QuotaExhausted { .. })
        | (Restore, Available)
        | (Disable, Disabled { .. })
        | (Enable, Available)
        | (ClearCooldown, Available) => AlreadyApplied,

        (ClearCooldown, CoolingDown { .. })
        | (Restore, Expired { .. } | QuotaExhausted { .. })
        | (Expire, Available | CoolingDown { .. } | QuotaExhausted { .. })
        | (QuotaExhaust, Available | CoolingDown { .. } | Expired { .. })
        | (Disable, Available | CoolingDown { .. } | QuotaExhausted { .. })
        | (Enable, Disabled { .. }) => Ready,

        (ClearCooldown, Expired { .. } | QuotaExhausted { .. } | Disabled { .. })
        | (Restore, CoolingDown { .. } | Disabled { .. })
        | (Expire, Disabled { .. })
        | (QuotaExhaust, Disabled { .. })
        | (Disable, Expired { .. })
        | (Enable, CoolingDown { .. } | Expired { .. } | QuotaExhausted { .. }) => Rejected,
    }
}

pub fn command_precondition_state_matches(
    before: &CredentialStateSnapshot,
    current: &CredentialStateSnapshot,
) -> bool {
    match (before, current) {
        (CredentialStateSnapshot::Available, CredentialStateSnapshot::Available) => true,
        (
            CredentialStateSnapshot::CoolingDown { .. },
            CredentialStateSnapshot::CoolingDown { .. },
        ) => true,
        (
            CredentialStateSnapshot::Expired { reason: before },
            CredentialStateSnapshot::Expired { reason: current },
        ) => before == current,
        (
            CredentialStateSnapshot::QuotaExhausted { reason: before },
            CredentialStateSnapshot::QuotaExhausted { reason: current },
        ) => before == current,
        (
            CredentialStateSnapshot::Disabled { reason: before },
            CredentialStateSnapshot::Disabled { reason: current },
        ) => before == current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_command_state_decision_exposes_lifecycle_transition_contract() {
        let available = CredentialStateSnapshot::Available;
        let cooling_down = CredentialStateSnapshot::CoolingDown {
            reason: "rate limit".to_string(),
            remaining_seconds: 20,
        };
        let expired = CredentialStateSnapshot::Expired {
            reason: "invalid auth".to_string(),
        };
        let quota_exhausted = CredentialStateSnapshot::QuotaExhausted {
            reason: "quota exhausted".to_string(),
        };
        let disabled = CredentialStateSnapshot::Disabled {
            reason: "operator pause".to_string(),
        };

        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::Disable, &quota_exhausted),
            CredentialCommandStateDecision::Ready
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::Disable, &expired),
            CredentialCommandStateDecision::Rejected
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::Enable, &disabled),
            CredentialCommandStateDecision::Ready
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::Enable, &quota_exhausted),
            CredentialCommandStateDecision::Rejected
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::Restore, &quota_exhausted),
            CredentialCommandStateDecision::Ready
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::Restore, &cooling_down),
            CredentialCommandStateDecision::Rejected
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::ClearCooldown, &cooling_down),
            CredentialCommandStateDecision::Ready
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::ClearCooldown, &expired),
            CredentialCommandStateDecision::Rejected
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::Expire, &expired),
            CredentialCommandStateDecision::AlreadyApplied
        );
        assert_eq!(
            credential_command_state_decision(
                CredentialCommandKind::QuotaExhaust,
                &quota_exhausted
            ),
            CredentialCommandStateDecision::AlreadyApplied
        );
        assert_eq!(
            credential_command_state_decision(CredentialCommandKind::Restore, &available),
            CredentialCommandStateDecision::AlreadyApplied
        );
    }
}
