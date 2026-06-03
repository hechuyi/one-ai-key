use crate::{
    error::{ClassifiedFailure, FailureConfidence, FailureKind, FailureScope, RetryAfterSource},
    events::RoutingTelemetry,
    failure_state_executor::apply_state_mutation,
    routing::{
        transition_after_failure, FailureSource, RequestSelectionSnapshot, RetryDecisionReason,
        RetryDirective, RoutingPolicy, TransitionInput,
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
    let (directive, telemetry) =
        apply_error_action(state, pool_state, snapshot, failure.clone(), failure_source).await;
    record_routing_telemetry(
        state,
        RoutingTelemetry::UpstreamFailureObserved {
            request_id: snapshot.request_id.clone(),
            channel_id: snapshot.channel_id.0.clone(),
            failure: upstream_failure_telemetry(&failure, &directive),
        },
    );
    record_transition_telemetry(state, snapshot, telemetry);
    directive
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
) -> (RetryDirective, Vec<RoutingTelemetry>) {
    let _mutation_guard = pool_state.mutation_gate.lock().await;
    let mut pool = pool_state.pool.lock().await;
    let policy = routing_policy_for_pool(pool_state);
    let result = transition_after_failure(TransitionInput {
        snapshot,
        failure,
        failure_source,
        now: std::time::Instant::now(),
        policy,
    });
    let telemetry = apply_state_mutation(state, pool_state, &mut pool, snapshot, result.mutation);
    (result.retry, telemetry)
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
    failure: &ClassifiedFailure,
    directive: &RetryDirective,
) -> crate::events::UpstreamFailureTelemetry {
    let (retry_decision, retry_decision_reason) = retry_decision_telemetry(directive);
    crate::events::UpstreamFailureTelemetry {
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
        retry_decision: retry_decision.to_string(),
        retry_decision_reason: retry_decision_reason.map(str::to_string),
    }
}

fn retry_decision_telemetry(directive: &RetryDirective) -> (&'static str, Option<&'static str>) {
    match directive {
        RetryDirective::ReturnCurrentError { reason } => (
            "return_current_error",
            Some(retry_decision_reason_code(*reason)),
        ),
        RetryDirective::RetryCredential { .. } => ("retry_credential", None),
        RetryDirective::RetryRouteTarget => ("retry_route_target", None),
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
        RetryDecisionReason::AttemptLimitReached => "attempt_limit_reached",
        RetryDecisionReason::NoFrozenCandidate => "no_frozen_candidate",
    }
}
