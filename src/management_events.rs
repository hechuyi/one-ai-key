use serde::Serialize;

use crate::{
    events::{ManagementEvent, ManagementEventActor},
    management_status::redact_management_reason,
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
    pub kind: String,
    pub channel_id: String,
    pub credential_id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<ManagementEventActor>,
}

pub fn management_event_status(event: ManagementEvent) -> ManagementEventStatus {
    ManagementEventStatus {
        id: event.id,
        kind: event.kind,
        channel_id: event.channel_id,
        credential_id: event.credential_id,
        reason: redact_management_reason(&event.reason),
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
