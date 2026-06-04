use crate::{
    credentials::short_hash,
    error::{ClassifiedFailure, FailureConfidence, FailureKind, FailureScope, RetryAfterSource},
    events::RoutingTelemetry,
    failure_state_executor::apply_state_mutation,
    routing::{
        transition_after_failure, DuplicateChargeRisk, FailureSource, RequestSelectionSnapshot,
        RetryDecisionReason, RetryDirective, RoutingPolicy, TransitionInput, TransitionResult,
        FAILURE_SOURCE_TELEMETRY_CONTRACT,
    },
    state::{AppState, PoolState},
};

pub async fn transition_observed_upstream_failure(
    state: &AppState,
    pool_state: &PoolState,
    snapshot: &RequestSelectionSnapshot,
    failure: ClassifiedFailure,
) -> RetryDirective {
    transition_observed_failure(
        state,
        pool_state,
        snapshot,
        failure,
        FailureSource::UpstreamTransaction,
    )
    .await
}

pub async fn transition_observed_failure(
    state: &AppState,
    pool_state: &PoolState,
    snapshot: &RequestSelectionSnapshot,
    failure: ClassifiedFailure,
    failure_source: FailureSource,
) -> RetryDirective {
    let (result, telemetry) =
        apply_error_action(state, pool_state, snapshot, failure.clone(), failure_source).await;
    record_routing_telemetry(
        state,
        RoutingTelemetry::UpstreamFailureObserved {
            request_id: snapshot.request_id.clone(),
            channel_id: snapshot.channel_id.0.clone(),
            failure: Box::new(upstream_failure_telemetry(
                snapshot,
                &failure,
                failure_source,
                &result,
            )),
        },
    );
    record_transition_telemetry(state, snapshot, telemetry);
    result.retry
}

pub fn record_routing_telemetry(state: &AppState, event: RoutingTelemetry) {
    if let Ok(mut telemetry) = state.routing_telemetry.try_lock() {
        telemetry.push(event);
    }
}

async fn apply_error_action(
    state: &AppState,
    pool_state: &PoolState,
    snapshot: &RequestSelectionSnapshot,
    failure: ClassifiedFailure,
    failure_source: FailureSource,
) -> (TransitionResult, Vec<RoutingTelemetry>) {
    let _mutation_guard = pool_state.mutation_gate.lock().await;
    let mut pool = pool_state.pool.lock().await;
    let policy = routing_policy_for_pool(pool_state);
    let mut result = transition_after_failure(TransitionInput {
        snapshot,
        failure,
        failure_source,
        now: std::time::Instant::now(),
        next_attempt_budget: (!snapshot.streaming).then_some(state.timeout_profile.connect),
        policy,
    });
    let telemetry = apply_state_mutation(
        state,
        pool_state,
        &mut pool,
        snapshot,
        result.mutation.clone(),
    );
    if matches!(result.retry, RetryDirective::RetrySameTarget) && telemetry.is_empty() {
        result.retry = RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::NoRouteCandidate,
        };
        result.duplicate_charge_risk = DuplicateChargeRisk::None;
    }
    (result, telemetry)
}

fn routing_policy_for_pool(pool_state: &PoolState) -> RoutingPolicy {
    RoutingPolicy {
        retry_switched_key_in_same_request: pool_state.retry_switched_key_in_same_request,
        max_same_request_retries: pool_state.max_same_request_retries,
        route_target_retry_enabled: pool_state.route_target_retry_enabled,
        default_credential_cooldown: pool_state.default_credential_cooldown,
    }
}

fn record_transition_telemetry(
    state: &AppState,
    snapshot: &RequestSelectionSnapshot,
    telemetry: Vec<RoutingTelemetry>,
) {
    let applied = !telemetry.is_empty();
    for event in telemetry {
        record_routing_telemetry(state, event);
    }
    if applied {
        record_routing_telemetry(
            state,
            RoutingTelemetry::TransitionApplied {
                request_id: snapshot.request_id.clone(),
                channel_id: snapshot.channel_id.0.clone(),
            },
        );
    }
}

