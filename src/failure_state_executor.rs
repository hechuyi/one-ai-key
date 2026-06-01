use crate::{
    credentials::CredentialId,
    events::{RoutingTelemetry, RoutingTelemetryBuffer},
    pool::KeyPool,
    routing::{FailureReason, RequestSelectionSnapshot, StateMutation},
    state::{
        AppState, AutomaticLifecyclePersistenceRequest, AutomaticLifecyclePersistenceState,
        ChannelHealth, ChannelId, PoolState,
    },
};

pub fn apply_state_mutation(
    state: &AppState,
    pool_state: &PoolState,
    pool: &mut KeyPool,
    snapshot: &RequestSelectionSnapshot,
    mutation: StateMutation,
) -> Vec<RoutingTelemetry> {
    match mutation {
        StateMutation::MarkCredentialCoolingDown {
            credential_id,
            until,
            reason,
            ..
        } => {
            let before = pool
                .credential_snapshot_by_id(&credential_id)
                .map(|snapshot| snapshot.state);
            pool.apply_credential_cooldown_until(&credential_id, until, reason_text(reason));
            if let Some(after) = pool
                .credential_snapshot_by_id(&credential_id)
                .map(|snapshot| snapshot.state)
            {
                pool_state
                    .advance_selector_generation_if_state_kind_changed(before.as_ref(), &after);
                return vec![credential_transition_telemetry(
                    snapshot,
                    &credential_id,
                    "cooling_down",
                    reason,
                )];
            }
        }
        StateMutation::ExpireCredential {
            credential_id,
            reason,
            ..
        } => {
            let before = pool
                .credential_snapshot_by_id(&credential_id)
                .map(|snapshot| snapshot.state);
            pool.apply_credential_expired(&credential_id, reason_text(reason));
            if let Some(after) = pool
                .credential_snapshot_by_id(&credential_id)
                .map(|snapshot| snapshot.state)
            {
                pool_state
                    .advance_selector_generation_if_state_kind_changed(before.as_ref(), &after);
                enqueue_automatic_lifecycle_persistence(
                    state,
                    pool_state,
                    &credential_id,
                    AutomaticLifecyclePersistenceState::expired(reason_text(reason)),
                    reason_code(reason),
                    &snapshot.channel_id,
                    &snapshot.request_id,
                );
                return vec![credential_transition_telemetry(
                    snapshot,
                    &credential_id,
                    "expired",
                    reason,
                )];
            }
        }
        StateMutation::MarkCredentialQuotaExhausted {
            credential_id,
            reason,
            ..
        } => {
            let before = pool
                .credential_snapshot_by_id(&credential_id)
                .map(|snapshot| snapshot.state);
            pool.apply_credential_quota_exhausted(&credential_id, reason_text(reason));
            if let Some(after) = pool
                .credential_snapshot_by_id(&credential_id)
                .map(|snapshot| snapshot.state)
            {
                pool_state
                    .advance_selector_generation_if_state_kind_changed(before.as_ref(), &after);
                enqueue_automatic_lifecycle_persistence(
                    state,
                    pool_state,
                    &credential_id,
                    AutomaticLifecyclePersistenceState::quota_exhausted(reason_text(reason)),
                    reason_code(reason),
                    &snapshot.channel_id,
                    &snapshot.request_id,
                );
                return vec![credential_transition_telemetry(
                    snapshot,
                    &credential_id,
                    "quota_exhausted",
                    reason,
                )];
            }
        }
        StateMutation::MarkProviderAccountChannelCoolingDownOrDegraded {
            provider_id,
            account_id,
            until,
            reason,
            ..
        } => {
            let health = match until {
                Some(until) => ChannelHealth::CoolingDown {
                    until,
                    reason: reason_text(reason).to_string(),
                },
                None => ChannelHealth::Degraded {
                    reason: reason_text(reason).to_string(),
                },
            };
            let applied = pool_state.apply_automatic_channel_health_transition(
                snapshot.channel_health_generation,
                health.clone(),
            );
            state.channels.apply_failure_domain_transition(
                &provider_id,
                &account_id,
                until,
                reason_text(reason),
            );
            if applied {
                return vec![channel_health_transition_telemetry(
                    snapshot, &health, reason,
                )];
            }
        }
        StateMutation::Noop { .. } => {}
    }
    Vec::new()
}

