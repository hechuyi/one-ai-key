use serde::Serialize;

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
    alerts_response(&state.credential_store, operations_by_credential_set).await
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
    pub resource_type: &'static str,
    pub resource_id: String,
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
        resource_type: "credential_set",
        resource_id: projection.resource_id.to_string(),
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