fn upstream_failure_telemetry(
    snapshot: &RequestSelectionSnapshot,
    failure: &ClassifiedFailure,
    failure_source: FailureSource,
    result: &TransitionResult,
) -> crate::events::UpstreamFailureTelemetry {
    let (directive, retry_decision, retry_decision_reason) =
        retry_decision_telemetry(&result.retry);
    crate::events::UpstreamFailureTelemetry {
        public_model: snapshot.requested_model.clone(),
        credential_id_hash: short_hash(&snapshot.credential_id.0),
        attempt: snapshot.attempt,
        failure_source: failure_source_code(failure_source).to_string(),
        failure_kind: failure_kind_code(failure.kind).to_string(),
        failure_scope: failure_scope_code(failure.primary_scope).to_string(),
        retryable: failure.retryable,
        confidence: failure_confidence_code(failure.confidence).to_string(),
        status: failure.upstream_status,
        classifier_id: failure.classifier_id.clone(),
        classifier_version: failure.classifier_version.clone(),
        adaptation_rule_id: failure.adaptation_rule_id.clone(),
        retry_after_source: failure
            .retry_after_source
            .map(retry_after_source_code)
            .map(str::to_string),
        cooldown_seconds: failure.cooldown.map(|cooldown| cooldown.as_secs()),
        directive: directive.to_string(),
        denial_reason: retry_decision_reason.map(str::to_string),
        duplicate_charge_risk: duplicate_charge_risk_code(result.duplicate_charge_risk).to_string(),
        effective_deadline_remaining_ms: result.effective_deadline_remaining_ms,
        retry_pressure_accounted: retry_pressure_accounted(&result.retry),
        retry_decision: retry_decision.to_string(),
        retry_decision_reason: retry_decision_reason.map(str::to_string),
    }
}

fn retry_pressure_accounted(directive: &RetryDirective) -> bool {
    !matches!(
        directive,
        RetryDirective::ReturnCurrentError {
            reason: RetryDecisionReason::PartialOutputStarted
        }
    )
}

fn failure_source_code(source: FailureSource) -> &'static str {
    FAILURE_SOURCE_TELEMETRY_CONTRACT
        .iter()
        .find_map(|(candidate, code)| (*candidate == source).then_some(*code))
        .expect("failure source telemetry contract must cover all variants")
}

fn duplicate_charge_risk_code(risk: DuplicateChargeRisk) -> &'static str {
    match risk {
        DuplicateChargeRisk::None => "none",
        DuplicateChargeRisk::Unknown => "unknown",
    }
}

fn retry_decision_telemetry(
    directive: &RetryDirective,
) -> (&'static str, &'static str, Option<&'static str>) {
    match directive {
        RetryDirective::ReturnCurrentError { reason } => (
            "return_error",
            "return_current_error",
            Some(retry_decision_reason_code(*reason)),
        ),
        RetryDirective::RetryCredential { .. } => ("retry_credential", "retry_credential", None),
        RetryDirective::RetryRouteTarget => ("retry_route_target", "retry_route_target", None),
        RetryDirective::RetrySameTarget => ("retry_same_target", "retry_same_target", None),
    }
}

fn failure_kind_code(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::RateLimited => "rate_limited",
        FailureKind::KeySwitchCooldown => "key_switch_cooldown",
        FailureKind::AuthInvalid => "auth_invalid",
        FailureKind::QuotaExhausted => "quota_exhausted",
        FailureKind::RelayBalanceUnavailable => "relay_balance_unavailable",
        FailureKind::ProviderUnavailable => "provider_unavailable",
        FailureKind::ResponseFilterRejected => "response_filter_rejected",
        FailureKind::ClientError => "client_error",
        FailureKind::Unknown => "unknown",
    }
}

fn failure_scope_code(scope: FailureScope) -> &'static str {
    match scope {
        FailureScope::RequestOnly => "request_only",
        FailureScope::Credential => "credential",
        FailureScope::Account => "account",
        FailureScope::Channel => "channel",
        FailureScope::Deployment => "deployment",
        FailureScope::ModelGroup => "model_group",
        FailureScope::ProviderAdapter => "provider_adapter",
        FailureScope::ClientToken => "client_token",
    }
}

