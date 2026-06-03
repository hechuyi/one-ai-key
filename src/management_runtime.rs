use std::time::{Duration, Instant};

use serde::Serialize;

use crate::{
    events::{ManagementAuditEvent, ManagementEventActor, RoutingTelemetry},
    management_errors::{registry_store_error, ManagementServiceError},
    management_registry::resolve_staged_registry_document,
    management_status::{
        credential_pool_alerts, runtime_readiness_projection, CredentialPoolAlertStatus,
        CredentialSetSnapshotCache, RuntimeCredentialCounts, RuntimeReadinessProjection,
    },
    state::{AppState, ChannelHealth, RuntimeTopologySummary},
};

#[derive(Debug, Serialize)]
pub struct RoutingTelemetryResponse {
    pub buffered_events: usize,
    pub offset: usize,
    pub limit: usize,
    pub events: Vec<RoutingTelemetry>,
}

pub fn routing_telemetry_response(
    snapshot: Vec<RoutingTelemetry>,
    offset: usize,
    limit: usize,
) -> RoutingTelemetryResponse {
    let buffered_events = snapshot.len();
    let events = snapshot.into_iter().skip(offset).take(limit).collect();
    RoutingTelemetryResponse {
        buffered_events,
        offset,
        limit,
        events,
    }
}

pub fn routing_telemetry_snapshot_response(
    state: &AppState,
    offset: usize,
    limit: usize,
) -> RoutingTelemetryResponse {
    let snapshot = state
        .routing_telemetry
        .lock()
        .expect("routing telemetry mutex poisoned")
        .snapshot();
    routing_telemetry_response(snapshot, offset, limit)
}

#[derive(Debug, Serialize)]
pub struct RuntimeResponse {
    pub uptime_seconds: u64,
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub runtime_reload_required: bool,
    pub channels: usize,
    pub client_tokens: usize,
    pub credentials: RuntimeCredentialCounts,
    pub credential_pool_alerts: Vec<CredentialPoolAlertStatus>,
    pub retry_policy: RuntimeRetryPolicy,
    pub management_events: usize,
    pub management_event_window_capacity: usize,
    pub routing_telemetry_events: usize,
    pub routing_telemetry_capacity: usize,
    pub request_limits: RequestLimits,
    pub timeout_seconds: TimeoutSeconds,
}

pub struct RuntimeResponseParts {
    pub uptime_seconds: u64,
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub channels: usize,
    pub client_tokens: usize,
    pub credentials: RuntimeCredentialCounts,
    pub retry_policy: RuntimeRetryPolicy,
    pub management_events: usize,
    pub management_event_window_capacity: usize,
    pub routing_telemetry_events: usize,
    pub routing_telemetry_capacity: usize,
    pub max_request_body_bytes: usize,
    pub max_model_catalog_body_bytes: usize,
    pub max_model_catalog_channels: usize,
    pub max_error_body_bytes: usize,
    pub max_route_candidates: usize,
    pub connect_timeout: Duration,
    pub non_streaming_total_timeout: Duration,
    pub streaming_idle_timeout: Duration,
}

pub fn runtime_response_from_parts(parts: RuntimeResponseParts) -> RuntimeResponse {
    let credential_pool_alerts = credential_pool_alerts(&parts.credentials);
    RuntimeResponse {
        uptime_seconds: parts.uptime_seconds,
        active_registry_generation: parts.active_registry_generation,
        active_registry_version: parts.active_registry_version,
        staged_registry_version: parts.staged_registry_version,
        runtime_reload_required: parts.staged_registry_version != parts.active_registry_version,
        channels: parts.channels,
        client_tokens: parts.client_tokens,
        credential_pool_alerts,
        credentials: parts.credentials,
        retry_policy: parts.retry_policy,
        management_events: parts.management_events,
        management_event_window_capacity: parts.management_event_window_capacity,
        routing_telemetry_events: parts.routing_telemetry_events,
        routing_telemetry_capacity: parts.routing_telemetry_capacity,
        request_limits: RequestLimits {
            max_request_body_bytes: parts.max_request_body_bytes,
            max_model_catalog_body_bytes: parts.max_model_catalog_body_bytes,
            max_model_catalog_channels: parts.max_model_catalog_channels,
            max_error_body_bytes: parts.max_error_body_bytes,
            max_route_candidates: parts.max_route_candidates,
        },
        timeout_seconds: TimeoutSeconds {
            connect: duration_secs(parts.connect_timeout),
            non_streaming_total: duration_secs(parts.non_streaming_total_timeout),
            streaming_idle: duration_secs(parts.streaming_idle_timeout),
        },
    }
}

