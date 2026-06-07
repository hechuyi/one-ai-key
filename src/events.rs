use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManagementEvent {
    pub id: u64,
    #[serde(default)]
    pub created_at_unix_seconds: u64,
    pub kind: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub resource_type: String,
    #[serde(default)]
    pub resource_id: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub generation: Option<u64>,
    pub channel_id: String,
    pub credential_id: String,
    pub reason: String,
    #[serde(default)]
    pub reason_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ManagementEventActor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagementEventActor {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone)]
pub struct ManagementAuditEvent {
    pub kind: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub channel_id: String,
    pub credential_id: String,
    pub outcome: String,
    pub request_id: Option<String>,
    pub generation: Option<u64>,
    pub reason_code: String,
    pub actor: Option<ManagementEventActor>,
}

impl ManagementAuditEvent {
    pub fn applied(
        kind: impl Into<String>,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        reason_code: impl Into<String>,
        actor: Option<ManagementEventActor>,
    ) -> Self {
        let kind = kind.into();
        Self {
            action: kind.clone(),
            kind,
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            channel_id: String::new(),
            credential_id: String::new(),
            outcome: "applied".to_string(),
            request_id: None,
            generation: None,
            reason_code: reason_code.into(),
            actor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    Expired {
        id: u64,
        channel_id: String,
        credential_id: String,
        reason: String,
    },
    QuotaExhausted {
        id: u64,
        channel_id: String,
        credential_id: String,
        reason: String,
    },
    Restored {
        id: u64,
        channel_id: String,
        credential_id: String,
        reason: String,
    },
    Disabled {
        id: u64,
        channel_id: String,
        credential_id: String,
        reason: String,
    },
    Enabled {
        id: u64,
        channel_id: String,
        credential_id: String,
        reason: String,
    },
    CooldownCleared {
        id: u64,
        channel_id: String,
        credential_id: String,
        reason: String,
    },
    ChannelDisabled {
        id: u64,
        channel_id: String,
        reason: String,
    },
    ChannelEnabled {
        id: u64,
        channel_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoutingTelemetry {
    RouteSelected {
        request_id: String,
        registry_generation: u64,
        channel_id: String,
    },
    UpstreamFailureObserved {
        request_id: String,
        channel_id: String,
        failure: Box<UpstreamFailureTelemetry>,
    },
    TransitionApplied {
        request_id: String,
        channel_id: String,
    },
    ChannelHealthTransitionApplied {
        request_id: String,
        channel_id: String,
        state: String,
        reason: String,
    },
    CredentialTransitionApplied {
        request_id: String,
        channel_id: String,
        credential_id: String,
        state: String,
        reason: String,
    },
    CredentialLifecyclePersistenceDropped {
        request_id: String,
        channel_id: String,
        credential_id: String,
        state: String,
        reason: String,
        drop_reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpstreamFailureTelemetry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_model: Option<String>,
    pub credential_id_hash: String,
    pub attempt: usize,
    pub failure_source: String,
    pub failure_kind: String,
    pub failure_scope: String,
    pub retryable: bool,
    pub confidence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    pub classifier_id: String,
    pub classifier_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptation_rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_seconds: Option<u64>,
    pub directive: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denial_reason: Option<String>,
    pub duplicate_charge_risk: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_deadline_remaining_ms: Option<u64>,
    pub retry_pressure_accounted: bool,
    pub retry_decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_decision_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseFilterEvent {
    pub event_id: u64,
    pub created_at_unix_seconds: u64,
    pub request_id: String,
    pub channel_id: String,
    pub public_model: String,
    pub rule_id: String,
    pub action: String,
    pub content_kind: String,
    pub reason_code: String,
    pub outcome: String,
    pub body_committed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseFilterEventInput {
    pub request_id: String,
    pub channel_id: String,
    pub public_model: String,
    pub rule_id: String,
    pub action: String,
    pub content_kind: String,
    pub reason_code: String,
    pub outcome: String,
    pub body_committed: bool,
}

#[derive(Debug, Clone)]
pub struct ResponseFilterEventBuffer {
    capacity: usize,
    dropped_events: u64,
    next_id: u64,
    events: VecDeque<ResponseFilterEvent>,
}

impl ResponseFilterEventBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            dropped_events: 0,
            next_id: 0,
            events: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, input: ResponseFilterEventInput) -> Option<ResponseFilterEvent> {
        self.push_at_unix_seconds(input, current_unix_seconds())
    }

    pub fn push_at_unix_seconds(
        &mut self,
        input: ResponseFilterEventInput,
        created_at_unix_seconds: u64,
    ) -> Option<ResponseFilterEvent> {
        if self.capacity == 0 {
            self.record_dropped_event();
            return None;
        }
        self.next_id = self.next_id.saturating_add(1);
        let event = ResponseFilterEvent {
            event_id: self.next_id,
            created_at_unix_seconds,
            request_id: input.request_id,
            channel_id: input.channel_id,
            public_model: input.public_model,
            rule_id: input.rule_id,
            action: input.action,
            content_kind: input.content_kind,
            reason_code: input.reason_code,
            outcome: input.outcome,
            body_committed: input.body_committed,
        };
        if self.events.len() == self.capacity {
            self.events.pop_front();
            self.record_dropped_event();
        }
        self.events.push_back(event.clone());
        Some(event)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    pub fn record_dropped_event(&mut self) {
        self.record_dropped_events(1);
    }

    pub fn record_dropped_events(&mut self, dropped: u64) {
        self.dropped_events = self.dropped_events.saturating_add(dropped);
    }

    pub fn snapshot(&self) -> Vec<ResponseFilterEvent> {
        self.events.iter().cloned().collect()
    }

    pub fn replace_capacity(&mut self, capacity: usize) {
        let dropped = self.events.len().saturating_sub(capacity);
        self.record_dropped_events(u64::try_from(dropped).unwrap_or(u64::MAX));
        self.capacity = capacity;
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
        self.events.shrink_to_fit();
        self.events
            .reserve(self.capacity.saturating_sub(self.events.len()));
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetryPressureDirectiveCounters {
    pub retry_credential: u64,
    pub retry_route_target: u64,
    pub retry_same_target: u64,
    pub return_error: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetryPressureDuplicateChargeRiskCounters {
    pub none: u64,
    pub known_no_charge: u64,
    pub unknown: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetryPressureCounters {
    pub by_directive: RetryPressureDirectiveCounters,
    pub by_denial_reason: BTreeMap<String, u64>,
    pub by_duplicate_charge_risk: RetryPressureDuplicateChargeRiskCounters,
}

impl RetryPressureCounters {
    fn observe(&mut self, failure: &UpstreamFailureTelemetry) {
        if !failure.retry_pressure_accounted {
            return;
        }
        match failure.directive.as_str() {
            "retry_credential" => {
                self.by_directive.retry_credential =
                    self.by_directive.retry_credential.saturating_add(1)
            }
            "retry_route_target" => {
                self.by_directive.retry_route_target =
                    self.by_directive.retry_route_target.saturating_add(1)
            }
            "retry_same_target" => {
                self.by_directive.retry_same_target =
                    self.by_directive.retry_same_target.saturating_add(1)
            }
            "return_error" | "return_current_error" => {
                self.by_directive.return_error = self.by_directive.return_error.saturating_add(1)
            }
            _ => {}
        }
        if let Some(denial_reason) = failure.denial_reason.as_deref() {
            let count = self
                .by_denial_reason
                .entry(denial_reason.to_string())
                .or_default();
            *count = count.saturating_add(1);
        }
        match failure.duplicate_charge_risk.as_str() {
            "none" => {
                self.by_duplicate_charge_risk.none =
                    self.by_duplicate_charge_risk.none.saturating_add(1)
            }
            "known_no_charge" => {
                self.by_duplicate_charge_risk.known_no_charge = self
                    .by_duplicate_charge_risk
                    .known_no_charge
                    .saturating_add(1)
            }
            "unknown" => {
                self.by_duplicate_charge_risk.unknown =
                    self.by_duplicate_charge_risk.unknown.saturating_add(1)
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoutingTelemetryBuffer {
    capacity: usize,
    dropped_events: u64,
    events: VecDeque<RoutingTelemetry>,
}

impl RoutingTelemetryBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            dropped_events: 0,
            events: VecDeque::with_capacity(capacity),
        }
    }

    pub fn push(&mut self, event: RoutingTelemetry) {
        if self.capacity == 0 {
            self.record_dropped_event();
            return;
        }
        if self.events.len() == self.capacity {
            self.events.pop_front();
            self.record_dropped_event();
        }
        self.events.push_back(event);
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    pub fn record_dropped_event(&mut self) {
        self.record_dropped_events(1);
    }

    pub fn record_dropped_events(&mut self, dropped: u64) {
        self.dropped_events = self.dropped_events.saturating_add(dropped);
    }

    pub fn snapshot(&self) -> Vec<RoutingTelemetry> {
        self.events.iter().cloned().collect()
    }

    pub fn retry_pressure_snapshot(&self) -> RetryPressureCounters {
        let mut counters = RetryPressureCounters::default();
        for event in &self.events {
            if let RoutingTelemetry::UpstreamFailureObserved { failure, .. } = event {
                counters.observe(failure);
            }
        }
        counters
    }

    pub fn replace_capacity(&mut self, capacity: usize) {
        let dropped = self.events.len().saturating_sub(capacity);
        self.record_dropped_events(u64::try_from(dropped).unwrap_or(u64::MAX));
        self.capacity = capacity;
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
        self.events.shrink_to_fit();
        self.events
            .reserve(self.capacity.saturating_sub(self.events.len()));
    }
}

impl ManagementEvent {
    pub fn audit_action(&self) -> String {
        if self.action.is_empty() {
            self.kind.clone()
        } else {
            self.action.clone()
        }
    }

    pub fn audit_resource_type(&self) -> String {
        if self.resource_type.is_empty() {
            management_event_resource(&self.kind, &self.channel_id, &self.credential_id).0
        } else {
            self.resource_type.clone()
        }
    }

    pub fn audit_resource_id(&self) -> String {
        if self.resource_id.is_empty() {
            management_event_resource(&self.kind, &self.channel_id, &self.credential_id).1
        } else {
            self.resource_id.clone()
        }
    }

    pub fn audit_outcome(&self) -> String {
        if self.outcome.is_empty() {
            "applied".to_string()
        } else {
            self.outcome.clone()
        }
    }

    pub fn audit_reason_code(&self) -> String {
        if self.reason_code.is_empty() {
            management_event_reason_code(&self.kind)
        } else {
            self.reason_code.clone()
        }
    }

    fn replay_reason(&self) -> String {
        if self.reason_code.is_empty() {
            self.reason.clone()
        } else {
            self.reason_code.clone()
        }
    }

    pub fn to_domain_event(&self) -> Option<DomainEvent> {
        let reason = self.replay_reason();
        match self.kind.as_str() {
            "credential_expired" => Some(DomainEvent::Expired {
                id: self.id,
                channel_id: self.channel_id.clone(),
                credential_id: self.credential_id.clone(),
                reason: reason.clone(),
            }),
            "credential_quota_exhausted" => Some(DomainEvent::QuotaExhausted {
                id: self.id,
                channel_id: self.channel_id.clone(),
                credential_id: self.credential_id.clone(),
                reason: reason.clone(),
            }),
            "credential_restored" => Some(DomainEvent::Restored {
                id: self.id,
                channel_id: self.channel_id.clone(),
                credential_id: self.credential_id.clone(),
                reason: reason.clone(),
            }),
            "credential_disabled" => Some(DomainEvent::Disabled {
                id: self.id,
                channel_id: self.channel_id.clone(),
                credential_id: self.credential_id.clone(),
                reason: reason.clone(),
            }),
            "credential_enabled" => Some(DomainEvent::Enabled {
                id: self.id,
                channel_id: self.channel_id.clone(),
                credential_id: self.credential_id.clone(),
                reason: reason.clone(),
            }),
            "credential_cooldown_cleared" => Some(DomainEvent::CooldownCleared {
                id: self.id,
                channel_id: self.channel_id.clone(),
                credential_id: self.credential_id.clone(),
                reason: reason.clone(),
            }),
            "channel_disabled" => Some(DomainEvent::ChannelDisabled {
                id: self.id,
                channel_id: self.channel_id.clone(),
                reason: reason.clone(),
            }),
            "channel_enabled" => Some(DomainEvent::ChannelEnabled {
                id: self.id,
                channel_id: self.channel_id.clone(),
                reason,
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EventLog {
    inner: Arc<Mutex<EventLogInner>>,
    path: Option<PathBuf>,
    window_capacity: usize,
    #[cfg(test)]
    record_delay: Option<Duration>,
    #[cfg(test)]
    append_delay: Option<Duration>,
}

impl Default for EventLog {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventLogInner::default())),
            path: None,
            window_capacity: Self::DEFAULT_WINDOW_CAPACITY,
            #[cfg(test)]
            record_delay: None,
            #[cfg(test)]
            append_delay: None,
        }
    }
}

#[derive(Debug, Default)]
struct EventLogInner {
    next_id: u64,
    total_events: usize,
    events: VecDeque<ManagementEvent>,
}

impl EventLog {
    const DEFAULT_WINDOW_CAPACITY: usize = 1024;

    #[cfg(test)]
    pub fn open(path: Option<PathBuf>) -> anyhow::Result<Self> {
        Self::open_with_window_capacity(path, Self::DEFAULT_WINDOW_CAPACITY)
    }

    pub(crate) fn open_with_window_capacity(
        path: Option<PathBuf>,
        window_capacity: usize,
    ) -> anyhow::Result<Self> {
        let events = match &path {
            Some(path) => Self::read_events_from_path(path)?,
            None => Vec::new(),
        };
        let mut next_id = 0u64;
        for event in &events {
            next_id = next_id.max(event.id);
        }
        let total_events = events.len();
        let events = bounded_event_window(events, window_capacity);
        Ok(Self {
            inner: Arc::new(Mutex::new(EventLogInner {
                next_id,
                total_events,
                events,
            })),
            path,
            window_capacity,
            #[cfg(test)]
            record_delay: None,
            #[cfg(test)]
            append_delay: None,
        })
    }

    #[cfg(test)]
    pub fn with_record_delay(delay: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventLogInner::default())),
            path: None,
            window_capacity: Self::DEFAULT_WINDOW_CAPACITY,
            record_delay: Some(delay),
            append_delay: None,
        }
    }

    #[cfg(test)]
    pub fn with_append_delay(delay: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventLogInner::default())),
            path: None,
            window_capacity: Self::DEFAULT_WINDOW_CAPACITY,
            record_delay: None,
            append_delay: Some(delay),
        }
    }

    #[cfg(test)]
    fn with_window_capacity(window_capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventLogInner::default())),
            path: None,
            window_capacity,
            record_delay: None,
            append_delay: None,
        }
    }

    #[cfg(test)]
    pub fn record_credential_expired_blocking(
        &self,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.record_event(
            "credential_expired",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            None,
        )
    }

    pub async fn record_credential_expired_transaction<T, F>(
        &self,
        actor: ManagementEventActor,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        self.record_event_transaction(
            "credential_expired",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            Some(actor),
            transaction,
        )
        .await
    }

    pub async fn record_credential_quota_exhausted_transaction<T, F>(
        &self,
        actor: ManagementEventActor,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        self.record_event_transaction(
            "credential_quota_exhausted",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            Some(actor),
            transaction,
        )
        .await
    }

    #[cfg(test)]
    pub fn record_credential_restored_blocking(
        &self,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.record_event(
            "credential_restored",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            None,
        )
    }

    #[cfg(test)]
    pub fn record_credential_disabled_blocking(
        &self,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.record_event(
            "credential_disabled",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            None,
        )
    }

    #[cfg(test)]
    pub fn record_credential_enabled_blocking(
        &self,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.record_event(
            "credential_enabled",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            None,
        )
    }

    pub async fn record_credential_restored_transaction<T, F>(
        &self,
        actor: ManagementEventActor,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        self.record_event_transaction(
            "credential_restored",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            Some(actor),
            transaction,
        )
        .await
    }

    pub async fn record_credential_disabled_transaction<T, F>(
        &self,
        actor: ManagementEventActor,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        self.record_event_transaction(
            "credential_disabled",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            Some(actor),
            transaction,
        )
        .await
    }

    pub async fn record_credential_enabled_transaction<T, F>(
        &self,
        actor: ManagementEventActor,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        self.record_event_transaction(
            "credential_enabled",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            Some(actor),
            transaction,
        )
        .await
    }

    pub async fn record_credential_cooldown_cleared_transaction<T, F>(
        &self,
        actor: ManagementEventActor,
        channel_id: impl Into<String>,
        credential_id: impl Into<String>,
        reason: impl Into<String>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        self.record_event_transaction(
            "credential_cooldown_cleared",
            channel_id.into(),
            credential_id.into(),
            reason.into(),
            Some(actor),
            transaction,
        )
        .await
    }

    pub async fn record_channel_disabled_transaction<T, F>(
        &self,
        actor: ManagementEventActor,
        channel_id: impl Into<String>,
        reason: impl Into<String>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        self.record_event_transaction(
            "channel_disabled",
            channel_id.into(),
            String::new(),
            reason.into(),
            Some(actor),
            transaction,
        )
        .await
    }

    pub async fn record_channel_enabled_transaction<T, F>(
        &self,
        actor: ManagementEventActor,
        channel_id: impl Into<String>,
        reason: impl Into<String>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        self.record_event_transaction(
            "channel_enabled",
            channel_id.into(),
            String::new(),
            reason.into(),
            Some(actor),
            transaction,
        )
        .await
    }

    pub async fn record_audit_event(&self, audit: ManagementAuditEvent) -> anyhow::Result<()> {
        let log = self.clone();
        tokio::task::spawn_blocking(move || {
            log.apply_record_delay();
            log.record_audit_event_now(audit)
        })
        .await?
    }

    async fn record_event_transaction<T, F>(
        &self,
        kind: impl Into<String>,
        channel_id: String,
        credential_id: String,
        reason: String,
        actor: Option<ManagementEventActor>,
        transaction: F,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(PendingManagementEvent) -> anyhow::Result<T> + Send + 'static,
    {
        let pending = PendingManagementEvent {
            log: self.clone(),
            kind: kind.into(),
            channel_id,
            credential_id,
            reason,
            actor,
        };
        tokio::task::spawn_blocking(move || {
            pending.log.apply_record_delay();
            transaction(pending)
        })
        .await?
    }

    #[cfg(test)]
    fn record_event(
        &self,
        kind: impl Into<String>,
        channel_id: String,
        credential_id: String,
        reason: String,
        actor: Option<ManagementEventActor>,
    ) -> anyhow::Result<()> {
        self.apply_record_delay();
        self.record_event_now(kind, channel_id, credential_id, reason, actor)
    }

    fn apply_record_delay(&self) {
        #[cfg(test)]
        if let Some(delay) = self.record_delay {
            std::thread::sleep(delay);
        }
    }

    fn record_event_now(
        &self,
        kind: impl Into<String>,
        channel_id: String,
        credential_id: String,
        _reason: String,
        actor: Option<ManagementEventActor>,
    ) -> anyhow::Result<()> {
        let kind = kind.into();
        let reason_code = management_event_reason_code(&kind);
        let (resource_type, resource_id) =
            management_event_resource(&kind, &channel_id, &credential_id);
        self.record_audit_event_now(ManagementAuditEvent {
            action: kind.clone(),
            kind,
            resource_type,
            resource_id,
            channel_id,
            credential_id,
            outcome: "applied".to_string(),
            request_id: None,
            generation: None,
            reason_code,
            actor,
        })
    }

    fn record_audit_event_now(&self, audit: ManagementAuditEvent) -> anyhow::Result<()> {
        self.apply_append_delay();
        let mut inner = self.inner.lock().unwrap();
        inner.next_id += 1;
        let id = inner.next_id;
        let reason_code = if audit.reason_code.is_empty() {
            management_event_reason_code(&audit.kind)
        } else {
            audit.reason_code
        };
        let event = ManagementEvent {
            id,
            created_at_unix_seconds: current_unix_seconds(),
            action: if audit.action.is_empty() {
                audit.kind.clone()
            } else {
                audit.action
            },
            resource_type: if audit.resource_type.is_empty() {
                management_event_resource(&audit.kind, &audit.channel_id, &audit.credential_id).0
            } else {
                audit.resource_type
            },
            resource_id: if audit.resource_id.is_empty() {
                management_event_resource(&audit.kind, &audit.channel_id, &audit.credential_id).1
            } else {
                audit.resource_id
            },
            outcome: if audit.outcome.is_empty() {
                "applied".to_string()
            } else {
                audit.outcome
            },
            request_id: audit.request_id,
            generation: audit.generation,
            kind: audit.kind,
            channel_id: audit.channel_id,
            credential_id: audit.credential_id,
            reason: reason_code.clone(),
            reason_code,
            actor: audit.actor,
        };
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            serde_json::to_writer(&mut file, &event)?;
            file.write_all(b"\n")?;
        }
        inner.total_events += 1;
        push_event_window(&mut inner.events, self.window_capacity, event);
        Ok(())
    }

    fn apply_append_delay(&self) {
        #[cfg(test)]
        if let Some(delay) = self.append_delay {
            std::thread::sleep(delay);
        }
    }

    pub async fn window(&self, offset: usize, limit: usize) -> (usize, Vec<ManagementEvent>) {
        self.window_blocking(offset, limit)
    }

    pub fn window_blocking(&self, offset: usize, limit: usize) -> (usize, Vec<ManagementEvent>) {
        let inner = self.inner.lock().unwrap();
        let total = inner.total_events;
        let retained_offset = total.saturating_sub(inner.events.len());
        let local_offset = offset.saturating_sub(retained_offset);
        let events = inner
            .events
            .iter()
            .skip(local_offset)
            .take(limit)
            .cloned()
            .collect();
        (total, events)
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().total_events
    }

    pub fn window_capacity(&self) -> usize {
        self.window_capacity
    }

    #[cfg(test)]
    fn snapshot_blocking(&self) -> Vec<ManagementEvent> {
        self.inner.lock().unwrap().events.iter().cloned().collect()
    }

    pub fn replay_events_from_path(path: Option<&Path>) -> anyhow::Result<Vec<ManagementEvent>> {
        match path {
            Some(path) => Self::read_events_from_path(path),
            None => Ok(Vec::new()),
        }
    }

    fn read_events_from_path(path: &Path) -> anyhow::Result<Vec<ManagementEvent>> {
        let mut events = Vec::new();
        if path.exists() {
            let file = fs::File::open(path)?;
            let mut lines = BufReader::new(file).lines().peekable();
            while let Some(line) = lines.next() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let event: ManagementEvent = match serde_json::from_str(&line) {
                    Ok(event) => event,
                    Err(err) if lines.peek().is_none() => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %err,
                            "ignoring corrupt trailing management event log line"
                        );
                        break;
                    }
                    Err(err) => return Err(err.into()),
                };
                events.push(event);
            }
        }
        Ok(events)
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn management_event_reason_code(kind: &str) -> String {
    match kind {
        "credential_expired" => "manual_expire",
        "credential_quota_exhausted" => "manual_quota_exhaust",
        "credential_restored" => "manual_restore",
        "credential_disabled" => "manual_disable",
        "credential_enabled" => "manual_enable",
        "credential_cooldown_cleared" => "manual_clear_cooldown",
        "channel_disabled" => "manual_channel_disable",
        "channel_enabled" => "manual_channel_enable",
        "client_token_created" => "manual_client_token_create",
        "client_token_scope_updated" => "manual_client_token_scope_update",
        "client_token_disabled" => "manual_client_token_disable",
        "client_token_enabled" => "manual_client_token_enable",
        other => other,
    }
    .to_string()
}

fn management_event_resource(
    kind: &str,
    channel_id: &str,
    credential_id: &str,
) -> (String, String) {
    if kind.starts_with("credential_") {
        ("credential".to_string(), credential_id.to_string())
    } else if kind.starts_with("client_token_") {
        ("client_token".to_string(), channel_id.to_string())
    } else if kind.starts_with("channel_") {
        ("channel".to_string(), channel_id.to_string())
    } else {
        ("management".to_string(), channel_id.to_string())
    }
}

fn bounded_event_window(
    events: Vec<ManagementEvent>,
    window_capacity: usize,
) -> VecDeque<ManagementEvent> {
    let retain_from = events.len().saturating_sub(window_capacity);
    events.into_iter().skip(retain_from).collect()
}

fn push_event_window(
    events: &mut VecDeque<ManagementEvent>,
    capacity: usize,
    event: ManagementEvent,
) {
    if capacity == 0 {
        return;
    }
    if events.len() == capacity {
        events.pop_front();
    }
    events.push_back(event);
}

pub struct PendingManagementEvent {
    log: EventLog,
    kind: String,
    channel_id: String,
    credential_id: String,
    reason: String,
    actor: Option<ManagementEventActor>,
}

impl PendingManagementEvent {
    pub fn append(self) -> anyhow::Result<()> {
        self.log.record_event_now(
            self.kind,
            self.channel_id,
            self.credential_id,
            self.reason,
            self.actor,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn event_log_persists_events_to_jsonl() {
        let mut path = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("key-pool-router-events-{suffix}.jsonl"));

        let log = EventLog::open(Some(path.clone())).unwrap();
        log.record_credential_expired_blocking("channel-a", "cred-a", "manual")
            .unwrap();
        log.record_credential_restored_blocking("channel-a", "cred-a", "manual restore")
            .unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"kind\":\"credential_expired\""));
        assert!(raw.contains("\"kind\":\"credential_restored\""));
        assert!(raw.contains("\"credential_id\":\"cred-a\""));

        let reopened = EventLog::open(Some(path)).unwrap();
        let events = reopened.snapshot_blocking();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "credential_expired");
        assert_eq!(events[0].channel_id, "channel-a");
        assert_eq!(events[0].credential_id, "cred-a");
        assert_eq!(events[0].reason, "manual_expire");
        assert_eq!(events[0].reason_code, "manual_expire");
        assert_eq!(events[1].kind, "credential_restored");
        assert_eq!(events[1].reason, "manual_restore");
        assert_eq!(events[1].reason_code, "manual_restore");
    }

    #[tokio::test]
    async fn event_log_writes_stable_audit_schema_without_freeform_reason() {
        let mut path = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("key-pool-router-audit-schema-{suffix}.jsonl"));

        let log = EventLog::open(Some(path.clone())).unwrap();
        log.record_credential_expired_transaction(
            ManagementEventActor {
                id: "principal-admin".to_string(),
                name: "admin".to_string(),
                role: "admin".to_string(),
            },
            "channel-a",
            "cred-a",
            "freeform operator note",
            |pending| {
                pending.append()?;
                Ok(())
            },
        )
        .await
        .unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("freeform operator note"));
        let value = serde_json::from_str::<serde_json::Value>(raw.lines().next().unwrap()).unwrap();
        assert!(value["created_at_unix_seconds"].as_u64().unwrap() > 0);
        assert_eq!(value["actor"]["id"], "principal-admin");
        assert_eq!(value["actor"]["role"], "admin");
        assert_eq!(value["action"], "credential_expired");
        assert_eq!(value["resource_type"], "credential");
        assert_eq!(value["resource_id"], "cred-a");
        assert_eq!(value["outcome"], "applied");
        assert_eq!(value["reason_code"], "manual_expire");
        assert_eq!(value["reason"], "manual_expire");
        assert!(value.get("request_id").is_some());
        assert!(value.get("generation").is_some());
    }

    #[test]
    fn event_log_replays_legacy_records_without_audit_schema_fields() {
        let mut path = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("key-pool-router-legacy-audit-{suffix}.jsonl"));
        fs::write(
            &path,
            r#"{"id":7,"kind":"credential_expired","channel_id":"channel-a","credential_id":"cred-a","reason":"legacy operator reason"}"#,
        )
        .unwrap();

        let reopened = EventLog::open(Some(path)).unwrap();
        let events = reopened.snapshot_blocking();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].created_at_unix_seconds, 0);
        assert_eq!(events[0].reason_code, "");
        assert_eq!(
            events[0].to_domain_event(),
            Some(DomainEvent::Expired {
                id: 7,
                channel_id: "channel-a".to_string(),
                credential_id: "cred-a".to_string(),
                reason: "legacy operator reason".to_string(),
            })
        );
    }

    #[test]
    fn event_log_rejects_corrupt_middle_line() {
        let mut path = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "key-pool-router-events-corrupt-middle-{suffix}.jsonl"
        ));

        let log = EventLog::open(Some(path.clone())).unwrap();
        log.record_credential_expired_blocking("channel-a", "cred-a", "manual")
            .unwrap();
        {
            use std::io::Write;
            writeln!(
                fs::OpenOptions::new().append(true).open(&path).unwrap(),
                "{{"
            )
            .unwrap();
        }
        log.record_credential_restored_blocking("channel-a", "cred-a", "restore")
            .unwrap();

        let err = EventLog::open(Some(path)).unwrap_err().to_string();
        assert!(err.contains("EOF") || err.contains("expected") || err.contains("key"));
    }

    #[test]
    fn event_log_tolerates_corrupt_trailing_line() {
        let mut path = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!(
            "key-pool-router-events-corrupt-tail-{suffix}.jsonl"
        ));

        let log = EventLog::open(Some(path.clone())).unwrap();
        log.record_credential_expired_blocking("channel-a", "cred-a", "manual")
            .unwrap();
        use std::io::Write;
        writeln!(
            fs::OpenOptions::new().append(true).open(&path).unwrap(),
            "{{"
        )
        .unwrap();

        let reopened = EventLog::open(Some(path)).unwrap();
        let events = reopened.snapshot_blocking();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].credential_id, "cred-a");
    }

    #[test]
    fn event_log_window_returns_bounded_slice_and_total() {
        let log = EventLog::default();
        log.record_credential_expired_blocking("channel-a", "cred-a", "one")
            .unwrap();
        log.record_credential_restored_blocking("channel-a", "cred-a", "two")
            .unwrap();
        log.record_credential_expired_blocking("channel-a", "cred-b", "three")
            .unwrap();

        let (total, events) = log.window_blocking(1, 1);

        assert_eq!(total, 3);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, "manual_restore");
    }

    #[test]
    fn event_log_keeps_bounded_memory_window_and_total_count() {
        let log = EventLog::with_window_capacity(2);
        log.record_credential_expired_blocking("channel-a", "cred-a", "one")
            .unwrap();
        log.record_credential_restored_blocking("channel-a", "cred-a", "two")
            .unwrap();
        log.record_credential_expired_blocking("channel-a", "cred-b", "three")
            .unwrap();

        let (total, events) = log.window_blocking(0, 10);

        assert_eq!(total, 3);
        assert_eq!(log.len(), 3);
        let reasons: Vec<&str> = events.iter().map(|event| event.reason.as_str()).collect();
        assert_eq!(reasons, vec!["manual_restore", "manual_expire"]);
    }

    #[test]
    fn event_log_replay_reads_full_jsonl_even_when_memory_window_is_bounded() {
        let mut path = std::env::temp_dir();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("key-pool-router-bounded-events-{suffix}.jsonl"));

        let log = EventLog::open_with_window_capacity(Some(path.clone()), 2).unwrap();
        log.record_credential_expired_blocking("channel-a", "cred-a", "one")
            .unwrap();
        log.record_credential_restored_blocking("channel-a", "cred-a", "two")
            .unwrap();
        log.record_credential_expired_blocking("channel-a", "cred-b", "three")
            .unwrap();

        let replay_events = EventLog::replay_events_from_path(Some(&path)).unwrap();

        assert_eq!(replay_events.len(), 3);
        let reasons: Vec<&str> = replay_events
            .iter()
            .map(|event| event.reason.as_str())
            .collect();
        assert_eq!(
            reasons,
            vec!["manual_expire", "manual_restore", "manual_expire"]
        );
    }

    #[test]
    fn management_event_projects_replayable_domain_event_without_actor_metadata() {
        let event = ManagementEvent {
            id: 7,
            kind: "credential_expired".to_string(),
            channel_id: "channel-a".to_string(),
            credential_id: "cred-a".to_string(),
            reason: "manual".to_string(),
            actor: Some(ManagementEventActor {
                id: "admin".to_string(),
                name: "local-admin".to_string(),
                role: "admin".to_string(),
            }),
            ..Default::default()
        };

        assert_eq!(
            event.to_domain_event(),
            Some(DomainEvent::Expired {
                id: 7,
                channel_id: "channel-a".to_string(),
                credential_id: "cred-a".to_string(),
                reason: "manual".to_string(),
            })
        );
    }

    #[test]
    fn quota_exhausted_event_projects_replayable_domain_event() {
        let event = ManagementEvent {
            id: 11,
            kind: "credential_quota_exhausted".to_string(),
            channel_id: "channel-a".to_string(),
            credential_id: "cred-a".to_string(),
            reason: "quota evidence".to_string(),
            actor: None,
            ..Default::default()
        };

        assert_eq!(
            event.to_domain_event(),
            Some(DomainEvent::QuotaExhausted {
                id: 11,
                channel_id: "channel-a".to_string(),
                credential_id: "cred-a".to_string(),
                reason: "quota evidence".to_string(),
            })
        );
    }

    #[test]
    fn cooldown_clear_event_projects_replayable_domain_event() {
        let event = ManagementEvent {
            id: 9,
            kind: "credential_cooldown_cleared".to_string(),
            channel_id: "channel-a".to_string(),
            credential_id: "cred-a".to_string(),
            reason: "manual reset".to_string(),
            actor: None,
            ..Default::default()
        };

        assert_eq!(
            event.to_domain_event(),
            Some(DomainEvent::CooldownCleared {
                id: 9,
                channel_id: "channel-a".to_string(),
                credential_id: "cred-a".to_string(),
                reason: "manual reset".to_string(),
            })
        );
    }

    #[test]
    fn non_domain_management_event_is_not_replayed() {
        let event = ManagementEvent {
            id: 8,
            kind: "management_login".to_string(),
            channel_id: "channel-a".to_string(),
            credential_id: "cred-a".to_string(),
            reason: "audit only".to_string(),
            actor: None,
            ..Default::default()
        };

        assert_eq!(event.to_domain_event(), None);
    }

    #[test]
    fn routing_telemetry_buffer_drops_oldest_when_full() {
        let mut buffer = RoutingTelemetryBuffer::new(2);
        buffer.push(RoutingTelemetry::RouteSelected {
            request_id: "req-1".to_string(),
            registry_generation: 2,
            channel_id: "channel-a".to_string(),
        });
        buffer.push(RoutingTelemetry::UpstreamFailureObserved {
            request_id: "req-2".to_string(),
            channel_id: "channel-a".to_string(),
            failure: Box::new(test_failure_telemetry()),
        });
        buffer.push(RoutingTelemetry::TransitionApplied {
            request_id: "req-3".to_string(),
            channel_id: "channel-a".to_string(),
        });

        assert_eq!(
            buffer.snapshot(),
            vec![
                RoutingTelemetry::UpstreamFailureObserved {
                    request_id: "req-2".to_string(),
                    channel_id: "channel-a".to_string(),
                    failure: Box::new(test_failure_telemetry()),
                },
                RoutingTelemetry::TransitionApplied {
                    request_id: "req-3".to_string(),
                    channel_id: "channel-a".to_string(),
                },
            ]
        );
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.dropped_events(), 1);
    }

    #[test]
    fn routing_telemetry_zero_capacity_drops_everything() {
        let mut buffer = RoutingTelemetryBuffer::new(0);
        buffer.push(RoutingTelemetry::RouteSelected {
            request_id: "req-1".to_string(),
            registry_generation: 2,
            channel_id: "channel-a".to_string(),
        });

        assert!(buffer.snapshot().is_empty());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.dropped_events(), 1);
    }

    #[test]
    fn routing_telemetry_buffer_shrink_counts_removed_old_events() {
        let mut buffer = RoutingTelemetryBuffer::new(4);
        for request_id in ["req-1", "req-2", "req-3", "req-4"] {
            buffer.push(RoutingTelemetry::RouteSelected {
                request_id: request_id.to_string(),
                registry_generation: 2,
                channel_id: "channel-a".to_string(),
            });
        }

        buffer.replace_capacity(2);

        let events = buffer.snapshot();
        assert_eq!(buffer.capacity(), 2);
        assert_eq!(events.len(), 2);
        assert_eq!(
            events
                .iter()
                .map(|event| match event {
                    RoutingTelemetry::RouteSelected { request_id, .. } => request_id.as_str(),
                    _ => panic!("expected route_selected event"),
                })
                .collect::<Vec<_>>(),
            vec!["req-3", "req-4"]
        );
        assert_eq!(buffer.dropped_events(), 2);
    }

    #[test]
    fn routing_telemetry_buffer_can_record_external_dropped_event() {
        let mut buffer = RoutingTelemetryBuffer::new(2);

        buffer.record_dropped_event();

        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.dropped_events(), 1);
    }

    #[test]
    fn credential_transition_routing_telemetry_serializes_without_secret_material() {
        let event = RoutingTelemetry::CredentialTransitionApplied {
            request_id: "req-1".to_string(),
            channel_id: "channel-a".to_string(),
            credential_id: "cred-a".to_string(),
            state: "expired".to_string(),
            reason: "upstream_auth_invalid".to_string(),
        };

        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["kind"], "credential_transition_applied");
        assert_eq!(value["credential_id"], "cred-a");
        assert_eq!(value["state"], "expired");
        assert_eq!(value["reason"], "upstream_auth_invalid");
        assert!(!value.to_string().contains("sk-"));
    }

    #[test]
    fn upstream_failure_routing_telemetry_serializes_only_safe_metadata() {
        let event = RoutingTelemetry::UpstreamFailureObserved {
            request_id: "req-2".to_string(),
            channel_id: "channel-a".to_string(),
            failure: Box::new(test_failure_telemetry()),
        };

        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["kind"], "upstream_failure_observed");
        assert_eq!(value["failure"]["failure_kind"], "rate_limited");
        assert_eq!(value["failure"]["failure_scope"], "credential");
        assert_eq!(value["failure"]["retryable"], true);
        assert_eq!(value["failure"]["confidence"], "medium");
        assert_eq!(value["failure"]["status"], 429);
        assert_eq!(value["failure"]["classifier_id"], "test-classifier");
        assert_eq!(value["failure"]["classifier_version"], "1");
        assert_eq!(value["failure"]["adaptation_rule_id"], "rule-a");
        assert_eq!(value["failure"]["retry_after_source"], "delta_seconds");
        assert_eq!(value["failure"]["cooldown_seconds"], 30);
        assert_eq!(value["failure"]["public_model"], "gpt-test");
        assert_eq!(value["failure"]["credential_id_hash"], "hash-a");
        assert_eq!(value["failure"]["attempt"], 2);
        assert_eq!(value["failure"]["failure_source"], "upstream_transaction");
        assert_eq!(value["failure"]["directive"], "retry_credential");
        assert!(value["failure"]["denial_reason"].is_null());
        assert_eq!(value["failure"]["duplicate_charge_risk"], "unknown");
        assert_eq!(value["failure"]["effective_deadline_remaining_ms"], 42);
        assert_eq!(value["failure"]["retry_pressure_accounted"], true);
        assert_eq!(value["failure"]["retry_decision"], "retry_credential");
        assert!(value["failure"]["retry_decision_reason"].is_null());
        assert!(value.get("credential_id").is_none());
        assert!(value["failure"].get("upstream_code").is_none());
        assert!(value["failure"].get("upstream_limit_type").is_none());
        assert!(!value.to_string().contains("sk-"));
    }

    #[test]
    fn routing_telemetry_buffer_counts_retry_pressure_from_bounded_window() {
        let mut buffer = RoutingTelemetryBuffer::new(1);
        buffer.push(RoutingTelemetry::UpstreamFailureObserved {
            request_id: "req-1".to_string(),
            channel_id: "channel-a".to_string(),
            failure: Box::new(test_failure_telemetry()),
        });
        let mut terminal = test_failure_telemetry();
        terminal.directive = "return_error".to_string();
        terminal.retry_decision = "return_current_error".to_string();
        terminal.denial_reason = Some("attempt_limit_reached".to_string());
        terminal.retry_decision_reason = Some("attempt_limit_reached".to_string());
        terminal.duplicate_charge_risk = "none".to_string();
        buffer.push(RoutingTelemetry::UpstreamFailureObserved {
            request_id: "req-2".to_string(),
            channel_id: "channel-a".to_string(),
            failure: Box::new(terminal),
        });

        let counters = buffer.retry_pressure_snapshot();
        assert_eq!(buffer.snapshot().len(), 1);
        assert_eq!(buffer.dropped_events(), 1);
        assert_eq!(counters.by_directive.retry_credential, 0);
        assert_eq!(counters.by_directive.return_error, 1);
        assert_eq!(
            counters.by_denial_reason.get("attempt_limit_reached"),
            Some(&1)
        );
        assert_eq!(counters.by_duplicate_charge_risk.unknown, 0);
        assert_eq!(counters.by_duplicate_charge_risk.none, 1);
    }

    #[test]
    fn routing_telemetry_buffer_skips_unaccounted_retry_pressure_events() {
        let mut buffer = RoutingTelemetryBuffer::new(2);
        let mut unaccounted = test_failure_telemetry();
        unaccounted.failure_source = "local_transport".to_string();
        unaccounted.directive = "return_error".to_string();
        unaccounted.retry_decision = "return_current_error".to_string();
        unaccounted.denial_reason = Some("partial_output_started".to_string());
        unaccounted.retry_decision_reason = Some("partial_output_started".to_string());
        unaccounted.retry_pressure_accounted = false;

        buffer.push(RoutingTelemetry::UpstreamFailureObserved {
            request_id: "req-1".to_string(),
            channel_id: "channel-a".to_string(),
            failure: Box::new(unaccounted),
        });

        let events = buffer.snapshot();
        let counters = buffer.retry_pressure_snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(counters.by_directive.return_error, 0);
        assert_eq!(
            counters.by_denial_reason.get("partial_output_started"),
            None
        );
        assert_eq!(counters.by_duplicate_charge_risk.none, 0);
    }

    #[test]
    fn response_filter_event_buffer_assigns_monotonic_ids_and_drops_oldest() {
        let mut buffer = ResponseFilterEventBuffer::new(2);

        buffer.push(test_response_filter_event_input("req-1", "rule-a"));
        buffer.push(test_response_filter_event_input("req-2", "rule-b"));
        buffer.push(test_response_filter_event_input("req-3", "rule-c"));

        let events = buffer.snapshot();
        assert_eq!(buffer.len(), 2);
        assert_eq!(events[0].event_id, 2);
        assert_eq!(events[0].request_id, "req-2");
        assert_eq!(events[1].event_id, 3);
        assert_eq!(events[1].rule_id, "rule-c");
        assert!(events.iter().all(|event| event.created_at_unix_seconds > 0));
        assert_eq!(buffer.dropped_events(), 1);
    }

    #[test]
    fn response_filter_event_buffer_zero_capacity_counts_dropped_events() {
        let mut buffer = ResponseFilterEventBuffer::new(0);

        let pushed = buffer.push(test_response_filter_event_input("req-1", "rule-a"));

        assert_eq!(pushed, None);
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.snapshot(), Vec::new());
        assert_eq!(buffer.dropped_events(), 1);
    }

    #[test]
    fn response_filter_event_serializes_only_safe_metadata() {
        let mut buffer = ResponseFilterEventBuffer::new(1);
        let event = buffer
            .push(test_response_filter_event_input(
                "req-safe",
                "blocked-marker",
            ))
            .unwrap();

        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["event_id"], 1);
        assert_eq!(value["request_id"], "req-safe");
        assert_eq!(value["channel_id"], "channel-a");
        assert_eq!(value["public_model"], "gpt-test");
        assert_eq!(value["rule_id"], "blocked-marker");
        assert_eq!(value["action"], "reject");
        assert_eq!(value["content_kind"], "json");
        assert_eq!(value["reason_code"], "rule_matched");
        assert_eq!(value["outcome"], "rejected");
        assert_eq!(value["body_committed"], false);
        let serialized = value.to_string();
        assert!(!serialized.contains("sk-"));
        assert!(!serialized.contains("unsafe-marker"));
        assert!(!serialized.contains("/tmp/"));
    }

    #[test]
    fn response_filter_event_buffer_can_replace_capacity_without_resetting_ids() {
        let mut buffer = ResponseFilterEventBuffer::new(3);
        buffer.push(test_response_filter_event_input("req-1", "rule-a"));
        buffer.push(test_response_filter_event_input("req-2", "rule-b"));
        buffer.push(test_response_filter_event_input("req-3", "rule-c"));

        buffer.replace_capacity(1);
        let event = buffer
            .push(test_response_filter_event_input("req-4", "rule-d"))
            .unwrap();

        assert_eq!(event.event_id, 4);
        assert_eq!(buffer.capacity(), 1);
        assert_eq!(buffer.snapshot()[0].request_id, "req-4");
        assert_eq!(buffer.dropped_events(), 3);
    }

    #[test]
    fn response_filter_event_buffer_shrink_counts_removed_old_events() {
        let mut buffer = ResponseFilterEventBuffer::new(4);
        buffer.push(test_response_filter_event_input("req-1", "rule-a"));
        buffer.push(test_response_filter_event_input("req-2", "rule-b"));
        buffer.push(test_response_filter_event_input("req-3", "rule-c"));
        buffer.push(test_response_filter_event_input("req-4", "rule-d"));

        buffer.replace_capacity(2);

        let events = buffer.snapshot();
        assert_eq!(buffer.capacity(), 2);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].request_id, "req-3");
        assert_eq!(events[1].request_id, "req-4");
        assert_eq!(buffer.dropped_events(), 2);
    }

    #[test]
    fn response_filter_event_buffer_can_record_external_dropped_event() {
        let mut buffer = ResponseFilterEventBuffer::new(2);

        buffer.record_dropped_event();

        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.dropped_events(), 1);
    }

    #[test]
    fn lifecycle_persistence_drop_routing_telemetry_serializes_without_secret_material() {
        let event = RoutingTelemetry::CredentialLifecyclePersistenceDropped {
            request_id: "req-1".to_string(),
            channel_id: "channel-a".to_string(),
            credential_id: "cred-a".to_string(),
            state: "expired".to_string(),
            reason: "upstream_auth_invalid".to_string(),
            drop_reason: "queue_unavailable".to_string(),
        };

        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["kind"], "credential_lifecycle_persistence_dropped");
        assert_eq!(value["credential_id"], "cred-a");
        assert_eq!(value["state"], "expired");
        assert_eq!(value["reason"], "upstream_auth_invalid");
        assert_eq!(value["drop_reason"], "queue_unavailable");
        assert!(!value.to_string().contains("sk-"));
    }

    fn test_failure_telemetry() -> UpstreamFailureTelemetry {
        UpstreamFailureTelemetry {
            public_model: Some("gpt-test".to_string()),
            credential_id_hash: "hash-a".to_string(),
            attempt: 2,
            failure_source: "upstream_transaction".to_string(),
            failure_kind: "rate_limited".to_string(),
            failure_scope: "credential".to_string(),
            retryable: true,
            confidence: "medium".to_string(),
            status: Some(429),
            classifier_id: "test-classifier".to_string(),
            classifier_version: "1".to_string(),
            adaptation_rule_id: Some("rule-a".to_string()),
            retry_after_source: Some("delta_seconds".to_string()),
            cooldown_seconds: Some(30),
            directive: "retry_credential".to_string(),
            denial_reason: None,
            duplicate_charge_risk: "unknown".to_string(),
            effective_deadline_remaining_ms: Some(42),
            retry_pressure_accounted: true,
            retry_decision: "retry_credential".to_string(),
            retry_decision_reason: None,
        }
    }

    fn test_response_filter_event_input(
        request_id: &str,
        rule_id: &str,
    ) -> ResponseFilterEventInput {
        ResponseFilterEventInput {
            request_id: request_id.to_string(),
            channel_id: "channel-a".to_string(),
            public_model: "gpt-test".to_string(),
            rule_id: rule_id.to_string(),
            action: "reject".to_string(),
            content_kind: "json".to_string(),
            reason_code: "rule_matched".to_string(),
            outcome: "rejected".to_string(),
            body_committed: false,
        }
    }
}
