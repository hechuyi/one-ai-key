use crate::{
    credentials::{CredentialFingerprint, CredentialId},
    error::{ClassifiedFailure, FailureKind, FailureScope},
    provider::{EndpointKind, ProviderKind},
    state::ChannelId,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSelectionSnapshot {
    pub request_id: String,
    pub config_generation: u64,
    pub channel_health_generation: u64,
    pub client_token_id: String,
    pub requested_model: Option<String>,
    pub endpoint: EndpointKind,
    pub route_target_index: Option<usize>,
    pub channel_id: ChannelId,
    pub provider_id: String,
    pub account_id: String,
    pub provider_kind: ProviderKind,
    pub credential_id: CredentialId,
    pub credential_fingerprint: CredentialFingerprint,
    pub classifier_id: String,
    pub classifier_version: String,
    pub retry_candidates: Vec<CredentialId>,
    pub body_replayable: bool,
    pub streaming: bool,
    pub partial_output_started: bool,
    pub route_target_available: bool,
    pub effective_deadline: Option<Instant>,
    pub attempt: usize,
    pub selection_reason: SelectionReason,
}

#[cfg(test)]
static TEST_TRANSITION_SNAPSHOTS: std::sync::Mutex<Vec<RequestSelectionSnapshot>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
pub(crate) fn clear_test_transition_snapshots() {
    TEST_TRANSITION_SNAPSHOTS
        .lock()
        .expect("test transition snapshot mutex poisoned")
        .clear();
}

#[cfg(test)]
pub(crate) fn take_test_transition_snapshots() -> Vec<RequestSelectionSnapshot> {
    std::mem::take(
        &mut *TEST_TRANSITION_SNAPSHOTS
            .lock()
            .expect("test transition snapshot mutex poisoned"),
    )
}

#[cfg(test)]
fn record_test_transition_snapshot(snapshot: &RequestSelectionSnapshot) {
    TEST_TRANSITION_SNAPSHOTS
        .lock()
        .expect("test transition snapshot mutex poisoned")
        .push(snapshot.clone());
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenRetryCandidates {
    candidates: Vec<CredentialId>,
    next_index: usize,
}

impl FrozenRetryCandidates {
    pub fn new(candidates: Vec<CredentialId>) -> Self {
        Self {
            candidates,
            next_index: 0,
        }
    }

    pub fn remaining(&self) -> Vec<CredentialId> {
        self.candidates
            .iter()
            .skip(self.next_index)
            .cloned()
            .collect()
    }

    pub fn advance_if_next(&mut self, credential_id: &CredentialId) -> bool {
        if self
            .candidates
            .get(self.next_index)
            .is_some_and(|candidate| candidate == credential_id)
        {
            self.next_index += 1;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionReason {
    DefaultPool,
    ModelMapping,
    NamedChannel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDirective {
    ReturnCurrentError { reason: RetryDecisionReason },
    RetryCredential { credential_id: CredentialId },
    RetryRouteTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryAttemptContinuation {
    RetryCredential { credential_id: CredentialId },
    RetryRouteTarget,
    ReturnCurrentError { reason: RetryDecisionReason },
    FrozenCandidateDrift { credential_id: CredentialId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecisionReason {
    PolicyDisabled,
    FailureNotRetryable,
    BodyNotReplayable,
    StreamingNotRetryable,
    PartialOutputStarted,
    AttemptLimitReached,
    NoFrozenCandidate,
    EffectiveDeadlineExhausted,
    RouteTargetRetryDisabled,
    NoRouteCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    UpstreamAuthInvalid,
    UpstreamRateLimited,
    UpstreamQuotaExhausted,
    RelayBalanceUnavailable,
    UpstreamProviderUnavailable,
    KeySwitchCooldown,
    ClientOrModelError,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureSource {
    UpstreamTransaction,
    LocalTransport,
    GuardedSuccessEnvelope,
}

pub const FAILURE_SOURCE_TELEMETRY_CONTRACT: [(FailureSource, &str); 3] = [
    (FailureSource::UpstreamTransaction, "upstream_transaction"),
    (FailureSource::LocalTransport, "local_transport"),
    (
        FailureSource::GuardedSuccessEnvelope,
        "guarded_success_envelope",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateChargeRisk {
    None,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateMutation {
    Noop {
        reason: FailureReason,
    },
    MarkCredentialCoolingDown {
        channel_id: ChannelId,
        credential_id: CredentialId,
        until: Instant,
        reason: FailureReason,
    },
    ExpireCredential {
        channel_id: ChannelId,
        credential_id: CredentialId,
        reason: FailureReason,
    },
    MarkCredentialQuotaExhausted {
        channel_id: ChannelId,
        credential_id: CredentialId,
        reason: FailureReason,
    },
    MarkRelayBalanceChannelCoolingDown {
        channel_id: ChannelId,
        until: Instant,
        reason: FailureReason,
    },
    MarkProviderAccountChannelCoolingDownOrDegraded {
        channel_id: ChannelId,
        provider_id: String,
        account_id: String,
        until: Option<Instant>,
        reason: FailureReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionInput<'a> {
    pub snapshot: &'a RequestSelectionSnapshot,
    pub failure: ClassifiedFailure,
    pub failure_source: FailureSource,
    pub now: Instant,
    pub next_attempt_budget: Option<Duration>,
    pub policy: RoutingPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionResult {
    pub mutation: StateMutation,
    pub retry: RetryDirective,
    pub duplicate_charge_risk: DuplicateChargeRisk,
    pub effective_deadline_remaining_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingPolicy {
    pub retry_switched_key_in_same_request: bool,
    pub max_same_request_retries: usize,
    pub route_target_retry_enabled: bool,
    pub default_credential_cooldown: Duration,
}

pub fn apply_retry_directive_to_attempt_state(
    directive: RetryDirective,
    frozen_retry_candidates: &mut Option<FrozenRetryCandidates>,
    attempt: &mut usize,
) -> RetryAttemptContinuation {
    match directive {
        RetryDirective::RetryCredential { credential_id } => {
            if frozen_retry_candidates
                .as_mut()
                .is_some_and(|frozen| frozen.advance_if_next(&credential_id))
            {
                *attempt += 1;
                RetryAttemptContinuation::RetryCredential { credential_id }
            } else {
                RetryAttemptContinuation::FrozenCandidateDrift { credential_id }
            }
        }
        RetryDirective::RetryRouteTarget => RetryAttemptContinuation::RetryRouteTarget,
        RetryDirective::ReturnCurrentError { reason } => {
            RetryAttemptContinuation::ReturnCurrentError { reason }
        }
    }
}

pub fn transition_after_failure(input: TransitionInput<'_>) -> TransitionResult {
    #[cfg(test)]
    record_test_transition_snapshot(input.snapshot);

    let mutation = match (input.failure.kind, input.failure.primary_scope) {
        (FailureKind::AuthInvalid, FailureScope::Credential) => StateMutation::ExpireCredential {
            channel_id: input.snapshot.channel_id.clone(),
            credential_id: input.snapshot.credential_id.clone(),
            reason: FailureReason::UpstreamAuthInvalid,
        },
        (FailureKind::RateLimited, FailureScope::Credential) => {
            StateMutation::MarkCredentialCoolingDown {
                channel_id: input.snapshot.channel_id.clone(),
                credential_id: input.snapshot.credential_id.clone(),
                until: input.now
                    + input
                        .failure
                        .cooldown
                        .unwrap_or(input.policy.default_credential_cooldown),
                reason: FailureReason::UpstreamRateLimited,
            }
        }
        (FailureKind::QuotaExhausted, FailureScope::Credential) => {
            StateMutation::MarkCredentialQuotaExhausted {
                channel_id: input.snapshot.channel_id.clone(),
                credential_id: input.snapshot.credential_id.clone(),
                reason: FailureReason::UpstreamQuotaExhausted,
            }
        }
        (FailureKind::RelayBalanceUnavailable, FailureScope::Channel)
            if input.failure_source == FailureSource::UpstreamTransaction =>
        {
            StateMutation::MarkRelayBalanceChannelCoolingDown {
                channel_id: input.snapshot.channel_id.clone(),
                until: input.now
                    + input
                        .failure
                        .cooldown
                        .unwrap_or(input.policy.default_credential_cooldown),
                reason: FailureReason::RelayBalanceUnavailable,
            }
        }
        (FailureKind::RelayBalanceUnavailable, FailureScope::Channel) => StateMutation::Noop {
            reason: FailureReason::RelayBalanceUnavailable,
        },
        (FailureKind::ProviderUnavailable, FailureScope::Channel) => {
            StateMutation::MarkProviderAccountChannelCoolingDownOrDegraded {
                channel_id: input.snapshot.channel_id.clone(),
                provider_id: input.snapshot.provider_id.clone(),
                account_id: input.snapshot.account_id.clone(),
                until: input.failure.cooldown.map(|cooldown| input.now + cooldown),
                reason: FailureReason::UpstreamProviderUnavailable,
            }
        }
        (FailureKind::KeySwitchCooldown, _) => StateMutation::Noop {
            reason: FailureReason::KeySwitchCooldown,
        },
        (FailureKind::ClientError, FailureScope::ModelGroup | FailureScope::RequestOnly) => {
            StateMutation::Noop {
                reason: FailureReason::ClientOrModelError,
            }
        }
        _ => StateMutation::Noop {
            reason: FailureReason::Unknown,
        },
    };

    let effective_deadline_remaining = input
        .snapshot
        .effective_deadline
        .map(|deadline| deadline.saturating_duration_since(input.now));
    let effective_deadline_remaining_ms = effective_deadline_remaining
        .map(|remaining| remaining.as_millis().min(u128::from(u64::MAX)) as u64);
    let effective_deadline_exhausted = effective_deadline_remaining.is_some_and(|remaining| {
        remaining.is_zero()
            || input
                .next_attempt_budget
                .is_some_and(|budget| remaining < budget)
    });
    let retry = if rejects_source_gated_channel_balance(&input) || !input.failure.retryable {
        RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::FailureNotRetryable,
        }
    } else if !input.snapshot.body_replayable {
        RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::BodyNotReplayable,
        }
    } else if input.snapshot.streaming {
        RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::StreamingNotRetryable,
        }
    } else if input.snapshot.partial_output_started {
        RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::PartialOutputStarted,
        }
    } else if effective_deadline_exhausted {
        RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::EffectiveDeadlineExhausted,
        }
    } else if matches!(input.failure.primary_scope, FailureScope::ProviderAdapter) {
        RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::FailureNotRetryable,
        }
    } else if matches!(input.failure.primary_scope, FailureScope::Channel) {
        if !input.policy.route_target_retry_enabled {
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::RouteTargetRetryDisabled,
            }
        } else if !input.snapshot.route_target_available {
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::NoRouteCandidate,
            }
        } else {
            RetryDirective::RetryRouteTarget
        }
    } else if input.snapshot.attempt >= input.policy.max_same_request_retries {
        if credential_retry_exhausted_route_fallback_allowed(&input) {
            RetryDirective::RetryRouteTarget
        } else {
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::AttemptLimitReached,
            }
        }
    } else if !input.policy.retry_switched_key_in_same_request {
        credential_retry_exhausted_directive(&input, RetryDecisionReason::PolicyDisabled)
    } else if matches!(input.failure.primary_scope, FailureScope::Credential) {
        if let Some(credential_id) = input.snapshot.retry_candidates.first() {
            if credential_id == &input.snapshot.credential_id {
                credential_retry_exhausted_directive(&input, RetryDecisionReason::NoFrozenCandidate)
            } else {
                RetryDirective::RetryCredential {
                    credential_id: credential_id.clone(),
                }
            }
        } else {
            credential_retry_exhausted_directive(&input, RetryDecisionReason::NoFrozenCandidate)
        }
    } else {
        RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::FailureNotRetryable,
        }
    };

    TransitionResult {
        mutation,
        duplicate_charge_risk: duplicate_charge_risk(input.failure_source, &retry),
        retry,
        effective_deadline_remaining_ms,
    }
}

fn credential_retry_exhausted_directive(
    input: &TransitionInput<'_>,
    terminal_reason: RetryDecisionReason,
) -> RetryDirective {
    if credential_retry_exhausted_route_fallback_allowed(input) {
        RetryDirective::RetryRouteTarget
    } else {
        RetryDirective::ReturnCurrentError {
            reason: terminal_reason,
        }
    }
}

fn credential_retry_exhausted_route_fallback_allowed(input: &TransitionInput<'_>) -> bool {
    matches!(input.failure.primary_scope, FailureScope::Credential)
        && input.policy.route_target_retry_enabled
        && input.snapshot.route_target_available
}

fn duplicate_charge_risk(
    failure_source: FailureSource,
    retry: &RetryDirective,
) -> DuplicateChargeRisk {
    match (failure_source, retry) {
        (_, RetryDirective::ReturnCurrentError { .. }) => DuplicateChargeRisk::None,
        (FailureSource::UpstreamTransaction, _) => DuplicateChargeRisk::Unknown,
        (FailureSource::LocalTransport, _) => DuplicateChargeRisk::None,
        (FailureSource::GuardedSuccessEnvelope, _) => DuplicateChargeRisk::Unknown,
    }
}

fn rejects_source_gated_channel_balance(input: &TransitionInput<'_>) -> bool {
    matches!(
        (input.failure.kind, input.failure.primary_scope),
        (FailureKind::RelayBalanceUnavailable, FailureScope::Channel)
    ) && input.failure_source != FailureSource::UpstreamTransaction
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        credentials::CredentialSource,
        error::{FailureConfidence, FailureKind, FailureScope},
        pool::{KeyPool, KeyPoolConfig, PoolCredentialInput},
    };

    fn failure(kind: FailureKind, scope: FailureScope) -> ClassifiedFailure {
        ClassifiedFailure {
            kind,
            primary_scope: scope,
            retryable: false,
            cooldown: None,
            retry_after_source: None,
            confidence: FailureConfidence::High,
            upstream_status: None,
            upstream_code: None,
            upstream_limit_type: None,
            classifier_id: "test".to_string(),
            classifier_version: "1".to_string(),
            adaptation_rule_id: None,
        }
    }

    fn failure_with_cooldown(
        kind: FailureKind,
        scope: FailureScope,
        cooldown: std::time::Duration,
    ) -> ClassifiedFailure {
        ClassifiedFailure {
            cooldown: Some(cooldown),
            ..failure(kind, scope)
        }
    }

    fn retryable_failure(kind: FailureKind, scope: FailureScope) -> ClassifiedFailure {
        ClassifiedFailure {
            retryable: true,
            ..failure(kind, scope)
        }
    }

    fn upstream_input<'a>(
        snapshot: &'a RequestSelectionSnapshot,
        failure: ClassifiedFailure,
        now: std::time::Instant,
        policy: RoutingPolicy,
    ) -> TransitionInput<'a> {
        TransitionInput {
            snapshot,
            failure,
            failure_source: FailureSource::UpstreamTransaction,
            now,
            next_attempt_budget: Some(std::time::Duration::from_millis(50)),
            policy,
        }
    }

    fn imported(secret: &str) -> PoolCredentialInput {
        PoolCredentialInput {
            secret: secret.to_string(),
            source: CredentialSource::unknown(),
        }
    }

    fn pool() -> KeyPool {
        KeyPool::new(KeyPoolConfig {
            name: "test".to_string(),
            credential_namespace: "test".to_string(),
            api_base: "https://example.com/v1".to_string(),
            credentials: vec![imported("k1"), imported("k2")],
        })
        .unwrap()
    }

    fn test_config_generation() -> u64 {
        1
    }

    #[test]
    fn snapshot_carries_channel_and_credential_identity() {
        let mut pool = pool();
        let selected = pool.select().unwrap();

        let snapshot = RequestSelectionSnapshot {
            request_id: "req_test".to_string(),
            config_generation: test_config_generation(),
            channel_health_generation: 1,
            client_token_id: "client_test".to_string(),
            requested_model: Some("gpt-test".to_string()),
            endpoint: EndpointKind::ChatCompletions,
            route_target_index: None,
            channel_id: crate::state::ChannelId("test".to_string()),
            provider_id: "provider:test".to_string(),
            account_id: "account:test".to_string(),
            provider_kind: ProviderKind::OpenAiCompatible,
            credential_id: selected.credential_id.clone(),
            credential_fingerprint: selected.credential_fingerprint.clone(),
            classifier_id: "openai-compatible-default".to_string(),
            classifier_version: "1".to_string(),
            retry_candidates: Vec::new(),
            body_replayable: true,
            streaming: false,
            partial_output_started: false,
            route_target_available: true,
            effective_deadline: None,
            attempt: 0,
            selection_reason: SelectionReason::DefaultPool,
        };

        assert_eq!(
            snapshot.channel_id,
            crate::state::ChannelId("test".to_string())
        );
        assert_eq!(snapshot.credential_id, selected.credential_id);
        assert_eq!(
            snapshot.credential_fingerprint,
            selected.credential_fingerprint
        );
        assert_eq!(snapshot.config_generation, test_config_generation());
        assert_eq!(snapshot.classifier_id, "openai-compatible-default");
        assert_eq!(snapshot.classifier_version, "1");
        assert_eq!(snapshot.route_target_index, None);
    }

    fn snapshot_for(selected: &crate::pool::SelectedKey) -> RequestSelectionSnapshot {
        RequestSelectionSnapshot {
            request_id: "req_test".to_string(),
            config_generation: test_config_generation(),
            channel_health_generation: 1,
            client_token_id: "client_test".to_string(),
            requested_model: Some("gpt-test".to_string()),
            endpoint: EndpointKind::ChatCompletions,
            route_target_index: None,
            channel_id: crate::state::ChannelId("test".to_string()),
            provider_id: "provider:test".to_string(),
            account_id: "account:test".to_string(),
            provider_kind: ProviderKind::OpenAiCompatible,
            credential_id: selected.credential_id.clone(),
            credential_fingerprint: selected.credential_fingerprint.clone(),
            classifier_id: "openai-compatible-default".to_string(),
            classifier_version: "1".to_string(),
            retry_candidates: Vec::new(),
            body_replayable: true,
            streaming: false,
            partial_output_started: false,
            route_target_available: true,
            effective_deadline: None,
            attempt: 0,
            selection_reason: SelectionReason::DefaultPool,
        }
    }

    fn retry_snapshot_for(
        selected: &crate::pool::SelectedKey,
        retry_candidate: CredentialId,
    ) -> RequestSelectionSnapshot {
        let mut snapshot = snapshot_for(selected);
        snapshot.retry_candidates = vec![retry_candidate];
        snapshot
    }

    #[test]
    fn frozen_retry_candidates_expose_only_unconsumed_candidates() {
        let first = CredentialId("cred_1".to_string());
        let second = CredentialId("cred_2".to_string());
        let mut frozen = FrozenRetryCandidates::new(vec![first.clone(), second.clone()]);

        assert_eq!(frozen.remaining(), vec![first.clone(), second.clone()]);
        assert!(frozen.advance_if_next(&first));
        assert_eq!(frozen.remaining(), vec![second]);
    }

    #[test]
    fn frozen_retry_candidates_reject_out_of_order_candidate() {
        let first = CredentialId("cred_1".to_string());
        let second = CredentialId("cred_2".to_string());
        let mut frozen = FrozenRetryCandidates::new(vec![first.clone(), second.clone()]);

        assert!(!frozen.advance_if_next(&second));
        assert_eq!(frozen.remaining(), vec![first, second]);
    }

    #[test]
    fn retry_attempt_continuation_advances_frozen_candidate_and_attempt() {
        let first = CredentialId("cred_1".to_string());
        let second = CredentialId("cred_2".to_string());
        let mut frozen = Some(FrozenRetryCandidates::new(vec![
            first.clone(),
            second.clone(),
        ]));
        let mut attempt = 0;

        let continuation = apply_retry_directive_to_attempt_state(
            RetryDirective::RetryCredential {
                credential_id: first.clone(),
            },
            &mut frozen,
            &mut attempt,
        );

        assert_eq!(
            continuation,
            RetryAttemptContinuation::RetryCredential {
                credential_id: first
            }
        );
        assert_eq!(attempt, 1);
        assert_eq!(frozen.unwrap().remaining(), vec![second]);
    }

    #[test]
    fn retry_attempt_continuation_reports_frozen_candidate_drift_without_advancing_attempt() {
        let first = CredentialId("cred_1".to_string());
        let second = CredentialId("cred_2".to_string());
        let mut frozen = Some(FrozenRetryCandidates::new(vec![first.clone()]));
        let mut attempt = 0;

        let continuation = apply_retry_directive_to_attempt_state(
            RetryDirective::RetryCredential {
                credential_id: second.clone(),
            },
            &mut frozen,
            &mut attempt,
        );

        assert_eq!(
            continuation,
            RetryAttemptContinuation::FrozenCandidateDrift {
                credential_id: second
            }
        );
        assert_eq!(attempt, 0);
        assert_eq!(frozen.unwrap().remaining(), vec![first]);
    }

    #[test]
    fn retry_attempt_continuation_preserves_route_target_and_terminal_directives() {
        let mut frozen = Some(FrozenRetryCandidates::new(vec![CredentialId(
            "cred_1".to_string(),
        )]));
        let mut attempt = 0;

        assert_eq!(
            apply_retry_directive_to_attempt_state(
                RetryDirective::RetryRouteTarget,
                &mut frozen,
                &mut attempt,
            ),
            RetryAttemptContinuation::RetryRouteTarget
        );
        assert_eq!(attempt, 0);
        assert_eq!(
            apply_retry_directive_to_attempt_state(
                RetryDirective::ReturnCurrentError {
                    reason: RetryDecisionReason::PolicyDisabled,
                },
                &mut frozen,
                &mut attempt,
            ),
            RetryAttemptContinuation::ReturnCurrentError {
                reason: RetryDecisionReason::PolicyDisabled,
            }
        );
        assert_eq!(attempt, 0);
    }

    fn policy() -> RoutingPolicy {
        RoutingPolicy {
            retry_switched_key_in_same_request: false,
            max_same_request_retries: 0,
            route_target_retry_enabled: true,
            default_credential_cooldown: std::time::Duration::from_secs(20),
        }
    }

    fn retry_policy() -> RoutingPolicy {
        RoutingPolicy {
            retry_switched_key_in_same_request: true,
            max_same_request_retries: 1,
            route_target_retry_enabled: true,
            default_credential_cooldown: std::time::Duration::from_secs(20),
        }
    }

    fn apply_state_mutation_for_test(
        pool: &mut KeyPool,
        mutation: StateMutation,
    ) -> crate::pool::SwitchOutcome {
        match mutation {
            StateMutation::MarkCredentialCoolingDown {
                credential_id,
                until,
                reason,
                ..
            } => pool.apply_credential_cooldown_until(&credential_id, until, format!("{reason:?}")),
            StateMutation::ExpireCredential {
                credential_id,
                reason,
                ..
            } => pool.apply_credential_expired(&credential_id, format!("{reason:?}")),
            StateMutation::MarkCredentialQuotaExhausted {
                credential_id,
                reason,
                ..
            } => pool.apply_credential_quota_exhausted(&credential_id, format!("{reason:?}")),
            StateMutation::Noop { .. }
            | StateMutation::MarkRelayBalanceChannelCoolingDown { .. }
            | StateMutation::MarkProviderAccountChannelCoolingDownOrDegraded { .. } => {
                crate::pool::SwitchOutcome::StaleFailure
            }
        }
    }

    #[test]
    fn auth_invalid_credential_expires_only_selected_credential() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let now = std::time::Instant::now();

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure(FailureKind::AuthInvalid, FailureScope::Credential),
            now,
            policy(),
        ));

        assert_eq!(
            result.mutation,
            StateMutation::ExpireCredential {
                channel_id: snapshot.channel_id.clone(),
                credential_id: selected.credential_id,
                reason: FailureReason::UpstreamAuthInvalid,
            }
        );
        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::FailureNotRetryable
            }
        );
        assert_eq!(pool.select().unwrap().key, "k1");
    }

    #[test]
    fn rate_limited_credential_uses_retry_after_cooldown() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let cooldown = std::time::Duration::from_secs(7);
        let now = std::time::Instant::now();

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure_with_cooldown(FailureKind::RateLimited, FailureScope::Credential, cooldown),
            now,
            policy(),
        ));

        assert_eq!(
            result.mutation,
            StateMutation::MarkCredentialCoolingDown {
                channel_id: snapshot.channel_id.clone(),
                credential_id: selected.credential_id,
                until: now + cooldown,
                reason: FailureReason::UpstreamRateLimited,
            }
        );
        assert_eq!(pool.select().unwrap().key, "k1");
    }

    #[test]
    fn rate_limited_credential_uses_retry_after_until() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let cooldown = std::time::Duration::from_secs(7);
        let now = std::time::Instant::now();

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure_with_cooldown(FailureKind::RateLimited, FailureScope::Credential, cooldown),
            now,
            policy(),
        ));

        assert_eq!(
            result.mutation,
            StateMutation::MarkCredentialCoolingDown {
                channel_id: snapshot.channel_id.clone(),
                credential_id: selected.credential_id,
                until: now + cooldown,
                reason: FailureReason::UpstreamRateLimited,
            }
        );
    }

    #[test]
    fn rate_limited_credential_uses_policy_default_until() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let now = std::time::Instant::now();
        let policy = RoutingPolicy {
            retry_switched_key_in_same_request: false,
            max_same_request_retries: 0,
            route_target_retry_enabled: true,
            default_credential_cooldown: std::time::Duration::from_secs(11),
        };

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure(FailureKind::RateLimited, FailureScope::Credential),
            now,
            policy,
        ));

        assert_eq!(
            result.mutation,
            StateMutation::MarkCredentialCoolingDown {
                channel_id: snapshot.channel_id.clone(),
                credential_id: selected.credential_id,
                until: now + std::time::Duration::from_secs(11),
                reason: FailureReason::UpstreamRateLimited,
            }
        );
    }

    #[test]
    fn quota_exhausted_credential_marks_quota_exhausted() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let now = std::time::Instant::now();

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure(FailureKind::QuotaExhausted, FailureScope::Credential),
            now,
            policy(),
        ));

        assert_eq!(
            result.mutation,
            StateMutation::MarkCredentialQuotaExhausted {
                channel_id: snapshot.channel_id.clone(),
                credential_id: selected.credential_id,
                reason: FailureReason::UpstreamQuotaExhausted,
            }
        );
        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::FailureNotRetryable
            }
        );
    }

    #[test]
    fn relay_balance_unavailable_channel_marks_selected_channel_cooling_down() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let now = std::time::Instant::now();
        let cooldown = std::time::Duration::from_secs(13);

        let result = transition_after_failure(upstream_input(
            &snapshot,
            ClassifiedFailure {
                retryable: true,
                cooldown: Some(cooldown),
                ..failure(FailureKind::RelayBalanceUnavailable, FailureScope::Channel)
            },
            now,
            policy(),
        ));

        assert_eq!(
            result.mutation,
            StateMutation::MarkRelayBalanceChannelCoolingDown {
                channel_id: snapshot.channel_id.clone(),
                until: now + cooldown,
                reason: FailureReason::RelayBalanceUnavailable,
            }
        );
        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(pool.snapshot().quota_exhausted_credentials, 0);
    }

    #[test]
    fn relay_balance_unavailable_channel_from_non_upstream_source_is_rejected() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);

        let result = transition_after_failure(TransitionInput {
            snapshot: &snapshot,
            failure: ClassifiedFailure {
                retryable: true,
                ..failure(FailureKind::RelayBalanceUnavailable, FailureScope::Channel)
            },
            failure_source: FailureSource::LocalTransport,
            now: std::time::Instant::now(),
            next_attempt_budget: Some(std::time::Duration::from_millis(50)),
            policy: policy(),
        });

        assert_eq!(
            result.mutation,
            StateMutation::Noop {
                reason: FailureReason::RelayBalanceUnavailable
            }
        );
        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::FailureNotRetryable
            }
        );
    }

    #[test]
    fn provider_unavailable_channel_does_not_expire_credential() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::ProviderUnavailable, FailureScope::Channel),
            std::time::Instant::now(),
            policy(),
        ));

        assert_eq!(
            result.mutation,
            StateMutation::MarkProviderAccountChannelCoolingDownOrDegraded {
                channel_id: snapshot.channel_id.clone(),
                provider_id: snapshot.provider_id.clone(),
                account_id: snapshot.account_id.clone(),
                until: None,
                reason: FailureReason::UpstreamProviderUnavailable,
            }
        );
        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(pool.snapshot().expired_credentials, 0);
    }

    #[test]
    fn provider_adapter_failure_is_terminal_noop_even_when_retryable() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(
                FailureKind::ProviderUnavailable,
                FailureScope::ProviderAdapter,
            ),
            std::time::Instant::now(),
            policy(),
        ));

        assert_eq!(
            result.mutation,
            StateMutation::Noop {
                reason: FailureReason::Unknown,
            }
        );
        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::FailureNotRetryable,
            }
        );
    }

    #[test]
    fn rate_limited_credential_does_not_emit_failure_domain_mutation() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let now = std::time::Instant::now();

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure_with_cooldown(
                FailureKind::RateLimited,
                FailureScope::Credential,
                std::time::Duration::from_secs(3),
            ),
            now,
            policy(),
        ));

        assert!(matches!(
            result.mutation,
            StateMutation::MarkCredentialCoolingDown { .. }
        ));
    }

    #[test]
    fn model_group_client_error_does_not_mutate_credential() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure(FailureKind::ClientError, FailureScope::ModelGroup),
            std::time::Instant::now(),
            policy(),
        ));

        assert_eq!(
            result.mutation,
            StateMutation::Noop {
                reason: FailureReason::ClientOrModelError,
            }
        );
        assert_eq!(pool.select().unwrap().key, "k1");
    }

    #[test]
    fn retry_gate_denies_non_replayable_body() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let mut snapshot = retry_snapshot_for(&selected, next);
        snapshot.body_replayable = false;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            retry_policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::BodyNotReplayable
            }
        );
    }

    #[test]
    fn retry_gate_denies_streaming_request() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let mut snapshot = retry_snapshot_for(&selected, next);
        snapshot.streaming = true;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            retry_policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::StreamingNotRetryable
            }
        );
    }

    #[test]
    fn retry_gate_denies_after_partial_output_started() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let mut snapshot = retry_snapshot_for(&selected, next);
        snapshot.partial_output_started = true;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            retry_policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::PartialOutputStarted
            }
        );
    }

    #[test]
    fn retry_gate_denies_after_effective_deadline_exhausted() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let now = std::time::Instant::now();
        let mut snapshot = retry_snapshot_for(&selected, next);
        snapshot.effective_deadline = Some(now);

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            now,
            retry_policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::EffectiveDeadlineExhausted
            }
        );
        assert_eq!(result.effective_deadline_remaining_ms, Some(0));
    }

    #[test]
    fn retry_gate_denies_when_effective_deadline_cannot_fit_next_attempt() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let now = std::time::Instant::now();
        let mut snapshot = retry_snapshot_for(&selected, next);
        snapshot.effective_deadline = Some(now + std::time::Duration::from_millis(10));

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            now,
            retry_policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::EffectiveDeadlineExhausted
            }
        );
        assert_eq!(result.effective_deadline_remaining_ms, Some(10));
    }

    #[test]
    fn retry_gate_denies_attempt_at_limit() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let mut snapshot = retry_snapshot_for(&selected, next);
        snapshot.attempt = 1;
        snapshot.route_target_available = false;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            retry_policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::AttemptLimitReached
            }
        );
    }

    #[test]
    fn phase3b_credential_attempt_limit_falls_back_to_route_target_when_available() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let mut snapshot = retry_snapshot_for(&selected, next);
        snapshot.attempt = 1;
        snapshot.route_target_available = true;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            retry_policy(),
        ));

        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(result.duplicate_charge_risk, DuplicateChargeRisk::Unknown);
    }

    #[test]
    fn phase3b_empty_frozen_credential_candidates_falls_back_to_route_target_when_available() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let mut snapshot = snapshot_for(&selected);
        snapshot.route_target_available = true;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            retry_policy(),
        ));

        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(result.duplicate_charge_risk, DuplicateChargeRisk::Unknown);
    }

    #[test]
    fn phase3b_same_request_retry_disabled_falls_back_to_route_target_when_available() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let mut snapshot = snapshot_for(&selected);
        snapshot.route_target_available = true;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            policy(),
        ));

        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(result.duplicate_charge_risk, DuplicateChargeRisk::Unknown);
    }

    #[test]
    fn phase3b_same_request_retry_disabled_with_positive_max_retries_falls_back_to_route_target() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let mut snapshot = snapshot_for(&selected);
        snapshot.route_target_available = true;
        let policy = RoutingPolicy {
            retry_switched_key_in_same_request: false,
            max_same_request_retries: 2,
            route_target_retry_enabled: true,
            default_credential_cooldown: std::time::Duration::from_secs(20),
        };

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            policy,
        ));

        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(result.duplicate_charge_risk, DuplicateChargeRisk::Unknown);
    }

    #[test]
    fn phase3b_global_retry_gates_still_deny_credential_exhaustion_route_fallback() {
        let mut pool = pool();
        let selected = pool.select().unwrap();

        let cases = [
            (false, false, false, RetryDecisionReason::BodyNotReplayable),
            (
                true,
                true,
                false,
                RetryDecisionReason::StreamingNotRetryable,
            ),
            (true, false, true, RetryDecisionReason::PartialOutputStarted),
        ];

        for (body_replayable, streaming, partial_output_started, expected) in cases {
            let mut snapshot = snapshot_for(&selected);
            snapshot.body_replayable = body_replayable;
            snapshot.streaming = streaming;
            snapshot.partial_output_started = partial_output_started;
            snapshot.route_target_available = true;

            let result = transition_after_failure(upstream_input(
                &snapshot,
                retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
                std::time::Instant::now(),
                retry_policy(),
            ));

            assert_eq!(
                result.retry,
                RetryDirective::ReturnCurrentError { reason: expected }
            );
        }
    }

    #[test]
    fn retry_gate_allows_only_frozen_candidate() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let snapshot = retry_snapshot_for(&selected, next.clone());

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            retry_policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::RetryCredential {
                credential_id: next
            }
        );
    }

    #[test]
    fn retry_gate_rejects_candidate_equal_to_failed_credential() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let mut snapshot = retry_snapshot_for(&selected, selected.credential_id.clone());
        snapshot.route_target_available = false;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            retry_policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::NoFrozenCandidate
            }
        );
    }

    #[test]
    fn route_target_retry_disabled_has_specific_denial_reason() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let mut policy = policy();
        policy.route_target_retry_enabled = false;
        let snapshot = snapshot_for(&selected);

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::ProviderUnavailable, FailureScope::Channel),
            std::time::Instant::now(),
            policy,
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::RouteTargetRetryDisabled,
            }
        );
    }

    #[test]
    fn route_target_retry_denies_when_no_fallback_candidate_remains() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let mut snapshot = snapshot_for(&selected);
        snapshot.route_target_available = false;

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::ProviderUnavailable, FailureScope::Channel),
            std::time::Instant::now(),
            policy(),
        ));

        assert_eq!(
            result.retry,
            RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::NoRouteCandidate,
            }
        );
    }

    #[test]
    fn upstream_transaction_fallback_has_unknown_duplicate_charge_risk() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::ProviderUnavailable, FailureScope::Channel),
            std::time::Instant::now(),
            policy(),
        ));

        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(result.duplicate_charge_risk, DuplicateChargeRisk::Unknown);
    }

    #[test]
    fn local_transport_retry_has_no_duplicate_charge_risk_without_upstream_transaction_evidence() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);

        let result = transition_after_failure(TransitionInput {
            snapshot: &snapshot,
            failure: retryable_failure(FailureKind::ProviderUnavailable, FailureScope::Channel),
            failure_source: FailureSource::LocalTransport,
            now: std::time::Instant::now(),
            next_attempt_budget: Some(std::time::Duration::from_millis(50)),
            policy: policy(),
        });

        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(result.duplicate_charge_risk, DuplicateChargeRisk::None);
    }

    #[test]
    fn guarded_success_envelope_retry_has_unknown_duplicate_charge_risk() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);

        let result = transition_after_failure(TransitionInput {
            snapshot: &snapshot,
            failure: retryable_failure(FailureKind::ProviderUnavailable, FailureScope::Channel),
            failure_source: FailureSource::GuardedSuccessEnvelope,
            now: std::time::Instant::now(),
            next_attempt_budget: Some(std::time::Duration::from_millis(50)),
            policy: policy(),
        });

        assert_eq!(result.retry, RetryDirective::RetryRouteTarget);
        assert_eq!(result.duplicate_charge_risk, DuplicateChargeRisk::Unknown);
    }

    #[test]
    fn conservative_policy_switches_future_request_only() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let policy = retry_policy();

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            policy,
        ));
        let directive = result.retry;
        apply_state_mutation_for_test(&mut pool, result.mutation);

        assert!(matches!(
            directive,
            RetryDirective::ReturnCurrentError { .. }
        ));
        assert_eq!(pool.select().unwrap().key, "k2");
    }

    #[test]
    fn opt_in_policy_retries_once_after_switch() {
        let mut pool = pool();
        let selected = pool.select().unwrap();
        let next = pool.retry_candidates_from_current(1)[0]
            .credential_id
            .clone();
        let snapshot = retry_snapshot_for(&selected, next);
        let policy = retry_policy();

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            policy,
        ));
        let directive = result.retry;
        apply_state_mutation_for_test(&mut pool, result.mutation);

        assert!(matches!(directive, RetryDirective::RetryCredential { .. }));
        assert_eq!(pool.select().unwrap().key, "k2");
    }

    #[test]
    fn opt_in_policy_does_not_retry_when_switch_has_no_alternative() {
        let mut pool = KeyPool::new(KeyPoolConfig {
            name: "test".to_string(),
            credential_namespace: "test".to_string(),
            api_base: "https://example.com/v1".to_string(),
            credentials: vec![imported("k1")],
        })
        .unwrap();
        let selected = pool.select().unwrap();
        let mut snapshot = snapshot_for(&selected);
        snapshot.route_target_available = false;
        let policy = RoutingPolicy {
            retry_switched_key_in_same_request: false,
            max_same_request_retries: 0,
            route_target_retry_enabled: true,
            default_credential_cooldown: std::time::Duration::from_secs(20),
        };

        let result = transition_after_failure(upstream_input(
            &snapshot,
            retryable_failure(FailureKind::RateLimited, FailureScope::Credential),
            std::time::Instant::now(),
            policy,
        ));
        let directive = result.retry;
        apply_state_mutation_for_test(&mut pool, result.mutation);

        assert!(matches!(
            directive,
            RetryDirective::ReturnCurrentError { .. }
        ));
        assert!(pool.select().is_err());
    }

    #[test]
    fn explicit_failure_cooldown_overrides_pool_default_switch_cooldown() {
        let mut pool = KeyPool::new(KeyPoolConfig {
            name: "test".to_string(),
            credential_namespace: "test".to_string(),
            api_base: "https://example.com/v1".to_string(),
            credentials: vec![imported("k1"), imported("k2")],
        })
        .unwrap();
        let selected = pool.select().unwrap();
        let snapshot = snapshot_for(&selected);
        let policy = RoutingPolicy {
            retry_switched_key_in_same_request: false,
            max_same_request_retries: 0,
            route_target_retry_enabled: true,
            default_credential_cooldown: std::time::Duration::from_millis(10),
        };

        let result = transition_after_failure(upstream_input(
            &snapshot,
            failure_with_cooldown(
                FailureKind::RateLimited,
                FailureScope::Credential,
                std::time::Duration::from_millis(100),
            ),
            std::time::Instant::now(),
            policy,
        ));
        let directive = result.retry;
        apply_state_mutation_for_test(&mut pool, result.mutation);
        std::thread::sleep(std::time::Duration::from_millis(30));

        assert!(matches!(
            directive,
            RetryDirective::ReturnCurrentError { .. }
        ));
        assert_eq!(pool.snapshot().cooling_down_credentials, 1);
    }
}
