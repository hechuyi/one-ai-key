use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    credential_probe::probe_result_is_default_key_switch_cooldown,
    credential_repository::{
        CredentialProbeOutcome, CredentialProbeResultRecord, CredentialSetId, CredentialStoreError,
        CredentialStoreHandle,
    },
    error::DEFAULT_ERROR_CLASSIFIER_ID,
    management_errors::ManagementServiceError,
    management_operations::{
        credential_set_operations_for_all_sets, CredentialSetOperationsStatus,
    },
    management_status::RuntimeCredentialCounts,
    route_plan::{preview_route, RoutePreviewCandidate, RoutePreviewInput, RoutePreviewReason},
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct AlertsResponse {
    pub total_alerts: usize,
    pub critical_alerts: usize,
    pub warning_alerts: usize,
    pub info_alerts: usize,
    pub operator_input_alerts: usize,
    pub blocking_alerts: usize,
    pub alerts: Vec<ManagementAlertStatus>,
}

pub async fn alerts_response(
    credential_store: &CredentialStoreHandle,
    operations_by_credential_set: Vec<CredentialSetOperationsStatus>,
) -> Result<AlertsResponse, ManagementServiceError> {
    let mut alerts = Vec::new();
    for operations in operations_by_credential_set {
        alerts.extend(credential_set_alert_statuses(credential_store, &operations).await?);
    }

    alerts.sort_by(|a, b| {
        severity_rank(a.severity)
            .cmp(&severity_rank(b.severity))
            .then_with(|| a.resource_id.cmp(&b.resource_id))
            .then_with(|| a.kind.cmp(b.kind))
    });
    Ok(AlertsResponse {
        total_alerts: alerts.len(),
        critical_alerts: alerts
            .iter()
            .filter(|alert| alert.severity == "critical")
            .count(),
        warning_alerts: alerts
            .iter()
            .filter(|alert| alert.severity == "warning")
            .count(),
        info_alerts: alerts
            .iter()
            .filter(|alert| alert.severity == "info")
            .count(),
        operator_input_alerts: alerts
            .iter()
            .filter(|alert| alert.needs_operator_input)
            .count(),
        blocking_alerts: alerts
            .iter()
            .filter(|alert| !alert.accepting_requests)
            .count(),
        alerts,
    })
}

pub async fn alerts_response_for_state(
    state: &AppState,
) -> Result<AlertsResponse, ManagementServiceError> {
    let operations_by_credential_set = credential_set_operations_for_all_sets(state).await?;
    let mut response =
        alerts_response(&state.credential_store, operations_by_credential_set).await?;
    response
        .alerts
        .extend(model_route_all_target_suppression_alerts(state));
    response
        .alerts
        .extend(response_filter_contamination_alerts_for_state(state));
    recompute_alert_totals(&mut response);
    Ok(response)
}

async fn credential_set_alert_statuses(
    credential_store: &CredentialStoreHandle,
    operations: &CredentialSetOperationsStatus,
) -> Result<Vec<ManagementAlertStatus>, ManagementServiceError> {
    let mut alerts = Vec::new();
    for alert in &operations.alerts {
        alerts.push(management_alert_status(AlertProjection {
            kind: alert.kind,
            severity: alert.severity,
            resource_id: &operations.credential_set_id,
            message: alert.message,
            serving_mode: operations.serving_mode,
            accepting_requests: operations.accepting_requests,
            needs_operator_input: operations.needs_operator_input,
            required_action: operations.required_action,
            credentials: &operations.credentials,
        }));
    }
    if let Some(probe_alert) = latest_probe_operational_alert(credential_store, operations).await? {
        alerts.push(probe_alert);
    }
    Ok(alerts)
}

#[derive(Debug, Serialize)]
pub struct ManagementAlertStatus {
    pub kind: &'static str,
    pub severity: &'static str,
    pub resource_kind: &'static str,
    pub resource_type: &'static str,
    pub resource_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppressed_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redact_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub channel_ids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reason_codes: Vec<String>,
    pub message: &'static str,
    pub serving_mode: &'static str,
    pub accepting_requests: bool,
    pub needs_operator_input: bool,
    pub required_action: &'static str,
    pub credentials: RuntimeCredentialCounts,
}

#[derive(Debug, Clone, Copy)]
pub struct AlertProjection<'a> {
    pub kind: &'static str,
    pub severity: &'static str,
    pub resource_id: &'a str,
    pub message: &'static str,
    pub serving_mode: &'static str,
    pub accepting_requests: bool,
    pub needs_operator_input: bool,
    pub required_action: &'static str,
    pub credentials: &'a RuntimeCredentialCounts,
}

pub fn severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 0,
        "warning" => 1,
        "info" => 2,
        _ => 3,
    }
}