pub async fn runtime_response(
    state: &AppState,
    staged_registry_version: Option<u64>,
) -> RuntimeResponse {
    let sample = collect_runtime_snapshot(state).await;

    runtime_response_from_parts(RuntimeResponseParts {
        uptime_seconds: state.started_at.elapsed().as_secs(),
        active_registry_generation: sample.active_registry_generation,
        active_registry_version: sample.active_registry_version,
        staged_registry_version,
        channels: sample.channels,
        client_tokens: sample.client_tokens,
        credentials: sample.credentials,
        retry_policy: sample.retry_policy,
        management_events: sample.management_events,
        management_event_window_capacity: sample.management_event_window_capacity,
        routing_telemetry_events: sample.routing_telemetry_events,
        routing_telemetry_capacity: state.routing.telemetry_buffer_capacity,
        max_request_body_bytes: state.max_request_body_bytes,
        max_model_catalog_body_bytes: state.max_model_catalog_body_bytes,
        max_model_catalog_channels: state.routing.max_model_catalog_channels,
        max_error_body_bytes: state.max_error_body_bytes,
        max_route_candidates: state.routing.max_route_candidates,
        connect_timeout: state.timeout_profile.connect,
        non_streaming_total_timeout: state.timeout_profile.non_streaming_total,
        streaming_idle_timeout: state.timeout_profile.streaming_idle,
    })
}

pub async fn runtime_response_for_state(
    state: &AppState,
) -> Result<RuntimeResponse, ManagementServiceError> {
    let staged_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?;
    Ok(runtime_response(state, staged_registry_version).await)
}

#[derive(Debug)]
struct RuntimeSnapshotSample {
    active_registry_generation: u64,
    active_registry_version: Option<u64>,
    channels: usize,
    client_tokens: usize,
    credentials: RuntimeCredentialCounts,
    retry_policy: RuntimeRetryPolicy,
    management_events: usize,
    management_event_window_capacity: usize,
    routing_telemetry_events: usize,
    serving_channels: usize,
    credential_set_blocking_alerts: usize,
    operator_input_alerts: usize,
    credential_sets_without_spare: usize,
    channels_cooling_down: usize,
}

#[derive(Debug)]
struct RuntimeReloadSample {
    active_registry_generation: u64,
    active_registry_version: Option<u64>,
    channels: usize,
}

