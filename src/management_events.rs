use serde::Serialize;

use crate::{
    events::{ManagementEvent, ManagementEventActor},
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct EventsResponse {
    pub total_events: usize,
    pub offset: usize,
    pub limit: usize,
    pub events: Vec<ManagementEventStatus>,
}

#[derive(Debug, Serialize)]
pub struct ManagementEventStatus {
    pub id: u64,
    pub created_at_unix_seconds: u64,
    pub kind: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub outcome: String,
    pub request_id: Option<String>,
    pub generation: Option<u64>,
    pub channel_id: String,
    pub credential_id: String,
    pub reason: String,
    pub reason_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<ManagementEventActor>,
}

pub fn management_event_status(event: ManagementEvent) -> ManagementEventStatus {
    let action = event.audit_action();
    let resource_type = event.audit_resource_type();
    let resource_id = event.audit_resource_id();
    let outcome = event.audit_outcome();
    let reason_code = event.audit_reason_code();
    ManagementEventStatus {
        id: event.id,
        created_at_unix_seconds: event.created_at_unix_seconds,
        kind: event.kind,
        action,
        resource_type,
        resource_id,
        outcome,
        request_id: event.request_id,
        generation: event.generation,
        channel_id: event.channel_id,
        credential_id: event.credential_id,
        reason: reason_code.clone(),
        reason_code,
        actor: event.actor,
    }
}

pub fn events_response(
    total_events: usize,
    offset: usize,
    limit: usize,
    events: Vec<ManagementEvent>,
) -> EventsResponse {
    EventsResponse {
        total_events,
        offset,
        limit,
        events: events.into_iter().map(management_event_status).collect(),
    }
}

pub async fn events_snapshot_response(
    state: &AppState,
    offset: usize,
    limit: usize,
) -> EventsResponse {
    let (total_events, events) = state.events.window(offset, limit).await;
    events_response(total_events, offset, limit, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_event_status_projects_legacy_records_without_freeform_reason_code() {
        let status = management_event_status(ManagementEvent {
            id: 7,
            kind: "credential_expired".to_string(),
            channel_id: "channel-a".to_string(),
            credential_id: "cred-a".to_string(),
            reason: "legacy operator note".to_string(),
            ..Default::default()
        });

        assert_eq!(status.action, "credential_expired");
        assert_eq!(status.resource_type, "credential");
        assert_eq!(status.resource_id, "cred-a");
        assert_eq!(status.outcome, "applied");
        assert_eq!(status.reason, "manual_expire");
        assert_eq!(status.reason_code, "manual_expire");
    }
}