fn failure_confidence_code(confidence: FailureConfidence) -> &'static str {
    match confidence {
        FailureConfidence::Low => "low",
        FailureConfidence::Medium => "medium",
        FailureConfidence::High => "high",
    }
}

fn retry_after_source_code(source: RetryAfterSource) -> &'static str {
    match source {
        RetryAfterSource::DeltaSeconds => "delta_seconds",
        RetryAfterSource::HttpDate => "http_date",
    }
}

fn retry_decision_reason_code(reason: RetryDecisionReason) -> &'static str {
    match reason {
        RetryDecisionReason::PolicyDisabled => "policy_disabled",
        RetryDecisionReason::FailureNotRetryable => "failure_not_retryable",
        RetryDecisionReason::BodyNotReplayable => "body_not_replayable",
        RetryDecisionReason::StreamingNotRetryable => "streaming_not_retryable",
        RetryDecisionReason::PartialOutputStarted => "partial_output_started",
        RetryDecisionReason::AttemptLimitReached => "attempt_limit_reached",
        RetryDecisionReason::NoFrozenCandidate => "no_frozen_candidate",
        RetryDecisionReason::EffectiveDeadlineExhausted => "effective_deadline_exhausted",
        RetryDecisionReason::RouteTargetRetryDisabled => "route_target_retry_disabled",
        RetryDecisionReason::NoRouteCandidate => "no_route_candidate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        credentials::{CredentialFingerprint, CredentialId},
        error::ClassifiedFailure,
        provider::{EndpointKind, ProviderKind},
        routing::{RequestSelectionSnapshot, SelectionReason, StateMutation},
        state::ChannelId,
    };

    #[test]
    fn retry_decision_telemetry_uses_stable_terminal_directive() {
        let (directive, retry_decision, reason) =
            retry_decision_telemetry(&RetryDirective::ReturnCurrentError {
                reason: RetryDecisionReason::AttemptLimitReached,
            });

        assert_eq!(directive, "return_error");
        assert_eq!(retry_decision, "return_current_error");
        assert_eq!(reason, Some("attempt_limit_reached"));
    }

    fn snapshot() -> RequestSelectionSnapshot {
        RequestSelectionSnapshot {
            request_id: "req-1".to_string(),
            config_generation: 1,
            channel_health_generation: 1,
            client_token_id: "client-a".to_string(),
            requested_model: Some("gpt-test".to_string()),
            endpoint: EndpointKind::ChatCompletions,
            route_target_index: Some(0),
            channel_id: ChannelId("channel-a".to_string()),
            provider_id: "provider-a".to_string(),
            account_id: "account-a".to_string(),
            provider_kind: ProviderKind::OpenAiCompatible,
            credential_id: CredentialId("credential-a".to_string()),
            credential_fingerprint: CredentialFingerprint("fingerprint-a".to_string()),
            classifier_id: "classifier-a".to_string(),
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

    fn failure() -> ClassifiedFailure {
        ClassifiedFailure {
            kind: FailureKind::ProviderUnavailable,
            primary_scope: FailureScope::Channel,
            retryable: true,
            cooldown: None,
            retry_after_source: None,
            confidence: FailureConfidence::Medium,
            upstream_status: Some(200),
            upstream_code: None,
            upstream_limit_type: None,
            classifier_id: "classifier-a".to_string(),
            classifier_version: "1".to_string(),
            adaptation_rule_id: None,
        }
    }

    #[test]
    fn guarded_success_envelope_failure_source_serializes_for_telemetry() {
        let result = TransitionResult {
            mutation: StateMutation::Noop {
                reason: crate::routing::FailureReason::Unknown,
            },
            retry: RetryDirective::RetryRouteTarget,
            duplicate_charge_risk: DuplicateChargeRisk::Unknown,
            effective_deadline_remaining_ms: Some(100),
        };

        let telemetry = upstream_failure_telemetry(
            &snapshot(),
            &failure(),
            FailureSource::GuardedSuccessEnvelope,
            &result,
        );

        assert_eq!(telemetry.failure_source, "guarded_success_envelope");
        assert_eq!(telemetry.duplicate_charge_risk, "unknown");
    }
}
