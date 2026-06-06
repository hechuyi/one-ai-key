use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{
    params, params_from_iter, types::Value, Connection, OptionalExtension, TransactionBehavior,
};
use sha2::{Digest, Sha256};

use crate::credentials::{short_hash, CredentialId, CredentialSource};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialSetId(pub String);

#[derive(Debug, Clone)]
pub struct ImportedCredential {
    pub secret: String,
    pub source: CredentialSource,
}

#[derive(Debug, Clone)]
pub struct KeyImport {
    pub credentials: Vec<ImportedCredential>,
    pub report: KeyImportReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSetSource {
    File { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateCredentialOccurrence {
    pub first_source_line: usize,
    pub duplicate_source_line: usize,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyImportReport {
    pub source_path: PathBuf,
    pub physical_line_count: usize,
    pub non_empty_count: usize,
    pub unique_count: usize,
    pub duplicate_occurrence_count: usize,
    pub ignored_empty_count: usize,
    pub invalid_line_count: usize,
    pub claimed_count: Option<usize>,
    pub claim_source: Option<String>,
    pub import_generation: u64,
    pub last_imported_at_unix_seconds: i64,
    pub duplicate_fingerprints: Vec<DuplicateCredentialOccurrence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialImportSourceKind {
    FileBootstrap,
    ManagementApi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialImportBatchRecord {
    pub credential_set_id: CredentialSetId,
    pub batch_id: String,
    pub source_kind: CredentialImportSourceKind,
    pub source_ref: Option<String>,
    pub physical_line_count: usize,
    pub non_empty_count: usize,
    pub unique_count: usize,
    pub duplicate_occurrence_count: usize,
    pub ignored_empty_count: usize,
    pub invalid_line_count: usize,
    pub created_at_unix_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialResourceRecord {
    pub credential_set_id: CredentialSetId,
    pub credential_id: CredentialId,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialProbeOutcome {
    Success,
    Invalid,
    QuotaExhausted,
    RateLimited,
    ProviderUnavailable,
    UnsupportedModel,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialProbeResultRecordInput {
    pub credential_set_id: CredentialSetId,
    pub credential_id: CredentialId,
    pub channel_id: String,
    pub provider_id: String,
    pub account_id: String,
    pub outcome: CredentialProbeOutcome,
    pub classifier_id: Option<String>,
    pub adaptation_rule_id: Option<String>,
    pub upstream_status: Option<u16>,
    pub upstream_code: Option<String>,
    pub upstream_limit_type: Option<String>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialProbeResultRecord {
    pub id: i64,
    pub credential_set_id: CredentialSetId,
    pub credential_id: CredentialId,
    pub channel_id: String,
    pub provider_id: String,
    pub account_id: String,
    pub outcome: CredentialProbeOutcome,
    pub classifier_id: Option<String>,
    pub adaptation_rule_id: Option<String>,
    pub upstream_status: Option<u16>,
    pub upstream_code: Option<String>,
    pub upstream_limit_type: Option<String>,
    pub latency_ms: u64,
    pub created_at_unix_seconds: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CredentialProbeSummaryRecord {
    pub success: usize,
    pub invalid: usize,
    pub quota_exhausted: usize,
    pub rate_limited: usize,
    pub provider_unavailable: usize,
    pub unsupported_model: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialProbeOutcomePage {
    pub total: usize,
    pub credential_ids: Vec<CredentialId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialIdPage {
    pub total: usize,
    pub credential_ids: Vec<CredentialId>,
}

pub trait CredentialRepository {
    fn load_credential_set(
        &self,
        credential_set_id: &CredentialSetId,
        source: &CredentialSetSource,
    ) -> anyhow::Result<KeyImport>;
}

#[derive(Debug, Clone)]
pub enum CredentialStoreHandle {
    ReadOnlyFileBootstrap,
    Sqlite(Arc<SqliteCredentialStore>),
}

#[derive(Debug, Clone)]
pub struct SqliteCredentialStore {
    repository: SqliteCredentialRepository,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialStoreError {
    NotWritable,
    Persistence(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialLifecycleState {
    Available,
    Expired { reason: String },
    QuotaExhausted { reason: String },
    Disabled { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialLifecycleSnapshot {
    pub credential_set_id: CredentialSetId,
    pub credential_id: CredentialId,
    pub state: CredentialLifecycleState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialLifecyclePersistedUpdate {
    pub snapshot: CredentialLifecycleSnapshot,
    pub history_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialLifecycleUpdate {
    pub credential_set_id: CredentialSetId,
    pub credential_id: CredentialId,
    pub state: CredentialLifecycleState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialLifecycleHistoryRecord {
    pub credential_set_id: CredentialSetId,
    pub credential_id: CredentialId,
    pub state: CredentialLifecycleState,
    pub source: CredentialLifecycleHistorySource,
    pub reason_class: Option<String>,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub actor_role: Option<String>,
    pub channel_id: Option<String>,
    pub created_at_unix_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialLifecycleHistorySource {
    ManagementCommand,
    Compensation,
    AutomaticFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialLifecycleEvidence {
    pub source: CredentialLifecycleHistorySource,
    pub reason_class: Option<String>,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub actor_role: Option<String>,
    pub channel_id: Option<String>,
}

impl CredentialLifecycleEvidence {
    pub fn management_command(
        reason_class: impl Into<String>,
        actor_id: impl Into<String>,
        actor_name: impl Into<String>,
        actor_role: impl Into<String>,
        channel_id: impl Into<String>,
    ) -> Self {
        Self {
            source: CredentialLifecycleHistorySource::ManagementCommand,
            reason_class: Some(reason_class.into()),
            actor_id: Some(actor_id.into()),
            actor_name: Some(actor_name.into()),
            actor_role: Some(actor_role.into()),
            channel_id: Some(channel_id.into()),
        }
    }

    pub fn automatic_failure(
        reason_class: impl Into<String>,
        channel_id: impl Into<String>,
    ) -> Self {
        Self {
            source: CredentialLifecycleHistorySource::AutomaticFailure,
            reason_class: Some(reason_class.into()),
            actor_id: None,
            actor_name: None,
            actor_role: None,
            channel_id: Some(channel_id.into()),
        }
    }
}

impl CredentialStoreHandle {
    pub fn read_only_file_bootstrap() -> Self {
        Self::ReadOnlyFileBootstrap
    }

    pub fn sqlite(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        SqliteCredentialRepository::open(path)
            .map(|repository| Self::Sqlite(Arc::new(SqliteCredentialStore { repository })))
            .map_err(|err| anyhow::anyhow!("credential store open failed: {err}"))
    }

    pub async fn append_credentials_excluding_runtime_secrets(
        &self,
        credential_set_id: CredentialSetId,
        keys: Vec<String>,
        batch_id: String,
        excluded_secrets: HashSet<String>,
    ) -> Result<KeyImport, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store.repository.append_credentials_excluding_secrets(
                        &credential_set_id,
                        keys,
                        batch_id,
                        excluded_secrets,
                    )
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub(crate) fn persist_lifecycle_update_with_evidence_blocking(
        &self,
        update: CredentialLifecycleUpdate,
        evidence: CredentialLifecycleEvidence,
    ) -> Result<CredentialLifecyclePersistedUpdate, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => store
                .repository
                .persist_lifecycle_update_with_evidence(update, evidence)
                .map_err(|err| CredentialStoreError::Persistence(err.to_string())),
        }
    }

    pub(crate) async fn persist_lifecycle_snapshot(
        &self,
        update: CredentialLifecycleUpdate,
        stale_history_id: Option<i64>,
    ) -> Result<CredentialLifecycleSnapshot, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .persist_lifecycle_snapshot(update, stale_history_id)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub fn load_lifecycle_snapshots_for_startup(
        &self,
        credential_set_id: &CredentialSetId,
    ) -> Result<Vec<CredentialLifecycleSnapshot>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Ok(Vec::new()),
            CredentialStoreHandle::Sqlite(store) => store
                .repository
                .load_lifecycle_snapshots(credential_set_id)
                .map_err(|err| CredentialStoreError::Persistence(err.to_string())),
        }
    }

    pub fn has_lifecycle_snapshot_authority(&self) -> bool {
        matches!(self, CredentialStoreHandle::Sqlite(_))
    }

    pub async fn load_import_batches(
        &self,
        credential_set_id: CredentialSetId,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CredentialImportBatchRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .load_import_batches(&credential_set_id, offset, limit)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_lifecycle_history(
        &self,
        credential_set_id: CredentialSetId,
        credential_id: CredentialId,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CredentialLifecycleHistoryRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store.repository.load_lifecycle_history(
                        &credential_set_id,
                        &credential_id,
                        offset,
                        limit,
                    )
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_import_batch(
        &self,
        credential_set_id: CredentialSetId,
        batch_id: String,
    ) -> Result<Option<CredentialImportBatchRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .load_import_batch(&credential_set_id, &batch_id)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_credential_resource(
        &self,
        credential_set_id: CredentialSetId,
        credential_id: CredentialId,
    ) -> Result<Option<CredentialResourceRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .load_credential_resource(&credential_set_id, &credential_id)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_credential_resource_by_position(
        &self,
        credential_set_id: CredentialSetId,
        position: usize,
    ) -> Result<Option<CredentialResourceRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .load_credential_resource_by_position(&credential_set_id, position)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_credential_positions_for_credentials(
        &self,
        credential_set_id: CredentialSetId,
        credential_ids: Vec<CredentialId>,
    ) -> Result<HashMap<CredentialId, usize>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store.repository.load_credential_positions_for_credentials(
                        &credential_set_id,
                        &credential_ids,
                    )
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn update_credential_operator_metadata(
        &self,
        credential_set_id: CredentialSetId,
        credential_id: CredentialId,
        label: Option<String>,
        note: Option<String>,
    ) -> Result<CredentialResourceRecord, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store.repository.update_credential_operator_metadata(
                        &credential_set_id,
                        &credential_id,
                        label,
                        note,
                    )
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn record_probe_result(
        &self,
        input: CredentialProbeResultRecordInput,
    ) -> Result<CredentialProbeResultRecord, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || store.repository.record_probe_result(input))
                    .await
                    .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                    .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_latest_probe_result(
        &self,
        credential_set_id: CredentialSetId,
        credential_id: CredentialId,
    ) -> Result<Option<CredentialProbeResultRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .load_latest_probe_result(&credential_set_id, &credential_id)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_probe_results(
        &self,
        credential_set_id: CredentialSetId,
        credential_id: CredentialId,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CredentialProbeResultRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store.repository.load_probe_results(
                        &credential_set_id,
                        &credential_id,
                        offset,
                        limit,
                    )
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_latest_probe_results_for_credentials(
        &self,
        credential_set_id: CredentialSetId,
        credential_ids: Vec<CredentialId>,
    ) -> Result<HashMap<CredentialId, CredentialProbeResultRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store.repository.load_latest_probe_results_for_credentials(
                        &credential_set_id,
                        &credential_ids,
                    )
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_latest_probe_outcome_credential_ids(
        &self,
        credential_set_id: CredentialSetId,
        outcome: CredentialProbeOutcome,
        offset: usize,
        limit: usize,
    ) -> Result<CredentialProbeOutcomePage, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store.repository.load_latest_probe_outcome_credential_ids(
                        &credential_set_id,
                        outcome,
                        offset,
                        limit,
                    )
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_unprobed_credential_ids(
        &self,
        credential_set_id: CredentialSetId,
        offset: usize,
        limit: usize,
    ) -> Result<CredentialIdPage, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .load_unprobed_credential_ids(&credential_set_id, offset, limit)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_latest_probe_result_for_credential_set(
        &self,
        credential_set_id: CredentialSetId,
    ) -> Result<Option<CredentialProbeResultRecord>, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .load_latest_probe_result_for_credential_set(&credential_set_id)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }

    pub async fn load_probe_summary_for_credential_set(
        &self,
        credential_set_id: CredentialSetId,
    ) -> Result<CredentialProbeSummaryRecord, CredentialStoreError> {
        match self {
            CredentialStoreHandle::ReadOnlyFileBootstrap => Err(CredentialStoreError::NotWritable),
            CredentialStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .repository
                        .load_probe_summary_for_credential_set(&credential_set_id)
                })
                .await
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))?
                .map_err(|err| CredentialStoreError::Persistence(err.to_string()))
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FileCredentialRepository;

impl FileCredentialRepository {
    pub fn new() -> Self {
        Self
    }
}

impl CredentialRepository for FileCredentialRepository {
    fn load_credential_set(
        &self,
        credential_set_id: &CredentialSetId,
        source: &CredentialSetSource,
    ) -> anyhow::Result<KeyImport> {
        let CredentialSetSource::File { path } = source;
        import_raw_credentials(
            fs::read_to_string(path)?,
            CredentialImportOrigin {
                credential_set_id: credential_set_id.clone(),
                source_path: path.clone(),
                claimed_count: claimed_count_from_path(path),
                claim_source: claimed_count_from_path(path).map(|_| "filename".to_string()),
            },
        )
    }
}

#[derive(Debug, Clone)]
pub struct SqliteCredentialRepository {
    database_path: PathBuf,
}

impl SqliteCredentialRepository {
    pub fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let repository = Self {
            database_path: path.into(),
        };
        let connection = repository.connect()?;
        initialize_schema(&connection)?;
        Ok(repository)
    }

    fn connect(&self) -> anyhow::Result<Connection> {
        let connection = Connection::open(&self.database_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(connection)
    }

    #[cfg(test)]
    pub fn append_credentials(
        &self,
        credential_set_id: &CredentialSetId,
        secrets: Vec<String>,
        batch_id: impl Into<String>,
    ) -> anyhow::Result<KeyImport> {
        self.append_credentials_excluding_secrets(
            credential_set_id,
            secrets,
            batch_id,
            HashSet::new(),
        )
    }

    pub fn append_credentials_excluding_secrets(
        &self,
        credential_set_id: &CredentialSetId,
        secrets: Vec<String>,
        batch_id: impl Into<String>,
        excluded_secrets: HashSet<String>,
    ) -> anyhow::Result<KeyImport> {
        let batch_id = batch_id.into();
        let mut connection = self.connect()?;
        initialize_schema(&connection)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            credential_set_metadata_exists(&tx, credential_set_id)?,
            "unknown credential set {}",
            credential_set_id.0
        );
        let mut batch_seen = HashMap::new();
        let mut credentials = Vec::new();
        let mut duplicate_fingerprints = Vec::new();
        let mut non_empty_count = 0usize;
        let mut ignored_empty_count = 0usize;
        let mut next_position = next_credential_position(&tx, credential_set_id)?;
        for (offset, raw) in secrets.into_iter().enumerate() {
            let source_line = offset + 1;
            let secret = raw.trim().to_string();
            if secret.is_empty() {
                ignored_empty_count += 1;
                continue;
            }
            non_empty_count += 1;
            if let Some(first_source_line) = batch_seen.get(&secret).copied() {
                duplicate_fingerprints.push(DuplicateCredentialOccurrence {
                    first_source_line,
                    duplicate_source_line: source_line,
                    fingerprint: persistent_fingerprint(&secret),
                });
                continue;
            }
            if let Some(first_source_line) =
                existing_secret_source_line(&tx, credential_set_id, &secret)?
            {
                touch_existing_credential_last_seen(&tx, credential_set_id, &secret)?;
                duplicate_fingerprints.push(DuplicateCredentialOccurrence {
                    first_source_line,
                    duplicate_source_line: source_line,
                    fingerprint: persistent_fingerprint(&secret),
                });
                continue;
            }
            if excluded_secrets.contains(&secret) {
                duplicate_fingerprints.push(DuplicateCredentialOccurrence {
                    first_source_line: source_line,
                    duplicate_source_line: source_line,
                    fingerprint: persistent_fingerprint(&secret),
                });
                continue;
            }
            batch_seen.insert(secret.clone(), source_line);
            let imported = ImportedCredential {
                secret,
                source: CredentialSource {
                    source_path: None,
                    source_line: Some(source_line),
                    batch_id: Some(batch_id.clone()),
                },
            };
            insert_sqlite_credential(&tx, credential_set_id, &imported, next_position)?;
            next_position += 1;
            credentials.push(imported);
        }
        update_sqlite_import_counts(
            &tx,
            credential_set_id,
            non_empty_count,
            ignored_empty_count,
            credentials.len(),
            duplicate_fingerprints.len(),
        )?;
        insert_import_batch(
            &tx,
            credential_set_id,
            &batch_id,
            CredentialImportSourceKind::ManagementApi,
            None,
            ImportBatchCounts {
                physical_line_count: non_empty_count + ignored_empty_count,
                non_empty_count,
                unique_count: credentials.len(),
                duplicate_occurrence_count: duplicate_fingerprints.len(),
                ignored_empty_count,
                invalid_line_count: 0,
            },
        )?;
        let set_report = load_sqlite_credential_set_report(&tx, credential_set_id)?;
        tx.commit()?;
        let imported_count = credentials.len();
        Ok(KeyImport {
            credentials,
            report: KeyImportReport {
                source_path: PathBuf::from(batch_id),
                physical_line_count: non_empty_count + ignored_empty_count,
                non_empty_count,
                unique_count: imported_count,
                duplicate_occurrence_count: duplicate_fingerprints.len(),
                ignored_empty_count,
                invalid_line_count: 0,
                claimed_count: None,
                claim_source: None,
                import_generation: set_report.import_generation,
                last_imported_at_unix_seconds: set_report.last_imported_at_unix_seconds,
                duplicate_fingerprints,
            },
        })
    }

    pub fn load_lifecycle_snapshots(
        &self,
        credential_set_id: &CredentialSetId,
    ) -> anyhow::Result<Vec<CredentialLifecycleSnapshot>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_lifecycle_snapshots(&connection, credential_set_id)
    }

    pub fn load_lifecycle_history(
        &self,
        credential_set_id: &CredentialSetId,
        credential_id: &CredentialId,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<Vec<CredentialLifecycleHistoryRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_lifecycle_history(&connection, credential_set_id, credential_id, offset, limit)
    }

    pub fn load_import_batches(
        &self,
        credential_set_id: &CredentialSetId,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<Vec<CredentialImportBatchRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_import_batches(&connection, credential_set_id, offset, limit)
    }

    pub fn load_import_batch(
        &self,
        credential_set_id: &CredentialSetId,
        batch_id: &str,
    ) -> anyhow::Result<Option<CredentialImportBatchRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_import_batch(&connection, credential_set_id, batch_id)
    }

    pub fn load_credential_resource(
        &self,
        credential_set_id: &CredentialSetId,
        credential_id: &CredentialId,
    ) -> anyhow::Result<Option<CredentialResourceRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_credential_resource(&connection, credential_set_id, credential_id)
    }

    pub fn load_credential_resource_by_position(
        &self,
        credential_set_id: &CredentialSetId,
        position: usize,
    ) -> anyhow::Result<Option<CredentialResourceRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_credential_resource_by_position(&connection, credential_set_id, position)
    }

    pub fn load_credential_positions_for_credentials(
        &self,
        credential_set_id: &CredentialSetId,
        credential_ids: &[CredentialId],
    ) -> anyhow::Result<HashMap<CredentialId, usize>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_credential_positions_for_credentials(
            &connection,
            credential_set_id,
            credential_ids,
        )
    }

    pub fn update_credential_operator_metadata(
        &self,
        credential_set_id: &CredentialSetId,
        credential_id: &CredentialId,
        label: Option<String>,
        note: Option<String>,
    ) -> anyhow::Result<CredentialResourceRecord> {
        let mut connection = self.connect()?;
        initialize_schema(&connection)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            sqlite_credential_id_exists(&tx, credential_set_id, credential_id)?,
            "unknown credential {} in credential set {}",
            credential_id.0,
            credential_set_id.0
        );
        update_sqlite_credential_operator_metadata(
            &tx,
            credential_set_id,
            credential_id,
            label,
            note,
        )?;
        let record = load_sqlite_credential_resource(&tx, credential_set_id, credential_id)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "updated credential {} in credential set {} was not found",
                    credential_id.0,
                    credential_set_id.0
                )
            })?;
        tx.commit()?;
        Ok(record)
    }

    pub fn record_probe_result(
        &self,
        input: CredentialProbeResultRecordInput,
    ) -> anyhow::Result<CredentialProbeResultRecord> {
        let mut connection = self.connect()?;
        initialize_schema(&connection)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            sqlite_credential_id_exists(&tx, &input.credential_set_id, &input.credential_id)?,
            "unknown credential {} in credential set {}",
            input.credential_id.0,
            input.credential_set_id.0
        );
        let record = insert_probe_result(&tx, input)?;
        tx.commit()?;
        Ok(record)
    }

    pub fn load_probe_results(
        &self,
        credential_set_id: &CredentialSetId,
        credential_id: &CredentialId,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<Vec<CredentialProbeResultRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_probe_results(&connection, credential_set_id, credential_id, offset, limit)
    }

    pub fn load_latest_probe_result(
        &self,
        credential_set_id: &CredentialSetId,
        credential_id: &CredentialId,
    ) -> anyhow::Result<Option<CredentialProbeResultRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_latest_sqlite_probe_result(&connection, credential_set_id, credential_id)
    }

    pub fn load_latest_probe_results_for_credentials(
        &self,
        credential_set_id: &CredentialSetId,
        credential_ids: &[CredentialId],
    ) -> anyhow::Result<HashMap<CredentialId, CredentialProbeResultRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_latest_sqlite_probe_results_for_credentials(
            &connection,
            credential_set_id,
            credential_ids,
        )
    }

    pub fn load_latest_probe_outcome_credential_ids(
        &self,
        credential_set_id: &CredentialSetId,
        outcome: CredentialProbeOutcome,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<CredentialProbeOutcomePage> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_latest_sqlite_probe_outcome_credential_ids(
            &connection,
            credential_set_id,
            outcome,
            offset,
            limit,
        )
    }

    pub fn load_unprobed_credential_ids(
        &self,
        credential_set_id: &CredentialSetId,
        offset: usize,
        limit: usize,
    ) -> anyhow::Result<CredentialIdPage> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_unprobed_credential_ids(&connection, credential_set_id, offset, limit)
    }

    pub fn load_latest_probe_result_for_credential_set(
        &self,
        credential_set_id: &CredentialSetId,
    ) -> anyhow::Result<Option<CredentialProbeResultRecord>> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_latest_sqlite_probe_result_for_credential_set(&connection, credential_set_id)
    }

    pub fn load_probe_summary_for_credential_set(
        &self,
        credential_set_id: &CredentialSetId,
    ) -> anyhow::Result<CredentialProbeSummaryRecord> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_sqlite_probe_summary_for_credential_set(&connection, credential_set_id)
    }

    pub fn persist_lifecycle_update_with_evidence(
        &self,
        update: CredentialLifecycleUpdate,
        evidence: CredentialLifecycleEvidence,
    ) -> anyhow::Result<CredentialLifecyclePersistedUpdate> {
        let mut connection = self.connect()?;
        initialize_schema(&connection)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            sqlite_credential_id_exists(&tx, &update.credential_set_id, &update.credential_id)?,
            "unknown credential {} in credential set {}",
            update.credential_id.0,
            update.credential_set_id.0
        );
        let history_id = persist_sqlite_lifecycle_update(&tx, &update, &evidence)?;
        tx.commit()?;
        Ok(CredentialLifecyclePersistedUpdate {
            snapshot: CredentialLifecycleSnapshot {
                credential_set_id: update.credential_set_id,
                credential_id: update.credential_id,
                state: update.state,
            },
            history_id,
        })
    }

    pub fn persist_lifecycle_snapshot(
        &self,
        update: CredentialLifecycleUpdate,
        stale_history_id: Option<i64>,
    ) -> anyhow::Result<CredentialLifecycleSnapshot> {
        let mut connection = self.connect()?;
        initialize_schema(&connection)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(
            sqlite_credential_id_exists(&tx, &update.credential_set_id, &update.credential_id)?,
            "unknown credential {} in credential set {}",
            update.credential_id.0,
            update.credential_set_id.0
        );
        persist_sqlite_lifecycle_snapshot(&tx, &update)?;
        if let Some(history_id) = stale_history_id {
            tx.execute(
                r#"
                DELETE FROM credential_lifecycle_history
                WHERE id = ?1 AND credential_set_id = ?2 AND credential_id = ?3
                "#,
                params![
                    history_id,
                    update.credential_set_id.0,
                    update.credential_id.0
                ],
            )?;
        }
        tx.commit()?;
        Ok(CredentialLifecycleSnapshot {
            credential_set_id: update.credential_set_id,
            credential_id: update.credential_id,
            state: update.state,
        })
    }
}

impl CredentialRepository for SqliteCredentialRepository {
    fn load_credential_set(
        &self,
        credential_set_id: &CredentialSetId,
        source: &CredentialSetSource,
    ) -> anyhow::Result<KeyImport> {
        let CredentialSetSource::File { path } = source;
        let mut connection = self.connect()?;
        initialize_schema(&connection)?;
        if credential_set_has_rows(&connection, credential_set_id)? {
            return load_from_sqlite(&connection, credential_set_id);
        }

        let imported = import_raw_credentials(
            fs::read_to_string(path)?,
            CredentialImportOrigin {
                credential_set_id: credential_set_id.clone(),
                source_path: path.clone(),
                claimed_count: claimed_count_from_path(path),
                claim_source: claimed_count_from_path(path).map(|_| "filename".to_string()),
            },
        )?;
        persist_import(&mut connection, credential_set_id, &imported)?;
        Ok(imported)
    }
}

#[derive(Debug, Clone)]
struct CredentialImportOrigin {
    credential_set_id: CredentialSetId,
    source_path: PathBuf,
    claimed_count: Option<usize>,
    claim_source: Option<String>,
}

fn initialize_schema(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS credential_sets (
            id TEXT PRIMARY KEY,
            source_path TEXT NOT NULL,
            physical_line_count INTEGER NOT NULL,
            non_empty_count INTEGER NOT NULL,
            unique_count INTEGER NOT NULL,
            duplicate_occurrence_count INTEGER NOT NULL,
            ignored_empty_count INTEGER NOT NULL,
            invalid_line_count INTEGER NOT NULL,
            claimed_count INTEGER,
            claim_source TEXT,
            import_generation INTEGER NOT NULL,
            last_imported_at_unix_seconds INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY,
            applied_at_unix_seconds INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS credentials (
            credential_set_id TEXT NOT NULL,
            credential_id TEXT,
            secret TEXT NOT NULL,
            source_path TEXT,
            source_line INTEGER,
            batch_id TEXT,
            fingerprint TEXT NOT NULL,
            label TEXT,
            note TEXT,
            position INTEGER NOT NULL,
            first_imported_at_unix_seconds INTEGER NOT NULL,
            last_seen_at_unix_seconds INTEGER NOT NULL,
            PRIMARY KEY (credential_set_id, fingerprint),
            FOREIGN KEY (credential_set_id)
                REFERENCES credential_sets(id)
                ON DELETE CASCADE
        );

        CREATE UNIQUE INDEX IF NOT EXISTS credentials_set_position_unique
            ON credentials(credential_set_id, position);

        CREATE INDEX IF NOT EXISTS credentials_set_secret_lookup
            ON credentials(credential_set_id, secret);

        CREATE TABLE IF NOT EXISTS credential_lifecycle_states (
            credential_set_id TEXT NOT NULL,
            credential_id TEXT NOT NULL,
            state_kind TEXT NOT NULL CHECK (state_kind IN ('expired', 'quota_exhausted', 'disabled')),
            reason TEXT NOT NULL,
            updated_at_unix_seconds INTEGER NOT NULL,
            PRIMARY KEY (credential_set_id, credential_id),
            FOREIGN KEY (credential_set_id)
                REFERENCES credential_sets(id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS credential_lifecycle_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            credential_set_id TEXT NOT NULL,
            credential_id TEXT NOT NULL,
            state_kind TEXT NOT NULL CHECK (state_kind IN ('available', 'expired', 'quota_exhausted', 'disabled')),
            reason TEXT,
            source TEXT NOT NULL CHECK (source IN ('management_command', 'compensation', 'automatic_failure')),
            reason_class TEXT,
            actor_id TEXT,
            actor_name TEXT,
            actor_role TEXT,
            channel_id TEXT,
            created_at_unix_seconds INTEGER NOT NULL,
            FOREIGN KEY (credential_set_id)
                REFERENCES credential_sets(id)
                ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS credential_lifecycle_history_lookup
            ON credential_lifecycle_history(credential_set_id, credential_id, id);

        CREATE TABLE IF NOT EXISTS credential_import_batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            credential_set_id TEXT NOT NULL,
            batch_id TEXT NOT NULL,
            source_kind TEXT NOT NULL CHECK (source_kind IN ('file_bootstrap', 'management_api')),
            source_ref TEXT,
            physical_line_count INTEGER NOT NULL,
            non_empty_count INTEGER NOT NULL,
            unique_count INTEGER NOT NULL,
            duplicate_occurrence_count INTEGER NOT NULL,
            ignored_empty_count INTEGER NOT NULL,
            invalid_line_count INTEGER NOT NULL,
            created_at_unix_seconds INTEGER NOT NULL,
            FOREIGN KEY (credential_set_id)
                REFERENCES credential_sets(id)
                ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS credential_import_batches_set_created
            ON credential_import_batches(credential_set_id, created_at_unix_seconds, id);

        CREATE TABLE IF NOT EXISTS credential_probe_results (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            credential_set_id TEXT NOT NULL,
            credential_id TEXT NOT NULL,
            channel_id TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            outcome TEXT NOT NULL CHECK (
                outcome IN (
                    'success',
                    'invalid',
                    'quota_exhausted',
                    'rate_limited',
                    'provider_unavailable',
                    'unsupported_model',
                    'unknown'
                )
            ),
            classifier_id TEXT,
            adaptation_rule_id TEXT,
            upstream_status INTEGER,
            upstream_code TEXT,
            upstream_limit_type TEXT,
            latency_ms INTEGER NOT NULL,
            created_at_unix_seconds INTEGER NOT NULL,
            FOREIGN KEY (credential_set_id)
                REFERENCES credential_sets(id)
                ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS credential_probe_results_lookup
            ON credential_probe_results(credential_set_id, credential_id, id);

        CREATE INDEX IF NOT EXISTS credential_probe_results_set_latest
            ON credential_probe_results(credential_set_id, id);

        "#,
    )?;
    ensure_column(
        connection,
        "credential_lifecycle_history",
        "reason_class",
        "TEXT",
    )?;
    ensure_column(
        connection,
        "credential_lifecycle_history",
        "actor_id",
        "TEXT",
    )?;
    ensure_column(
        connection,
        "credential_lifecycle_history",
        "actor_name",
        "TEXT",
    )?;
    ensure_column(
        connection,
        "credential_lifecycle_history",
        "actor_role",
        "TEXT",
    )?;
    ensure_column(
        connection,
        "credential_lifecycle_history",
        "channel_id",
        "TEXT",
    )?;
    ensure_column(
        connection,
        "credentials",
        "first_imported_at_unix_seconds",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        connection,
        "credentials",
        "last_seen_at_unix_seconds",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(connection, "credentials", "label", "TEXT")?;
    ensure_column(connection, "credentials", "note", "TEXT")?;
    ensure_column(connection, "credentials", "credential_id", "TEXT")?;
    run_sqlite_migration_once(
        connection,
        "credentials_credential_id_backfill",
        backfill_sqlite_credential_ids,
    )?;
    connection.execute(
        "CREATE INDEX IF NOT EXISTS credentials_set_id_lookup ON credentials(credential_set_id, credential_id)",
        [],
    )?;
    ensure_column(
        connection,
        "credential_sets",
        "import_generation",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    ensure_column(
        connection,
        "credential_sets",
        "last_imported_at_unix_seconds",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    backfill_credential_set_import_metadata(connection)?;
    backfill_credential_import_timestamps(connection)?;
    ensure_lifecycle_schema_accepts_quota_exhausted(connection)?;
    Ok(())
}

fn backfill_credential_set_import_metadata(connection: &Connection) -> anyhow::Result<()> {
    let now = unix_timestamp_seconds()?;
    connection.execute(
        r#"
        UPDATE credential_sets
        SET last_imported_at_unix_seconds = ?1
        WHERE last_imported_at_unix_seconds = 0
        "#,
        params![now],
    )?;
    Ok(())
}

fn run_sqlite_migration_once(
    connection: &Connection,
    name: &str,
    migration: impl FnOnce(&Connection) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let already_applied: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE name = ?1)",
        params![name],
        |row| row.get::<_, bool>(0),
    )?;
    if already_applied {
        return Ok(());
    }

    migration(connection)?;
    connection.execute(
        "INSERT OR IGNORE INTO schema_migrations (name, applied_at_unix_seconds) VALUES (?1, ?2)",
        params![name, unix_timestamp_seconds()?],
    )?;
    Ok(())
}

fn backfill_sqlite_credential_ids(connection: &Connection) -> anyhow::Result<()> {
    let mut statement = connection.prepare(
        r#"
        SELECT credential_set_id, secret
        FROM credentials
        WHERE credential_id IS NULL OR credential_id = ''
        "#,
    )?;
    let missing = statement
        .query_map([], |row| {
            Ok((
                CredentialSetId(row.get::<_, String>(0)?),
                row.get::<_, String>(1)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    for (credential_set_id, secret) in missing {
        let credential_id = credential_id_for_secret(&credential_set_id, &secret);
        connection.execute(
            r#"
            UPDATE credentials
            SET credential_id = ?3
            WHERE credential_set_id = ?1 AND secret = ?2
            "#,
            params![credential_set_id.0, secret, credential_id.0],
        )?;
    }
    Ok(())
}

fn backfill_credential_import_timestamps(connection: &Connection) -> anyhow::Result<()> {
    let now = unix_timestamp_seconds()?;
    connection.execute(
        r#"
        UPDATE credentials
        SET first_imported_at_unix_seconds = ?1
        WHERE first_imported_at_unix_seconds = 0
        "#,
        params![now],
    )?;
    connection.execute(
        r#"
        UPDATE credentials
        SET last_seen_at_unix_seconds = first_imported_at_unix_seconds
        WHERE last_seen_at_unix_seconds = 0
        "#,
        [],
    )?;
    Ok(())
}

fn ensure_lifecycle_schema_accepts_quota_exhausted(connection: &Connection) -> anyhow::Result<()> {
    if !table_sql_contains(connection, "credential_lifecycle_states", "quota_exhausted")? {
        connection.execute_batch(
            r#"
            ALTER TABLE credential_lifecycle_states RENAME TO credential_lifecycle_states_old;
            CREATE TABLE credential_lifecycle_states (
                credential_set_id TEXT NOT NULL,
                credential_id TEXT NOT NULL,
                state_kind TEXT NOT NULL CHECK (state_kind IN ('expired', 'quota_exhausted', 'disabled')),
                reason TEXT NOT NULL,
                updated_at_unix_seconds INTEGER NOT NULL,
                PRIMARY KEY (credential_set_id, credential_id),
                FOREIGN KEY (credential_set_id)
                    REFERENCES credential_sets(id)
                    ON DELETE CASCADE
            );
            INSERT INTO credential_lifecycle_states (
                credential_set_id,
                credential_id,
                state_kind,
                reason,
                updated_at_unix_seconds
            )
            SELECT
                credential_set_id,
                credential_id,
                state_kind,
                reason,
                updated_at_unix_seconds
            FROM credential_lifecycle_states_old;
            DROP TABLE credential_lifecycle_states_old;
            "#,
        )?;
    }
    if !table_sql_contains(
        connection,
        "credential_lifecycle_history",
        "quota_exhausted",
    )? {
        connection.execute_batch(
            r#"
            ALTER TABLE credential_lifecycle_history RENAME TO credential_lifecycle_history_old;
            CREATE TABLE credential_lifecycle_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                credential_set_id TEXT NOT NULL,
                credential_id TEXT NOT NULL,
                state_kind TEXT NOT NULL CHECK (state_kind IN ('available', 'expired', 'quota_exhausted', 'disabled')),
                reason TEXT,
                source TEXT NOT NULL CHECK (source IN ('management_command', 'compensation', 'automatic_failure')),
                reason_class TEXT,
                actor_id TEXT,
                actor_name TEXT,
                actor_role TEXT,
                channel_id TEXT,
                created_at_unix_seconds INTEGER NOT NULL,
                FOREIGN KEY (credential_set_id)
                    REFERENCES credential_sets(id)
                    ON DELETE CASCADE
            );
            INSERT INTO credential_lifecycle_history (
                id,
                credential_set_id,
                credential_id,
                state_kind,
                reason,
                source,
                reason_class,
                actor_id,
                actor_name,
                actor_role,
                channel_id,
                created_at_unix_seconds
            )
            SELECT
                id,
                credential_set_id,
                credential_id,
                state_kind,
                reason,
                source,
                reason_class,
                actor_id,
                actor_name,
                actor_role,
                channel_id,
                created_at_unix_seconds
            FROM credential_lifecycle_history_old;
            DROP TABLE credential_lifecycle_history_old;
            "#,
        )?;
    }
    Ok(())
}

fn table_sql_contains(connection: &Connection, table: &str, needle: &str) -> anyhow::Result<bool> {
    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )
        .optional()?;
    Ok(sql.is_some_and(|sql| sql.contains(needle)))
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|existing| existing == column);
    if !exists {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn credential_set_has_rows(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
) -> anyhow::Result<bool> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM credentials WHERE credential_set_id = ?1",
        params![credential_set_id.0],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn credential_set_metadata_exists(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
) -> anyhow::Result<bool> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM credential_sets WHERE id = ?1",
            params![credential_set_id.0],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn existing_secret_source_line(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    secret: &str,
) -> anyhow::Result<Option<usize>> {
    let source_line = connection
        .query_row(
            r#"
        SELECT COALESCE(source_line, position + 1)
        FROM credentials
        WHERE credential_set_id = ?1 AND secret = ?2
        LIMIT 1
        "#,
            params![credential_set_id.0, secret],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(source_line.map(|line| line as usize))
}

fn touch_existing_credential_last_seen(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    secret: &str,
) -> anyhow::Result<()> {
    connection.execute(
        r#"
        UPDATE credentials
        SET last_seen_at_unix_seconds = ?3
        WHERE credential_set_id = ?1 AND secret = ?2
        "#,
        params![credential_set_id.0, secret, unix_timestamp_seconds()?],
    )?;
    Ok(())
}

fn update_sqlite_credential_operator_metadata(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential_id: &CredentialId,
    label: Option<String>,
    note: Option<String>,
) -> anyhow::Result<()> {
    let mut statement = connection.prepare(
        r#"
        SELECT secret
        FROM credentials
        WHERE credential_set_id = ?1
        "#,
    )?;
    let mut rows = statement.query(params![credential_set_id.0])?;
    while let Some(row) = rows.next()? {
        let secret: String = row.get(0)?;
        if credential_id_for_secret(credential_set_id, &secret) != *credential_id {
            continue;
        }
        connection.execute(
            r#"
            UPDATE credentials
            SET label = ?3, note = ?4
            WHERE credential_set_id = ?1 AND secret = ?2
            "#,
            params![
                credential_set_id.0,
                secret,
                label.as_deref(),
                note.as_deref()
            ],
        )?;
        return Ok(());
    }
    anyhow::bail!(
        "unknown credential {} in credential set {}",
        credential_id.0,
        credential_set_id.0
    )
}

fn next_credential_position(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
) -> anyhow::Result<usize> {
    let next: i64 = connection.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM credentials WHERE credential_set_id = ?1",
        params![credential_set_id.0],
        |row| row.get(0),
    )?;
    Ok(next as usize)
}

fn insert_sqlite_credential(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential: &ImportedCredential,
    position: usize,
) -> anyhow::Result<()> {
    let imported_at = unix_timestamp_seconds()?;
    connection.execute(
        r#"
        INSERT INTO credentials (
            credential_set_id,
            credential_id,
            secret,
            source_path,
            source_line,
            batch_id,
            fingerprint,
            label,
            note,
            position,
            first_imported_at_unix_seconds,
            last_seen_at_unix_seconds
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?9, ?9)
        "#,
        params![
            credential_set_id.0,
            credential_id_for_secret(credential_set_id, &credential.secret).0,
            credential.secret,
            credential
                .source
                .source_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string()),
            credential.source.source_line.map(|line| line as i64),
            credential.source.batch_id.as_deref(),
            persistent_fingerprint(&credential.secret),
            position as i64,
            imported_at,
        ],
    )?;
    Ok(())
}

fn sqlite_credential_id_exists(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential_id: &CredentialId,
) -> anyhow::Result<bool> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM credentials WHERE credential_set_id = ?1 AND credential_id = ?2",
        params![credential_set_id.0, credential_id.0],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn load_sqlite_lifecycle_snapshots(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
) -> anyhow::Result<Vec<CredentialLifecycleSnapshot>> {
    let mut lifecycle_statement = connection.prepare(
        r#"
        SELECT credential_id, state_kind, reason
        FROM credential_lifecycle_states
        WHERE credential_set_id = ?1
        "#,
    )?;
    let mut lifecycle_rows = lifecycle_statement
        .query_map(params![credential_set_id.0], |row| {
            let credential_id: String = row.get(0)?;
            let state_kind: String = row.get(1)?;
            let reason: Option<String> = row.get(2)?;
            Ok((CredentialId(credential_id), state_kind, reason))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut credential_statement = connection.prepare(
        r#"
        SELECT secret
        FROM credentials
        WHERE credential_set_id = ?1
        ORDER BY position ASC
        "#,
    )?;
    let credential_ids = credential_statement
        .query_map(params![credential_set_id.0], |row| {
            let secret: String = row.get(0)?;
            Ok(credential_id_for_secret(credential_set_id, &secret))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut lifecycle_by_id = HashMap::with_capacity(lifecycle_rows.len());
    for (credential_id, state_kind, reason) in lifecycle_rows.drain(..) {
        lifecycle_by_id.insert(credential_id, (state_kind, reason));
    }

    let mut snapshots = Vec::with_capacity(lifecycle_by_id.len());
    for credential_id in credential_ids {
        let Some((state_kind, reason)) = lifecycle_by_id.remove(&credential_id) else {
            continue;
        };
        let state = match state_kind.as_str() {
            "expired" => CredentialLifecycleState::Expired {
                reason: reason.unwrap_or_default(),
            },
            "quota_exhausted" => CredentialLifecycleState::QuotaExhausted {
                reason: reason.unwrap_or_default(),
            },
            "disabled" => CredentialLifecycleState::Disabled {
                reason: reason.unwrap_or_default(),
            },
            other => anyhow::bail!(
                "unknown credential lifecycle state {other} for credential {}",
                credential_id.0
            ),
        };
        snapshots.push(CredentialLifecycleSnapshot {
            credential_set_id: credential_set_id.clone(),
            credential_id,
            state,
        });
    }
    if let Some(orphan_id) = lifecycle_by_id.keys().next() {
        anyhow::bail!(
            "credential lifecycle state references unknown credential {} in credential set {}",
            orphan_id.0,
            credential_set_id.0
        );
    }
    Ok(snapshots)
}

fn persist_sqlite_lifecycle_update(
    connection: &Connection,
    update: &CredentialLifecycleUpdate,
    evidence: &CredentialLifecycleEvidence,
) -> anyhow::Result<i64> {
    persist_sqlite_lifecycle_snapshot(connection, update)?;
    insert_sqlite_lifecycle_history(connection, update, evidence)
}

fn persist_sqlite_lifecycle_snapshot(
    connection: &Connection,
    update: &CredentialLifecycleUpdate,
) -> anyhow::Result<()> {
    match &update.state {
        CredentialLifecycleState::Available => {
            connection.execute(
                r#"
                DELETE FROM credential_lifecycle_states
                WHERE credential_set_id = ?1 AND credential_id = ?2
                "#,
                params![update.credential_set_id.0, update.credential_id.0],
            )?;
        }
        CredentialLifecycleState::Expired { reason } => {
            upsert_sqlite_lifecycle_state(connection, update, "expired", reason)?;
        }
        CredentialLifecycleState::QuotaExhausted { reason } => {
            upsert_sqlite_lifecycle_state(connection, update, "quota_exhausted", reason)?;
        }
        CredentialLifecycleState::Disabled { reason } => {
            upsert_sqlite_lifecycle_state(connection, update, "disabled", reason)?;
        }
    }
    Ok(())
}

fn load_sqlite_lifecycle_history(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential_id: &CredentialId,
    offset: usize,
    limit: usize,
) -> anyhow::Result<Vec<CredentialLifecycleHistoryRecord>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            credential_set_id,
            credential_id,
            state_kind,
            reason,
            source,
            reason_class,
            actor_id,
            actor_name,
            actor_role,
            channel_id,
            created_at_unix_seconds
        FROM credential_lifecycle_history
        WHERE credential_set_id = ?1 AND credential_id = ?2
        ORDER BY id DESC
        LIMIT ?3 OFFSET ?4
        "#,
    )?;
    let records = statement
        .query_map(
            params![
                credential_set_id.0,
                credential_id.0,
                limit as i64,
                offset as i64
            ],
            lifecycle_history_record_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(records)
}

fn insert_sqlite_lifecycle_history(
    connection: &Connection,
    update: &CredentialLifecycleUpdate,
    evidence: &CredentialLifecycleEvidence,
) -> anyhow::Result<i64> {
    let (state_kind, reason) = lifecycle_state_parts(&update.state);
    connection.execute(
        r#"
        INSERT INTO credential_lifecycle_history (
            credential_set_id,
            credential_id,
            state_kind,
            reason,
            source,
            reason_class,
            actor_id,
            actor_name,
            actor_role,
            channel_id,
            created_at_unix_seconds
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            update.credential_set_id.0,
            update.credential_id.0,
            state_kind,
            reason,
            lifecycle_history_source_to_str(evidence.source.clone()),
            evidence.reason_class.as_deref(),
            evidence.actor_id.as_deref(),
            evidence.actor_name.as_deref(),
            evidence.actor_role.as_deref(),
            evidence.channel_id.as_deref(),
            unix_timestamp_seconds()?,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

fn upsert_sqlite_lifecycle_state(
    connection: &Connection,
    update: &CredentialLifecycleUpdate,
    state_kind: &str,
    reason: &str,
) -> anyhow::Result<()> {
    connection.execute(
        r#"
        INSERT INTO credential_lifecycle_states (
            credential_set_id,
            credential_id,
            state_kind,
            reason,
            updated_at_unix_seconds
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(credential_set_id, credential_id) DO UPDATE SET
            state_kind = excluded.state_kind,
            reason = excluded.reason,
            updated_at_unix_seconds = excluded.updated_at_unix_seconds
        "#,
        params![
            update.credential_set_id.0,
            update.credential_id.0,
            state_kind,
            reason,
            unix_timestamp_seconds()?,
        ],
    )?;
    Ok(())
}

fn lifecycle_state_parts(state: &CredentialLifecycleState) -> (&'static str, Option<&str>) {
    match state {
        CredentialLifecycleState::Available => ("available", None),
        CredentialLifecycleState::Expired { reason } => ("expired", Some(reason.as_str())),
        CredentialLifecycleState::QuotaExhausted { reason } => {
            ("quota_exhausted", Some(reason.as_str()))
        }
        CredentialLifecycleState::Disabled { reason } => ("disabled", Some(reason.as_str())),
    }
}

fn lifecycle_state_from_parts(
    state_kind: &str,
    reason: Option<String>,
) -> anyhow::Result<CredentialLifecycleState> {
    match state_kind {
        "available" => Ok(CredentialLifecycleState::Available),
        "expired" => Ok(CredentialLifecycleState::Expired {
            reason: reason.unwrap_or_default(),
        }),
        "quota_exhausted" => Ok(CredentialLifecycleState::QuotaExhausted {
            reason: reason.unwrap_or_default(),
        }),
        "disabled" => Ok(CredentialLifecycleState::Disabled {
            reason: reason.unwrap_or_default(),
        }),
        other => anyhow::bail!("unknown credential lifecycle state {other}"),
    }
}

fn lifecycle_history_source_to_str(source: CredentialLifecycleHistorySource) -> &'static str {
    match source {
        CredentialLifecycleHistorySource::ManagementCommand => "management_command",
        CredentialLifecycleHistorySource::Compensation => "compensation",
        CredentialLifecycleHistorySource::AutomaticFailure => "automatic_failure",
    }
}

fn lifecycle_history_source_from_str(
    raw: &str,
) -> anyhow::Result<CredentialLifecycleHistorySource> {
    match raw {
        "management_command" => Ok(CredentialLifecycleHistorySource::ManagementCommand),
        "compensation" => Ok(CredentialLifecycleHistorySource::Compensation),
        "automatic_failure" => Ok(CredentialLifecycleHistorySource::AutomaticFailure),
        other => anyhow::bail!("unknown credential lifecycle history source {other}"),
    }
}

fn lifecycle_history_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CredentialLifecycleHistoryRecord> {
    let state_kind: String = row.get(2)?;
    let reason: Option<String> = row.get(3)?;
    let source: String = row.get(4)?;
    let state = lifecycle_state_from_parts(&state_kind, reason).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let source = lifecycle_history_source_from_str(&source).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    Ok(CredentialLifecycleHistoryRecord {
        credential_set_id: CredentialSetId(row.get(0)?),
        credential_id: CredentialId(row.get(1)?),
        state,
        source,
        reason_class: row.get(5)?,
        actor_id: row.get(6)?,
        actor_name: row.get(7)?,
        actor_role: row.get(8)?,
        channel_id: row.get(9)?,
        created_at_unix_seconds: row.get(10)?,
    })
}

fn credential_id_for_secret(credential_set_id: &CredentialSetId, secret: &str) -> CredentialId {
    CredentialId(format!(
        "cred_{}",
        short_hash(&format!("{}:{secret}", credential_set_id.0))
    ))
}

fn unix_timestamp_seconds() -> anyhow::Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

#[derive(Debug, Clone, Copy)]
struct ImportBatchCounts {
    physical_line_count: usize,
    non_empty_count: usize,
    unique_count: usize,
    duplicate_occurrence_count: usize,
    ignored_empty_count: usize,
    invalid_line_count: usize,
}

fn import_source_kind_to_str(kind: CredentialImportSourceKind) -> &'static str {
    match kind {
        CredentialImportSourceKind::FileBootstrap => "file_bootstrap",
        CredentialImportSourceKind::ManagementApi => "management_api",
    }
}

fn import_source_kind_from_str(raw: &str) -> anyhow::Result<CredentialImportSourceKind> {
    match raw {
        "file_bootstrap" => Ok(CredentialImportSourceKind::FileBootstrap),
        "management_api" => Ok(CredentialImportSourceKind::ManagementApi),
        other => anyhow::bail!("unknown credential import source kind {other}"),
    }
}

fn redacted_source_ref(path: &std::path::Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
}

fn insert_import_batch(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    batch_id: &str,
    source_kind: CredentialImportSourceKind,
    source_ref: Option<&str>,
    counts: ImportBatchCounts,
) -> anyhow::Result<()> {
    connection.execute(
        r#"
        INSERT INTO credential_import_batches (
            credential_set_id,
            batch_id,
            source_kind,
            source_ref,
            physical_line_count,
            non_empty_count,
            unique_count,
            duplicate_occurrence_count,
            ignored_empty_count,
            invalid_line_count,
            created_at_unix_seconds
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            credential_set_id.0,
            batch_id,
            import_source_kind_to_str(source_kind),
            source_ref,
            counts.physical_line_count as i64,
            counts.non_empty_count as i64,
            counts.unique_count as i64,
            counts.duplicate_occurrence_count as i64,
            counts.ignored_empty_count as i64,
            counts.invalid_line_count as i64,
            unix_timestamp_seconds()?,
        ],
    )?;
    Ok(())
}

fn load_sqlite_import_batches(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    offset: usize,
    limit: usize,
) -> anyhow::Result<Vec<CredentialImportBatchRecord>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            credential_set_id,
            batch_id,
            source_kind,
            source_ref,
            physical_line_count,
            non_empty_count,
            unique_count,
            duplicate_occurrence_count,
            ignored_empty_count,
            invalid_line_count,
            created_at_unix_seconds
        FROM credential_import_batches
        WHERE credential_set_id = ?1
        ORDER BY created_at_unix_seconds DESC, id DESC
        LIMIT ?2 OFFSET ?3
        "#,
    )?;
    let rows = statement
        .query_map(
            params![credential_set_id.0, limit as i64, offset as i64],
            import_batch_record_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn load_sqlite_import_batch(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    batch_id: &str,
) -> anyhow::Result<Option<CredentialImportBatchRecord>> {
    connection
        .query_row(
            r#"
            SELECT
                credential_set_id,
                batch_id,
                source_kind,
                source_ref,
                physical_line_count,
                non_empty_count,
                unique_count,
                duplicate_occurrence_count,
                ignored_empty_count,
                invalid_line_count,
                created_at_unix_seconds
            FROM credential_import_batches
            WHERE credential_set_id = ?1 AND batch_id = ?2
            ORDER BY created_at_unix_seconds DESC, id DESC
            LIMIT 1
            "#,
            params![credential_set_id.0, batch_id],
            import_batch_record_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn import_batch_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CredentialImportBatchRecord> {
    let source_kind: String = row.get(2)?;
    let source_kind = import_source_kind_from_str(&source_kind).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    Ok(CredentialImportBatchRecord {
        credential_set_id: CredentialSetId(row.get(0)?),
        batch_id: row.get(1)?,
        source_kind,
        source_ref: row.get(3)?,
        physical_line_count: row.get::<_, i64>(4)? as usize,
        non_empty_count: row.get::<_, i64>(5)? as usize,
        unique_count: row.get::<_, i64>(6)? as usize,
        duplicate_occurrence_count: row.get::<_, i64>(7)? as usize,
        ignored_empty_count: row.get::<_, i64>(8)? as usize,
        invalid_line_count: row.get::<_, i64>(9)? as usize,
        created_at_unix_seconds: row.get(10)?,
    })
}

fn load_sqlite_credential_resource(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential_id: &CredentialId,
) -> anyhow::Result<Option<CredentialResourceRecord>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            credential_id,
            source_path,
            source_line,
            batch_id,
            fingerprint,
            label,
            note,
            position,
            first_imported_at_unix_seconds,
            last_seen_at_unix_seconds
        FROM credentials
        WHERE credential_set_id = ?1 AND credential_id = ?2
        "#,
    )?;
    statement
        .query_row(params![credential_set_id.0, credential_id.0], |row| {
            let stored_credential_id: String = row.get(0)?;
            let source_path: Option<String> = row.get(1)?;
            let source_line: Option<i64> = row.get(2)?;
            let batch_id: Option<String> = row.get(3)?;
            let fingerprint: String = row.get(4)?;
            let label: Option<String> = row.get(5)?;
            let note: Option<String> = row.get(6)?;
            let position: i64 = row.get(7)?;
            let first_imported_at_unix_seconds: i64 = row.get(8)?;
            let last_seen_at_unix_seconds: i64 = row.get(9)?;
            Ok(CredentialResourceRecord {
                credential_set_id: credential_set_id.clone(),
                credential_id: CredentialId(stored_credential_id),
                fingerprint,
                label,
                note,
                source_ref: source_path
                    .as_deref()
                    .and_then(|path| redacted_source_ref(std::path::Path::new(path))),
                source_line: source_line.map(|line| line as usize),
                batch_id,
                position: position as usize,
                first_imported_at_unix_seconds,
                last_seen_at_unix_seconds,
            })
        })
        .optional()
        .map_err(Into::into)
}

fn load_sqlite_credential_resource_by_position(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    position: usize,
) -> anyhow::Result<Option<CredentialResourceRecord>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            credential_id,
            source_path,
            source_line,
            batch_id,
            fingerprint,
            label,
            note,
            position,
            first_imported_at_unix_seconds,
            last_seen_at_unix_seconds
        FROM credentials
        WHERE credential_set_id = ?1 AND position = ?2
        "#,
    )?;
    statement
        .query_row(params![credential_set_id.0, position as i64], |row| {
            let stored_credential_id: String = row.get(0)?;
            let source_path: Option<String> = row.get(1)?;
            let source_line: Option<i64> = row.get(2)?;
            let batch_id: Option<String> = row.get(3)?;
            let fingerprint: String = row.get(4)?;
            let label: Option<String> = row.get(5)?;
            let note: Option<String> = row.get(6)?;
            let position: i64 = row.get(7)?;
            let first_imported_at_unix_seconds: i64 = row.get(8)?;
            let last_seen_at_unix_seconds: i64 = row.get(9)?;
            Ok(CredentialResourceRecord {
                credential_set_id: credential_set_id.clone(),
                credential_id: CredentialId(stored_credential_id),
                fingerprint,
                label,
                note,
                source_ref: source_path
                    .as_deref()
                    .and_then(|path| redacted_source_ref(std::path::Path::new(path))),
                source_line: source_line.map(|line| line as usize),
                batch_id,
                position: position as usize,
                first_imported_at_unix_seconds,
                last_seen_at_unix_seconds,
            })
        })
        .optional()
        .map_err(Into::into)
}

fn load_sqlite_credential_positions_for_credentials(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential_ids: &[CredentialId],
) -> anyhow::Result<HashMap<CredentialId, usize>> {
    if credential_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let requested: Vec<&CredentialId> = credential_ids
        .iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let placeholders = (0..requested.len())
        .map(|index| format!("?{}", index + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        r#"
        SELECT credential_id, position
        FROM credentials
        WHERE credential_set_id = ?1 AND credential_id IN ({placeholders})
        "#
    );
    let mut query_params = Vec::with_capacity(requested.len() + 1);
    query_params.push(Value::Text(credential_set_id.0.clone()));
    query_params.extend(
        requested
            .iter()
            .map(|credential_id| Value::Text(credential_id.0.clone())),
    );

    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(query_params.iter()), |row| {
        let credential_id: String = row.get(0)?;
        let position: i64 = row.get(1)?;
        Ok((CredentialId(credential_id), position as usize))
    })?;
    let positions = rows.collect::<rusqlite::Result<HashMap<_, _>>>()?;
    Ok(positions)
}

fn insert_probe_result(
    connection: &Connection,
    input: CredentialProbeResultRecordInput,
) -> anyhow::Result<CredentialProbeResultRecord> {
    let created_at_unix_seconds = unix_timestamp_seconds()?;
    connection.execute(
        r#"
        INSERT INTO credential_probe_results (
            credential_set_id,
            credential_id,
            channel_id,
            provider_id,
            account_id,
            outcome,
            classifier_id,
            adaptation_rule_id,
            upstream_status,
            upstream_code,
            upstream_limit_type,
            latency_ms,
            created_at_unix_seconds
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            input.credential_set_id.0,
            input.credential_id.0,
            input.channel_id,
            input.provider_id,
            input.account_id,
            probe_outcome_to_str(input.outcome),
            input.classifier_id.as_deref(),
            input.adaptation_rule_id.as_deref(),
            input.upstream_status.map(i64::from),
            input.upstream_code.as_deref(),
            input.upstream_limit_type.as_deref(),
            input.latency_ms as i64,
            created_at_unix_seconds,
        ],
    )?;
    let id = connection.last_insert_rowid();
    load_sqlite_probe_result_by_id(connection, id)?
        .ok_or_else(|| anyhow::anyhow!("inserted credential probe result {id} was not found"))
}

fn load_sqlite_probe_results(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential_id: &CredentialId,
    offset: usize,
    limit: usize,
) -> anyhow::Result<Vec<CredentialProbeResultRecord>> {
    let mut statement = connection.prepare(
        r#"
        SELECT
            id,
            credential_set_id,
            credential_id,
            channel_id,
            provider_id,
            account_id,
            outcome,
            classifier_id,
            adaptation_rule_id,
            upstream_status,
            upstream_code,
            upstream_limit_type,
            latency_ms,
            created_at_unix_seconds
        FROM credential_probe_results
        WHERE credential_set_id = ?1 AND credential_id = ?2
        ORDER BY id DESC
        LIMIT ?3 OFFSET ?4
        "#,
    )?;
    let records = statement
        .query_map(
            params![
                credential_set_id.0,
                credential_id.0,
                limit as i64,
                offset as i64
            ],
            probe_result_record_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(records)
}

fn load_latest_sqlite_probe_result(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential_id: &CredentialId,
) -> anyhow::Result<Option<CredentialProbeResultRecord>> {
    connection
        .query_row(
            r#"
            SELECT
                id,
                credential_set_id,
                credential_id,
                channel_id,
                provider_id,
                account_id,
                outcome,
                classifier_id,
                adaptation_rule_id,
                upstream_status,
                upstream_code,
                upstream_limit_type,
                latency_ms,
                created_at_unix_seconds
            FROM credential_probe_results
            WHERE credential_set_id = ?1 AND credential_id = ?2
            ORDER BY id DESC
            LIMIT 1
            "#,
            params![credential_set_id.0, credential_id.0],
            probe_result_record_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn load_latest_sqlite_probe_result_for_credential_set(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
) -> anyhow::Result<Option<CredentialProbeResultRecord>> {
    connection
        .query_row(
            r#"
            SELECT
                id,
                credential_set_id,
                credential_id,
                channel_id,
                provider_id,
                account_id,
                outcome,
                classifier_id,
                adaptation_rule_id,
                upstream_status,
                upstream_code,
                upstream_limit_type,
                latency_ms,
                created_at_unix_seconds
            FROM credential_probe_results
            WHERE credential_set_id = ?1
            ORDER BY id DESC
            LIMIT 1
            "#,
            params![credential_set_id.0],
            probe_result_record_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn load_sqlite_probe_summary_for_credential_set(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
) -> anyhow::Result<CredentialProbeSummaryRecord> {
    connection
        .query_row(
            r#"
            SELECT
                COALESCE(SUM(CASE WHEN r.outcome = 'success' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN r.outcome = 'invalid' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN r.outcome = 'quota_exhausted' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN r.outcome = 'rate_limited' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN r.outcome = 'provider_unavailable' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN r.outcome = 'unsupported_model' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN r.outcome = 'unknown' THEN 1 ELSE 0 END), 0)
            FROM credential_probe_results r
            JOIN (
                SELECT credential_id, MAX(id) AS id
                FROM credential_probe_results
                WHERE credential_set_id = ?1
                GROUP BY credential_id
            ) latest ON latest.id = r.id
            "#,
            params![credential_set_id.0],
            |row| {
                Ok(CredentialProbeSummaryRecord {
                    success: row.get::<_, i64>(0)? as usize,
                    invalid: row.get::<_, i64>(1)? as usize,
                    quota_exhausted: row.get::<_, i64>(2)? as usize,
                    rate_limited: row.get::<_, i64>(3)? as usize,
                    provider_unavailable: row.get::<_, i64>(4)? as usize,
                    unsupported_model: row.get::<_, i64>(5)? as usize,
                    unknown: row.get::<_, i64>(6)? as usize,
                })
            },
        )
        .map_err(Into::into)
}

fn load_latest_sqlite_probe_outcome_credential_ids(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    outcome: CredentialProbeOutcome,
    offset: usize,
    limit: usize,
) -> anyhow::Result<CredentialProbeOutcomePage> {
    let outcome = probe_outcome_to_str(outcome);
    let total = connection.query_row(
        r#"
        SELECT COUNT(*)
        FROM credential_probe_results r
        JOIN (
            SELECT credential_id, MAX(id) AS id
            FROM credential_probe_results
            WHERE credential_set_id = ?1
            GROUP BY credential_id
        ) latest ON latest.id = r.id
        WHERE r.credential_set_id = ?1 AND r.outcome = ?2
        "#,
        params![credential_set_id.0, outcome],
        |row| row.get::<_, i64>(0),
    )? as usize;
    let mut statement = connection.prepare(
        r#"
        SELECT r.credential_id
        FROM credential_probe_results r
        JOIN (
            SELECT credential_id, MAX(id) AS id
            FROM credential_probe_results
            WHERE credential_set_id = ?1
            GROUP BY credential_id
        ) latest ON latest.id = r.id
        JOIN credentials c
            ON c.credential_set_id = r.credential_set_id
            AND c.credential_id = r.credential_id
        WHERE r.credential_set_id = ?1 AND r.outcome = ?2
        ORDER BY c.position ASC
        LIMIT ?3 OFFSET ?4
        "#,
    )?;
    let credential_ids = statement
        .query_map(
            params![credential_set_id.0, outcome, limit as i64, offset as i64],
            |row| Ok(CredentialId(row.get::<_, String>(0)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(CredentialProbeOutcomePage {
        total,
        credential_ids,
    })
}

fn load_sqlite_unprobed_credential_ids(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    offset: usize,
    limit: usize,
) -> anyhow::Result<CredentialIdPage> {
    let total = connection.query_row(
        r#"
        SELECT COUNT(*)
        FROM credentials c
        WHERE c.credential_set_id = ?1
            AND NOT EXISTS (
                SELECT 1
                FROM credential_probe_results r
                WHERE r.credential_set_id = c.credential_set_id
                    AND r.credential_id = c.credential_id
            )
        "#,
        params![credential_set_id.0],
        |row| row.get::<_, i64>(0),
    )? as usize;
    let mut statement = connection.prepare(
        r#"
        SELECT c.credential_id
        FROM credentials c
        WHERE c.credential_set_id = ?1
            AND NOT EXISTS (
                SELECT 1
                FROM credential_probe_results r
                WHERE r.credential_set_id = c.credential_set_id
                    AND r.credential_id = c.credential_id
            )
        ORDER BY c.position ASC
        LIMIT ?2 OFFSET ?3
        "#,
    )?;
    let credential_ids = statement
        .query_map(
            params![credential_set_id.0, limit as i64, offset as i64],
            |row| Ok(CredentialId(row.get::<_, String>(0)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(CredentialIdPage {
        total,
        credential_ids,
    })
}

fn load_latest_sqlite_probe_results_for_credentials(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    credential_ids: &[CredentialId],
) -> anyhow::Result<HashMap<CredentialId, CredentialProbeResultRecord>> {
    if credential_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let requested: Vec<&CredentialId> = credential_ids
        .iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let sql = latest_probe_results_for_credentials_query(requested.len());
    let mut query_params = Vec::with_capacity(requested.len() + 1);
    query_params.push(Value::Text(credential_set_id.0.clone()));
    query_params.extend(
        requested
            .iter()
            .map(|credential_id| Value::Text(credential_id.0.clone())),
    );

    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(query_params.iter()), |row| {
        probe_result_record_from_row(row)
    })?;
    let records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let latest = records
        .into_iter()
        .map(|record| (record.credential_id.clone(), record))
        .collect();
    Ok(latest)
}

fn latest_probe_results_for_credentials_query(credential_count: usize) -> String {
    let placeholders = (0..credential_count)
        .map(|index| format!("?{}", index + 2))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
        SELECT
            r.id,
            r.credential_set_id,
            r.credential_id,
            r.channel_id,
            r.provider_id,
            r.account_id,
            r.outcome,
            r.classifier_id,
            r.adaptation_rule_id,
            r.upstream_status,
            r.upstream_code,
            r.upstream_limit_type,
            r.latency_ms,
            r.created_at_unix_seconds
        FROM credential_probe_results r
        JOIN (
            SELECT credential_id, MAX(id) AS id
            FROM credential_probe_results
            WHERE credential_set_id = ?1 AND credential_id IN ({placeholders})
            GROUP BY credential_id
        ) latest ON latest.id = r.id
        ORDER BY r.id DESC
        "#
    )
}

fn load_sqlite_probe_result_by_id(
    connection: &Connection,
    id: i64,
) -> anyhow::Result<Option<CredentialProbeResultRecord>> {
    connection
        .query_row(
            r#"
            SELECT
                id,
                credential_set_id,
                credential_id,
                channel_id,
                provider_id,
                account_id,
                outcome,
                classifier_id,
                adaptation_rule_id,
                upstream_status,
                upstream_code,
                upstream_limit_type,
                latency_ms,
                created_at_unix_seconds
            FROM credential_probe_results
            WHERE id = ?1
            "#,
            params![id],
            probe_result_record_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn probe_result_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CredentialProbeResultRecord> {
    let outcome: String = row.get(6)?;
    let outcome = probe_outcome_from_str(&outcome).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let upstream_status: Option<i64> = row.get(9)?;
    let latency_ms: i64 = row.get(12)?;
    Ok(CredentialProbeResultRecord {
        id: row.get(0)?,
        credential_set_id: CredentialSetId(row.get(1)?),
        credential_id: CredentialId(row.get(2)?),
        channel_id: row.get(3)?,
        provider_id: row.get(4)?,
        account_id: row.get(5)?,
        outcome,
        classifier_id: row.get(7)?,
        adaptation_rule_id: row.get(8)?,
        upstream_status: upstream_status.map(|status| status as u16),
        upstream_code: row.get(10)?,
        upstream_limit_type: row.get(11)?,
        latency_ms: latency_ms as u64,
        created_at_unix_seconds: row.get(13)?,
    })
}

fn probe_outcome_to_str(outcome: CredentialProbeOutcome) -> &'static str {
    match outcome {
        CredentialProbeOutcome::Success => "success",
        CredentialProbeOutcome::Invalid => "invalid",
        CredentialProbeOutcome::QuotaExhausted => "quota_exhausted",
        CredentialProbeOutcome::RateLimited => "rate_limited",
        CredentialProbeOutcome::ProviderUnavailable => "provider_unavailable",
        CredentialProbeOutcome::UnsupportedModel => "unsupported_model",
        CredentialProbeOutcome::Unknown => "unknown",
    }
}

fn probe_outcome_from_str(raw: &str) -> anyhow::Result<CredentialProbeOutcome> {
    match raw {
        "success" => Ok(CredentialProbeOutcome::Success),
        "invalid" => Ok(CredentialProbeOutcome::Invalid),
        "quota_exhausted" => Ok(CredentialProbeOutcome::QuotaExhausted),
        "rate_limited" => Ok(CredentialProbeOutcome::RateLimited),
        "provider_unavailable" => Ok(CredentialProbeOutcome::ProviderUnavailable),
        "unsupported_model" => Ok(CredentialProbeOutcome::UnsupportedModel),
        "unknown" => Ok(CredentialProbeOutcome::Unknown),
        other => anyhow::bail!("unknown credential probe outcome {other}"),
    }
}

fn update_sqlite_import_counts(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
    non_empty_count: usize,
    ignored_empty_count: usize,
    unique_count: usize,
    duplicate_occurrence_count: usize,
) -> anyhow::Result<()> {
    connection.execute(
        r#"
        UPDATE credential_sets
        SET
            physical_line_count = physical_line_count + ?2,
            non_empty_count = non_empty_count + ?3,
            unique_count = unique_count + ?4,
            duplicate_occurrence_count = duplicate_occurrence_count + ?5,
            ignored_empty_count = ignored_empty_count + ?6,
            import_generation = import_generation + 1,
            last_imported_at_unix_seconds = ?7
        WHERE id = ?1
        "#,
        params![
            credential_set_id.0,
            (non_empty_count + ignored_empty_count) as i64,
            non_empty_count as i64,
            unique_count as i64,
            duplicate_occurrence_count as i64,
            ignored_empty_count as i64,
            unix_timestamp_seconds()?,
        ],
    )?;
    Ok(())
}

fn persist_import(
    connection: &mut Connection,
    credential_set_id: &CredentialSetId,
    import: &KeyImport,
) -> anyhow::Result<()> {
    let tx = connection.transaction()?;
    tx.execute(
        r#"
        INSERT OR REPLACE INTO credential_sets (
            id,
            source_path,
            physical_line_count,
            non_empty_count,
            unique_count,
            duplicate_occurrence_count,
            ignored_empty_count,
            invalid_line_count,
            claimed_count,
            claim_source,
            import_generation,
            last_imported_at_unix_seconds
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            credential_set_id.0,
            import.report.source_path.to_string_lossy().as_ref(),
            import.report.physical_line_count as i64,
            import.report.non_empty_count as i64,
            import.report.unique_count as i64,
            import.report.duplicate_occurrence_count as i64,
            import.report.ignored_empty_count as i64,
            import.report.invalid_line_count as i64,
            import.report.claimed_count.map(|count| count as i64),
            import.report.claim_source.as_deref(),
            import.report.import_generation as i64,
            import.report.last_imported_at_unix_seconds,
        ],
    )?;
    tx.execute(
        "DELETE FROM credentials WHERE credential_set_id = ?1",
        params![credential_set_id.0],
    )?;
    for (position, credential) in import.credentials.iter().enumerate() {
        insert_sqlite_credential(&tx, credential_set_id, credential, position)?;
    }
    insert_import_batch(
        &tx,
        credential_set_id,
        &credential_set_id.0,
        CredentialImportSourceKind::FileBootstrap,
        redacted_source_ref(&import.report.source_path).as_deref(),
        ImportBatchCounts {
            physical_line_count: import.report.physical_line_count,
            non_empty_count: import.report.non_empty_count,
            unique_count: import.report.unique_count,
            duplicate_occurrence_count: import.report.duplicate_occurrence_count,
            ignored_empty_count: import.report.ignored_empty_count,
            invalid_line_count: import.report.invalid_line_count,
        },
    )?;
    tx.commit()?;
    Ok(())
}

fn load_from_sqlite(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
) -> anyhow::Result<KeyImport> {
    let report = load_sqlite_credential_set_report(connection, credential_set_id)?;

    let mut statement = connection.prepare(
        r#"
        SELECT secret, source_path, source_line, batch_id
        FROM credentials
        WHERE credential_set_id = ?1
        ORDER BY position ASC
        "#,
    )?;
    let credentials = statement
        .query_map(params![credential_set_id.0], |row| {
            let source_path: Option<String> = row.get(1)?;
            let source_line: Option<i64> = row.get(2)?;
            Ok(ImportedCredential {
                secret: row.get(0)?,
                source: CredentialSource {
                    source_path: source_path.map(PathBuf::from),
                    source_line: source_line.map(|line| line as usize),
                    batch_id: row.get(3)?,
                },
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    anyhow::ensure!(
        !credentials.is_empty(),
        "credential set {} has no credentials",
        credential_set_id.0
    );
    Ok(KeyImport {
        credentials,
        report,
    })
}

fn load_sqlite_credential_set_report(
    connection: &Connection,
    credential_set_id: &CredentialSetId,
) -> anyhow::Result<KeyImportReport> {
    connection
        .query_row(
            r#"
            SELECT
                source_path,
                physical_line_count,
                non_empty_count,
                unique_count,
                duplicate_occurrence_count,
                ignored_empty_count,
                invalid_line_count,
                claimed_count,
                claim_source,
                import_generation,
                last_imported_at_unix_seconds
            FROM credential_sets
            WHERE id = ?1
            "#,
            params![credential_set_id.0],
            |row| {
                let source_path: String = row.get(0)?;
                let claimed_count: Option<i64> = row.get(7)?;
                Ok(KeyImportReport {
                    source_path: PathBuf::from(source_path),
                    physical_line_count: row.get::<_, i64>(1)? as usize,
                    non_empty_count: row.get::<_, i64>(2)? as usize,
                    unique_count: row.get::<_, i64>(3)? as usize,
                    duplicate_occurrence_count: row.get::<_, i64>(4)? as usize,
                    ignored_empty_count: row.get::<_, i64>(5)? as usize,
                    invalid_line_count: row.get::<_, i64>(6)? as usize,
                    claimed_count: claimed_count.map(|count| count as usize),
                    claim_source: row.get(8)?,
                    import_generation: row.get::<_, i64>(9)? as u64,
                    last_imported_at_unix_seconds: row.get(10)?,
                    duplicate_fingerprints: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "credential set {} has credentials but no metadata",
                credential_set_id.0
            )
        })
}

fn import_raw_credentials(
    raw: String,
    origin: CredentialImportOrigin,
) -> anyhow::Result<KeyImport> {
    let mut credentials = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut non_empty_count = 0usize;
    let mut ignored_empty_count = 0usize;
    let mut duplicates = Vec::new();

    for (offset, line) in raw.lines().enumerate() {
        let source_line = offset + 1;
        let key = line.trim();
        if key.is_empty() {
            ignored_empty_count += 1;
            continue;
        }
        non_empty_count += 1;
        let fingerprint = short_hash(key);
        if let Some(first_source_line) = seen.get(key) {
            duplicates.push(DuplicateCredentialOccurrence {
                first_source_line: *first_source_line,
                duplicate_source_line: source_line,
                fingerprint,
            });
            continue;
        }
        seen.insert(key.to_string(), source_line);
        credentials.push(ImportedCredential {
            secret: key.to_string(),
            source: CredentialSource {
                source_path: Some(origin.source_path.clone()),
                source_line: Some(source_line),
                batch_id: Some(origin.credential_set_id.0.clone()),
            },
        });
    }

    anyhow::ensure!(
        !credentials.is_empty(),
        "no keys in {}",
        origin.source_path.display()
    );
    Ok(KeyImport {
        report: KeyImportReport {
            source_path: origin.source_path,
            physical_line_count: raw.lines().count(),
            non_empty_count,
            unique_count: credentials.len(),
            duplicate_occurrence_count: duplicates.len(),
            ignored_empty_count,
            invalid_line_count: 0,
            claimed_count: origin.claimed_count,
            claim_source: origin.claim_source,
            import_generation: 1,
            last_imported_at_unix_seconds: unix_timestamp_seconds()?,
            duplicate_fingerprints: duplicates,
        },
        credentials,
    })
}

fn claimed_count_from_path(path: &std::path::Path) -> Option<usize> {
    let name = path.file_name()?.to_string_lossy();
    let digits: String = name.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn persistent_fingerprint(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    format!("{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::fixtures;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Barrier,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_KEY_PATH_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn test_lifecycle_evidence() -> CredentialLifecycleEvidence {
        CredentialLifecycleEvidence::management_command(
            "test_lifecycle_update",
            "test:credential-repository",
            "credential-repository-test",
            "test",
            "test",
        )
    }

    fn test_probe_input(
        credential_set_id: CredentialSetId,
        credential_id: CredentialId,
        outcome: CredentialProbeOutcome,
    ) -> CredentialProbeResultRecordInput {
        CredentialProbeResultRecordInput {
            credential_set_id,
            credential_id,
            channel_id: "primary".to_string(),
            provider_id: "openai".to_string(),
            account_id: "relay".to_string(),
            outcome,
            classifier_id: Some("openai-compatible-default".to_string()),
            adaptation_rule_id: None,
            upstream_status: Some(401),
            upstream_code: Some("invalid_api_key".to_string()),
            upstream_limit_type: None,
            latency_ms: 12,
        }
    }

    fn temp_key_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_KEY_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        path.push(format!("{name}-{suffix}-{counter}.txt"));
        path
    }

    fn fixture_credential_a() -> &'static str {
        fixtures().upstream_credentials.a.as_str()
    }

    fn fixture_credential_b() -> &'static str {
        fixtures().upstream_credentials.b.as_str()
    }

    fn fixture_credential_c() -> &'static str {
        fixtures().upstream_credentials.c.as_str()
    }

    fn fixture_credential_manual() -> &'static str {
        fixtures().upstream_credentials.manual.as_str()
    }

    fn fixture_credential_legacy() -> &'static str {
        fixtures().upstream_credentials.legacy.as_str()
    }

    fn credential_file_contents(credentials: &[&str]) -> String {
        let mut contents = credentials.join("\n");
        contents.push('\n');
        contents
    }

    fn write_fixture_credentials(path: &std::path::Path, credentials: &[&str]) {
        fs::write(path, credential_file_contents(credentials)).unwrap();
    }

    fn assert_debug_omits_credentials(debug: &str, credentials: &[&str]) {
        for credential in credentials {
            assert!(!debug.contains(credential));
        }
    }

    #[test]
    fn file_repository_imports_unique_credentials_with_lineage_report() {
        let path = temp_key_path("520-key-pool-router-repository-test");
        fs::write(&path, "\nk1\n k2 \nk1\n").unwrap();

        let import = FileCredentialRepository::new()
            .load_credential_set(
                &CredentialSetId("ai2_test".to_string()),
                &CredentialSetSource::File { path: path.clone() },
            )
            .unwrap();

        assert_eq!(import.credentials.len(), 2);
        assert_eq!(import.credentials[0].secret, "k1");
        assert_eq!(import.credentials[0].source.source_path, Some(path.clone()));
        assert_eq!(import.credentials[0].source.source_line, Some(2));
        assert_eq!(
            import.credentials[0].source.batch_id,
            Some("ai2_test".to_string())
        );
        assert_eq!(import.report.physical_line_count, 4);
        assert_eq!(import.report.non_empty_count, 3);
        assert_eq!(import.report.unique_count, 2);
        assert_eq!(import.report.duplicate_occurrence_count, 1);
        assert_eq!(import.report.ignored_empty_count, 1);
        assert_eq!(import.report.claimed_count, Some(520));
    }

    #[test]
    fn sqlite_repository_imports_once_then_loads_without_source_file() {
        let db_path = temp_key_path("key-pool-router-sqlite-repository").with_extension("sqlite");
        let source_path = temp_key_path("2-key-pool-router-sqlite-source");
        fs::write(&source_path, "k1\nk2\n").unwrap();

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let imported = repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();
        assert_eq!(imported.credentials.len(), 2);
        assert_eq!(imported.report.source_path, source_path);

        fs::remove_file(&source_path).unwrap();
        let loaded = repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();

        assert_eq!(loaded.credentials.len(), 2);
        assert_eq!(loaded.credentials[0].secret, "k1");
        assert_eq!(loaded.credentials[1].secret, "k2");
        assert_eq!(loaded.report.unique_count, 2);
        assert_eq!(loaded.report.claimed_count, Some(2));
    }

    #[test]
    fn sqlite_repository_treats_existing_database_as_authoritative() {
        let db_path =
            temp_key_path("key-pool-router-sqlite-authoritative").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();
        fs::write(&source_path, "second\n").unwrap();

        let loaded = repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();

        assert_eq!(loaded.credentials.len(), 1);
        assert_eq!(loaded.credentials[0].secret, "first");
    }

    #[test]
    fn sqlite_repository_appends_only_new_credentials_to_existing_set() {
        let db_path = temp_key_path("key-pool-router-sqlite-append").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();

        let appended = repository
            .append_credentials(
                &CredentialSetId("sqlite_set".to_string()),
                vec!["first".to_string(), "second".to_string()],
                "management-api",
            )
            .unwrap();
        assert_eq!(appended.credentials.len(), 1);
        assert_eq!(appended.credentials[0].secret, "second");
        assert_eq!(appended.report.unique_count, 1);
        assert_eq!(
            appended.credentials[0].source.batch_id.as_deref(),
            Some("management-api")
        );

        let loaded = repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let secrets: Vec<&str> = loaded
            .credentials
            .iter()
            .map(|credential| credential.secret.as_str())
            .collect();
        assert_eq!(secrets, vec!["first", "second"]);
        assert_eq!(loaded.report.unique_count, 2);
        assert_eq!(loaded.report.import_generation, 2);
        assert!(loaded.report.last_imported_at_unix_seconds > 0);
    }

    #[test]
    fn sqlite_repository_records_import_batches_for_bootstrap_and_management_append() {
        let db_path = temp_key_path("key-pool-router-import-batches").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-import-batches-source");
        fs::write(&source_path, "k1\nk2\nk1\n").unwrap();
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());

        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();
        repository
            .append_credentials(
                &credential_set_id,
                vec!["k3".to_string(), "k2".to_string()],
                "manual",
            )
            .unwrap();

        let batches = repository
            .load_import_batches(&credential_set_id, 0, 10)
            .unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].credential_set_id, credential_set_id);
        assert_eq!(batches[0].batch_id, "manual");
        assert_eq!(
            batches[0].source_kind,
            CredentialImportSourceKind::ManagementApi
        );
        assert_eq!(batches[0].unique_count, 1);
        assert_eq!(batches[0].duplicate_occurrence_count, 1);
        assert_eq!(
            batches[1].source_kind,
            CredentialImportSourceKind::FileBootstrap
        );
        assert_eq!(batches[1].unique_count, 2);
        assert_eq!(batches[1].duplicate_occurrence_count, 1);
        assert_ne!(
            batches[1].source_ref.as_deref(),
            Some(source_path.to_str().unwrap())
        );
        assert_eq!(
            batches[1].source_ref.as_deref(),
            source_path.file_name().and_then(|name| name.to_str())
        );
    }

    #[test]
    fn sqlite_repository_import_batches_are_paginated_newest_first() {
        let db_path = temp_key_path("key-pool-router-import-batches-page").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-import-batches-page-source");
        fs::write(&source_path, "k1\n").unwrap();
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());

        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        repository
            .append_credentials(&credential_set_id, vec!["k2".to_string()], "manual-1")
            .unwrap();
        repository
            .append_credentials(&credential_set_id, vec!["k3".to_string()], "manual-2")
            .unwrap();

        let page = repository
            .load_import_batches(&credential_set_id, 1, 1)
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].batch_id, "manual-1");

        let missing = repository
            .load_import_batches(&CredentialSetId("missing".to_string()), 0, 10)
            .unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn sqlite_repository_loads_one_import_batch_by_id() {
        let db_path = temp_key_path("key-pool-router-import-batch-detail").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-import-batch-detail-source");
        fs::write(&source_path, "k1\n").unwrap();
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());

        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        repository
            .append_credentials(&credential_set_id, vec!["k2".to_string()], "manual")
            .unwrap();

        let batch = repository
            .load_import_batch(&credential_set_id, "manual")
            .unwrap()
            .unwrap();

        assert_eq!(batch.credential_set_id, credential_set_id);
        assert_eq!(batch.batch_id, "manual");
        assert_eq!(batch.source_kind, CredentialImportSourceKind::ManagementApi);
        assert_eq!(batch.unique_count, 1);
        assert_eq!(batch.duplicate_occurrence_count, 0);

        let missing = repository
            .load_import_batch(&credential_set_id, "missing")
            .unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn sqlite_repository_projects_credential_resource_without_raw_secret() {
        let db_path = temp_key_path("key-pool-router-credential-resource").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-credential-resource-source");
        write_fixture_credentials(
            &source_path,
            &[fixture_credential_a(), fixture_credential_b()],
        );
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();

        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());
        let detail = repository
            .load_credential_resource(&credential_set_id, &credential_id)
            .unwrap()
            .unwrap();

        assert_eq!(detail.credential_set_id, credential_set_id);
        assert_eq!(detail.credential_id, credential_id);
        assert_eq!(
            detail.fingerprint,
            persistent_fingerprint(fixture_credential_a())
        );
        assert!(detail.label.is_none());
        assert!(detail.note.is_none());
        assert_eq!(detail.source_line, Some(1));
        assert_eq!(detail.batch_id.as_deref(), Some("set"));
        assert_eq!(detail.position, 0);
        assert!(detail.first_imported_at_unix_seconds > 0);
        assert_eq!(
            detail.first_imported_at_unix_seconds,
            detail.last_seen_at_unix_seconds
        );
        assert_eq!(
            detail.source_ref.as_deref(),
            source_path.file_name().and_then(|name| name.to_str())
        );
        assert_debug_omits_credentials(
            &format!("{detail:?}"),
            &[fixture_credential_a(), fixture_credential_b()],
        );

        let missing = repository
            .load_credential_resource(
                &credential_set_id,
                &CredentialId("cred_missing".to_string()),
            )
            .unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn sqlite_repository_persists_credential_operator_metadata() {
        let db_path = temp_key_path("key-pool-router-credential-metadata").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-credential-metadata-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());

        repository
            .update_credential_operator_metadata(
                &credential_set_id,
                &credential_id,
                Some("primary relay".to_string()),
                Some("use for low latency models".to_string()),
            )
            .unwrap();

        let detail = repository
            .load_credential_resource(&credential_set_id, &credential_id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.label.as_deref(), Some("primary relay"));
        assert_eq!(detail.note.as_deref(), Some("use for low latency models"));
    }

    #[test]
    fn sqlite_repository_updates_credential_last_seen_on_duplicate_append() {
        let db_path =
            temp_key_path("key-pool-router-credential-last-seen").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-credential-last-seen-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();

        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());
        let initial = repository
            .load_credential_resource(&credential_set_id, &credential_id)
            .unwrap()
            .unwrap();
        repository
            .connect()
            .unwrap()
            .execute(
                r#"
                UPDATE credentials
                SET last_seen_at_unix_seconds = first_imported_at_unix_seconds - 10
                WHERE credential_set_id = ?1 AND fingerprint = ?2
                "#,
                params![
                    credential_set_id.0,
                    persistent_fingerprint(fixture_credential_a())
                ],
            )
            .unwrap();

        let appended = repository
            .append_credentials(
                &credential_set_id,
                vec![fixture_credential_a().to_string()],
                "management-api",
            )
            .unwrap();
        assert!(appended.credentials.is_empty());

        let updated = repository
            .load_credential_resource(&credential_set_id, &credential_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            updated.first_imported_at_unix_seconds,
            initial.first_imported_at_unix_seconds
        );
        assert!(updated.last_seen_at_unix_seconds >= initial.first_imported_at_unix_seconds);
    }

    #[test]
    fn sqlite_repository_records_credential_probe_result_without_raw_secret_or_body() {
        let db_path = temp_key_path("key-pool-router-probe-results").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-probe-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());

        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_id.clone(),
                CredentialProbeOutcome::Invalid,
            ))
            .unwrap();

        let history = repository
            .load_probe_results(&credential_set_id, &credential_id, 0, 10)
            .unwrap();
        let latest = repository
            .load_latest_probe_result(&credential_set_id, &credential_id)
            .unwrap()
            .unwrap();

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].outcome, CredentialProbeOutcome::Invalid);
        assert_eq!(history[0], latest);
        assert_eq!(history[0].channel_id, "primary");
        assert_eq!(history[0].provider_id, "openai");
        assert_eq!(history[0].account_id, "relay");
        assert_eq!(
            history[0].classifier_id.as_deref(),
            Some("openai-compatible-default")
        );
        assert_eq!(history[0].upstream_status, Some(401));
        assert_eq!(history[0].upstream_code.as_deref(), Some("invalid_api_key"));
        assert_eq!(history[0].latency_ms, 12);
        assert!(!format!("{history:?}").contains(fixture_credential_a()));
        assert!(!format!("{history:?}").contains("upstream raw body"));
    }

    #[test]
    fn sqlite_repository_probe_results_are_paginated_newest_first() {
        let db_path = temp_key_path("key-pool-router-probe-results-page").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-probe-page-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());

        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_id.clone(),
                CredentialProbeOutcome::RateLimited,
            ))
            .unwrap();
        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_id.clone(),
                CredentialProbeOutcome::Success,
            ))
            .unwrap();

        let page = repository
            .load_probe_results(&credential_set_id, &credential_id, 0, 1)
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].outcome, CredentialProbeOutcome::Success);
        assert_eq!(
            repository
                .load_latest_probe_result(&credential_set_id, &credential_id)
                .unwrap()
                .unwrap()
                .outcome,
            CredentialProbeOutcome::Success
        );
    }

    #[test]
    fn sqlite_repository_lists_latest_probe_outcome_credential_ids_with_limit() {
        let db_path = temp_key_path("key-pool-router-probe-outcome-page").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-probe-outcome-page-source");
        write_fixture_credentials(
            &source_path,
            &[
                fixture_credential_a(),
                fixture_credential_b(),
                fixture_credential_c(),
            ],
        );
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let credential_a = credential_id_for_secret(&credential_set_id, fixture_credential_a());
        let credential_b = credential_id_for_secret(&credential_set_id, fixture_credential_b());
        let credential_c = credential_id_for_secret(&credential_set_id, fixture_credential_c());

        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_a.clone(),
                CredentialProbeOutcome::Invalid,
            ))
            .unwrap();
        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_b.clone(),
                CredentialProbeOutcome::Invalid,
            ))
            .unwrap();
        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_c.clone(),
                CredentialProbeOutcome::RateLimited,
            ))
            .unwrap();
        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_a.clone(),
                CredentialProbeOutcome::Success,
            ))
            .unwrap();

        let page = repository
            .load_latest_probe_outcome_credential_ids(
                &credential_set_id,
                CredentialProbeOutcome::Invalid,
                0,
                1,
            )
            .unwrap();

        assert_eq!(page.total, 1);
        assert_eq!(page.credential_ids, vec![credential_b]);
    }

    #[test]
    fn sqlite_repository_lists_unprobed_credential_ids_with_limit() {
        let db_path = temp_key_path("key-pool-router-unprobed-page").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-unprobed-page-source");
        write_fixture_credentials(
            &source_path,
            &[
                fixture_credential_a(),
                fixture_credential_b(),
                fixture_credential_c(),
            ],
        );
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let credential_a = credential_id_for_secret(&credential_set_id, fixture_credential_a());
        let credential_b = credential_id_for_secret(&credential_set_id, fixture_credential_b());
        let credential_c = credential_id_for_secret(&credential_set_id, fixture_credential_c());

        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_a,
                CredentialProbeOutcome::Invalid,
            ))
            .unwrap();

        let page = repository
            .load_unprobed_credential_ids(&credential_set_id, 1, 1)
            .unwrap();

        assert_eq!(page.total, 2);
        assert_eq!(page.credential_ids, vec![credential_c]);
        assert_ne!(page.credential_ids, vec![credential_b]);
    }

    #[test]
    fn sqlite_latest_probe_batch_query_uses_credential_scoped_index() {
        let db_path =
            temp_key_path("key-pool-router-latest-probe-batch-index").with_extension("sqlite");
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let connection = repository.connect().unwrap();
        initialize_schema(&connection).unwrap();

        let sql = latest_probe_results_for_credentials_query(2);
        let plan: Vec<String> = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map(params!["set", "cred_a", "cred_b"], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            plan.iter().any(|detail| {
                detail.contains("credential_probe_results_lookup")
                    && detail.contains("credential_set_id=?")
                    && detail.contains("credential_id=?")
            }),
            "latest probe batch query should use credential_probe_results_lookup with credential-set and credential-id constraints, got {plan:?}"
        );
    }

    #[test]
    fn sqlite_latest_probe_set_query_uses_set_latest_index() {
        let db_path =
            temp_key_path("key-pool-router-latest-probe-set-index").with_extension("sqlite");
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let connection = repository.connect().unwrap();
        initialize_schema(&connection).unwrap();

        let plan: Vec<String> = connection
            .prepare(
                r#"
                EXPLAIN QUERY PLAN
                SELECT
                    id,
                    credential_set_id,
                    credential_id,
                    channel_id,
                    provider_id,
                    account_id,
                    outcome,
                    classifier_id,
                    adaptation_rule_id,
                    upstream_status,
                    upstream_code,
                    upstream_limit_type,
                    latency_ms,
                    created_at_unix_seconds
                FROM credential_probe_results
                WHERE credential_set_id = ?1
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .unwrap()
            .query_map(params!["set"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            plan.iter()
                .any(|detail| detail.contains("credential_probe_results_set_latest")),
            "latest set probe query should use credential_probe_results_set_latest, got {plan:?}"
        );
    }

    #[test]
    fn sqlite_latest_probe_outcome_page_uses_probe_and_position_indexes() {
        let db_path =
            temp_key_path("key-pool-router-latest-probe-outcome-index").with_extension("sqlite");
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let connection = repository.connect().unwrap();
        initialize_schema(&connection).unwrap();

        let plan: Vec<String> = connection
            .prepare(
                r#"
                EXPLAIN QUERY PLAN
                SELECT r.credential_id
                FROM credential_probe_results r
                JOIN (
                    SELECT credential_id, MAX(id) AS id
                    FROM credential_probe_results
                    WHERE credential_set_id = ?1
                    GROUP BY credential_id
                ) latest ON latest.id = r.id
                JOIN credentials c
                    ON c.credential_set_id = r.credential_set_id
                    AND c.credential_id = r.credential_id
                WHERE r.credential_set_id = ?1 AND r.outcome = ?2
                ORDER BY c.position ASC
                LIMIT ?3 OFFSET ?4
                "#,
            )
            .unwrap()
            .query_map(params!["set", "invalid", 10_i64, 0_i64], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            plan.iter()
                .any(|detail| detail.contains("credential_probe_results_lookup")),
            "latest probe outcome page query should use credential_probe_results_lookup, got {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|detail| detail.contains("credentials_set_position_unique")),
            "latest probe outcome page query should preserve credential position order with credentials_set_position_unique, got {plan:?}"
        );
    }

    #[test]
    fn sqlite_unprobed_page_uses_position_and_probe_indexes() {
        let db_path = temp_key_path("key-pool-router-unprobed-index").with_extension("sqlite");
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let connection = repository.connect().unwrap();
        initialize_schema(&connection).unwrap();

        let plan: Vec<String> = connection
            .prepare(
                r#"
                EXPLAIN QUERY PLAN
                SELECT c.credential_id
                FROM credentials c
                WHERE c.credential_set_id = ?1
                    AND NOT EXISTS (
                        SELECT 1
                        FROM credential_probe_results r
                        WHERE r.credential_set_id = c.credential_set_id
                            AND r.credential_id = c.credential_id
                    )
                ORDER BY c.position ASC
                LIMIT ?2 OFFSET ?3
                "#,
            )
            .unwrap()
            .query_map(params!["set", 10_i64, 0_i64], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            plan.iter()
                .any(|detail| detail.contains("credentials_set_position_unique")),
            "unprobed page query should preserve credential position order with credentials_set_position_unique, got {plan:?}"
        );
        assert!(
            plan.iter()
                .any(|detail| detail.contains("credential_probe_results_lookup")),
            "unprobed page query should check probe existence through credential_probe_results_lookup, got {plan:?}"
        );
    }

    #[test]
    fn sqlite_credential_id_backfill_runs_once() {
        let db_path =
            temp_key_path("key-pool-router-credential-id-backfill-once").with_extension("sqlite");
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let connection = repository.connect().unwrap();
        initialize_schema(&connection).unwrap();

        let applied_before: i64 = connection
            .query_row(
                "SELECT applied_at_unix_seconds FROM schema_migrations WHERE name = ?1",
                params!["credentials_credential_id_backfill"],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM credentials WHERE credential_set_id = ?1",
                params!["manual"],
            )
            .unwrap();
        connection
            .execute(
                r#"
                INSERT INTO credential_sets (
                    id,
                    source_path,
                    physical_line_count,
                    non_empty_count,
                    unique_count,
                    duplicate_occurrence_count,
                    ignored_empty_count,
                    invalid_line_count,
                    import_generation,
                    last_imported_at_unix_seconds
                ) VALUES (?1, ?2, 1, 1, 1, 0, 0, 0, 1, 1)
                "#,
                params!["manual", "manual"],
            )
            .unwrap();
        connection
            .execute(
                r#"
                INSERT INTO credentials (
                    credential_set_id,
                    credential_id,
                    secret,
                    fingerprint,
                    position,
                    first_imported_at_unix_seconds,
                    last_seen_at_unix_seconds
                ) VALUES (?1, NULL, ?2, ?3, 0, 1, 1)
                "#,
                params![
                    "manual",
                    fixture_credential_manual(),
                    persistent_fingerprint(fixture_credential_manual())
                ],
            )
            .unwrap();

        initialize_schema(&connection).unwrap();

        let applied_after: i64 = connection
            .query_row(
                "SELECT applied_at_unix_seconds FROM schema_migrations WHERE name = ?1",
                params!["credentials_credential_id_backfill"],
                |row| row.get(0),
            )
            .unwrap();
        let missing_id_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM credentials WHERE credential_set_id = ?1 AND credential_id IS NULL",
                params!["manual"],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(applied_after, applied_before);
        assert_eq!(missing_id_count, 1);
    }

    #[test]
    fn sqlite_probe_summary_query_uses_credential_scoped_latest_index() {
        let db_path = temp_key_path("key-pool-router-probe-summary-index").with_extension("sqlite");
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let connection = repository.connect().unwrap();
        initialize_schema(&connection).unwrap();

        let plan: Vec<String> = connection
            .prepare(
                r#"
                EXPLAIN QUERY PLAN
                SELECT
                    SUM(CASE WHEN r.outcome = 'success' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN r.outcome = 'invalid' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN r.outcome = 'quota_exhausted' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN r.outcome = 'rate_limited' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN r.outcome = 'provider_unavailable' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN r.outcome = 'unsupported_model' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN r.outcome = 'unknown' THEN 1 ELSE 0 END)
                FROM credential_probe_results r
                JOIN (
                    SELECT credential_id, MAX(id) AS id
                    FROM credential_probe_results
                    WHERE credential_set_id = ?1
                    GROUP BY credential_id
                ) latest ON latest.id = r.id
                "#,
            )
            .unwrap()
            .query_map(params!["set"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            plan.iter().any(|detail| {
                detail.contains("credential_probe_results_lookup")
                    && detail.contains("credential_set_id=?")
            }),
            "probe summary should derive latest rows through credential_probe_results_lookup, got {plan:?}"
        );
        assert!(
            !plan
                .iter()
                .any(|detail| detail.contains("USE TEMP B-TREE FOR ORDER BY")),
            "probe summary should not sort probe history, got {plan:?}"
        );
    }

    #[test]
    fn sqlite_repository_loads_latest_probe_results_only_for_requested_credentials() {
        let db_path = temp_key_path("key-pool-router-latest-probe-batch").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-latest-probe-batch-source");
        write_fixture_credentials(
            &source_path,
            &[
                fixture_credential_a(),
                fixture_credential_b(),
                fixture_credential_c(),
            ],
        );
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let credential_a = credential_id_for_secret(&credential_set_id, fixture_credential_a());
        let credential_b = credential_id_for_secret(&credential_set_id, fixture_credential_b());
        let credential_c = credential_id_for_secret(&credential_set_id, fixture_credential_c());

        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_a.clone(),
                CredentialProbeOutcome::Invalid,
            ))
            .unwrap();
        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_b.clone(),
                CredentialProbeOutcome::RateLimited,
            ))
            .unwrap();
        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_a.clone(),
                CredentialProbeOutcome::Success,
            ))
            .unwrap();
        repository
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_c.clone(),
                CredentialProbeOutcome::QuotaExhausted,
            ))
            .unwrap();

        let latest = repository
            .load_latest_probe_results_for_credentials(
                &credential_set_id,
                &[credential_a.clone(), credential_b.clone()],
            )
            .unwrap();

        assert_eq!(latest.len(), 2);
        assert_eq!(
            latest.get(&credential_a).unwrap().outcome,
            CredentialProbeOutcome::Success
        );
        assert_eq!(
            latest.get(&credential_b).unwrap().outcome,
            CredentialProbeOutcome::RateLimited
        );
        assert!(!latest.contains_key(&credential_c));
    }

    #[test]
    fn sqlite_repository_rejects_probe_result_for_unknown_credential() {
        let db_path =
            temp_key_path("key-pool-router-probe-results-unknown").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-probe-unknown-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();

        let err = repository
            .record_probe_result(test_probe_input(
                credential_set_id,
                CredentialId("cred_missing".to_string()),
                CredentialProbeOutcome::Unknown,
            ))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown credential cred_missing"));
    }

    #[test]
    fn sqlite_repository_adds_probe_results_schema_to_existing_database() {
        let db_path =
            temp_key_path("key-pool-router-probe-results-migration").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-probe-migration-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let credential_set_id = CredentialSetId("set".to_string());
        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        repository
            .connect()
            .unwrap()
            .execute("DROP TABLE credential_probe_results", [])
            .unwrap();

        let reopened = SqliteCredentialRepository::open(&db_path).unwrap();
        reopened
            .record_probe_result(test_probe_input(
                credential_set_id.clone(),
                credential_id.clone(),
                CredentialProbeOutcome::Success,
            ))
            .unwrap();

        let latest = reopened
            .load_latest_probe_result(&credential_set_id, &credential_id)
            .unwrap()
            .unwrap();
        assert_eq!(latest.outcome, CredentialProbeOutcome::Success);
    }

    #[test]
    fn sqlite_repository_migrates_credentials_without_credential_id_column() {
        let db_path = temp_key_path("key-pool-router-legacy-credential-id-migration")
            .with_extension("sqlite");
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                r#"
                PRAGMA foreign_keys = ON;

                CREATE TABLE credential_sets (
                    id TEXT PRIMARY KEY,
                    source_path TEXT NOT NULL,
                    physical_line_count INTEGER NOT NULL,
                    non_empty_count INTEGER NOT NULL,
                    unique_count INTEGER NOT NULL,
                    duplicate_occurrence_count INTEGER NOT NULL,
                    ignored_empty_count INTEGER NOT NULL,
                    invalid_line_count INTEGER NOT NULL,
                    claimed_count INTEGER,
                    claim_source TEXT,
                    import_generation INTEGER NOT NULL,
                    last_imported_at_unix_seconds INTEGER NOT NULL
                );

                CREATE TABLE credentials (
                    credential_set_id TEXT NOT NULL,
                    secret TEXT NOT NULL,
                    source_path TEXT,
                    source_line INTEGER,
                    batch_id TEXT,
                    fingerprint TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    PRIMARY KEY (credential_set_id, fingerprint),
                    FOREIGN KEY (credential_set_id)
                        REFERENCES credential_sets(id)
                        ON DELETE CASCADE
                );

                "#,
            )
            .unwrap();
        connection
            .execute(
                r#"
                INSERT INTO credential_sets (
                    id,
                    source_path,
                    physical_line_count,
                    non_empty_count,
                    unique_count,
                    duplicate_occurrence_count,
                    ignored_empty_count,
                    invalid_line_count,
                    claimed_count,
                    claim_source,
                    import_generation,
                    last_imported_at_unix_seconds
                ) VALUES (?1, ?2, 1, 1, 1, 0, 0, 0, NULL, NULL, 1, 1)
                "#,
                params!["set", "old-keys.txt"],
            )
            .unwrap();
        connection
            .execute(
                r#"
                INSERT INTO credentials (
                    credential_set_id,
                    secret,
                    source_path,
                    source_line,
                    batch_id,
                    fingerprint,
                    position
                ) VALUES (?1, ?2, ?3, 1, ?1, ?4, 0)
                "#,
                params![
                    "set",
                    fixture_credential_legacy(),
                    "old-keys.txt",
                    "legacy-fingerprint"
                ],
            )
            .unwrap();
        drop(connection);

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        let credential_id =
            credential_id_for_secret(&credential_set_id, fixture_credential_legacy());

        let resource = repository
            .load_credential_resource(&credential_set_id, &credential_id)
            .unwrap()
            .unwrap();

        assert_eq!(resource.credential_id, credential_id);
        assert_eq!(resource.fingerprint, "legacy-fingerprint");
    }

    #[test]
    fn sqlite_repository_records_credential_lifecycle_history_without_raw_secret() {
        let db_path = temp_key_path("key-pool-router-lifecycle-history").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-lifecycle-history-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());

        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: credential_id.clone(),
                    state: CredentialLifecycleState::Expired {
                        reason: "manual expire".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap();
        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: credential_id.clone(),
                    state: CredentialLifecycleState::Available,
                },
                test_lifecycle_evidence(),
            )
            .unwrap();
        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: credential_id.clone(),
                    state: CredentialLifecycleState::Disabled {
                        reason: "manual disable".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap();

        let history = repository
            .load_lifecycle_history(&credential_set_id, &credential_id, 0, 10)
            .unwrap();

        assert_eq!(history.len(), 3);
        assert_eq!(history[0].credential_set_id, credential_set_id);
        assert_eq!(history[0].credential_id, credential_id);
        assert_eq!(
            history[0].state,
            CredentialLifecycleState::Disabled {
                reason: "manual disable".to_string()
            }
        );
        assert_eq!(history[1].state, CredentialLifecycleState::Available);
        assert_eq!(
            history[2].state,
            CredentialLifecycleState::Expired {
                reason: "manual expire".to_string()
            }
        );
        assert!(history
            .iter()
            .all(|record| record.source == CredentialLifecycleHistorySource::ManagementCommand));
        assert!(history
            .iter()
            .all(|record| record.reason_class.as_deref() == Some("test_lifecycle_update")));
        assert_debug_omits_credentials(&format!("{history:?}"), &[fixture_credential_a()]);

        let page = repository
            .load_lifecycle_history(&credential_set_id, &credential_id, 1, 1)
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].state, CredentialLifecycleState::Available);
    }

    #[test]
    fn sqlite_repository_records_lifecycle_history_evidence() {
        let db_path = temp_key_path("key-pool-router-lifecycle-evidence").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-lifecycle-evidence-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let credential_set_id = CredentialSetId("set".to_string());
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());

        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: credential_id.clone(),
                    state: CredentialLifecycleState::Expired {
                        reason: "operator saw invalid auth".to_string(),
                    },
                },
                CredentialLifecycleEvidence::management_command(
                    "manual_expire",
                    "management:local-admin",
                    "local-admin",
                    "admin",
                    "primary",
                ),
            )
            .unwrap();

        let history = repository
            .load_lifecycle_history(&credential_set_id, &credential_id, 0, 10)
            .unwrap();

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].reason_class.as_deref(), Some("manual_expire"));
        assert_eq!(
            history[0].actor_id.as_deref(),
            Some("management:local-admin")
        );
        assert_eq!(history[0].actor_name.as_deref(), Some("local-admin"));
        assert_eq!(history[0].actor_role.as_deref(), Some("admin"));
        assert_eq!(history[0].channel_id.as_deref(), Some("primary"));
        assert_debug_omits_credentials(&format!("{history:?}"), &[fixture_credential_a()]);
    }

    #[test]
    fn sqlite_repository_deduplicates_existing_and_batch_duplicates_without_full_existing_load() {
        let db_path =
            temp_key_path("key-pool-router-sqlite-append-duplicates").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();

        let appended = repository
            .append_credentials(
                &CredentialSetId("sqlite_set".to_string()),
                vec![
                    "first".to_string(),
                    "second".to_string(),
                    "second".to_string(),
                    "third".to_string(),
                ],
                "management-api",
            )
            .unwrap();

        let secrets: Vec<&str> = appended
            .credentials
            .iter()
            .map(|credential| credential.secret.as_str())
            .collect();
        assert_eq!(secrets, vec!["second", "third"]);
        assert_eq!(appended.report.unique_count, 2);
        assert_eq!(appended.report.duplicate_occurrence_count, 2);
        assert_eq!(
            appended.report.duplicate_fingerprints[0].first_source_line,
            1
        );
        assert_eq!(
            appended.report.duplicate_fingerprints[0].duplicate_source_line,
            1
        );
        assert_eq!(
            appended.report.duplicate_fingerprints[1].first_source_line,
            2
        );
        assert_eq!(
            appended.report.duplicate_fingerprints[1].duplicate_source_line,
            3
        );

        let loaded = repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        let loaded_secrets: Vec<&str> = loaded
            .credentials
            .iter()
            .map(|credential| credential.secret.as_str())
            .collect();
        assert_eq!(loaded_secrets, vec!["first", "second", "third"]);
        assert_eq!(loaded.report.unique_count, 3);
        assert_eq!(loaded.report.duplicate_occurrence_count, 2);
    }

    #[test]
    fn sqlite_schema_indexes_secret_lookup_for_append_deduplication() {
        let db_path = temp_key_path("key-pool-router-sqlite-secret-index").with_extension("sqlite");
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let connection = repository.connect().unwrap();

        let plan: Vec<String> = connection
            .prepare(
                r#"
                EXPLAIN QUERY PLAN
                SELECT COALESCE(source_line, position + 1)
                FROM credentials
                WHERE credential_set_id = ?1 AND secret = ?2
                LIMIT 1
                "#,
            )
            .unwrap()
            .query_map(params!["sqlite_set", "first"], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            plan.iter()
                .any(|detail| detail.contains("credentials_set_secret_lookup")),
            "query plan should use credentials_set_secret_lookup, got {plan:?}"
        );
    }

    #[test]
    fn sqlite_repository_uses_sha256_fingerprints_for_persistent_identity() {
        assert_eq!(
            persistent_fingerprint("hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn sqlite_repository_deduplicates_existing_secret_even_if_stored_fingerprint_format_differs() {
        let db_path =
            temp_key_path("key-pool-router-sqlite-fingerprint-change").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();
        repository
            .connect()
            .unwrap()
            .execute(
                "UPDATE credentials SET fingerprint = ?1 WHERE credential_set_id = ?2 AND secret = ?3",
                params![short_hash("first"), "sqlite_set", "first"],
            )
            .unwrap();

        let appended = repository
            .append_credentials(
                &CredentialSetId("sqlite_set".to_string()),
                vec!["first".to_string()],
                "management-api",
            )
            .unwrap();
        assert!(appended.credentials.is_empty());
        assert_eq!(appended.report.duplicate_occurrence_count, 1);

        let loaded = repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        assert_eq!(loaded.credentials.len(), 1);
    }

    #[test]
    fn sqlite_repository_serializes_concurrent_appends_without_position_collisions() {
        let db_path = temp_key_path("key-pool-router-sqlite-concurrent").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|worker| {
                let repository = SqliteCredentialRepository::open(&db_path).unwrap();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let keys = (0..20)
                        .map(|index| format!("worker-{worker}-key-{index}"))
                        .collect();
                    repository
                        .append_credentials(
                            &CredentialSetId("sqlite_set".to_string()),
                            keys,
                            format!("worker-{worker}"),
                        )
                        .unwrap()
                })
            })
            .collect();

        let imported_count: usize = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().credentials.len())
            .sum();
        assert_eq!(imported_count, 40);

        let loaded = repository
            .load_credential_set(
                &CredentialSetId("sqlite_set".to_string()),
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();
        assert_eq!(loaded.credentials.len(), 41);
        assert_eq!(loaded.report.unique_count, 41);
    }

    #[test]
    fn sqlite_repository_persists_credential_lifecycle() {
        let db_path = temp_key_path("key-pool-router-sqlite-lifecycle").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\nsecond\n").unwrap();
        let credential_set_id = CredentialSetId("sqlite_set".to_string());
        let first_id = credential_id_for_secret(&credential_set_id, "first");
        let second_id = credential_id_for_secret(&credential_set_id, "second");

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();

        let expired = repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: first_id.clone(),
                    state: CredentialLifecycleState::Expired {
                        reason: "quota exhausted".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap();
        assert_eq!(
            expired.snapshot.state,
            CredentialLifecycleState::Expired {
                reason: "quota exhausted".to_string()
            }
        );

        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: second_id.clone(),
                    state: CredentialLifecycleState::Disabled {
                        reason: "manual pause".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap();

        let snapshots = repository
            .load_lifecycle_snapshots(&credential_set_id)
            .unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].credential_id, first_id);
        assert_eq!(
            snapshots[0].state,
            CredentialLifecycleState::Expired {
                reason: "quota exhausted".to_string()
            }
        );
        assert_eq!(snapshots[1].credential_id, second_id);
        assert_eq!(
            snapshots[1].state,
            CredentialLifecycleState::Disabled {
                reason: "manual pause".to_string()
            }
        );

        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: second_id,
                    state: CredentialLifecycleState::Available,
                },
                test_lifecycle_evidence(),
            )
            .unwrap();
        let snapshots = repository
            .load_lifecycle_snapshots(&credential_set_id)
            .unwrap();
        assert_eq!(snapshots.len(), 1);
    }

    #[test]
    fn sqlite_repository_rejects_lifecycle_for_unknown_credential() {
        let db_path =
            temp_key_path("key-pool-router-sqlite-lifecycle-unknown").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();
        let credential_set_id = CredentialSetId("sqlite_set".to_string());

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();

        let err = repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id,
                    credential_id: CredentialId("cred_missing".to_string()),
                    state: CredentialLifecycleState::Expired {
                        reason: "bad id".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("unknown credential cred_missing"));
    }

    #[test]
    fn sqlite_repository_rejects_lifecycle_for_unknown_credential_set() {
        let db_path =
            temp_key_path("key-pool-router-sqlite-lifecycle-unknown-set").with_extension("sqlite");
        let repository = SqliteCredentialRepository::open(&db_path).unwrap();

        let err = repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: CredentialSetId("missing_set".to_string()),
                    credential_id: CredentialId("cred_missing".to_string()),
                    state: CredentialLifecycleState::Disabled {
                        reason: "unknown set".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("unknown credential cred_missing in credential set missing_set"));
    }

    #[test]
    fn sqlite_repository_adds_lifecycle_schema_to_existing_database() {
        let db_path =
            temp_key_path("key-pool-router-sqlite-lifecycle-migration").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();
        let credential_set_id = CredentialSetId("sqlite_set".to_string());
        let credential_id = credential_id_for_secret(&credential_set_id, "first");

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();
        repository
            .connect()
            .unwrap()
            .execute("DROP TABLE credential_lifecycle_states", [])
            .unwrap();

        let reopened = SqliteCredentialRepository::open(&db_path).unwrap();
        reopened
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id,
                    state: CredentialLifecycleState::Disabled {
                        reason: "after migration".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap();

        assert_eq!(
            reopened
                .load_lifecycle_snapshots(&credential_set_id)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn sqlite_repository_adds_lifecycle_history_evidence_columns_to_existing_database() {
        let db_path = temp_key_path("key-pool-router-sqlite-lifecycle-history-migration")
            .with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-lifecycle-history-source");
        fs::write(&source_path, "first\n").unwrap();
        let credential_set_id = CredentialSetId("sqlite_set".to_string());
        let credential_id = credential_id_for_secret(&credential_set_id, "first");

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();
        let connection = repository.connect().unwrap();
        connection
            .execute("DROP TABLE credential_lifecycle_history", [])
            .unwrap();
        connection
            .execute(
                r#"
                CREATE TABLE credential_lifecycle_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    credential_set_id TEXT NOT NULL,
                    credential_id TEXT NOT NULL,
                    state_kind TEXT NOT NULL,
                    reason TEXT,
                    source TEXT NOT NULL,
                    created_at_unix_seconds INTEGER NOT NULL
                )
                "#,
                [],
            )
            .unwrap();
        drop(connection);

        let reopened = SqliteCredentialRepository::open(&db_path).unwrap();
        reopened
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: credential_id.clone(),
                    state: CredentialLifecycleState::Expired {
                        reason: "after migration".to_string(),
                    },
                },
                CredentialLifecycleEvidence::management_command(
                    "manual_expire",
                    "management:local-admin",
                    "local-admin",
                    "admin",
                    "primary",
                ),
            )
            .unwrap();

        let history = reopened
            .load_lifecycle_history(&credential_set_id, &credential_id, 0, 10)
            .unwrap();
        assert_eq!(history[0].reason_class.as_deref(), Some("manual_expire"));
        assert_eq!(history[0].actor_name.as_deref(), Some("local-admin"));
        assert_eq!(history[0].channel_id.as_deref(), Some("primary"));
    }

    #[test]
    fn sqlite_repository_migrates_lifecycle_schema_for_quota_exhausted() {
        let db_path =
            temp_key_path("key-pool-router-lifecycle-quota-migration").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-lifecycle-quota-source");
        write_fixture_credentials(&source_path, &[fixture_credential_a()]);
        let credential_set_id = CredentialSetId("set".to_string());
        let credential_id = credential_id_for_secret(&credential_set_id, fixture_credential_a());

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        repository
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();

        let connection = repository.connect().unwrap();
        connection
            .execute(
                "ALTER TABLE credential_lifecycle_states RENAME TO credential_lifecycle_states_new",
                [],
            )
            .unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE credential_lifecycle_states (
                    credential_set_id TEXT NOT NULL,
                    credential_id TEXT NOT NULL,
                    state_kind TEXT NOT NULL CHECK (state_kind IN ('expired', 'disabled')),
                    reason TEXT NOT NULL,
                    updated_at_unix_seconds INTEGER NOT NULL,
                    PRIMARY KEY (credential_set_id, credential_id)
                );
                DROP TABLE credential_lifecycle_states_new;
                ALTER TABLE credential_lifecycle_history RENAME TO credential_lifecycle_history_new;
                CREATE TABLE credential_lifecycle_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    credential_set_id TEXT NOT NULL,
                    credential_id TEXT NOT NULL,
                    state_kind TEXT NOT NULL CHECK (state_kind IN ('available', 'expired', 'disabled')),
                    reason TEXT,
                    source TEXT NOT NULL CHECK (source IN ('management_command', 'compensation')),
                    reason_class TEXT,
                    actor_id TEXT,
                    actor_name TEXT,
                    actor_role TEXT,
                    channel_id TEXT,
                    created_at_unix_seconds INTEGER NOT NULL
                );
                DROP TABLE credential_lifecycle_history_new;
                "#,
            )
            .unwrap();
        drop(connection);

        let reopened = SqliteCredentialRepository::open(&db_path).unwrap();
        reopened
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: credential_id.clone(),
                    state: CredentialLifecycleState::QuotaExhausted {
                        reason: "quota evidence".to_string(),
                    },
                },
                CredentialLifecycleEvidence::management_command(
                    "probe_quota_exhausted",
                    "admin",
                    "local-admin",
                    "admin",
                    "primary",
                ),
            )
            .unwrap();

        let snapshots = reopened
            .load_lifecycle_snapshots(&credential_set_id)
            .unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].state,
            CredentialLifecycleState::QuotaExhausted {
                reason: "quota evidence".to_string()
            }
        );
    }

    #[test]
    fn read_only_store_rejects_lifecycle_persistence() {
        let store = CredentialStoreHandle::read_only_file_bootstrap();
        let err = store
            .persist_lifecycle_update_with_evidence_blocking(
                CredentialLifecycleUpdate {
                    credential_set_id: CredentialSetId("set".to_string()),
                    credential_id: CredentialId("cred_missing".to_string()),
                    state: CredentialLifecycleState::Disabled {
                        reason: "manual pause".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap_err();

        assert_eq!(err, CredentialStoreError::NotWritable);
        assert!(!store.has_lifecycle_snapshot_authority());
        assert_eq!(
            store
                .load_lifecycle_snapshots_for_startup(&CredentialSetId("set".to_string()))
                .unwrap(),
            Vec::new()
        );
    }

    #[tokio::test]
    async fn read_only_store_rejects_resource_reads() {
        let store = CredentialStoreHandle::read_only_file_bootstrap();
        let credential_set_id = CredentialSetId("set".to_string());

        let import_err = store
            .load_import_batch(credential_set_id.clone(), "batch".to_string())
            .await
            .unwrap_err();
        assert_eq!(import_err, CredentialStoreError::NotWritable);

        let history_err = store
            .load_lifecycle_history(
                credential_set_id.clone(),
                CredentialId("cred_missing".to_string()),
                0,
                10,
            )
            .await
            .unwrap_err();
        assert_eq!(history_err, CredentialStoreError::NotWritable);

        let resource_err = store
            .load_credential_resource(credential_set_id, CredentialId("cred_missing".to_string()))
            .await
            .unwrap_err();
        assert_eq!(resource_err, CredentialStoreError::NotWritable);
    }

    #[tokio::test]
    async fn sqlite_store_handle_loads_import_batch_and_credential_resource() {
        let db_path =
            temp_key_path("key-pool-router-sqlite-handle-resources").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-handle-resource-source");
        fs::write(&source_path, "first\n").unwrap();
        let credential_set_id = CredentialSetId("sqlite_set".to_string());
        let credential_id = credential_id_for_secret(&credential_set_id, "first");
        SqliteCredentialRepository::open(&db_path)
            .unwrap()
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File {
                    path: source_path.clone(),
                },
            )
            .unwrap();

        let store = CredentialStoreHandle::sqlite(&db_path).unwrap();
        let import = store
            .load_import_batch(credential_set_id.clone(), "sqlite_set".to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            import.source_kind,
            CredentialImportSourceKind::FileBootstrap
        );
        assert_eq!(
            import.source_ref.as_deref(),
            source_path.file_name().and_then(|name| name.to_str())
        );

        let resource = store
            .load_credential_resource(credential_set_id, credential_id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resource.credential_id, credential_id);
        assert_eq!(resource.source_line, Some(1));
        assert_eq!(resource.batch_id.as_deref(), Some("sqlite_set"));

        store
            .persist_lifecycle_update_with_evidence_blocking(
                CredentialLifecycleUpdate {
                    credential_set_id: resource.credential_set_id.clone(),
                    credential_id: resource.credential_id.clone(),
                    state: CredentialLifecycleState::Expired {
                        reason: "manual expire".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap();
        let history = store
            .load_lifecycle_history(resource.credential_set_id, resource.credential_id, 0, 10)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0].state,
            CredentialLifecycleState::Expired {
                reason: "manual expire".to_string()
            }
        );
    }

    #[tokio::test]
    async fn sqlite_store_handle_persists_and_loads_lifecycle_snapshots() {
        let db_path =
            temp_key_path("key-pool-router-sqlite-handle-lifecycle").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();
        let credential_set_id = CredentialSetId("sqlite_set".to_string());
        let credential_id = credential_id_for_secret(&credential_set_id, "first");
        SqliteCredentialRepository::open(&db_path)
            .unwrap()
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();

        let store = CredentialStoreHandle::sqlite(&db_path).unwrap();
        assert!(store.has_lifecycle_snapshot_authority());
        store
            .persist_lifecycle_update_with_evidence_blocking(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: credential_id.clone(),
                    state: CredentialLifecycleState::Expired {
                        reason: "quota exhausted".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap();

        let snapshots = store
            .load_lifecycle_snapshots_for_startup(&credential_set_id)
            .unwrap();
        assert_eq!(
            snapshots,
            vec![CredentialLifecycleSnapshot {
                credential_set_id,
                credential_id,
                state: CredentialLifecycleState::Expired {
                    reason: "quota exhausted".to_string()
                },
            }]
        );
    }

    #[test]
    fn sqlite_store_handle_maps_lifecycle_repository_errors_to_persistence() {
        let db_path = temp_key_path("key-pool-router-sqlite-handle-error").with_extension("sqlite");
        let source_path = temp_key_path("key-pool-router-sqlite-source");
        fs::write(&source_path, "first\n").unwrap();
        let credential_set_id = CredentialSetId("sqlite_set".to_string());
        SqliteCredentialRepository::open(&db_path)
            .unwrap()
            .load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File { path: source_path },
            )
            .unwrap();

        let store = CredentialStoreHandle::sqlite(&db_path).unwrap();
        let err = store
            .persist_lifecycle_update_with_evidence_blocking(
                CredentialLifecycleUpdate {
                    credential_set_id,
                    credential_id: CredentialId("cred_missing".to_string()),
                    state: CredentialLifecycleState::Expired {
                        reason: "bad id".to_string(),
                    },
                },
                test_lifecycle_evidence(),
            )
            .unwrap_err();

        assert!(
            matches!(err, CredentialStoreError::Persistence(message) if message.contains("unknown credential cred_missing"))
        );
    }
}