fn credential_transition_telemetry(
    snapshot: &RequestSelectionSnapshot,
    credential_id: &CredentialId,
    state: &'static str,
    reason: FailureReason,
) -> RoutingTelemetry {
    RoutingTelemetry::CredentialTransitionApplied {
        request_id: snapshot.request_id.clone(),
        channel_id: snapshot.channel_id.0.clone(),
        credential_id: credential_id.0.clone(),
        state: state.to_string(),
        reason: reason_code(reason).to_string(),
    }
}

fn channel_health_transition_telemetry(
    snapshot: &RequestSelectionSnapshot,
    health: &ChannelHealth,
    reason: FailureReason,
) -> RoutingTelemetry {
    let state = match health {
        ChannelHealth::Available => "available",
        ChannelHealth::Degraded { .. } => "degraded",
        ChannelHealth::CoolingDown { .. } => "cooling_down",
        ChannelHealth::Disabled { .. } => "disabled",
    };
    RoutingTelemetry::ChannelHealthTransitionApplied {
        request_id: snapshot.request_id.clone(),
        channel_id: snapshot.channel_id.0.clone(),
        state: state.to_string(),
        reason: reason_code(reason).to_string(),
    }
}

fn enqueue_automatic_lifecycle_persistence(
    state: &AppState,
    pool_state: &PoolState,
    credential_id: &CredentialId,
    lifecycle_state: AutomaticLifecyclePersistenceState,
    reason_class: &'static str,
    channel_id: &ChannelId,
    request_id: &str,
) {
    let request = AutomaticLifecyclePersistenceRequest {
        credential_set_id: pool_state.credential_set_id.clone(),
        credential_id: credential_id.clone(),
        state: lifecycle_state,
        reason_class,
        channel_id: channel_id.0.clone(),
    };
    let persistence_state = request.lifecycle_state_name();
    if let Err(drop_reason) = state.lifecycle_persistence.try_enqueue(request) {
        tracing::warn!(
            request_id = %request_id,
            channel_id = %channel_id.0.as_str(),
            credential_id = %credential_id.0.as_str(),
            state = persistence_state,
            reason = reason_class,
            drop_reason = drop_reason.as_str(),
            "automatic credential lifecycle persistence enqueue dropped"
        );
        record_routing_telemetry(
            &state.routing_telemetry,
            RoutingTelemetry::CredentialLifecyclePersistenceDropped {
                request_id: request_id.to_string(),
                channel_id: channel_id.0.clone(),
                credential_id: credential_id.0.clone(),
                state: persistence_state.to_string(),
                reason: reason_class.to_string(),
                drop_reason: drop_reason.as_str().to_string(),
            },
        );
    }
}

fn record_routing_telemetry(
    telemetry: &std::sync::Arc<std::sync::Mutex<RoutingTelemetryBuffer>>,
    event: RoutingTelemetry,
) {
    if let Ok(mut telemetry) = telemetry.try_lock() {
        telemetry.push(event);
    }
}

fn reason_text(reason: FailureReason) -> &'static str {
    match reason {
        FailureReason::UpstreamAuthInvalid => "upstream reported expired credential",
        FailureReason::UpstreamRateLimited => "switchable upstream failure",
        FailureReason::UpstreamQuotaExhausted => "upstream reported quota exhausted",
        FailureReason::UpstreamProviderUnavailable => "upstream provider unavailable",
        FailureReason::KeySwitchCooldown => "upstream key switch cooldown",
        FailureReason::ClientOrModelError => "client or model error",
        FailureReason::Unknown => "unknown upstream failure",
    }
}

fn reason_code(reason: FailureReason) -> &'static str {
    match reason {
        FailureReason::UpstreamAuthInvalid => "upstream_auth_invalid",
        FailureReason::UpstreamRateLimited => "upstream_rate_limited",
        FailureReason::UpstreamQuotaExhausted => "upstream_quota_exhausted",
        FailureReason::UpstreamProviderUnavailable => "upstream_provider_unavailable",
        FailureReason::KeySwitchCooldown => "key_switch_cooldown",
        FailureReason::ClientOrModelError => "client_or_model_error",
        FailureReason::Unknown => "unknown",
    }
}