pub fn management_alert_status(projection: AlertProjection<'_>) -> ManagementAlertStatus {
    ManagementAlertStatus {
        kind: projection.kind,
        severity: projection.severity,
        resource_kind: "credential_set",
        resource_type: "credential_set",
        resource_id: projection.resource_id.to_string(),
        public_model: None,
        candidate_count: None,
        suppressed_count: None,
        rule_id: None,
        redact_count: None,
        reject_count: None,
        window_seconds: None,
        channel_ids: Vec::new(),
        reason_codes: Vec::new(),
        message: projection.message,
        serving_mode: projection.serving_mode,
        accepting_requests: projection.accepting_requests,
        needs_operator_input: projection.needs_operator_input,
        required_action: projection.required_action,
        credentials: projection.credentials.clone(),
    }
}

pub fn latest_probe_operational_alert_status(
    latest_probe: &CredentialProbeResultRecord,
    operations: &CredentialSetOperationsStatus,
) -> Option<ManagementAlertStatus> {
    if probe_result_is_default_key_switch_cooldown(latest_probe, DEFAULT_ERROR_CLASSIFIER_ID) {
        return None;
    }

    match latest_probe.outcome {
        CredentialProbeOutcome::Unknown => Some(management_alert_status(AlertProjection {
            kind: "credential_set_probe_unknown",
            severity: "warning",
            resource_id: &operations.credential_set_id,
            message: "latest credential probe returned ambiguous evidence; inspect upstream response behavior",
            serving_mode: operations.serving_mode,
            accepting_requests: operations.accepting_requests,
            needs_operator_input: true,
            required_action: "inspect_latest_probe",
            credentials: &operations.credentials,
        })),
        _ => None,
    }
}

pub async fn latest_probe_operational_alert(
    credential_store: &CredentialStoreHandle,
    operations: &CredentialSetOperationsStatus,
) -> Result<Option<ManagementAlertStatus>, ManagementServiceError> {
    let latest_probe = credential_store
        .load_latest_probe_result_for_credential_set(CredentialSetId(
            operations.credential_set_id.clone(),
        ))
        .await;
    let latest_probe = match latest_probe {
        Ok(latest_probe) => latest_probe,
        Err(CredentialStoreError::NotWritable) => None,
        Err(CredentialStoreError::Persistence(_)) => {
            return Err(ManagementServiceError::Persistence(
                "credential store persistence error".to_string(),
            ))
        }
    };
    let Some(latest_probe) = latest_probe else {
        return Ok(None);
    };
    Ok(latest_probe_operational_alert_status(
        &latest_probe,
        operations,
    ))
}

fn recompute_alert_totals(response: &mut AlertsResponse) {
    response.alerts.sort_by(|a, b| {
        severity_rank(a.severity)
            .cmp(&severity_rank(b.severity))
            .then_with(|| a.resource_id.cmp(&b.resource_id))
            .then_with(|| a.kind.cmp(b.kind))
    });
    response.total_alerts = response.alerts.len();
    response.critical_alerts = response
        .alerts
        .iter()
        .filter(|alert| alert.severity == "critical")
        .count();
    response.warning_alerts = response
        .alerts
        .iter()
        .filter(|alert| alert.severity == "warning")
        .count();
    response.info_alerts = response
        .alerts
        .iter()
        .filter(|alert| alert.severity == "info")
        .count();
    response.operator_input_alerts = response
        .alerts
        .iter()
        .filter(|alert| alert.needs_operator_input)
        .count();
    response.blocking_alerts = response
        .alerts
        .iter()
        .filter(|alert| !alert.accepting_requests)
        .count();
}

