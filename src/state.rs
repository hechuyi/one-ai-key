use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex, RwLock as StdRwLock,
    },
    time::{Duration, Instant},
};

use tokio::sync::{mpsc, Mutex, RwLock};

use crate::{
    client_token_store::ClientTokenStoreHandle,
    config::{
        ProbeResultPolicy, ResolvedClientToken, ResolvedConfig, ResolvedErrorPolicySources,
        ResolvedManagementIpAllowlist, ResolvedManagementPrincipal, ResolvedModelGroup,
        ResolvedPolicyProfile, ResolvedPoolConfig, ResolvedRoutingConfig, ResolvedRoutingProfile,
        ResolvedTimeoutProfile,
    },
    credential_repository::{
        CredentialLifecycleEvidence, CredentialLifecycleSnapshot, CredentialLifecycleState,
        CredentialLifecycleUpdate, CredentialSetId, CredentialStoreHandle, KeyImportReport,
    },
    credentials::{CredentialId, CredentialStateSnapshot},
    error::ErrorClassifier,
    events::{
        DomainEvent, EventLog, ManagementEvent, ResponseFilterEventBuffer, RoutingTelemetryBuffer,
    },
    pool::KeyPool,
    provider::{ProviderAdapter, ProviderKind},
    registry::RegistryDocument,
    registry_store::RegistryStoreHandle,
    response_filter::ResponseFilterPolicy,
    route_plan::{ChannelRouteState, ModelRoute},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelId(pub String);

#[derive(Clone)]
pub struct AppState {
    pub started_at: Instant,
    pub client_tokens: Arc<StdRwLock<Vec<ResolvedClientToken>>>,
    pub management_principals: Arc<Vec<ResolvedManagementPrincipal>>,
    pub management_ip_allowlist: Arc<StdRwLock<ResolvedManagementIpAllowlist>>,
    pub max_request_body_bytes: usize,
    pub max_model_catalog_body_bytes: usize,
    pub max_error_body_bytes: usize,
    pub timeout_profile: ResolvedTimeoutProfile,
    pub routing: ResolvedRoutingConfig,
    pub response_filter: Arc<StdRwLock<ResponseFilterPolicy>>,
    pub response_filter_events: Arc<StdMutex<ResponseFilterEventBuffer>>,
    pub response_filter_alert_window: Arc<StdRwLock<Duration>>,
    pub credential_store: CredentialStoreHandle,
    pub client_token_store: ClientTokenStoreHandle,
    pub registry_store: RegistryStoreHandle,
    pub registry_mutation_lock: Arc<tokio::sync::Mutex<()>>,
    pub active_registry_version: Arc<StdRwLock<Option<u64>>>,
    pub runtime_reload_status: Arc<StdRwLock<RuntimeReloadStatus>>,
    pub registry_validation_bootstrap: Arc<RegistryDocument>,
    pub runtime_catalogs: RuntimeCatalogs,
    pub channels: ChannelRegistry,
    pub http_client: reqwest::Client,
    pub events: EventLog,
    pub routing_telemetry: Arc<StdMutex<RoutingTelemetryBuffer>>,
    pub routing_telemetry_lock_contention_drops: Arc<AtomicU64>,
    pub response_filter_event_lock_contention_drops: Arc<AtomicU64>,
    pub lifecycle_persistence: LifecyclePersistenceQueue,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeReloadStatus {
    pub last_reload_at_unix_seconds: Option<u64>,
    pub last_reload_error_reason_code: Option<String>,
}

#[derive(Clone)]
pub struct LifecyclePersistenceQueue {
    tx: Option<mpsc::Sender<AutomaticLifecyclePersistenceRequest>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecyclePersistenceDropReason {
    Unavailable,
    Full,
    Closed,
}

impl LifecyclePersistenceDropReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "queue_unavailable",
            Self::Full => "queue_full",
            Self::Closed => "queue_closed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AutomaticLifecyclePersistenceRequest {
    pub credential_set_id: CredentialSetId,
    pub credential_id: CredentialId,
    pub state: AutomaticLifecyclePersistenceState,
    pub reason_class: &'static str,
    pub channel_id: String,
}

#[derive(Debug, Clone)]
pub enum AutomaticLifecyclePersistenceState {
    Expired { reason: String },
    QuotaExhausted { reason: String },
}

impl AutomaticLifecyclePersistenceState {
    pub fn expired(reason: impl Into<String>) -> Self {
        Self::Expired {
            reason: reason.into(),
        }
    }

    pub fn quota_exhausted(reason: impl Into<String>) -> Self {
        Self::QuotaExhausted {
            reason: reason.into(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Expired { .. } => "expired",
            Self::QuotaExhausted { .. } => "quota_exhausted",
        }
    }

    fn into_lifecycle_state(self) -> CredentialLifecycleState {
        match self {
            Self::Expired { reason } => CredentialLifecycleState::Expired { reason },
            Self::QuotaExhausted { reason } => CredentialLifecycleState::QuotaExhausted { reason },
        }
    }
}

impl AutomaticLifecyclePersistenceRequest {
    pub fn lifecycle_state_name(&self) -> &'static str {
        self.state.as_str()
    }
}

impl LifecyclePersistenceQueue {
    fn new(store: CredentialStoreHandle) -> Self {
        let (tx, rx) = mpsc::channel(1024);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(run_lifecycle_persistence_worker(store, rx));
                Self { tx: Some(tx) }
            }
            Err(_) => Self { tx: None },
        }
    }

    pub fn try_enqueue(
        &self,
        request: AutomaticLifecyclePersistenceRequest,
    ) -> Result<(), LifecyclePersistenceDropReason> {
        let Some(tx) = self.tx.as_ref() else {
            return Err(LifecyclePersistenceDropReason::Unavailable);
        };
        tx.try_send(request).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => LifecyclePersistenceDropReason::Full,
            mpsc::error::TrySendError::Closed(_) => LifecyclePersistenceDropReason::Closed,
        })
    }
}

async fn run_lifecycle_persistence_worker(
    store: CredentialStoreHandle,
    mut rx: mpsc::Receiver<AutomaticLifecyclePersistenceRequest>,
) {
    while let Some(request) = rx.recv().await {
        let store = store.clone();
        let result = tokio::task::spawn_blocking(move || {
            let update = CredentialLifecycleUpdate {
                credential_set_id: request.credential_set_id,
                credential_id: request.credential_id,
                state: request.state.into_lifecycle_state(),
            };
            let evidence = CredentialLifecycleEvidence::automatic_failure(
                request.reason_class,
                request.channel_id,
            );
            store.persist_lifecycle_update_with_evidence_blocking(update, evidence)
        })
        .await;
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                tracing::warn!(?err, "automatic credential lifecycle persistence failed");
            }
            Err(err) => {
                tracing::warn!(
                    ?err,
                    "automatic credential lifecycle persistence task failed"
                );
            }
        }
    }
}

#[derive(Clone)]
pub struct ChannelRegistry {
    snapshot: Arc<StdRwLock<ChannelRegistrySnapshot>>,
}

#[derive(Clone)]
pub struct RuntimeCatalogs {
    snapshot: Arc<StdRwLock<RuntimeCatalogSnapshot>>,
}

#[derive(Clone)]
struct RuntimeCatalogSnapshot {
    registry_generation: u64,
    policy_profiles: Arc<HashMap<String, ResolvedPolicyProfile>>,
    routing_profiles: Arc<HashMap<String, ResolvedRoutingProfile>>,
    model_groups: Arc<HashMap<String, Vec<String>>>,
    management_projection: Arc<ManagementProjection>,
}

#[derive(Clone)]
struct ChannelRegistrySnapshot {
    registry_generation: u64,
    default_channel: Option<String>,
    model_routes: Arc<HashMap<String, ModelRoute>>,
    public_model_catalog: Arc<Vec<PublicModelCatalogEntry>>,
    claimed_upstream_models: Arc<HashMap<String, Vec<String>>>,
    credential_set_channels: Arc<HashMap<CredentialSetId, Vec<String>>>,
    failure_domains: FailureDomainRegistry,
    channels: Arc<HashMap<String, PoolState>>,
}

#[derive(Clone)]
pub(crate) struct PublicModelCatalogEntry {
    pub(crate) public_model: String,
    pub(crate) targets: Vec<PublicModelCatalogTarget>,
}

#[derive(Clone)]
pub(crate) struct PublicModelCatalogTarget {
    pub(crate) channel_id: String,
    pub(crate) health: Arc<StdMutex<ChannelHealth>>,
}

pub struct ChannelRoutePlanContext {
    pub registry_generation: u64,
    pub model_route: Option<ModelRoute>,
    pub model_route_channel_states: HashMap<ChannelId, ChannelRouteState>,
    claimed_upstream_channels: Vec<String>,
    pub default_channel: Option<String>,
    pub default_channel_provider_kind: Option<ProviderKind>,
    pub default_channel_route_state: Option<ChannelRouteState>,
}

impl ChannelRoutePlanContext {
    pub fn has_claimed_upstream_model(&self, allowed_channels: &[String]) -> bool {
        any_allowed_claimed_upstream_channel(&self.claimed_upstream_channels, allowed_channels)
    }
}

pub struct ChannelNamedRoutePlanContext {
    pub registry_generation: u64,
    pub route_state: ChannelRouteState,
}

pub struct ChannelModelRoutesContext {
    pub registry_generation: u64,
    pub default_channel: Option<String>,
    pub routes: Vec<ChannelModelRouteContext>,
}

pub struct ChannelModelRouteContext {
    pub route: ModelRoute,
    pub target_health: HashMap<ChannelId, Option<ChannelHealth>>,
}

#[derive(Clone, Default)]
pub struct FailureDomainRegistry {
    providers: Arc<StdRwLock<HashMap<String, FailureDomainState>>>,
    accounts: Arc<StdRwLock<HashMap<String, FailureDomainState>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureDomainState {
    Closed,
    Degraded { reason: String },
    Open { until: Instant, reason: String },
}

#[derive(Clone, Default)]
pub struct ManagementProjection {
    pub channel_error_policies: HashMap<String, ResolvedErrorPolicySources>,
    pub policy_profile_channels: HashMap<String, Vec<String>>,
    pub routing_profile_channels: HashMap<String, Vec<String>>,
    pub runtime_topology_summary: RuntimeTopologySummary,
    pub channel_topologies: Vec<ChannelTopologyProjection>,
    pub provider_topologies: Vec<ProviderTopologyProjection>,
    pub account_topologies: Vec<AccountTopologyProjection>,
    pub credential_set_topologies: Vec<CredentialSetTopologyProjection>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeTopologySummary {
    pub channels: usize,
    pub same_request_retry_enabled_channels: usize,
    pub max_same_request_retries: usize,
}

#[derive(Clone)]
pub struct ChannelTopologyProjection {
    pub id: String,
    pub configured_enabled: bool,
    pub provider_id: String,
    pub account_id: String,
    pub credential_set_id: CredentialSetId,
    pub provider_kind: ProviderKind,
    pub endpoint_capabilities: crate::endpoint_capabilities::ResolvedEndpointCapabilities,
    pub api_base: String,
    pub auth_header: String,
    pub auth_prefix_configured: bool,
    pub routing_profile_id: String,
    pub retry_switched_key_in_same_request: bool,
    pub max_same_request_retries: usize,
    pub route_target_retry_enabled: bool,
    pub default_credential_cooldown: Duration,
    pub config_generation: u64,
}

#[derive(Clone)]
pub struct ProviderTopologyProjection {
    pub id: String,
    pub provider_kind: ProviderKind,
    pub enabled: bool,
    pub channel_ids: Vec<String>,
    pub account_ids: Vec<String>,
    pub credential_set_ids: Vec<CredentialSetId>,
}

#[derive(Clone)]
pub struct AccountTopologyProjection {
    pub id: String,
    pub provider_id: String,
    pub provider_kind: ProviderKind,
    pub enabled: bool,
    pub provider_enabled: bool,
    pub effective_enabled: bool,
    pub channel_ids: Vec<String>,
    pub credential_set_ids: Vec<CredentialSetId>,
}

#[derive(Clone)]
pub struct CredentialSetTopologyProjection {
    pub id: CredentialSetId,
    pub channel_ids: Vec<String>,
    pub account_ids: Vec<String>,
    pub provider_ids: Vec<String>,
}

#[derive(Clone)]
pub struct PoolState {
    pub config_generation: u64,
    pub configured_enabled: bool,
    pub channel_health_generation: Arc<AtomicU64>,
    pub selector_generation: Arc<AtomicU64>,
    pub provider_id: String,
    pub account_id: String,
    pub account_enabled: bool,
    pub failure_domains: FailureDomainRegistry,
    pub probe_result_policy: ProbeResultPolicy,
    pub credential_set_id: CredentialSetId,
    pub provider_kind: ProviderKind,
    pub api_base: String,
    pub auth_header: String,
    pub auth_prefix: String,
    pub retry_switched_key_in_same_request: bool,
    pub max_same_request_retries: usize,
    pub route_target_retry_enabled: bool,
    pub default_credential_cooldown: Duration,
    pub error_classifier: ErrorClassifier,
    pub key_import_report: Arc<StdMutex<KeyImportReport>>,
    pub mutation_gate: Arc<Mutex<()>>,
    pub send_gate: Arc<RwLock<()>>,
    pub pool: Arc<Mutex<KeyPool>>,
    pub health: Arc<StdMutex<ChannelHealth>>,
    pub relay_suppression_count: Arc<AtomicU64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelHealth {
    Available,
    CoolingDown { until: Instant, reason: String },
    Degraded { reason: String },
    Disabled { reason: String },
}

impl ChannelHealth {
    pub fn is_available_for_routing(&self) -> bool {
        match self {
            ChannelHealth::Available => true,
            ChannelHealth::CoolingDown { until, .. } => Instant::now() >= *until,
            ChannelHealth::Degraded { .. } | ChannelHealth::Disabled { .. } => false,
        }
    }
}

impl FailureDomainRegistry {
    fn new(channel_ids: impl Iterator<Item = (String, String)>) -> Self {
        let mut providers = HashMap::new();
        let mut accounts = HashMap::new();
        for (provider_id, account_id) in channel_ids {
            providers.insert(provider_id, FailureDomainState::Closed);
            accounts.insert(account_id, FailureDomainState::Closed);
        }
        Self {
            providers: Arc::new(StdRwLock::new(providers)),
            accounts: Arc::new(StdRwLock::new(accounts)),
        }
    }