async fn collect_runtime_snapshot(state: &AppState) -> RuntimeSnapshotSample {
    let topology_summary = state.runtime_catalogs.runtime_topology_summary();
    let mut credentials = RuntimeCredentialCounts::default();
    let mut snapshot_cache = CredentialSetSnapshotCache::default();
    let mut serving_channels = 0;
    let mut operator_input_alerts = 0usize;
    let mut credential_set_blocking_alerts = 0usize;
    let mut credential_sets_without_spare = 0usize;
    let mut channels_cooling_down = 0usize;

    for topology in state.runtime_catalogs.credential_set_topologies() {
        for channel_id in topology.channel_ids {
            let Some(pool_state) = state.channels.get(&channel_id) else {
                continue;
            };
            let sample = snapshot_cache.runtime_sample_for(&pool_state).await;
            add_runtime_counts(&mut credentials, &sample.counts);
            if sample.readiness.needs_operator_input {
                operator_input_alerts += 1;
            }
            if sample.readiness.blocking {
                credential_set_blocking_alerts += 1;
            }
            if sample.counts.available < 2 {
                credential_sets_without_spare += 1;
            }
            break;
        }
    }

    for topology in state.runtime_catalogs.channel_topologies() {
        let Some(pool_state) = state.channels.get(&topology.id) else {
            continue;
        };
        if !pool_state.configured_enabled || !pool_state.account_enabled {
            continue;
        }
        let health = pool_state
            .health
            .lock()
            .expect("channel health mutex poisoned")
            .clone();
        if matches!(
            health,
            ChannelHealth::CoolingDown { until, .. } if Instant::now() < until
        ) {
            channels_cooling_down += 1;
        }
        if !health.is_available_for_routing() && !matches!(health, ChannelHealth::Degraded { .. }) {
            continue;
        }
        let sample = snapshot_cache.runtime_sample_for(&pool_state).await;
        if sample.has_available_credentials() {
            serving_channels += 1;
        }
    }

    let active_registry_generation = state.channels.registry_generation();
    let active_registry_version = *state
        .active_registry_version
        .read()
        .expect("active registry version lock poisoned");
    let client_tokens = state
        .client_tokens
        .read()
        .expect("client token registry lock poisoned")
        .len();
    let routing_telemetry_events = state
        .routing_telemetry
        .lock()
        .expect("routing telemetry mutex poisoned")
        .len();

    RuntimeSnapshotSample {
        active_registry_generation,
        active_registry_version,
        channels: topology_summary.channels,
        client_tokens,
        credentials,
        retry_policy: runtime_retry_policy_from_topology_summary(topology_summary),
        management_events: state.events.len(),
        management_event_window_capacity: state.events.window_capacity(),
        routing_telemetry_events,
        serving_channels,
        credential_set_blocking_alerts,
        operator_input_alerts,
        credential_sets_without_spare,
        channels_cooling_down,
    }
}

fn add_runtime_counts(
    total: &mut RuntimeCredentialCounts,
    credential_set: &RuntimeCredentialCounts,
) {
    total.total += credential_set.total;
    total.available += credential_set.available;
    total.cooling_down += credential_set.cooling_down;
    total.expired += credential_set.expired;
    total.quota_exhausted += credential_set.quota_exhausted;
    total.disabled += credential_set.disabled;
}

fn collect_runtime_reload_sample(state: &AppState) -> RuntimeReloadSample {
    RuntimeReloadSample {
        active_registry_generation: state.channels.registry_generation(),
        active_registry_version: *state
            .active_registry_version
            .read()
            .expect("active registry version lock poisoned"),
        channels: state.runtime_catalogs.runtime_topology_summary().channels,
    }
}

#[derive(Debug, Serialize)]
pub struct ReadinessResponse {
    pub status: &'static str,
    pub channels: usize,
    pub serving_channels: usize,
    pub credentials: RuntimeCredentialCounts,
    pub blocking_alerts: usize,
    pub operator_input_alerts: usize,
}

#[derive(Debug, Serialize)]
pub struct RuntimeReloadResponse {
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub runtime_reload_required: bool,
    pub channels: usize,
}

pub struct RuntimeReloadResponseParts {
    pub active_registry_generation: u64,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub channels: usize,
}

pub fn runtime_reload_response(parts: RuntimeReloadResponseParts) -> RuntimeReloadResponse {
    RuntimeReloadResponse {
        active_registry_generation: parts.active_registry_generation,
        active_registry_version: parts.active_registry_version,
        staged_registry_version: parts.staged_registry_version,
        runtime_reload_required: parts.staged_registry_version != parts.active_registry_version,
        channels: parts.channels,
    }
}