pub(crate) fn model_route_all_target_suppression_alerts(
    state: &AppState,
) -> Vec<ManagementAlertStatus> {
    let routes = state.channels.model_routes_context();
    let mut alerts = Vec::new();
    for route_context in routes.routes {
        let public_model = route_context.route.public_model.clone();
        let plan_context = state.channels.route_plan_context(Some(&public_model));
        let preview = preview_route(RoutePreviewInput {
            request_id: format!("alert:{public_model}"),
            registry_generation: plan_context.registry_generation,
            public_model: Some(public_model.clone()),
            route: Some(&route_context.route),
            channel_states: &plan_context.model_route_channel_states,
            allowed_channels: &[],
            candidate_limit: state.routing.max_route_candidates,
        });
        let relevant_candidates = route_suppression_relevant_candidates(&preview.candidates);
        if relevant_candidates.is_empty()
            || preview
                .candidates
                .iter()
                .any(|candidate| candidate.included)
            || !relevant_candidates.iter().all(|candidate| {
                candidate
                    .reasons
                    .contains(&RoutePreviewReason::ChannelCoolingDown)
            })
        {
            continue;
        }

        let candidate_count = relevant_candidates.len();
        let channel_ids = relevant_candidates
            .iter()
            .map(|candidate| candidate.channel_id.0.clone())
            .collect();
        let reason_codes = relevant_candidates
            .iter()
            .flat_map(|candidate| candidate.reasons.iter().map(|reason| reason.as_str()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(str::to_string)
            .collect();
        alerts.push(ManagementAlertStatus {
            kind: "no_route_candidate",
            severity: "critical",
            resource_kind: "public_model",
            resource_type: "public_model",
            resource_id: public_model.clone(),
            public_model: Some(public_model),
            candidate_count: Some(candidate_count),
            suppressed_count: Some(candidate_count),
            rule_id: None,
            redact_count: None,
            reject_count: None,
            window_seconds: None,
            channel_ids,
            reason_codes,
            message: "public model route has no selectable candidates because all targets are cooling down or suppressed",
            serving_mode: "stopped",
            accepting_requests: false,
            needs_operator_input: true,
            required_action: "wait_for_cooldown_or_restore_route_capacity",
            credentials: RuntimeCredentialCounts::default(),
        });
    }
    alerts
}

fn route_suppression_relevant_candidates(
    candidates: &[RoutePreviewCandidate],
) -> Vec<&RoutePreviewCandidate> {
    candidates
        .iter()
        .filter(|candidate| {
            !candidate.reasons.iter().any(|reason| {
                matches!(
                    reason,
                    RoutePreviewReason::TargetDisabled
                        | RoutePreviewReason::ClientChannelScope
                        | RoutePreviewReason::ChannelDisabled
                        | RoutePreviewReason::UnknownChannel
                        | RoutePreviewReason::CandidateLimit
                )
            })
        })
        .collect()
}

const RESPONSE_FILTER_CONTAMINATION_THRESHOLD: usize = 3;

#[derive(Debug, Default)]
struct ResponseFilterContaminationBucket {
    channel_id: String,
    rule_id: String,
    redact_count: usize,
    reject_count: usize,
    reason_codes: BTreeSet<String>,
}

impl ResponseFilterContaminationBucket {
    fn event_count(&self) -> usize {
        self.redact_count.saturating_add(self.reject_count)
    }
}

pub fn response_filter_contamination_alerts_for_state(
    state: &AppState,
) -> Vec<ManagementAlertStatus> {
    let window_seconds = state
        .response_filter_alert_window
        .read()
        .expect("response filter alert window lock poisoned")
        .as_secs();
    let now = current_unix_seconds();
    let snapshot = state
        .response_filter_events
        .lock()
        .expect("response filter events mutex poisoned")
        .snapshot();
    let mut buckets: BTreeMap<(String, String), ResponseFilterContaminationBucket> =
        BTreeMap::new();

    for event in snapshot {
        let Some(action_class) = response_filter_alert_action_class(&event.action) else {
            continue;
        };
        if now.saturating_sub(event.created_at_unix_seconds) > window_seconds {
            continue;
        }

        let key = (event.channel_id.clone(), event.rule_id.clone());
        let bucket = buckets
            .entry(key)
            .or_insert_with(|| ResponseFilterContaminationBucket {
                channel_id: event.channel_id.clone(),
                rule_id: event.rule_id.clone(),
                ..Default::default()
            });
        match action_class {
            "redact" => bucket.redact_count = bucket.redact_count.saturating_add(1),
            "reject" => bucket.reject_count = bucket.reject_count.saturating_add(1),
            _ => {}
        }
        if !event.reason_code.is_empty() {
            bucket.reason_codes.insert(event.reason_code);
        }
    }

    buckets
        .into_values()
        .filter(|bucket| bucket.event_count() >= RESPONSE_FILTER_CONTAMINATION_THRESHOLD)
        .map(|bucket| ManagementAlertStatus {
            kind: "response_filter_contamination",
            severity: "warning",
            resource_kind: "channel",
            resource_type: "channel",
            resource_id: bucket.channel_id.clone(),
            public_model: None,
            candidate_count: None,
            suppressed_count: None,
            rule_id: safe_alert_id(&bucket.rule_id),
            redact_count: Some(bucket.redact_count),
            reject_count: Some(bucket.reject_count),
            window_seconds: Some(window_seconds),
            channel_ids: vec![bucket.channel_id],
            reason_codes: bucket.reason_codes.into_iter().collect(),
            message: "response filter repeatedly matched upstream output for this channel",
            serving_mode: "serving_degraded",
            accepting_requests: true,
            needs_operator_input: true,
            required_action: "inspect_response_filter_contamination",
            credentials: RuntimeCredentialCounts::default(),
        })
        .collect()
}

fn safe_alert_id(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 128
        || value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
        || value.contains("://")
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("secret")
        || lower.contains("authorization")
        || lower.contains("bearer")
        || lower.contains("api_key")
        || lower.contains("apikey")
    {
        return None;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
        .then(|| value.to_string())
}

fn response_filter_alert_action_class(action: &str) -> Option<&'static str> {
    match action {
        "redact" => Some("redact"),
        "reject" | "reject_and_expire_credential" | "reject_and_cooldown_channel" => Some("reject"),
        _ => None,
    }
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