    pub fn apply_provider_unavailable(
        &self,
        provider_id: &str,
        account_id: &str,
        until: Option<Instant>,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        let state = match until {
            Some(until) => FailureDomainState::Open {
                until,
                reason: reason.clone(),
            },
            None => FailureDomainState::Degraded {
                reason: reason.clone(),
            },
        };
        self.set_provider_state(provider_id, state.clone());
        self.set_account_state(account_id, state);
    }

    pub fn record_success(&self, provider_id: &str, account_id: &str) {
        self.set_account_state(account_id, FailureDomainState::Closed);
        self.set_provider_state(provider_id, FailureDomainState::Closed);
    }

    fn set_provider_state(&self, provider_id: &str, state: FailureDomainState) {
        if let Some(existing) = self
            .providers
            .write()
            .expect("provider failure domain lock poisoned")
            .get_mut(provider_id)
        {
            *existing = state;
        }
    }

    fn set_account_state(&self, account_id: &str, state: FailureDomainState) {
        if let Some(existing) = self
            .accounts
            .write()
            .expect("account failure domain lock poisoned")
            .get_mut(account_id)
        {
            *existing = state;
        }
    }

    fn route_state(&self, provider_id: &str, account_id: &str) -> Option<ChannelRouteState> {
        if failure_domain_state_is_open(
            self.accounts
                .read()
                .expect("account failure domain lock poisoned")
                .get(account_id),
        ) {
            return Some(ChannelRouteState::ProviderCoolingDown);
        }
        if failure_domain_state_is_open(
            self.providers
                .read()
                .expect("provider failure domain lock poisoned")
                .get(provider_id),
        ) {
            return Some(ChannelRouteState::ProviderCoolingDown);
        }
        None
    }
}

fn failure_domain_state_is_open(state: Option<&FailureDomainState>) -> bool {
    matches!(state, Some(FailureDomainState::Open { until, .. }) if Instant::now() < *until)
}

#[derive(Clone)]
struct RuntimeCredentialSet {
    selector_generation: Arc<AtomicU64>,
    mutation_gate: Arc<Mutex<()>>,
    send_gate: Arc<RwLock<()>>,
    pool: Arc<Mutex<KeyPool>>,
    key_import_report: Arc<StdMutex<KeyImportReport>>,
}

impl RuntimeCredentialSet {
    fn from_config(
        config: &ResolvedPoolConfig,
        channel_ids: &[String],
        persisted_events: &[ManagementEvent],
        lifecycle_snapshots: &[CredentialLifecycleSnapshot],
    ) -> anyhow::Result<Self> {
        let mut pool = KeyPool::new(config.key_pool.clone())?;
        apply_persisted_events(channel_ids, &mut pool, persisted_events);
        apply_lifecycle_snapshots(&mut pool, lifecycle_snapshots)?;

        Ok(Self {
            selector_generation: Arc::new(AtomicU64::new(1)),
            mutation_gate: Arc::new(Mutex::new(())),
            send_gate: Arc::new(RwLock::new(())),
            pool: Arc::new(Mutex::new(pool)),
            key_import_report: Arc::new(StdMutex::new(config.key_import_report.clone())),
        })
    }
}

impl AppState {
    #[cfg(test)]
    pub fn new(config: ResolvedConfig) -> anyhow::Result<Self> {
        Self::new_with_registry_store(config, RegistryStoreHandle::read_only())
    }

    #[cfg(test)]
    pub fn new_with_registry_store(
        config: ResolvedConfig,
        registry_store: RegistryStoreHandle,
    ) -> anyhow::Result<Self> {
        Self::new_with_registry_store_and_validation_bootstrap(config, registry_store, None)
    }

    pub fn new_with_registry_store_and_validation_bootstrap(
        config: ResolvedConfig,
        registry_store: RegistryStoreHandle,
        registry_validation_bootstrap: Option<RegistryDocument>,
    ) -> anyhow::Result<Self> {
        let persisted_events =
            EventLog::replay_events_from_path(config.management_event_log_path.as_deref())?;
        let events = EventLog::open_with_window_capacity(
            config.management_event_log_path.clone(),
            config.management_event_window_capacity,
        )?;
        let credential_store = match config.credential_store_path.clone() {
            Some(path) => CredentialStoreHandle::sqlite(path)?,
            None => CredentialStoreHandle::read_only_file_bootstrap(),
        };
        let client_token_store = match config.credential_store_path.clone() {
            Some(path) => ClientTokenStoreHandle::sqlite(path)?,
            None => ClientTokenStoreHandle::read_only_bootstrap(),
        };
        let client_tokens = client_token_store
            .load_or_bootstrap(&config.client_tokens)
            .map_err(|err| anyhow::anyhow!("client token startup restore failed: {err:?}"))?;
        let active_registry_version = registry_store
            .current_version_for_startup()
            .map_err(|err| anyhow::anyhow!("registry startup version restore failed: {err:?}"))?;
        let pool_configs: Vec<(String, ResolvedPoolConfig)> = config.pools.into_iter().collect();
        let failure_domains = FailureDomainRegistry::new(
            pool_configs
                .iter()
                .map(|(_, config)| (config.provider_id.clone(), config.account_id.clone())),
        );
        let management_projection = ManagementProjection::from_pool_configs(&pool_configs);
        let registry_generation = next_registry_generation();
        let runtime_catalogs = RuntimeCatalogs::new(
            registry_generation,
            config.policy_profiles,
            config.routing_profiles,
            config.model_groups,
            management_projection,
        );
        let mut credential_set_channels: HashMap<CredentialSetId, Vec<String>> = HashMap::new();
        for (name, pool_config) in &pool_configs {
            credential_set_channels
                .entry(pool_config.credential_set_id.clone())
                .or_default()
                .push(name.clone());
        }
        let mut pools = HashMap::new();
        let mut credential_set_runtimes: HashMap<CredentialSetId, RuntimeCredentialSet> =
            HashMap::new();
        for (name, pool_config) in pool_configs {
            let runtime = match credential_set_runtimes.get(&pool_config.credential_set_id) {
                Some(runtime) => runtime.clone(),
                None => {
                    let channel_ids = credential_set_channels
                        .get(&pool_config.credential_set_id)
                        .cloned()
                        .unwrap_or_else(|| vec![name.clone()]);
                    let runtime = RuntimeCredentialSet::from_config(
                        &pool_config,
                        &channel_ids,
                        credential_events_for_startup(
                            credential_store.has_lifecycle_snapshot_authority(),
                            &persisted_events,
                        ),
                        &credential_store
                            .load_lifecycle_snapshots_for_startup(&pool_config.credential_set_id)
                            .map_err(|err| {
                                anyhow::anyhow!(
                                    "credential lifecycle startup restore failed: {err:?}"
                                )
                            })?,
                    )?;
                    credential_set_runtimes
                        .insert(pool_config.credential_set_id.clone(), runtime.clone());
                    runtime
                }
            };
            pools.insert(
                name,
                PoolState::from_config(pool_config, runtime, failure_domains.clone())?,
            );
        }
        apply_persisted_channel_events(&pools, &persisted_events);

        let lifecycle_persistence = LifecyclePersistenceQueue::new(credential_store.clone());
        let channels = ChannelRegistry::new(
            registry_generation,
            config.default_pool,
            config.model_routes,
            failure_domains,
            pools,
        );
        debug_assert_eq!(
            runtime_catalogs.registry_generation(),
            channels.registry_generation()
        );

        Ok(Self {
            started_at: Instant::now(),
            client_tokens: Arc::new(StdRwLock::new(client_tokens)),
            management_principals: Arc::new(config.management_principals),
            management_ip_allowlist: Arc::new(StdRwLock::new(config.management_ip_allowlist)),
            max_request_body_bytes: config.max_request_body_bytes,
            max_model_catalog_body_bytes: config.max_model_catalog_body_bytes,
            max_error_body_bytes: config.max_error_body_bytes,
            timeout_profile: config.timeout_profile.clone(),
            routing: config.routing.clone(),
            response_filter: Arc::new(StdRwLock::new(config.response_filter)),
            response_filter_events: Arc::new(StdMutex::new(ResponseFilterEventBuffer::new(
                config.response_filter_event_window_capacity,
            ))),
            response_filter_alert_window: Arc::new(StdRwLock::new(
                config.response_filter_alert_window,
            )),
            credential_store,
            client_token_store,
            registry_store,
            registry_mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            active_registry_version: Arc::new(StdRwLock::new(active_registry_version)),
            runtime_reload_status: Arc::new(StdRwLock::new(RuntimeReloadStatus::default())),
            registry_validation_bootstrap: Arc::new(
                registry_validation_bootstrap.unwrap_or_else(empty_registry_validation_bootstrap),
            ),
            runtime_catalogs,
            channels,
            http_client: reqwest::Client::builder()
                .connect_timeout(config.timeout_profile.connect)
                .read_timeout(config.timeout_profile.streaming_idle)
                .build()?,
            events,
            routing_telemetry: Arc::new(StdMutex::new(RoutingTelemetryBuffer::new(
                config.routing.telemetry_buffer_capacity,
            ))),
            routing_telemetry_lock_contention_drops: Arc::new(AtomicU64::new(0)),
            response_filter_event_lock_contention_drops: Arc::new(AtomicU64::new(0)),
            lifecycle_persistence,
        })
    }

    pub fn rebuild_runtime_from_resolved_config(
        &self,
        config: ResolvedConfig,
        active_registry_version: Option<u64>,
    ) -> anyhow::Result<()> {
        let persisted_events =
            EventLog::replay_events_from_path(config.management_event_log_path.as_deref())?;
        let pool_configs: Vec<(String, ResolvedPoolConfig)> = config.pools.into_iter().collect();
        let failure_domains = FailureDomainRegistry::new(
            pool_configs
                .iter()
                .map(|(_, config)| (config.provider_id.clone(), config.account_id.clone())),
        );
        let management_projection = ManagementProjection::from_pool_configs(&pool_configs);
        let mut credential_set_channels: HashMap<CredentialSetId, Vec<String>> = HashMap::new();
        for (name, pool_config) in &pool_configs {
            credential_set_channels
                .entry(pool_config.credential_set_id.clone())
                .or_default()
                .push(name.clone());
        }
        let mut pools = HashMap::new();
        let mut credential_set_runtimes: HashMap<CredentialSetId, RuntimeCredentialSet> =
            HashMap::new();
        for (name, pool_config) in pool_configs {
            let runtime = match credential_set_runtimes.get(&pool_config.credential_set_id) {
                Some(runtime) => runtime.clone(),
                None => {
                    let channel_ids = credential_set_channels
                        .get(&pool_config.credential_set_id)
                        .cloned()
                        .unwrap_or_else(|| vec![name.clone()]);
                    let runtime = RuntimeCredentialSet::from_config(
                        &pool_config,
                        &channel_ids,
                        credential_events_for_startup(
                            self.credential_store.has_lifecycle_snapshot_authority(),
                            &persisted_events,
                        ),
                        &self
                            .credential_store
                            .load_lifecycle_snapshots_for_startup(&pool_config.credential_set_id)
                            .map_err(|err| {
                                anyhow::anyhow!(
                                    "credential lifecycle runtime reload failed: {err:?}"
                                )
                            })?,
                    )?;
                    credential_set_runtimes
                        .insert(pool_config.credential_set_id.clone(), runtime.clone());
                    runtime
                }
            };
            pools.insert(
                name,
                PoolState::from_config(pool_config, runtime, failure_domains.clone())?,
            );
        }
        apply_persisted_channel_events(&pools, &persisted_events);

        let registry_generation = next_registry_generation();
        self.channels.replace(
            registry_generation,
            config.default_pool,
            config.model_routes,
            failure_domains,
            pools,
        );
        self.runtime_catalogs.replace(
            registry_generation,
            config.policy_profiles,
            config.routing_profiles,
            config.model_groups,
            management_projection,
        );
        debug_assert_eq!(
            self.runtime_catalogs.registry_generation(),
            self.channels.registry_generation()
        );
        *self
            .response_filter
            .write()
            .expect("response filter lock poisoned") = config.response_filter;
        self.response_filter_events
            .lock()
            .expect("response filter events lock poisoned")
            .replace_capacity(config.response_filter_event_window_capacity);
        self.routing_telemetry
            .lock()
            .expect("routing telemetry lock poisoned")
            .replace_capacity(config.routing.telemetry_buffer_capacity);
        *self
            .response_filter_alert_window
            .write()
            .expect("response filter alert window lock poisoned") =
            config.response_filter_alert_window;
        *self
            .management_ip_allowlist
            .write()
            .expect("management IP allowlist lock poisoned") = config.management_ip_allowlist;
        *self
            .active_registry_version
            .write()
            .expect("active registry version lock poisoned") = active_registry_version;
        Ok(())
    }