pub async fn reload_runtime(
    state: &AppState,
    actor: ManagementEventActor,
) -> Result<RuntimeReloadResponse, ManagementServiceError> {
    let staged_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?;
    let Some(staged_registry_document) = state
        .registry_store
        .load_registry_for_validation()
        .await
        .map_err(registry_store_error)?
    else {
        return Err(ManagementServiceError::Conflict(
            "runtime reload requires a writable registry store".to_string(),
        ));
    };
    let validation_bootstrap = (*state.registry_validation_bootstrap).clone();
    let config = resolve_staged_registry_document(validation_bootstrap, staged_registry_document)
        .map_err(registry_store_error)?;
    state
        .events
        .record_audit_event(ManagementAuditEvent {
            kind: "runtime_reloaded".to_string(),
            action: "runtime_reloaded".to_string(),
            resource_type: "runtime".to_string(),
            resource_id: "runtime".to_string(),
            channel_id: String::new(),
            credential_id: String::new(),
            outcome: "applied".to_string(),
            request_id: None,
            generation: staged_registry_version,
            reason_code: "manual_runtime_reload".to_string(),
            actor: Some(actor),
        })
        .await
        .map_err(|err| ManagementServiceError::EventAppendFailed {
            message: err.to_string(),
            stale_history_id: None,
        })?;
    state
        .rebuild_runtime_from_resolved_config(config, staged_registry_version)
        .map_err(|err| {
            ManagementServiceError::Persistence(format!("runtime reload failed: {err}"))
        })?;
    let sample = collect_runtime_reload_sample(state);
    Ok(runtime_reload_response(RuntimeReloadResponseParts {
        active_registry_generation: sample.active_registry_generation,
        active_registry_version: sample.active_registry_version,
        staged_registry_version,
        channels: sample.channels,
    }))
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeRetryPolicy {
    pub same_request_retry_enabled_channels: usize,
    pub max_same_request_retries: usize,
}

fn runtime_retry_policy_from_topology_summary(
    summary: RuntimeTopologySummary,
) -> RuntimeRetryPolicy {
    RuntimeRetryPolicy {
        same_request_retry_enabled_channels: summary.same_request_retry_enabled_channels,
        max_same_request_retries: summary.max_same_request_retries,
    }
}

pub async fn readiness_response(state: &AppState) -> ReadinessResponse {
    let sample = collect_runtime_snapshot(state).await;
    let serving_channels = sample.serving_channels;
    let credential_set_blocking_alerts = sample.credential_set_blocking_alerts;
    let RuntimeReadinessProjection {
        status,
        blocking_alerts,
    } = runtime_readiness_projection(serving_channels, credential_set_blocking_alerts);
    ReadinessResponse {
        status,
        channels: sample.channels,
        serving_channels,
        credentials: sample.credentials,
        blocking_alerts,
        operator_input_alerts: sample.operator_input_alerts,
    }
}

#[derive(Debug, Serialize)]
pub struct ServingHealthResponse {
    pub status: &'static str,
    pub serving: bool,
    pub serving_channels: usize,
    pub blocking_alerts: usize,
    pub blocking_reasons: Vec<&'static str>,
}

pub async fn serving_health_response(state: &AppState) -> ServingHealthResponse {
    let sample = collect_runtime_snapshot(state).await;
    let serving_channels = sample.serving_channels;
    let credential_set_blocking_alerts = sample.credential_set_blocking_alerts;
    let RuntimeReadinessProjection {
        status,
        blocking_alerts,
    } = runtime_readiness_projection(serving_channels, credential_set_blocking_alerts);
    let mut blocking_reasons = Vec::new();
    if serving_channels == 0 {
        blocking_reasons.push("no_serving_channels");
    }
    if credential_set_blocking_alerts > 0 {
        blocking_reasons.push("credential_set_blocking_alerts");
    }
    ServingHealthResponse {
        status,
        serving: serving_channels > 0,
        serving_channels,
        blocking_alerts,
        blocking_reasons,
    }
}

#[derive(Debug, Serialize)]
pub struct ResilienceHealthResponse {
    pub status: &'static str,
    pub operator_input_alerts: usize,
    pub credential_sets_without_spare: usize,
    pub channels_cooling_down: usize,
    pub response_filter_alerts: usize,
}

pub async fn resilience_health_response(state: &AppState) -> ResilienceHealthResponse {
    let sample = collect_runtime_snapshot(state).await;
    let response_filter_alerts = 0usize;
    let status = if sample.serving_channels == 0 || sample.credential_set_blocking_alerts > 0 {
        "blocked"
    } else if sample.operator_input_alerts > 0
        || sample.credential_sets_without_spare > 0
        || sample.channels_cooling_down > 0
        || response_filter_alerts > 0
    {
        "degraded"
    } else {
        "ok"
    };
    ResilienceHealthResponse {
        status,
        operator_input_alerts: sample.operator_input_alerts,
        credential_sets_without_spare: sample.credential_sets_without_spare,
        channels_cooling_down: sample.channels_cooling_down,
        response_filter_alerts,
    }
}

#[derive(Debug, Serialize)]
pub struct RequestLimits {
    pub max_request_body_bytes: usize,
    pub max_model_catalog_body_bytes: usize,
    pub max_model_catalog_channels: usize,
    pub max_error_body_bytes: usize,
    pub max_route_candidates: usize,
}

#[derive(Debug, Serialize)]
pub struct TimeoutSeconds {
    pub connect: u64,
    pub non_streaming_total: u64,
    pub streaming_idle: u64,
}

pub fn duration_secs(duration: Duration) -> u64 {
    duration.as_secs()
}

#[cfg(test)]
mod tests {
    fn item_body<'a>(source: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
        let start = source
            .find(start_marker)
            .unwrap_or_else(|| panic!("{start_marker} exists"));
        let end = source[start..]
            .find(end_marker)
            .map(|offset| start + offset)
            .unwrap_or_else(|| panic!("{end_marker} follows {start_marker}"));
        &source[start..end]
    }

    #[test]
    fn runtime_and_readiness_responses_consume_runtime_snapshot_sample() {
        let source = include_str!("management_runtime.rs");
        assert!(source.contains("struct RuntimeSnapshotSample"));
        assert!(source.contains("async fn collect_runtime_snapshot"));

        let runtime_body = item_body(
            source,
            "pub async fn runtime_response",
            "\npub async fn runtime_response_for_state",
        );
        let readiness_body = item_body(
            source,
            "pub async fn readiness_response",
            "\n#[derive(Debug, Serialize)]\npub struct ServingHealthResponse",
        );
        let serving_health_body = item_body(
            source,
            "pub async fn serving_health_response",
            "\n#[derive(Debug, Serialize)]\npub struct ResilienceHealthResponse",
        );
        let resilience_health_body = item_body(
            source,
            "pub async fn resilience_health_response",
            "\n#[derive(Debug, Serialize)]\npub struct RequestLimits",
        );

        for body in [
            runtime_body,
            readiness_body,
            serving_health_body,
            resilience_health_body,
        ] {
            assert!(body.contains("collect_runtime_snapshot(state).await"));
            for forbidden_token in [
                "CredentialSetSnapshotCache::default()",
                "BTreeSet::new()",
                "add_counts_once(",
                "snapshot_for(",
            ] {
                assert!(
                    !body.contains(forbidden_token),
                    "response functions should consume sampled runtime state, not {forbidden_token}"
                );
            }
        }

        let collector_body = item_body(
            source,
            "async fn collect_runtime_snapshot",
            "\nfn collect_runtime_reload_sample",
        );
        for forbidden_token in [
            "credential_set_readiness_projection_from_snapshot(",
            "snapshot_for(",
            "runtime_retry_policy(state)",
            "state.channels.len()",
            "state.channels.channel_values_snapshot()",
            "mark_credential_set_seen(&pool_state)",
        ] {
            assert!(
                !collector_body.contains(forbidden_token),
                "runtime snapshot collector should consume credential-set runtime samples, not {forbidden_token}"
            );
        }
    }
}