    pub fn runtime_reload_status_snapshot(&self) -> RuntimeReloadStatus {
        self.runtime_reload_status
            .read()
            .expect("runtime reload status lock poisoned")
            .clone()
    }

    pub fn record_runtime_reload_success(&self, created_at_unix_seconds: u64) {
        *self
            .runtime_reload_status
            .write()
            .expect("runtime reload status lock poisoned") = RuntimeReloadStatus {
            last_reload_at_unix_seconds: Some(created_at_unix_seconds),
            last_reload_error_reason_code: None,
        };
    }

    pub fn record_runtime_reload_failure(
        &self,
        created_at_unix_seconds: u64,
        reason_code: impl Into<String>,
    ) {
        *self
            .runtime_reload_status
            .write()
            .expect("runtime reload status lock poisoned") = RuntimeReloadStatus {
            last_reload_at_unix_seconds: Some(created_at_unix_seconds),
            last_reload_error_reason_code: Some(reason_code.into()),
        };
    }
}

impl RuntimeCatalogs {
    fn new(
        registry_generation: u64,
        policy_profiles: HashMap<String, ResolvedPolicyProfile>,
        routing_profiles: HashMap<String, ResolvedRoutingProfile>,
        model_groups: HashMap<String, ResolvedModelGroup>,
        management_projection: ManagementProjection,
    ) -> Self {
        Self {
            snapshot: Arc::new(StdRwLock::new(RuntimeCatalogSnapshot {
                registry_generation,
                policy_profiles: Arc::new(policy_profiles),
                routing_profiles: Arc::new(routing_profiles),
                model_groups: Arc::new(model_group_memberships(model_groups)),
                management_projection: Arc::new(management_projection),
            })),
        }
    }

    pub fn replace(
        &self,
        registry_generation: u64,
        policy_profiles: HashMap<String, ResolvedPolicyProfile>,
        routing_profiles: HashMap<String, ResolvedRoutingProfile>,
        model_groups: HashMap<String, ResolvedModelGroup>,
        management_projection: ManagementProjection,
    ) {
        *self
            .snapshot
            .write()
            .expect("runtime catalog snapshot lock poisoned") = RuntimeCatalogSnapshot {
            registry_generation,
            policy_profiles: Arc::new(policy_profiles),
            routing_profiles: Arc::new(routing_profiles),
            model_groups: Arc::new(model_group_memberships(model_groups)),
            management_projection: Arc::new(management_projection),
        };
    }

    fn snapshot(&self) -> RuntimeCatalogSnapshot {
        self.snapshot
            .read()
            .expect("runtime catalog snapshot lock poisoned")
            .clone()
    }

    pub fn registry_generation(&self) -> u64 {
        self.snapshot().registry_generation
    }

    pub fn policy_profiles(&self) -> Vec<ResolvedPolicyProfile> {
        self.snapshot().policy_profiles.values().cloned().collect()
    }

    pub fn policy_profile(&self, profile_id: &str) -> Option<ResolvedPolicyProfile> {
        self.snapshot().policy_profiles.get(profile_id).cloned()
    }

    pub fn routing_profiles(&self) -> Vec<ResolvedRoutingProfile> {
        self.snapshot().routing_profiles.values().cloned().collect()
    }

    pub fn routing_profile(&self, profile_id: &str) -> Option<ResolvedRoutingProfile> {
        self.snapshot().routing_profiles.get(profile_id).cloned()
    }

    pub fn channel_error_policy(&self, channel_id: &str) -> Option<ResolvedErrorPolicySources> {
        self.snapshot()
            .management_projection
            .channel_error_policies
            .get(channel_id)
            .cloned()
    }

    pub fn policy_profile_channel_ids(&self, profile_id: &str) -> Vec<String> {
        self.snapshot()
            .management_projection
            .policy_profile_channels
            .get(profile_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn routing_profile_channel_ids(&self, profile_id: &str) -> Vec<String> {
        self.snapshot()
            .management_projection
            .routing_profile_channels
            .get(profile_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn runtime_topology_summary(&self) -> RuntimeTopologySummary {
        self.snapshot()
            .management_projection
            .runtime_topology_summary
            .clone()
    }

    pub fn channel_topologies(&self) -> Vec<ChannelTopologyProjection> {
        self.snapshot()
            .management_projection
            .channel_topologies
            .clone()
    }

    pub fn channel_endpoint_capabilities(
        &self,
    ) -> (
        u64,
        HashMap<String, crate::endpoint_capabilities::ResolvedEndpointCapabilities>,
    ) {
        let snapshot = self.snapshot();
        let capabilities = snapshot
            .management_projection
            .channel_topologies
            .iter()
            .map(|topology| (topology.id.clone(), topology.endpoint_capabilities.clone()))
            .collect();
        (snapshot.registry_generation, capabilities)
    }

    pub fn provider_topologies(&self) -> Vec<ProviderTopologyProjection> {
        self.snapshot()
            .management_projection
            .provider_topologies
            .clone()
    }

    pub fn account_topologies(&self) -> Vec<AccountTopologyProjection> {
        self.snapshot()
            .management_projection
            .account_topologies
            .clone()
    }

    pub fn credential_set_topologies(&self) -> Vec<CredentialSetTopologyProjection> {
        self.snapshot()
            .management_projection
            .credential_set_topologies
            .clone()
    }

    pub fn client_model_allowed(&self, allowed_model_groups: &[String], model: &str) -> bool {
        if allowed_model_groups.is_empty() {
            return true;
        }
        let snapshot = self.snapshot();
        allowed_model_groups.iter().any(|allowed| {
            allowed == model
                || snapshot
                    .model_groups
                    .get(allowed)
                    .is_some_and(|models| models.iter().any(|member| member == model))
        })
    }
}

fn model_group_memberships(
    model_groups: HashMap<String, ResolvedModelGroup>,
) -> HashMap<String, Vec<String>> {
    model_groups
        .into_iter()
        .map(|(id, group)| {
            debug_assert_eq!(id, group.id);
            (group.id, group.models)
        })
        .collect()
}

fn empty_registry_validation_bootstrap() -> RegistryDocument {
    RegistryDocument {
        listen: "127.0.0.1:0".parse().expect("valid bootstrap listen"),
        client_tokens: Vec::new(),
        management: None,
        max_request_body_bytes: 1024 * 1024,
        max_model_catalog_body_bytes: 512 * 1024,
        max_error_body_bytes: 1024,
        timeouts: Default::default(),
        routing: Default::default(),
        response_filter: Default::default(),
        default_pool: None,
        providers: HashMap::new(),
        accounts: HashMap::new(),
        credential_sets: HashMap::new(),
        model_groups: HashMap::new(),
        policy_profiles: HashMap::new(),
        default_routing_profile: None,
        routing_profiles: HashMap::new(),
        model_routes: HashMap::new(),
        pools: HashMap::new(),
    }
}

impl ChannelRegistry {
    fn new(
        registry_generation: u64,
        default_channel: Option<String>,
        model_routes: HashMap<String, ModelRoute>,
        failure_domains: FailureDomainRegistry,
        channels: HashMap<String, PoolState>,
    ) -> Self {
        Self {
            snapshot: Arc::new(StdRwLock::new(compile_channel_registry_snapshot(
                registry_generation,
                default_channel,
                model_routes,
                failure_domains,
                channels,
            ))),
        }
    }

    pub fn replace(
        &self,
        registry_generation: u64,
        default_channel: Option<String>,
        model_routes: HashMap<String, ModelRoute>,
        failure_domains: FailureDomainRegistry,
        channels: HashMap<String, PoolState>,
    ) {
        *self
            .snapshot
            .write()
            .expect("channel registry snapshot lock poisoned") = compile_channel_registry_snapshot(
            registry_generation,
            default_channel,
            model_routes,
            failure_domains,
            channels,
        );
    }

    fn snapshot(&self) -> ChannelRegistrySnapshot {
        self.snapshot
            .read()
            .expect("channel registry snapshot lock poisoned")
            .clone()
    }

    pub fn registry_generation(&self) -> u64 {
        self.snapshot().registry_generation
    }

    pub fn public_model_catalog(&self) -> Vec<PublicModelCatalogEntry> {
        self.snapshot().public_model_catalog.as_ref().clone()
    }

    pub fn model_routes_context(&self) -> ChannelModelRoutesContext {
        let snapshot = self.snapshot();
        let mut routes: Vec<ChannelModelRouteContext> = snapshot
            .model_routes
            .values()
            .cloned()
            .map(|route| {
                let target_health = route
                    .targets
                    .iter()
                    .map(|target| {
                        let health = snapshot.channels.get(&target.channel_id.0).map(|pool| {
                            pool.health
                                .lock()
                                .expect("channel health mutex poisoned")
                                .clone()
                        });
                        (target.channel_id.clone(), health)
                    })
                    .collect();
                ChannelModelRouteContext {
                    route,
                    target_health,
                }
            })
            .collect();
        routes.sort_by(|a, b| a.route.public_model.cmp(&b.route.public_model));
        ChannelModelRoutesContext {
            registry_generation: snapshot.registry_generation,
            default_channel: snapshot.default_channel,
            routes,
        }
    }

    pub fn route_plan_context(&self, model: Option<&str>) -> ChannelRoutePlanContext {
        let snapshot = self.snapshot();
        let model_route = model.and_then(|model| snapshot.model_routes.get(model).cloned());
        let claimed_upstream_channels = model
            .map(|model| claimed_upstream_channels_from_snapshot(&snapshot, model))
            .unwrap_or_default();
        let model_route_channel_states = model_route
            .as_ref()
            .map(|route| route_channel_states_from_snapshot(&snapshot, route))
            .unwrap_or_default();
        let default_channel = snapshot.default_channel.clone();
        let default_channel_provider_kind = default_channel.as_deref().and_then(|channel_id| {
            snapshot
                .channels
                .get(channel_id)
                .map(|pool| pool.provider_kind)
        });
        let default_channel_route_state = if model_route.is_none() {
            default_channel.as_deref().map(|channel_id| {
                snapshot
                    .channels
                    .get(channel_id)
                    .map(route_state_for_pool)
                    .unwrap_or(ChannelRouteState::UnknownChannel)
            })
        } else {
            None
        };

        ChannelRoutePlanContext {
            registry_generation: snapshot.registry_generation,
            model_route,
            model_route_channel_states,
            claimed_upstream_channels,
            default_channel,
            default_channel_provider_kind,
            default_channel_route_state,
        }
    }

    pub fn named_route_plan_context(&self, channel_id: &str) -> ChannelNamedRoutePlanContext {
        let snapshot = self.snapshot();
        let route_state = snapshot
            .channels
            .get(channel_id)
            .map(route_state_for_pool)
            .unwrap_or(ChannelRouteState::UnknownChannel);

        ChannelNamedRoutePlanContext {
            registry_generation: snapshot.registry_generation,
            route_state,
        }
    }

    pub fn has_claimed_upstream_model(
        &self,
        requested_model: &str,
        allowed_channels: &[String],
    ) -> bool {
        let snapshot = self.snapshot();
        let claimed_upstream_channels =
            claimed_upstream_channels_from_snapshot(&snapshot, requested_model);
        any_allowed_claimed_upstream_channel(&claimed_upstream_channels, allowed_channels)
    }

    #[cfg(test)]
    pub fn channel_route_state(&self, channel_id: &str) -> ChannelRouteState {
        self.snapshot()
            .channels
            .get(channel_id)
            .map(route_state_for_pool)
            .unwrap_or(ChannelRouteState::UnknownChannel)
    }

    pub fn apply_failure_domain_transition(
        &self,
        provider_id: &str,
        account_id: &str,
        until: Option<Instant>,
        reason: impl Into<String>,
    ) {
        self.snapshot().failure_domains.apply_provider_unavailable(
            provider_id,
            account_id,
            until,
            reason,
        );
    }

    pub fn get(&self, channel_id: &str) -> Option<PoolState> {
        self.snapshot().channels.get(channel_id).cloned()
    }

    pub fn channel_ids_for_credential_set(
        &self,
        credential_set_id: &CredentialSetId,
    ) -> Vec<String> {
        self.snapshot()
            .credential_set_channels
            .get(credential_set_id)
            .cloned()
            .unwrap_or_default()
    }
}

fn compile_channel_registry_snapshot(
    registry_generation: u64,
    default_channel: Option<String>,
    model_routes: HashMap<String, ModelRoute>,
    failure_domains: FailureDomainRegistry,
    channels: HashMap<String, PoolState>,
) -> ChannelRegistrySnapshot {
    ChannelRegistrySnapshot {
        registry_generation,
        default_channel,
        public_model_catalog: Arc::new(public_model_catalog_projection(&model_routes, &channels)),
        claimed_upstream_models: Arc::new(claimed_upstream_models_by_channel(&model_routes)),
        credential_set_channels: Arc::new(credential_set_channels_by_id(&channels)),
        failure_domains,
        model_routes: Arc::new(model_routes),
        channels: Arc::new(channels),
    }
}

fn public_model_catalog_projection(
    model_routes: &HashMap<String, ModelRoute>,
    channels: &HashMap<String, PoolState>,
) -> Vec<PublicModelCatalogEntry> {
    let mut entries = Vec::new();
    let mut routes: Vec<&ModelRoute> = model_routes.values().collect();
    routes.sort_by(|a, b| a.public_model.cmp(&b.public_model));

    for route in routes {
        let targets: Vec<PublicModelCatalogTarget> = route
            .targets
            .iter()
            .filter(|target| target.enabled)
            .filter_map(|target| {
                let channel_id = &target.channel_id.0;
                let pool = channels.get(channel_id)?;
                if !pool.configured_enabled || !pool.account_enabled {
                    return None;
                }
                ProviderAdapter::new(pool.provider_kind).model_catalog_capability()?;
                Some(PublicModelCatalogTarget {
                    channel_id: channel_id.clone(),
                    health: pool.health.clone(),
                })
            })
            .collect();

        if !targets.is_empty() {
            entries.push(PublicModelCatalogEntry {
                public_model: route.public_model.clone(),
                targets,
            });
        }
    }

    entries
}

fn claimed_upstream_models_by_channel(
    model_routes: &HashMap<String, ModelRoute>,
) -> HashMap<String, Vec<String>> {
    let mut claims: HashMap<String, Vec<String>> = HashMap::new();
    for route in model_routes.values() {
        for target in route.targets.iter().filter(|target| target.enabled) {
            let Some(upstream_model) = target.upstream_model.as_ref() else {
                continue;
            };
            claims
                .entry(upstream_model.clone())
                .or_default()
                .push(target.channel_id.0.clone());
        }
    }
    for channel_ids in claims.values_mut() {
        channel_ids.sort();
        channel_ids.dedup();
    }
    claims
}

fn claimed_upstream_channels_from_snapshot(
    snapshot: &ChannelRegistrySnapshot,
    requested_model: &str,
) -> Vec<String> {
    let Some(channel_ids) = snapshot.claimed_upstream_models.get(requested_model) else {
        return Vec::new();
    };
    channel_ids
        .iter()
        .filter(|channel_id| {
            snapshot
                .channels
                .get(*channel_id)
                .is_some_and(pool_claims_upstream_model)
        })
        .cloned()
        .collect()
}

fn any_allowed_claimed_upstream_channel(
    claimed_upstream_channels: &[String],
    allowed_channels: &[String],
) -> bool {
    claimed_upstream_channels.iter().any(|channel_id| {
        allowed_channels.is_empty()
            || allowed_channels
                .iter()
                .any(|allowed| allowed == channel_id.as_str())
    })
}

fn pool_claims_upstream_model(pool: &PoolState) -> bool {
    pool.configured_enabled
        && pool.account_enabled
        && !matches!(
            *pool.health.lock().expect("channel health mutex poisoned"),
            ChannelHealth::Disabled { .. }
        )
}

fn credential_set_channels_by_id(
    channels: &HashMap<String, PoolState>,
) -> HashMap<CredentialSetId, Vec<String>> {
    let mut index: HashMap<CredentialSetId, Vec<String>> = HashMap::new();
    for (channel_id, pool_state) in channels {
        index
            .entry(pool_state.credential_set_id.clone())
            .or_default()
            .push(channel_id.clone());
    }
    for channel_ids in index.values_mut() {
        channel_ids.sort();
    }
    index
}

fn route_state_for_pool(pool: &PoolState) -> ChannelRouteState {
    if !pool.configured_enabled {
        return ChannelRouteState::Disabled;
    }
    if !pool.account_enabled {
        return ChannelRouteState::Disabled;
    }
    let health = pool
        .health
        .lock()
        .expect("channel health mutex poisoned")
        .clone();
    if matches!(health, ChannelHealth::Disabled { .. }) {
        return ChannelRouteState::Disabled;
    }
    let failure_domain_state = pool
        .failure_domains
        .route_state(&pool.provider_id, &pool.account_id);
    if let ChannelHealth::CoolingDown { until, .. } = &health {
        if Instant::now() < *until
            && !channel_cooldown_is_provider_failure_domain_soft_state(
                &health,
                failure_domain_state,
            )
        {
            return ChannelRouteState::CoolingDown;
        }
    }
    let Ok(pool_guard) = pool.pool.try_lock() else {
        return failure_domain_state.unwrap_or(ChannelRouteState::RuntimeUnavailable);
    };
    let has_available_credentials = pool_guard.has_available_credentials_read_only();
    if !has_available_credentials {
        if pool_guard.has_cooling_down_credentials_read_only() {
            return ChannelRouteState::CredentialCoolingDown;
        }
        return ChannelRouteState::NoAvailableCredentials;
    }
    if let Some(state) = failure_domain_state {
        return state;
    }
    match health {
        ChannelHealth::Available => ChannelRouteState::Available,
        ChannelHealth::CoolingDown { .. } => ChannelRouteState::Available,
        ChannelHealth::Degraded { .. } => ChannelRouteState::Degraded,
        ChannelHealth::Disabled { .. } => ChannelRouteState::Disabled,
    }
}

fn channel_cooldown_is_provider_failure_domain_soft_state(
    health: &ChannelHealth,
    failure_domain_state: Option<ChannelRouteState>,
) -> bool {
    matches!(
        failure_domain_state,
        Some(ChannelRouteState::ProviderCoolingDown)
    ) && matches!(
        health,
        ChannelHealth::CoolingDown { reason, .. }
            if reason == "upstream provider unavailable"
    )
}

fn route_channel_states_from_snapshot(
    snapshot: &ChannelRegistrySnapshot,
    route: &ModelRoute,
) -> HashMap<ChannelId, ChannelRouteState> {
    route
        .targets
        .iter()
        .map(|target| {
            let state = snapshot
                .channels
                .get(&target.channel_id.0)
                .map(route_state_for_pool)
                .unwrap_or(ChannelRouteState::UnknownChannel);
            (target.channel_id.clone(), state)
        })
        .collect()
}

static REGISTRY_GENERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_registry_generation() -> u64 {
    REGISTRY_GENERATION_COUNTER.fetch_add(1, Ordering::AcqRel) + 1
}

impl ManagementProjection {
    fn from_pool_configs(pool_configs: &[(String, ResolvedPoolConfig)]) -> Self {
        let mut channel_error_policies = HashMap::new();
        let mut policy_profile_channels: HashMap<String, Vec<String>> = HashMap::new();
        let mut routing_profile_channels: HashMap<String, Vec<String>> = HashMap::new();
        let mut runtime_topology_summary = RuntimeTopologySummary::default();
        let mut channel_topologies = Vec::new();
        let mut provider_topologies: BTreeMap<String, ProviderTopologyProjectionBuilder> =
            BTreeMap::new();
        let mut account_topologies: BTreeMap<String, AccountTopologyProjectionBuilder> =
            BTreeMap::new();
        let mut credential_set_topologies: BTreeMap<
            String,
            CredentialSetTopologyProjectionBuilder,
        > = BTreeMap::new();
        for (channel_id, pool_config) in pool_configs {
            channel_error_policies
                .insert(channel_id.clone(), pool_config.error_policy_sources.clone());
            if let Some(profile_id) = &pool_config.error_policy_sources.profile_id {
                policy_profile_channels
                    .entry(profile_id.clone())
                    .or_default()
                    .push(channel_id.clone());
            }
            routing_profile_channels
                .entry(pool_config.routing_policy_sources.profile_id.clone())
                .or_default()
                .push(channel_id.clone());
            runtime_topology_summary.add_channel(pool_config);
            channel_topologies.push(ChannelTopologyProjection::from_pool_config(
                channel_id.clone(),
                pool_config,
            ));
            provider_topologies
                .entry(pool_config.provider_id.clone())
                .or_insert_with(|| ProviderTopologyProjectionBuilder {
                    id: pool_config.provider_id.clone(),
                    provider_kind: pool_config.provider_kind,
                    enabled: true,
                    channel_ids: Vec::new(),
                    account_ids: BTreeSet::new(),
                    credential_set_ids: BTreeSet::new(),
                })
                .add_channel(channel_id.clone(), pool_config);
            account_topologies
                .entry(pool_config.account_id.clone())
                .or_insert_with(|| AccountTopologyProjectionBuilder {
                    id: pool_config.account_id.clone(),
                    provider_id: pool_config.provider_id.clone(),
                    provider_kind: pool_config.provider_kind,
                    enabled: true,
                    provider_enabled: true,
                    effective_enabled: true,
                    channel_ids: Vec::new(),
                    credential_set_ids: BTreeSet::new(),
                })
                .add_channel(channel_id.clone(), pool_config);
            credential_set_topologies
                .entry(pool_config.credential_set_id.0.clone())
                .or_insert_with(|| CredentialSetTopologyProjectionBuilder {
                    id: pool_config.credential_set_id.clone(),
                    channel_ids: Vec::new(),
                    account_ids: BTreeSet::new(),
                    provider_ids: BTreeSet::new(),
                })
                .add_channel(channel_id.clone(), pool_config);
        }
        for channel_ids in policy_profile_channels.values_mut() {
            channel_ids.sort();
        }
        for channel_ids in routing_profile_channels.values_mut() {
            channel_ids.sort();
        }
        channel_topologies.sort_by(|left, right| left.id.cmp(&right.id));
        Self {
            channel_error_policies,
            policy_profile_channels,
            routing_profile_channels,
            runtime_topology_summary,
            channel_topologies,
            provider_topologies: provider_topologies
                .into_values()
                .map(ProviderTopologyProjectionBuilder::finish)
                .collect(),
            account_topologies: account_topologies
                .into_values()
                .map(AccountTopologyProjectionBuilder::finish)
                .collect(),
            credential_set_topologies: credential_set_topologies
                .into_values()
                .map(CredentialSetTopologyProjectionBuilder::finish)
                .collect(),
        }
    }
}

impl RuntimeTopologySummary {
    fn add_channel(&mut self, pool_config: &ResolvedPoolConfig) {
        self.channels += 1;
        if pool_config
            .routing_policy
            .retry_switched_key_in_same_request
        {
            self.same_request_retry_enabled_channels += 1;
            self.max_same_request_retries = self
                .max_same_request_retries
                .max(pool_config.routing_policy.max_same_request_retries);
        }
    }
}

impl ChannelTopologyProjection {
    fn from_pool_config(id: String, pool_config: &ResolvedPoolConfig) -> Self {
        Self {
            id,
            configured_enabled: pool_config.configured_enabled,
            provider_id: pool_config.provider_id.clone(),
            account_id: pool_config.account_id.clone(),
            credential_set_id: pool_config.credential_set_id.clone(),
            provider_kind: pool_config.provider_kind,
            endpoint_capabilities: pool_config.endpoint_capabilities.clone(),
            api_base: pool_config.key_pool.api_base.clone(),
            auth_header: pool_config.auth_header.clone(),
            auth_prefix_configured: !pool_config.auth_prefix.is_empty(),
            routing_profile_id: pool_config.routing_profile_id.clone(),
            retry_switched_key_in_same_request: pool_config
                .routing_policy
                .retry_switched_key_in_same_request,
            max_same_request_retries: pool_config.routing_policy.max_same_request_retries,
            route_target_retry_enabled: pool_config.routing_policy.route_target_retry_enabled,
            default_credential_cooldown: pool_config.routing_policy.default_credential_cooldown,
            config_generation: pool_config.config_generation,
        }
    }
}

struct ProviderTopologyProjectionBuilder {
    id: String,
    provider_kind: ProviderKind,
    enabled: bool,
    channel_ids: Vec<String>,
    account_ids: BTreeSet<String>,
    credential_set_ids: BTreeSet<String>,
}

impl ProviderTopologyProjectionBuilder {
    fn add_channel(&mut self, channel_id: String, pool_config: &ResolvedPoolConfig) {
        self.enabled = self.enabled && pool_config.provider_enabled;
        self.channel_ids.push(channel_id);
        self.account_ids.insert(pool_config.account_id.clone());
        self.credential_set_ids
            .insert(pool_config.credential_set_id.0.clone());
    }

    fn finish(mut self) -> ProviderTopologyProjection {
        self.channel_ids.sort();
        self.channel_ids.dedup();
        ProviderTopologyProjection {
            id: self.id,
            provider_kind: self.provider_kind,
            enabled: self.enabled,
            channel_ids: self.channel_ids,
            account_ids: self.account_ids.into_iter().collect(),
            credential_set_ids: self
                .credential_set_ids
                .into_iter()
                .map(CredentialSetId)
                .collect(),
        }
    }
}

struct AccountTopologyProjectionBuilder {
    id: String,
    provider_id: String,
    provider_kind: ProviderKind,
    enabled: bool,
    provider_enabled: bool,
    effective_enabled: bool,
    channel_ids: Vec<String>,
    credential_set_ids: BTreeSet<String>,
}

impl AccountTopologyProjectionBuilder {
    fn add_channel(&mut self, channel_id: String, pool_config: &ResolvedPoolConfig) {
        self.enabled = self.enabled && pool_config.account_configured_enabled;
        self.provider_enabled = self.provider_enabled && pool_config.provider_enabled;
        self.effective_enabled = self.effective_enabled && pool_config.account_enabled;
        self.channel_ids.push(channel_id);
        self.credential_set_ids
            .insert(pool_config.credential_set_id.0.clone());
    }

    fn finish(mut self) -> AccountTopologyProjection {
        self.channel_ids.sort();
        self.channel_ids.dedup();
        AccountTopologyProjection {
            id: self.id,
            provider_id: self.provider_id,
            provider_kind: self.provider_kind,
            enabled: self.enabled,
            provider_enabled: self.provider_enabled,
            effective_enabled: self.effective_enabled,
            channel_ids: self.channel_ids,
            credential_set_ids: self
                .credential_set_ids
                .into_iter()
                .map(CredentialSetId)
                .collect(),
        }
    }
}

struct CredentialSetTopologyProjectionBuilder {
    id: CredentialSetId,
    channel_ids: Vec<String>,
    account_ids: BTreeSet<String>,
    provider_ids: BTreeSet<String>,
}

impl CredentialSetTopologyProjectionBuilder {
    fn add_channel(&mut self, channel_id: String, pool_config: &ResolvedPoolConfig) {
        self.channel_ids.push(channel_id);
        self.account_ids.insert(pool_config.account_id.clone());
        self.provider_ids.insert(pool_config.provider_id.clone());
    }

    fn finish(mut self) -> CredentialSetTopologyProjection {
        self.channel_ids.sort();
        self.channel_ids.dedup();
        CredentialSetTopologyProjection {
            id: self.id,
            channel_ids: self.channel_ids,
            account_ids: self.account_ids.into_iter().collect(),
            provider_ids: self.provider_ids.into_iter().collect(),
        }
    }
}

impl PoolState {
    pub fn route_state(&self) -> ChannelRouteState {
        route_state_for_pool(self)
    }

    fn from_config(
        config: ResolvedPoolConfig,
        runtime: RuntimeCredentialSet,
        failure_domains: FailureDomainRegistry,
    ) -> anyhow::Result<Self> {
        let api_base = config.key_pool.api_base.clone();
        Ok(Self {
            config_generation: config.config_generation,
            configured_enabled: config.configured_enabled,
            channel_health_generation: Arc::new(AtomicU64::new(1)),
            selector_generation: runtime.selector_generation,
            provider_id: config.provider_id,
            account_id: config.account_id,
            account_enabled: config.account_enabled,
            failure_domains,
            probe_result_policy: config.probe_result_policy,
            credential_set_id: config.credential_set_id,
            provider_kind: config.provider_kind,
            api_base,
            auth_header: config.auth_header,
            auth_prefix: config.auth_prefix,
            retry_switched_key_in_same_request: config
                .routing_policy
                .retry_switched_key_in_same_request,
            max_same_request_retries: config.routing_policy.max_same_request_retries,
            route_target_retry_enabled: config.routing_policy.route_target_retry_enabled,
            default_credential_cooldown: config.routing_policy.default_credential_cooldown,
            error_classifier: config.error_classifier,
            key_import_report: runtime.key_import_report,
            mutation_gate: runtime.mutation_gate,
            send_gate: runtime.send_gate,
            pool: runtime.pool,
            health: Arc::new(StdMutex::new(ChannelHealth::Available)),
            relay_suppression_count: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn advance_selector_generation_if_state_kind_changed(
        &self,
        before: Option<&CredentialStateSnapshot>,
        after: &CredentialStateSnapshot,
    ) -> u64 {
        if before.is_some_and(|before| same_selector_state_kind(before, after)) {
            self.selector_generation.load(Ordering::Acquire)
        } else {
            self.selector_generation.fetch_add(1, Ordering::AcqRel) + 1
        }
    }

    pub fn apply_automatic_channel_health_transition(
        &self,
        expected_generation: u64,
        next_health: ChannelHealth,
    ) -> bool {
        let mut health = self.health.lock().expect("channel health mutex poisoned");
        if matches!(*health, ChannelHealth::Disabled { .. }) {
            return false;
        }
        if self.channel_health_generation.load(Ordering::Acquire) != expected_generation {
            return false;
        }
        *health = next_health;
        self.channel_health_generation
            .fetch_add(1, Ordering::AcqRel);
        true
    }

    pub fn apply_automatic_relay_balance_suppression(
        &self,
        expected_generation: u64,
        until: Instant,
        reason: impl Into<String>,
    ) -> Option<u64> {
        if !self.configured_enabled || !self.account_enabled {
            return None;
        }
        let mut health = self.health.lock().expect("channel health mutex poisoned");
        if matches!(*health, ChannelHealth::Disabled { .. }) {
            return None;
        }
        if self.channel_health_generation.load(Ordering::Acquire) != expected_generation {
            return None;
        }
        let can_apply_suppression = match *health {
            ChannelHealth::Available | ChannelHealth::Degraded { .. } => true,
            ChannelHealth::CoolingDown { until, .. } => Instant::now() >= until,
            ChannelHealth::Disabled { .. } => false,
        };
        if !can_apply_suppression {
            return None;
        }
        *health = ChannelHealth::CoolingDown {
            until,
            reason: reason.into(),
        };
        self.channel_health_generation
            .fetch_add(1, Ordering::AcqRel);
        Some(self.relay_suppression_count.fetch_add(1, Ordering::AcqRel) + 1)
    }

    pub fn record_selected_channel_success(&self) {
        self.relay_suppression_count.store(0, Ordering::Release);
        let mut health = self.health.lock().expect("channel health mutex poisoned");
        if !matches!(*health, ChannelHealth::Disabled { .. }) {
            *health = ChannelHealth::Available;
            self.channel_health_generation
                .fetch_add(1, Ordering::AcqRel);
        }
    }

    pub fn reset_relay_suppression_count(&self) {
        self.relay_suppression_count.store(0, Ordering::Release);
    }

    pub fn relay_suppression_count(&self) -> u64 {
        self.relay_suppression_count.load(Ordering::Acquire)
    }
}

fn apply_persisted_events(
    channel_ids: &[String],
    pool: &mut KeyPool,
    persisted_events: &[ManagementEvent],
) {
    for event in persisted_events {
        match event.to_domain_event() {
            Some(DomainEvent::Expired {
                channel_id,
                credential_id,
                reason,
                ..
            }) if event_applies_to_credential_set(channel_ids, &channel_id) => {
                pool.expire_credential_by_id(&CredentialId(credential_id), reason);
            }
            Some(DomainEvent::QuotaExhausted {
                channel_id,
                credential_id,
                reason,
                ..
            }) if event_applies_to_credential_set(channel_ids, &channel_id) => {
                pool.apply_credential_quota_exhausted(&CredentialId(credential_id), reason);
            }
            Some(DomainEvent::Restored {
                channel_id,
                credential_id,
                ..
            }) if event_applies_to_credential_set(channel_ids, &channel_id) => {
                pool.restore_credential_by_id(&CredentialId(credential_id));
            }
            Some(DomainEvent::Disabled {
                channel_id,
                credential_id,
                reason,
                ..
            }) if event_applies_to_credential_set(channel_ids, &channel_id) => {
                pool.disable_credential_by_id(&CredentialId(credential_id), reason);
            }
            Some(DomainEvent::Enabled {
                channel_id,
                credential_id,
                ..
            }) if event_applies_to_credential_set(channel_ids, &channel_id) => {
                pool.enable_credential_by_id(&CredentialId(credential_id));
            }
            Some(DomainEvent::CooldownCleared {
                channel_id,
                credential_id,
                ..
            }) if event_applies_to_credential_set(channel_ids, &channel_id) => {
                pool.clear_credential_cooldown_by_id(&CredentialId(credential_id));
            }
            _ => {}
        }
    }
}

fn credential_events_for_startup(
    lifecycle_snapshot_authority: bool,
    persisted_events: &[ManagementEvent],
) -> &[ManagementEvent] {
    if lifecycle_snapshot_authority {
        &[]
    } else {
        persisted_events
    }
}

fn apply_lifecycle_snapshots(
    pool: &mut KeyPool,
    lifecycle_snapshots: &[CredentialLifecycleSnapshot],
) -> anyhow::Result<()> {
    for snapshot in lifecycle_snapshots {
        let applied = match &snapshot.state {
            CredentialLifecycleState::Available => {
                pool.enable_credential_by_id(&snapshot.credential_id)
            }
            CredentialLifecycleState::Expired { reason } => {
                pool.expire_credential_by_id(&snapshot.credential_id, reason.clone())
            }
            CredentialLifecycleState::QuotaExhausted { reason } => {
                pool.apply_credential_quota_exhausted(&snapshot.credential_id, reason.clone());
                pool.credential_snapshot_by_id(&snapshot.credential_id)
            }
            CredentialLifecycleState::Disabled { reason } => {
                pool.disable_credential_by_id(&snapshot.credential_id, reason.clone())
            }
        };
        anyhow::ensure!(
            applied.is_some(),
            "credential lifecycle snapshot references unknown credential {} in credential set {}",
            snapshot.credential_id.0,
            snapshot.credential_set_id.0
        );
    }
    Ok(())
}

fn apply_persisted_channel_events(
    pools: &HashMap<String, PoolState>,
    persisted_events: &[ManagementEvent],
) {
    for event in persisted_events {
        match event.to_domain_event() {
            Some(DomainEvent::ChannelDisabled {
                channel_id, reason, ..
            }) => {
                if let Some(pool) = pools.get(&channel_id) {
                    *pool.health.lock().expect("channel health mutex poisoned") =
                        ChannelHealth::Disabled { reason };
                }
            }
            Some(DomainEvent::ChannelEnabled { channel_id, .. }) => {
                if let Some(pool) = pools.get(&channel_id) {
                    *pool.health.lock().expect("channel health mutex poisoned") =
                        ChannelHealth::Available;
                }
            }
            _ => {}
        }
    }
}

fn event_applies_to_credential_set(channel_ids: &[String], event_channel_id: &str) -> bool {
    channel_ids
        .iter()
        .any(|channel_id| channel_id == event_channel_id)
}

fn same_selector_state_kind(
    before: &CredentialStateSnapshot,
    after: &CredentialStateSnapshot,
) -> bool {
    matches!(
        (before, after),
        (
            CredentialStateSnapshot::Available,
            CredentialStateSnapshot::Available
        ) | (
            CredentialStateSnapshot::CoolingDown { .. },
            CredentialStateSnapshot::CoolingDown { .. }
        ) | (
            CredentialStateSnapshot::Expired { .. },
            CredentialStateSnapshot::Expired { .. }
        ) | (
            CredentialStateSnapshot::QuotaExhausted { .. },
            CredentialStateSnapshot::QuotaExhausted { .. }
        ) | (
            CredentialStateSnapshot::Disabled { .. },
            CredentialStateSnapshot::Disabled { .. }
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{
            AccountConfig, AppConfig, ClientTokenConfig, CredentialSetConfig, ErrorRulesConfig,
            ManagementConfig, PoolConfig, ProviderConfig, TimeoutConfig,
        },
        credential_repository::{
            CredentialLifecycleEvidence, CredentialLifecycleUpdate, SqliteCredentialRepository,
        },
        credentials::{Credential, CredentialSource},
        events::EventLog,
        provider::ProviderKind,
        test_fixtures::fixtures,
    };

    fn startup_lifecycle_evidence() -> CredentialLifecycleEvidence {
        CredentialLifecycleEvidence::management_command(
            "test_startup_state",
            "test:state",
            "state-test",
            "test",
            "test",
        )
    }
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    static TEMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(1);

    fn temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
        path.push(format!("{name}-{suffix}-{counter}"));
        path
    }

    fn credential_sets_from_files(
        entries: impl IntoIterator<Item = (impl Into<String>, PathBuf)>,
    ) -> HashMap<String, CredentialSetConfig> {
        entries
            .into_iter()
            .map(|(id, keys_file)| (id.into(), CredentialSetConfig { keys_file }))
            .collect()
    }

    fn single_pool_config(keys_file: PathBuf, event_log_path: Option<PathBuf>) -> AppConfig {
        let mut pools = HashMap::new();
        pools.insert(
            "test".to_string(),
            PoolConfig {
                endpoint_capabilities: Default::default(),
                enabled: true,
                account: None,
                policy_profile: None,
                routing_profile: None,
                provider_kind: ProviderKind::OpenAiCompatible,
                api_base: "https://example.com/v1".to_string(),
                credential_set: "test-credentials".to_string(),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                error_rules: ErrorRulesConfig::default(),
            },
        );
        AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("test".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets: credential_sets_from_files([("test-credentials", keys_file)]),
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
    }

    #[test]
    fn runtime_catalog_generation_tracks_channel_registry_generation_after_reload() {
        let keys_file = temp_path("key-pool-router-runtime-generation-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let config = single_pool_config(keys_file, None).resolve().unwrap();
        let reloaded = config.clone();
        let state =
            AppState::new_with_registry_store(config, RegistryStoreHandle::read_only()).unwrap();

        let initial_generation = state.channels.registry_generation();
        assert_eq!(
            state.runtime_catalogs.registry_generation(),
            initial_generation
        );

        state
            .rebuild_runtime_from_resolved_config(reloaded, Some(2))
            .unwrap();

        let reloaded_generation = state.channels.registry_generation();
        assert!(reloaded_generation > initial_generation);
        assert_eq!(
            state.runtime_catalogs.registry_generation(),
            reloaded_generation
        );
    }

    #[test]
    fn app_state_uses_configured_response_filter_event_settings() {
        let keys_file = temp_path("key-pool-router-response-filter-event-settings-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let mut document = single_pool_config(keys_file, None).into_registry_document();
        document.response_filter = crate::config::ResponseFilterConfig {
            enabled: false,
            replacement: None,
            event_window_capacity: Some(3),
            alert_window_seconds: Some(17),
            rules: Vec::new(),
        };
        let config = document.resolve().unwrap();

        let state = AppState::new(config).unwrap();

        assert_eq!(
            state
                .response_filter_events
                .lock()
                .expect("response filter events lock poisoned")
                .capacity(),
            3
        );
        assert_eq!(
            *state
                .response_filter_alert_window
                .read()
                .expect("response filter alert window lock poisoned"),
            Duration::from_secs(17)
        );
    }

    #[test]
    fn runtime_reload_updates_response_filter_event_settings_without_resetting_ids() {
        let keys_file = temp_path("key-pool-router-response-filter-reload-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let mut document = single_pool_config(keys_file.clone(), None).into_registry_document();
        document.response_filter = crate::config::ResponseFilterConfig {
            enabled: false,
            replacement: None,
            event_window_capacity: Some(4),
            alert_window_seconds: Some(17),
            rules: Vec::new(),
        };
        let config = document.resolve().unwrap();
        let state = AppState::new(config).unwrap();
        {
            let mut events = state
                .response_filter_events
                .lock()
                .expect("response filter events lock poisoned");
            for rule_id in ["one", "two", "three"] {
                events.push(crate::events::ResponseFilterEventInput {
                    request_id: format!("request-{rule_id}"),
                    channel_id: "test".to_string(),
                    public_model: "gpt-test".to_string(),
                    rule_id: rule_id.to_string(),
                    action: "redact".to_string(),
                    content_kind: "json".to_string(),
                    reason_code: "rule_matched".to_string(),
                    outcome: "redacted".to_string(),
                    body_committed: false,
                });
            }
        }

        let mut reloaded_document = single_pool_config(keys_file, None).into_registry_document();
        reloaded_document.response_filter = crate::config::ResponseFilterConfig {
            enabled: false,
            replacement: None,
            event_window_capacity: Some(2),
            alert_window_seconds: Some(23),
            rules: Vec::new(),
        };
        state
            .rebuild_runtime_from_resolved_config(reloaded_document.resolve().unwrap(), Some(2))
            .unwrap();

        let events = state
            .response_filter_events
            .lock()
            .expect("response filter events lock poisoned");
        assert_eq!(events.capacity(), 2);
        let snapshot = events.snapshot();
        assert_eq!(
            snapshot
                .iter()
                .map(|event| event.rule_id.as_str())
                .collect::<Vec<_>>(),
            vec!["two", "three"]
        );
        drop(events);
        assert_eq!(
            *state
                .response_filter_alert_window
                .read()
                .expect("response filter alert window lock poisoned"),
            Duration::from_secs(23)
        );
    }

    #[test]
    fn runtime_reload_updates_routing_telemetry_capacity_and_dropped_count() {
        let keys_file = temp_path("key-pool-router-routing-telemetry-reload-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let mut document = single_pool_config(keys_file.clone(), None).into_registry_document();
        document.routing = crate::config::RoutingConfig {
            max_route_candidates: None,
            max_model_catalog_channels: None,
            telemetry_buffer_capacity: Some(3),
        };
        let state = AppState::new(document.resolve().unwrap()).unwrap();
        {
            let mut telemetry = state
                .routing_telemetry
                .lock()
                .expect("routing telemetry lock poisoned");
            for request_id in ["request-one", "request-two", "request-three"] {
                telemetry.push(crate::events::RoutingTelemetry::RouteSelected {
                    request_id: request_id.to_string(),
                    registry_generation: 2,
                    channel_id: "test".to_string(),
                });
            }
        }

        let mut reloaded_document = single_pool_config(keys_file, None).into_registry_document();
        reloaded_document.routing = crate::config::RoutingConfig {
            max_route_candidates: None,
            max_model_catalog_channels: None,
            telemetry_buffer_capacity: Some(1),
        };
        state
            .rebuild_runtime_from_resolved_config(reloaded_document.resolve().unwrap(), Some(2))
            .unwrap();

        let telemetry = state
            .routing_telemetry
            .lock()
            .expect("routing telemetry lock poisoned");
        assert_eq!(telemetry.capacity(), 1);
        assert_eq!(telemetry.dropped_events(), 2);
        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert!(matches!(
            &snapshot[0],
            crate::events::RoutingTelemetry::RouteSelected { request_id, .. }
                if request_id == "request-three"
        ));
    }

    #[test]
    fn app_state_restores_sqlite_lifecycle_snapshots_on_startup() {
        let keys_file = temp_path("key-pool-router-sqlite-lifecycle-keys");
        fs::write(&keys_file, "k1\nk2\n").unwrap();
        let db_path = temp_path("key-pool-router-sqlite-lifecycle-store").with_extension("sqlite");
        let credential_set_id = CredentialSetId("test-credentials".to_string());
        let first = Credential::with_source(
            &credential_set_id.0,
            "k1".to_string(),
            CredentialSource::unknown(),
        );
        let second = Credential::with_source(
            &credential_set_id.0,
            "k2".to_string(),
            CredentialSource::unknown(),
        );

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let resolved = single_pool_config(keys_file, None)
            .resolve_with_credential_repository_and_store_path(&repository, Some(db_path.clone()))
            .unwrap();
        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id: credential_set_id.clone(),
                    credential_id: first.id().clone(),
                    state: CredentialLifecycleState::Expired {
                        reason: "quota exhausted".to_string(),
                    },
                },
                startup_lifecycle_evidence(),
            )
            .unwrap();
        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id,
                    credential_id: second.id().clone(),
                    state: CredentialLifecycleState::Disabled {
                        reason: "manual pause".to_string(),
                    },
                },
                startup_lifecycle_evidence(),
            )
            .unwrap();

        let state = AppState::new(resolved).unwrap();
        let channel = state.channels.get("test").unwrap();
        let pool = channel.pool.blocking_lock();
        let snapshot = pool.snapshot();

        assert_eq!(snapshot.expired_credentials, 1);
        assert_eq!(snapshot.disabled_credentials, 1);
        assert_eq!(snapshot.available_credentials, 0);
    }

    #[test]
    fn app_state_does_not_replay_credential_jsonl_when_sqlite_lifecycle_is_authoritative() {
        let keys_file = temp_path("key-pool-router-sqlite-authority-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let event_log_path = temp_path("key-pool-router-sqlite-authority-events.jsonl");
        let db_path = temp_path("key-pool-router-sqlite-authority-store").with_extension("sqlite");
        let credential_set_id = CredentialSetId("test-credentials".to_string());
        let credential = Credential::with_source(
            &credential_set_id.0,
            "k1".to_string(),
            CredentialSource::unknown(),
        );
        EventLog::open(Some(event_log_path.clone()))
            .unwrap()
            .record_credential_restored_blocking("test", credential.id().0.clone(), "jsonl restore")
            .unwrap();

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let resolved = single_pool_config(keys_file, Some(event_log_path))
            .resolve_with_credential_repository_and_store_path(&repository, Some(db_path.clone()))
            .unwrap();
        repository
            .persist_lifecycle_update_with_evidence(
                CredentialLifecycleUpdate {
                    credential_set_id,
                    credential_id: credential.id().clone(),
                    state: CredentialLifecycleState::Expired {
                        reason: "store authority".to_string(),
                    },
                },
                startup_lifecycle_evidence(),
            )
            .unwrap();

        let state = AppState::new(resolved).unwrap();
        let channel = state.channels.get("test").unwrap();
        let pool = channel.pool.blocking_lock();
        let snapshot = pool
            .credential_snapshot_by_id(credential.id())
            .expect("credential should exist");

        assert_eq!(
            snapshot.state,
            CredentialStateSnapshot::Expired {
                reason: "store authority".to_string()
            }
        );
    }

    #[test]
    fn app_state_restores_500_sqlite_lifecycle_snapshots_on_startup() {
        let keys = (0..500)
            .map(|index| format!("key-{index}\n"))
            .collect::<String>();
        let keys_file = temp_path("key-pool-router-500-lifecycle-keys");
        fs::write(&keys_file, keys).unwrap();
        let db_path = temp_path("key-pool-router-500-lifecycle-store").with_extension("sqlite");
        let credential_set_id = CredentialSetId("test-credentials".to_string());

        let repository = SqliteCredentialRepository::open(&db_path).unwrap();
        let resolved = single_pool_config(keys_file, None)
            .resolve_with_credential_repository_and_store_path(&repository, Some(db_path.clone()))
            .unwrap();
        for index in 0..500 {
            let credential = Credential::with_source(
                &credential_set_id.0,
                format!("key-{index}"),
                CredentialSource::unknown(),
            );
            let state = if index % 2 == 0 {
                CredentialLifecycleState::Expired {
                    reason: "quota exhausted".to_string(),
                }
            } else {
                CredentialLifecycleState::Disabled {
                    reason: "manual pause".to_string(),
                }
            };
            repository
                .persist_lifecycle_update_with_evidence(
                    CredentialLifecycleUpdate {
                        credential_set_id: credential_set_id.clone(),
                        credential_id: credential.id().clone(),
                        state,
                    },
                    startup_lifecycle_evidence(),
                )
                .unwrap();
        }

        let started = Instant::now();
        let state = AppState::new(resolved).unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        let channel = state.channels.get("test").unwrap();
        let pool = channel.pool.blocking_lock();
        let snapshot = pool.snapshot();

        assert_eq!(snapshot.total_credentials, 500);
        assert_eq!(snapshot.expired_credentials, 250);
        assert_eq!(snapshot.disabled_credentials, 250);
        assert_eq!(snapshot.available_credentials, 0);
    }

    #[test]
    fn app_state_replays_persisted_expired_credential_events() {
        let keys_file = temp_path("key-pool-router-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let event_log_path = temp_path("key-pool-router-events.jsonl");
        let credential = Credential::with_source(
            "test-credentials",
            "k1".to_string(),
            CredentialSource {
                source_path: Some(keys_file.clone()),
                source_line: Some(1),
                batch_id: None,
            },
        );
        EventLog::open(Some(event_log_path.clone()))
            .unwrap()
            .record_credential_expired_blocking("test", credential.id().0.clone(), "persisted")
            .unwrap();

        let mut pools = HashMap::new();
        pools.insert(
            "test".to_string(),
            PoolConfig {
                endpoint_capabilities: Default::default(),
                enabled: true,
                account: None,
                policy_profile: None,
                routing_profile: None,
                provider_kind: ProviderKind::OpenAiCompatible,
                api_base: "https://example.com/v1".to_string(),
                credential_set: "test-credentials".to_string(),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                error_rules: ErrorRulesConfig::default(),
            },
        );
        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: Some(event_log_path),
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("test".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets: credential_sets_from_files([("test-credentials", keys_file)]),
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();

        let state = AppState::new(config).unwrap();
        let channel = state.channels.get("test").unwrap();
        let pool = channel.pool.blocking_lock();
        let snapshot = pool.snapshot();

        assert_eq!(snapshot.expired_credentials, 1);
        assert_eq!(snapshot.available_credentials, 0);
    }

    #[test]
    fn app_state_uses_configured_management_event_window_capacity() {
        let keys_file = temp_path("key-pool-router-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let mut pools = HashMap::new();
        pools.insert(
            "test".to_string(),
            PoolConfig {
                endpoint_capabilities: Default::default(),
                enabled: true,
                account: None,
                policy_profile: None,
                routing_profile: None,
                provider_kind: ProviderKind::OpenAiCompatible,
                api_base: "https://example.com/v1".to_string(),
                credential_set: "test-credentials".to_string(),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                error_rules: ErrorRulesConfig::default(),
            },
        );
        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: Some(2),
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("test".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets: credential_sets_from_files([("test-credentials", keys_file)]),
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();

        let state = AppState::new(config).unwrap();

        assert_eq!(state.events.window_capacity(), 2);
    }

    #[test]
    fn app_state_replays_restored_credential_events_in_order() {
        let keys_file = temp_path("key-pool-router-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let event_log_path = temp_path("key-pool-router-events.jsonl");
        let credential = Credential::with_source(
            "test-credentials",
            "k1".to_string(),
            CredentialSource {
                source_path: Some(keys_file.clone()),
                source_line: Some(1),
                batch_id: None,
            },
        );
        let event_log = EventLog::open(Some(event_log_path.clone())).unwrap();
        event_log
            .record_credential_expired_blocking("test", credential.id().0.clone(), "persisted")
            .unwrap();
        event_log
            .record_credential_restored_blocking("test", credential.id().0.clone(), "restored")
            .unwrap();

        let mut pools = HashMap::new();
        pools.insert(
            "test".to_string(),
            PoolConfig {
                endpoint_capabilities: Default::default(),
                enabled: true,
                account: None,
                policy_profile: None,
                routing_profile: None,
                provider_kind: ProviderKind::OpenAiCompatible,
                api_base: "https://example.com/v1".to_string(),
                credential_set: "test-credentials".to_string(),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                error_rules: ErrorRulesConfig::default(),
            },
        );
        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: Some(event_log_path),
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("test".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets: credential_sets_from_files([("test-credentials", keys_file)]),
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();

        let state = AppState::new(config).unwrap();
        let channel = state.channels.get("test").unwrap();
        let pool = channel.pool.blocking_lock();
        let snapshot = pool.snapshot();

        assert_eq!(snapshot.expired_credentials, 0);
        assert_eq!(snapshot.available_credentials, 1);
    }

    #[test]
    fn app_state_replays_disabled_and_enabled_credential_events_in_order() {
        let keys_file = temp_path("key-pool-router-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let event_log_path = temp_path("key-pool-router-events.jsonl");
        let credential = Credential::with_source(
            "test-credentials",
            "k1".to_string(),
            CredentialSource {
                source_path: Some(keys_file.clone()),
                source_line: Some(1),
                batch_id: None,
            },
        );
        let event_log = EventLog::open(Some(event_log_path.clone())).unwrap();
        event_log
            .record_credential_disabled_blocking(
                "test",
                credential.id().0.clone(),
                "manual disable",
            )
            .unwrap();
        event_log
            .record_credential_enabled_blocking("test", credential.id().0.clone(), "manual enable")
            .unwrap();

        let mut pools = HashMap::new();
        pools.insert(
            "test".to_string(),
            PoolConfig {
                endpoint_capabilities: Default::default(),
                enabled: true,
                account: None,
                policy_profile: None,
                routing_profile: None,
                provider_kind: ProviderKind::OpenAiCompatible,
                api_base: "https://example.com/v1".to_string(),
                credential_set: "test-credentials".to_string(),
                auth_header: "authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                error_rules: ErrorRulesConfig::default(),
            },
        );
        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: Some(event_log_path),
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("test".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets: credential_sets_from_files([("test-credentials", keys_file)]),
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();

        let state = AppState::new(config).unwrap();
        let channel = state.channels.get("test").unwrap();
        let pool = channel.pool.blocking_lock();
        let snapshot = pool.snapshot();

        assert_eq!(snapshot.disabled_credentials, 0);
        assert_eq!(snapshot.available_credentials, 1);
    }

    #[test]
    fn app_state_exposes_multi_target_model_routes() {
        let keys_file_a = temp_path("key-pool-router-keys-a");
        fs::write(&keys_file_a, "k1\n").unwrap();
        let keys_file_b = temp_path("key-pool-router-keys-b");
        fs::write(&keys_file_b, "k2\n").unwrap();

        let mut pools = HashMap::new();
        let mut credential_sets = HashMap::new();
        for (name, keys_file, api_base) in [
            ("a", keys_file_a, "https://a.example/v1"),
            ("b", keys_file_b, "https://b.example/v1"),
        ] {
            let credential_set = format!("{name}-credentials");
            credential_sets.insert(
                credential_set.clone(),
                CredentialSetConfig {
                    keys_file: keys_file.clone(),
                },
            );
            pools.insert(
                name.to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: api_base.to_string(),
                    credential_set,
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            );
        }

        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("a".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets,
            model_routes: HashMap::from([(
                "gpt-test".to_string(),
                crate::config::ModelRouteConfig {
                    strategy: Some("priority".to_string()),
                    targets: vec![
                        crate::config::ModelRouteTargetConfig {
                            channel: "a".to_string(),
                            upstream_model: None,
                            priority: 10,
                            weight: 1,
                            enabled: true,
                        },
                        crate::config::ModelRouteTargetConfig {
                            channel: "b".to_string(),
                            upstream_model: None,
                            priority: 20,
                            weight: 1,
                            enabled: true,
                        },
                    ],
                },
            )]),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();

        let state = AppState::new(config).unwrap();
        let routes_context = state.channels.model_routes_context();
        assert_eq!(routes_context.routes.len(), 1);
        assert_eq!(routes_context.routes[0].route.public_model, "gpt-test");
        let target_channels: Vec<&str> = routes_context.routes[0]
            .route
            .targets
            .iter()
            .map(|target| target.channel_id.0.as_str())
            .collect();
        assert_eq!(target_channels, vec!["a", "b"]);
    }

    #[test]
    fn public_model_catalog_projection_filters_structurally_hidden_targets_and_keeps_live_health() {
        let keys_file = temp_path("key-pool-router-public-catalog-keys");
        fs::write(&keys_file, "k1\n").unwrap();

        let credential_sets = credential_sets_from_files([("catalog-credentials", keys_file)]);
        let pools = HashMap::from([
            (
                "visible".to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: "https://visible.example/v1".to_string(),
                    credential_set: "catalog-credentials".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            ),
            (
                "configured-disabled".to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: false,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: "https://configured-disabled.example/v1".to_string(),
                    credential_set: "catalog-credentials".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            ),
            (
                "account-disabled".to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: Some("disabled-account".to_string()),
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: "https://ignored.example/v1".to_string(),
                    credential_set: "catalog-credentials".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            ),
            (
                "generic".to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::GenericHttp,
                    api_base: "https://generic.example/v1".to_string(),
                    credential_set: "catalog-credentials".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            ),
        ]);

        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("visible".to_string()),
            providers: HashMap::from([(
                "disabled-provider".to_string(),
                crate::config::ProviderConfig {
                    provider_kind: ProviderKind::OpenAiCompatible,
                    enabled: true,
                },
            )]),
            accounts: HashMap::from([(
                "disabled-account".to_string(),
                AccountConfig {
                    provider: "disabled-provider".to_string(),
                    api_base: "https://account-disabled.example/v1".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    enabled: false,
                },
            )]),
            credential_sets,
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();

        let state = AppState::new(config).unwrap();
        let channels = state.channels.snapshot().channels.as_ref().clone();
        let route_target = |channel: &str, provider_kind, enabled| crate::route_plan::RouteTarget {
            channel_id: ChannelId(channel.to_string()),
            provider_kind,
            upstream_model: None,
            priority: 10,
            weight: 1,
            enabled,
        };
        let routes = HashMap::from([(
            "public-model".to_string(),
            crate::route_plan::ModelRoute {
                public_model: "public-model".to_string(),
                strategy: crate::route_plan::RouteStrategy::Priority,
                targets: vec![
                    route_target("visible", ProviderKind::OpenAiCompatible, true),
                    route_target("visible", ProviderKind::OpenAiCompatible, false),
                    route_target("missing", ProviderKind::OpenAiCompatible, true),
                    route_target("configured-disabled", ProviderKind::OpenAiCompatible, true),
                    route_target("account-disabled", ProviderKind::OpenAiCompatible, true),
                    route_target("generic", ProviderKind::GenericHttp, true),
                ],
            },
        )]);

        let catalog = public_model_catalog_projection(&routes, &channels);
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].public_model, "public-model");
        assert_eq!(catalog[0].targets.len(), 1);
        assert_eq!(catalog[0].targets[0].channel_id, "visible");
        assert!(matches!(
            *catalog[0].targets[0]
                .health
                .lock()
                .expect("channel health mutex poisoned"),
            ChannelHealth::Available
        ));

        *state
            .channels
            .get("visible")
            .unwrap()
            .health
            .lock()
            .expect("channel health mutex poisoned") = ChannelHealth::Disabled {
            reason: "maintenance".to_string(),
        };
        assert!(matches!(
            *catalog[0].targets[0]
                .health
                .lock()
                .expect("channel health mutex poisoned"),
            ChannelHealth::Disabled { .. }
        ));
    }

    #[test]
    fn app_state_indexes_claimed_upstream_models_by_channel_scope() {
        let keys_file_a = temp_path("key-pool-router-claimed-index-keys-a");
        fs::write(&keys_file_a, "k1\n").unwrap();
        let keys_file_b = temp_path("key-pool-router-claimed-index-keys-b");
        fs::write(&keys_file_b, "k2\n").unwrap();
        let keys_file_c = temp_path("key-pool-router-claimed-index-keys-c");
        fs::write(&keys_file_c, "k3\n").unwrap();

        let mut credential_sets = HashMap::new();
        let mut pools = HashMap::new();
        for (name, keys_file) in [
            ("a", keys_file_a),
            ("b", keys_file_b),
            ("disabled", keys_file_c),
        ] {
            let credential_set = format!("{name}-credentials");
            credential_sets.insert(
                credential_set.clone(),
                CredentialSetConfig {
                    keys_file: keys_file.clone(),
                },
            );
            pools.insert(
                name.to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: name != "disabled",
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: format!("https://{name}.example/v1"),
                    credential_set,
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            );
        }

        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("a".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets,
            model_routes: HashMap::from([
                (
                    "public-a".to_string(),
                    crate::config::ModelRouteConfig {
                        strategy: Some("priority".to_string()),
                        targets: vec![crate::config::ModelRouteTargetConfig {
                            channel: "a".to_string(),
                            upstream_model: Some("shared-upstream".to_string()),
                            priority: 10,
                            weight: 1,
                            enabled: true,
                        }],
                    },
                ),
                (
                    "public-b".to_string(),
                    crate::config::ModelRouteConfig {
                        strategy: Some("priority".to_string()),
                        targets: vec![crate::config::ModelRouteTargetConfig {
                            channel: "b".to_string(),
                            upstream_model: Some("shared-upstream".to_string()),
                            priority: 10,
                            weight: 1,
                            enabled: true,
                        }],
                    },
                ),
                (
                    "public-disabled".to_string(),
                    crate::config::ModelRouteConfig {
                        strategy: Some("priority".to_string()),
                        targets: vec![crate::config::ModelRouteTargetConfig {
                            channel: "disabled".to_string(),
                            upstream_model: Some("disabled-upstream".to_string()),
                            priority: 10,
                            weight: 1,
                            enabled: true,
                        }],
                    },
                ),
            ]),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();

        let state = AppState::new(config).unwrap();

        assert!(state
            .channels
            .has_claimed_upstream_model("shared-upstream", &[]));
        assert!(state
            .channels
            .has_claimed_upstream_model("shared-upstream", &["a".to_string()]));
        assert!(state
            .channels
            .has_claimed_upstream_model("shared-upstream", &["b".to_string()]));
        assert!(!state
            .channels
            .has_claimed_upstream_model("shared-upstream", &["c".to_string()]));
        assert!(!state
            .channels
            .has_claimed_upstream_model("disabled-upstream", &[]));
    }

    #[test]
    fn channels_sharing_credential_set_use_same_credential_identity_namespace() {
        let keys_file = temp_path("key-pool-router-shared-keys");
        fs::write(&keys_file, "shared-key\n").unwrap();
        let credential_sets = HashMap::from([(
            "shared-set".to_string(),
            CredentialSetConfig {
                keys_file: keys_file.clone(),
            },
        )]);
        let mut pools = HashMap::new();
        for (name, api_base) in [("a", "https://a.example/v1"), ("b", "https://b.example/v1")] {
            pools.insert(
                name.to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: api_base.to_string(),
                    credential_set: "shared-set".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            );
        }

        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("a".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets,
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();
        let state = AppState::new(config).unwrap();
        let a_id = state
            .channels
            .get("a")
            .unwrap()
            .pool
            .blocking_lock()
            .credential_snapshots()[0]
            .id
            .clone();
        let b_id = state
            .channels
            .get("b")
            .unwrap()
            .pool
            .blocking_lock()
            .credential_snapshots()[0]
            .id
            .clone();

        assert_eq!(a_id, b_id);
    }

    #[test]
    fn channels_sharing_credential_set_share_lifecycle_state() {
        let keys_file = temp_path("key-pool-router-shared-state-keys");
        fs::write(&keys_file, "shared-key\n").unwrap();
        let credential_sets = HashMap::from([(
            "shared-set".to_string(),
            CredentialSetConfig {
                keys_file: keys_file.clone(),
            },
        )]);
        let mut pools = HashMap::new();
        for (name, api_base) in [("a", "https://a.example/v1"), ("b", "https://b.example/v1")] {
            pools.insert(
                name.to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: api_base.to_string(),
                    credential_set: "shared-set".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            );
        }

        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("a".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets,
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();
        let state = AppState::new(config).unwrap();
        let credential_id = {
            let channel = state.channels.get("a").unwrap();
            let pool = channel.pool.blocking_lock();
            pool.credential_snapshots()[0].id.clone()
        };
        {
            let channel = state.channels.get("a").unwrap();
            let mut pool = channel.pool.blocking_lock();
            pool.expire_credential_by_id(&CredentialId(credential_id.clone()), "shared lifecycle");
        }

        let channel = state.channels.get("b").unwrap();
        let pool = channel.pool.blocking_lock();
        let snapshot = pool
            .credential_snapshot_by_id(&CredentialId(credential_id))
            .unwrap();
        assert_eq!(
            snapshot.state,
            CredentialStateSnapshot::Expired {
                reason: "shared lifecycle".to_string()
            }
        );
    }

    #[test]
    fn shared_credential_set_replays_events_from_any_referencing_channel() {
        let keys_file = temp_path("key-pool-router-shared-replay-keys");
        fs::write(&keys_file, "shared-key\n").unwrap();
        let event_log_path = temp_path("key-pool-router-shared-replay-events.jsonl");
        let credential = Credential::with_source(
            "shared-set",
            "shared-key".to_string(),
            CredentialSource {
                source_path: Some(keys_file.clone()),
                source_line: Some(1),
                batch_id: None,
            },
        );
        EventLog::open(Some(event_log_path.clone()))
            .unwrap()
            .record_credential_expired_blocking("b", credential.id().0.clone(), "shared replay")
            .unwrap();
        let credential_sets = HashMap::from([(
            "shared-set".to_string(),
            CredentialSetConfig {
                keys_file: keys_file.clone(),
            },
        )]);
        let mut pools = HashMap::new();
        for (name, api_base) in [("a", "https://a.example/v1"), ("b", "https://b.example/v1")] {
            pools.insert(
                name.to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: api_base.to_string(),
                    credential_set: "shared-set".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            );
        }

        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: Some(event_log_path),
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("a".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets,
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools,
        }
        .resolve()
        .unwrap();
        let state = AppState::new(config).unwrap();
        let channel = state.channels.get("a").unwrap();
        let pool = channel.pool.blocking_lock();
        let snapshot = pool.snapshot();

        assert_eq!(snapshot.expired_credentials, 1);
        assert_eq!(snapshot.available_credentials, 0);
    }

    #[test]
    fn no_available_credentials_remains_hard_blocker_during_failure_domain_soft_cooling() {
        let keys_file = temp_path("key-pool-router-soft-cooling-no-credentials-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("test".to_string()),
            providers: HashMap::from([(
                "soft-provider".to_string(),
                ProviderConfig {
                    provider_kind: ProviderKind::OpenAiCompatible,
                    enabled: true,
                },
            )]),
            accounts: HashMap::from([(
                "soft-account".to_string(),
                AccountConfig {
                    provider: "soft-provider".to_string(),
                    api_base: "https://example.com/v1".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    enabled: true,
                },
            )]),
            credential_sets: credential_sets_from_files([("test-credentials", keys_file)]),
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools: HashMap::from([(
                "test".to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: Some("soft-account".to_string()),
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: "https://ignored.example/v1".to_string(),
                    credential_set: "test-credentials".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            )]),
        }
        .resolve()
        .unwrap();
        let state = AppState::new(config).unwrap();
        state.channels.apply_failure_domain_transition(
            "provider:soft-provider",
            "account:soft-account",
            Some(Instant::now() + Duration::from_secs(60)),
            "provider unavailable",
        );
        assert_eq!(
            state.channels.channel_route_state("test"),
            ChannelRouteState::ProviderCoolingDown
        );

        let pool_state = state.channels.get("test").unwrap();
        {
            let _pool_guard = pool_state.pool.blocking_lock();
            assert_eq!(
                state.channels.channel_route_state("test"),
                ChannelRouteState::ProviderCoolingDown
            );
        }

        let credential_id = {
            let pool = pool_state.pool.blocking_lock();
            pool.credential_snapshots()[0].id.clone()
        };
        {
            let mut pool = pool_state.pool.blocking_lock();
            pool.expire_credential_by_id(&CredentialId(credential_id), "test exhaustion");
        }

        assert_eq!(
            state.channels.channel_route_state("test"),
            ChannelRouteState::NoAvailableCredentials
        );
    }

    #[test]
    fn active_channel_cooldown_remains_hard_blocker_over_credential_cooldown() {
        let keys_file = temp_path("key-pool-router-channel-cooldown-over-credential-cooldown-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let config = single_pool_config(keys_file, None).resolve().unwrap();
        let state = AppState::new(config).unwrap();
        let pool_state = state.channels.get("test").unwrap();
        let credential_id = {
            let pool = pool_state.pool.blocking_lock();
            CredentialId(pool.credential_snapshots()[0].id.clone())
        };
        {
            let mut pool = pool_state.pool.blocking_lock();
            pool.apply_credential_cooldown_until(
                &credential_id,
                Instant::now() + Duration::from_secs(60),
                "temporary credential cooldown",
            );
        }

        assert_eq!(
            state.channels.channel_route_state("test"),
            ChannelRouteState::CredentialCoolingDown
        );

        *pool_state
            .health
            .lock()
            .expect("channel health mutex poisoned") = ChannelHealth::CoolingDown {
            until: Instant::now() + Duration::from_secs(60),
            reason: "hard channel cooldown".to_string(),
        };

        assert_eq!(
            state.channels.channel_route_state("test"),
            ChannelRouteState::CoolingDown
        );
    }

    #[test]
    fn automatic_channel_health_transition_advances_generation_and_rejects_stale_updates() {
        let keys_file = temp_path("key-pool-router-channel-health-generation-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("test".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets: credential_sets_from_files([("test-credentials", keys_file)]),
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools: HashMap::from([(
                "test".to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: "https://example.com/v1".to_string(),
                    credential_set: "test-credentials".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            )]),
        }
        .resolve()
        .unwrap();
        let state = AppState::new(config).unwrap();
        let pool_state = state.channels.get("test").unwrap();
        let generation = pool_state.channel_health_generation.load(Ordering::Acquire);
        let cooldown_until = Instant::now() + Duration::from_secs(60);

        assert!(pool_state.apply_automatic_channel_health_transition(
            generation,
            ChannelHealth::CoolingDown {
                until: cooldown_until,
                reason: "first failure".to_string(),
            },
        ));
        assert_eq!(
            pool_state.channel_health_generation.load(Ordering::Acquire),
            generation + 1
        );
        assert!(!pool_state.apply_automatic_channel_health_transition(
            generation,
            ChannelHealth::Degraded {
                reason: "stale failure".to_string(),
            },
        ));
        assert_eq!(
            *pool_state
                .health
                .lock()
                .expect("channel health mutex poisoned"),
            ChannelHealth::CoolingDown {
                until: cooldown_until,
                reason: "first failure".to_string(),
            }
        );
    }

    #[test]
    fn relay_balance_suppression_reapplies_after_channel_cooldown_expires() {
        let keys_file = temp_path("key-pool-router-relay-balance-expired-cooldown-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let config = single_pool_config(keys_file, None).resolve().unwrap();
        let state = AppState::new(config).unwrap();
        let pool_state = state.channels.get("test").unwrap();
        let initial_generation = pool_state.channel_health_generation.load(Ordering::Acquire);

        let first_count = pool_state
            .apply_automatic_relay_balance_suppression(
                initial_generation,
                Instant::now() - Duration::from_secs(1),
                "relay balance unavailable",
            )
            .expect("initial suppression should apply");
        assert_eq!(first_count, 1);
        assert_eq!(pool_state.relay_suppression_count(), 1);
        assert_eq!(
            route_state_for_pool(&pool_state),
            ChannelRouteState::Available
        );

        let next_generation = pool_state.channel_health_generation.load(Ordering::Acquire);
        let next_until = Instant::now() + Duration::from_secs(60);
        let second_count = pool_state
            .apply_automatic_relay_balance_suppression(
                next_generation,
                next_until,
                "relay balance unavailable again",
            )
            .expect("expired cooldown should accept a new suppression");

        assert_eq!(second_count, 2);
        assert_eq!(pool_state.relay_suppression_count(), 2);
        assert_eq!(
            *pool_state
                .health
                .lock()
                .expect("channel health mutex poisoned"),
            ChannelHealth::CoolingDown {
                until: next_until,
                reason: "relay balance unavailable again".to_string(),
            }
        );

        let active_generation = pool_state.channel_health_generation.load(Ordering::Acquire);
        assert_eq!(
            pool_state.apply_automatic_relay_balance_suppression(
                active_generation,
                next_until + Duration::from_secs(60),
                "duplicate relay balance unavailable",
            ),
            None
        );
        assert_eq!(pool_state.relay_suppression_count(), 2);
        assert_eq!(
            *pool_state
                .health
                .lock()
                .expect("channel health mutex poisoned"),
            ChannelHealth::CoolingDown {
                until: next_until,
                reason: "relay balance unavailable again".to_string(),
            }
        );
    }

    #[test]
    fn automatic_channel_health_transition_does_not_override_manual_disabled_state() {
        let keys_file = temp_path("key-pool-router-channel-health-disabled-keys");
        fs::write(&keys_file, "k1\n").unwrap();
        let config = AppConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            client_tokens: vec![ClientTokenConfig {
                name: "test-client".to_string(),
                token: fixtures().client_token.clone(),
                enabled: true,
                allowed_model_groups: Vec::new(),
                allowed_channels: Vec::new(),
            }],
            management: Some(ManagementConfig {
                admin_token: fixtures().admin_token.clone(),
                ip_allowlist: None,
                principals: Vec::new(),
                event_log_path: None,
                event_window_capacity: None,
            }),
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: crate::config::RoutingConfig::default(),
            default_pool: Some("test".to_string()),
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets: credential_sets_from_files([("test-credentials", keys_file)]),
            model_routes: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: Some("default-routing".to_string()),
            routing_profiles: std::collections::HashMap::from([(
                "default-routing".to_string(),
                crate::config::RoutingProfileConfig {
                    key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                    default_credential_cooldown_seconds: 20,
                    same_request_credential_retry:
                        crate::config::SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 0,
                        },
                    route_target_retry: crate::config::RouteTargetRetryConfig { enabled: true },
                },
            )]),
            pools: HashMap::from([(
                "test".to_string(),
                PoolConfig {
                    endpoint_capabilities: Default::default(),
                    enabled: true,
                    account: None,
                    policy_profile: None,
                    routing_profile: None,
                    provider_kind: ProviderKind::OpenAiCompatible,
                    api_base: "https://example.com/v1".to_string(),
                    credential_set: "test-credentials".to_string(),
                    auth_header: "authorization".to_string(),
                    auth_prefix: "Bearer ".to_string(),
                    error_rules: ErrorRulesConfig::default(),
                },
            )]),
        }
        .resolve()
        .unwrap();
        let state = AppState::new(config).unwrap();
        let pool_state = state.channels.get("test").unwrap();
        let generation = pool_state.channel_health_generation.load(Ordering::Acquire);
        *pool_state
            .health
            .lock()
            .expect("channel health mutex poisoned") = ChannelHealth::Disabled {
            reason: "manual stop".to_string(),
        };

        assert!(!pool_state.apply_automatic_channel_health_transition(
            generation,
            ChannelHealth::Degraded {
                reason: "automatic failure".to_string(),
            },
        ));
        assert_eq!(
            *pool_state
                .health
                .lock()
                .expect("channel health mutex poisoned"),
            ChannelHealth::Disabled {
                reason: "manual stop".to_string(),
            }
        );
        assert_eq!(
            pool_state.channel_health_generation.load(Ordering::Acquire),
            generation
        );
    }
}
