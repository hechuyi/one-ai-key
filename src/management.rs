use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

use std::time::Duration;

use crate::{
    auth::{authorize_management, json_error, AuthorizedManagementPrincipal},
    config::{
        AccountConfig, ManagementRole, ModelRouteConfig, PolicyProfileConfig, PoolConfig,
        ProviderConfig, RoutingProfileConfig,
    },
    credential_probe::{CredentialProbeCommand, CredentialProbeKind},
    credentials::CredentialId,
    events::ManagementEventActor,
    management_alerts::alerts_response_for_state,
    management_client_tokens::{
        create_client_token_response_for_state, runtime_client_tokens_response,
        set_client_token_enabled_response_for_state, update_client_token_scope_response_for_state,
    },
    management_commands::{
        clear_credential_cooldown_response_for_channel, clear_credential_cooldown_response_for_set,
        disable_credential_response_for_channel, disable_credential_response_for_set,
        enable_credential_response_for_channel, enable_credential_response_for_set,
        expire_credential_response_for_channel, expire_credential_response_for_set,
        quota_exhaust_credential_response_for_channel, quota_exhaust_credential_response_for_set,
        restore_credential_response_for_channel, restore_credential_response_for_set,
    },
    management_credential_refs::resolve_credential_path_segment,
    management_credentials::{
        apply_latest_credential_probe_response_for_set,
        apply_latest_credential_probes_response_for_set, credential_import_for_existing_set,
        credential_import_response_for_set, credential_imports_for_existing_set,
        credential_lifecycle_history_response_for_set,
        credential_operator_metadata_response_for_set,
        credential_probe_apply_plan_response_for_set, credential_probe_results_response_for_set,
        credential_resource_response_for_set, credential_set_credentials_response_for_set_page,
        credentials_response_for_channel_page, probe_credential_response_for_command,
    },
    management_errors::ManagementServiceError,
    management_events::events_snapshot_response,
    management_operations::credential_set_operations_for_set,
    management_profiles::{
        channel_error_rules_response, policy_profile_response, policy_profiles_response,
        routing_profile_response, routing_profiles_response,
    },
    management_registry::{
        registry_account_enabled_response_for_state, registry_account_upsert_response_for_state,
        registry_channel_enabled_response_for_state, registry_channel_upsert_response_for_state,
        registry_model_route_upsert_response_for_state,
        registry_policy_profile_upsert_response_for_state,
        registry_provider_enabled_response_for_state, registry_provider_upsert_response_for_state,
        registry_routing_profile_upsert_response_for_state,
    },
    management_requests::{
        ApplyLatestProbeRequest, ChannelHealthMutationRequest, CreateClientTokenRequest,
        CredentialsQuery, EventsQuery, ExpireCredentialRequest,
        ImportCredentialSetCredentialsRequest, ModelDiscoverySyncPlanRequest,
        ProbeCredentialKindRequest, ProbeCredentialRequest, RoutingPreviewQuery,
        RuntimeReloadQuery, SetCredentialMetadataRequest, UpdateClientTokenScopeRequest,
    },
    management_resources::{
        accounts_response, credential_sets_response_for_state, disable_channel_resource,
        enable_channel_resource, pool_status_for_channel, pools_response, providers_response,
        reset_channel_health_status,
    },
    management_routing::{
        endpoint_family_availability_explain, model_routes_response, routing_preview_for_model,
    },
    management_runtime::{
        readiness_response, reload_runtime as reload_runtime_state, resilience_health_response,
        response_filter_events_snapshot_response, routing_telemetry_snapshot_response,
        runtime_explain_response_for_state, runtime_response_for_state, serving_health_response,
    },
    management_runtime_diff::runtime_reload_diff_response_for_state,
    model_discovery,
    state::AppState,
};

const MAX_CHAT_PROBE_EXPECTED_OUTPUT_CHARS: usize = 256;

#[derive(Debug, Deserialize)]
pub struct ModelAvailabilityQuery {
    model: String,
    endpoint_family: String,
    client_token_ref: Option<String>,
    client_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModelRouteUpsertQuery {
    expected_staged_registry_version: Option<u64>,
}

async fn credential_id_from_path_segment_response(
    state: &AppState,
    credential_set_id: &str,
    path_segment: String,
) -> Result<CredentialId, Response> {
    resolve_credential_path_segment(&state.credential_store, credential_set_id, path_segment)
        .await
        .map_err(service_error)
}

pub async fn list_pools(State(state): State<AppState>, headers: HeaderMap) -> Response {
    list_channel_statuses(state, headers).await
}

pub async fn list_channels(State(state): State<AppState>, headers: HeaderMap) -> Response {
    list_channel_statuses(state, headers).await
}

async fn list_channel_statuses(state: AppState, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);

    Json(pools_response(&state).await).into_response()
}

pub async fn get_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match pool_status_for_channel(&state, &id).await {
        Ok(status) => Json(status).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn discover_channel_models(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match model_discovery::discover_channel_models(&state, &id).await {
        Ok(status) => Json(status).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn model_discovery_sync_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ModelDiscoverySyncPlanRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match model_discovery::model_discovery_sync_plan(&state, payload.channel_ids).await {
        Ok(plan) => Json(plan).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn model_discovery_sync_apply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ModelDiscoverySyncPlanRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    match model_discovery::model_discovery_sync_apply(
        &state,
        management_actor(&principal),
        payload.channel_ids,
    )
    .await
    {
        Ok(applied) => Json(applied).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn reset_channel_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match reset_channel_health_status(&state, actor, &id).await {
        Ok(status) => Json(status).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn disable_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ChannelHealthMutationRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match disable_channel_resource(
        &state,
        actor,
        &id,
        request.reason_or("disabled by management"),
    )
    .await
    {
        Ok(status) => Json(status).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn enable_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ChannelHealthMutationRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match enable_channel_resource(
        &state,
        actor,
        &id,
        request.reason_or("enabled by management"),
    )
    .await
    {
        Ok(status) => Json(status).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn list_channel_credentials(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<CredentialsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let state_filter = match query.state_filter() {
        Ok(filter) => filter,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, message),
    };
    match credentials_response_for_channel_page(
        &state,
        &id,
        query.offset_or_zero(),
        query.bounded_limit(usize::MAX, 1000),
        state_filter,
    )
    .await
    {
        Ok(credentials) => Json(credentials).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn list_client_tokens(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    Json(runtime_client_tokens_response(&state)).into_response()
}

pub async fn create_client_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateClientTokenRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _actor = management_actor(&principal);
    if payload.name.trim().is_empty() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "client token name must not be empty",
        );
    }
    if payload.token.trim().is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "client token must not be empty");
    }
    match create_client_token_response_for_state(
        &state,
        _actor,
        payload.name,
        payload.token,
        payload.allowed_model_groups,
        payload.allowed_channels,
    )
    .await
    {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn update_client_token_scope(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<UpdateClientTokenScopeRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _actor = management_actor(&principal);
    if payload.allowed_model_groups.is_none() && payload.allowed_channels.is_none() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "client token scope update must include allowed_model_groups or allowed_channels",
        );
    }
    match update_client_token_scope_response_for_state(
        &state,
        _actor,
        &id,
        payload.allowed_model_groups,
        payload.allowed_channels,
    )
    .await
    {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn disable_client_token(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    _request: Option<Json<ChannelHealthMutationRequest>>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _actor = management_actor(&principal);
    match set_client_token_enabled_response_for_state(&state, _actor, &id, false).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn enable_client_token(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    _request: Option<Json<ChannelHealthMutationRequest>>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _actor = management_actor(&principal);
    match set_client_token_enabled_response_for_state(&state, _actor, &id, true).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn providers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    Json(providers_response(&state).await).into_response()
}

pub async fn upsert_registry_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ProviderConfig>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_provider_upsert_response_for_state(&state, actor, &id, payload).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn disable_registry_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_provider_enabled_response_for_state(&state, actor, &id, false).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn enable_registry_provider(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_provider_enabled_response_for_state(&state, actor, &id, true).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn accounts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    Json(accounts_response(&state).await).into_response()
}

pub async fn upsert_registry_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<AccountConfig>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_account_upsert_response_for_state(&state, actor, &id, payload).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn disable_registry_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_account_enabled_response_for_state(&state, actor, &id, false).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn enable_registry_account(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_account_enabled_response_for_state(&state, actor, &id, true).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn disable_registry_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_channel_enabled_response_for_state(&state, actor, &id, false).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn upsert_registry_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<PoolConfig>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_channel_upsert_response_for_state(&state, actor, &id, payload).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn enable_registry_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_channel_enabled_response_for_state(&state, actor, &id, true).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn credential_sets(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    Json(credential_sets_response_for_state(&state).await).into_response()
}

pub async fn credential_set_operations(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match credential_set_operations_for_set(&state, &id).await {
        Ok(status) => Json(status).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn list_credential_set_credentials(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<CredentialsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let state_filter = match query.state_filter() {
        Ok(filter) => filter,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, message),
    };
    let probe_filter = match query.probe_filter() {
        Ok(filter) => filter,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, message),
    };
    match credential_set_credentials_response_for_set_page(
        &state,
        &id,
        query.offset_or_zero(),
        query.bounded_limit(usize::MAX, 1000),
        state_filter,
        probe_filter,
    )
    .await
    {
        Ok(credentials) => Json(credentials).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn credential_set_imports(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<CredentialsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match credential_imports_for_existing_set(
        &state,
        &id,
        query.offset_or_zero(),
        query.bounded_limit(usize::MAX, 1000),
    )
    .await
    {
        Ok(imports) => Json(imports).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn credential_set_import(
    State(state): State<AppState>,
    Path((id, batch_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match credential_import_for_existing_set(&state, &id, &batch_id).await {
        Ok(import) => Json(import).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn credential_set_credential(
    State(state): State<AppState>,
    Path((id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let credential_id =
        match credential_id_from_path_segment_response(&state, &id, credential_id).await {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match credential_resource_response_for_set(&state, &id, credential_id).await {
        Ok(resource) => Json(resource).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn set_credential_set_credential_metadata(
    State(state): State<AppState>,
    Path((id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<SetCredentialMetadataRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let credential_id =
        match credential_id_from_path_segment_response(&state, &id, credential_id).await {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match credential_operator_metadata_response_for_set(
        &state,
        actor,
        &id,
        credential_id,
        request.label,
        request.note,
    )
    .await
    {
        Ok(resource) => Json(resource).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn credential_set_credential_history(
    State(state): State<AppState>,
    Path((id, credential_id)): Path<(String, String)>,
    Query(query): Query<CredentialsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let credential_id =
        match credential_id_from_path_segment_response(&state, &id, credential_id).await {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match credential_lifecycle_history_response_for_set(
        &state,
        &id,
        credential_id,
        query.offset_or_zero(),
        query.bounded_limit(usize::MAX, 1000),
    )
    .await
    {
        Ok(history) => Json(history).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn probe_credential_set_credential(
    State(state): State<AppState>,
    Path((id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<ProbeCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let credential_id =
        match credential_id_from_path_segment_response(&state, &id, credential_id).await {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    let model = payload.model.trim().to_string();
    if model.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "model must not be empty");
    }
    let kind = match payload
        .kind
        .unwrap_or(ProbeCredentialKindRequest::ModelRetrieve)
    {
        ProbeCredentialKindRequest::ModelRetrieve => CredentialProbeKind::ModelRetrieve,
        ProbeCredentialKindRequest::ChatCompletion => CredentialProbeKind::ChatCompletion,
    };
    let expected_output = payload
        .expected_output
        .map(|value| value.trim().to_string());
    if matches!(kind, CredentialProbeKind::ChatCompletion)
        && expected_output
            .as_ref()
            .is_some_and(|value| value.is_empty())
    {
        return json_error(StatusCode::BAD_REQUEST, "expected_output must not be empty");
    }
    if expected_output
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_CHAT_PROBE_EXPECTED_OUTPUT_CHARS)
    {
        return json_error(
            StatusCode::BAD_REQUEST,
            format!(
                "expected_output must be at most {MAX_CHAT_PROBE_EXPECTED_OUTPUT_CHARS} characters"
            ),
        );
    }
    let timeout = Duration::from_secs(payload.timeout_seconds.unwrap_or(10).clamp(1, 30));
    match probe_credential_response_for_command(
        &state,
        actor,
        CredentialProbeCommand {
            credential_set_id: id,
            credential_id,
            model,
            kind,
            expected_output,
            timeout,
        },
    )
    .await
    {
        Ok(result) => Json(result).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn credential_set_credential_probes(
    State(state): State<AppState>,
    Path((id, credential_id)): Path<(String, String)>,
    Query(query): Query<CredentialsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let credential_id =
        match credential_id_from_path_segment_response(&state, &id, credential_id).await {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match credential_probe_results_response_for_set(
        &state,
        &id,
        credential_id,
        query.offset_or_zero(),
        query.bounded_limit(usize::MAX, 1000),
    )
    .await
    {
        Ok(history) => Json(history).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn credential_probe_apply_plan(
    State(state): State<AppState>,
    Path((id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let credential_id =
        match credential_id_from_path_segment_response(&state, &id, credential_id).await {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match credential_probe_apply_plan_response_for_set(&state, &id, credential_id).await {
        Ok(plan) => Json(plan).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn apply_latest_credential_probe(
    State(state): State<AppState>,
    Path((id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<ApplyLatestProbeRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let reason = payload
        .reason
        .unwrap_or_else(|| "apply latest credential probe".to_string());
    let credential_id =
        match credential_id_from_path_segment_response(&state, &id, credential_id).await {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match apply_latest_credential_probe_response_for_set(
        &state,
        management_actor(&principal),
        &id,
        credential_id,
        payload.probe_result_ref,
        true,
        reason,
    )
    .await
    {
        Ok(applied) => Json(applied).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn apply_latest_credential_probes(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<CredentialsQuery>,
    headers: HeaderMap,
    Json(payload): Json<ApplyLatestProbeRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let Some(_) = query.probe.as_ref() else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "bulk apply latest probe requires probe filter",
        );
    };
    let probe_filter = match query.probe_filter() {
        Ok(filter) => filter,
        Err(message) => return json_error(StatusCode::BAD_REQUEST, message),
    };
    let reason = payload
        .reason
        .unwrap_or_else(|| "bulk apply latest credential probes".to_string());
    match apply_latest_credential_probes_response_for_set(
        &state,
        management_actor(&principal),
        &id,
        probe_filter,
        query.limit.unwrap_or(100).clamp(1, 100),
        reason,
    )
    .await
    {
        Ok(applied) => Json(applied).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn import_credential_set_credentials(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ImportCredentialSetCredentialsRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    if payload.keys.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "keys must not be empty");
    }
    match credential_import_response_for_set(&state, actor, &id, payload.keys, payload.batch_id)
        .await
    {
        Ok(imported) => Json(imported).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn model_routes(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    Json(model_routes_response(&state).await).into_response()
}

pub async fn model_availability(
    State(state): State<AppState>,
    Query(query): Query<ModelAvailabilityQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);

    let model = query.model.trim();
    if model.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "model must not be empty");
    }
    let endpoint_family = query.endpoint_family.trim();
    if endpoint_family.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "endpoint_family must not be empty");
    }
    let client_token_ref = query
        .client_token_ref
        .as_deref()
        .or(query.client_token.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    Json(
        endpoint_family_availability_explain(&state, client_token_ref, endpoint_family, model)
            .await,
    )
    .into_response()
}

pub async fn upsert_registry_model_route(
    State(state): State<AppState>,
    Path(public_model): Path<String>,
    Query(query): Query<ModelRouteUpsertQuery>,
    headers: HeaderMap,
    Json(payload): Json<ModelRouteConfig>,
) -> Response {
    let public_model = public_model.trim_start_matches('/');
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let Some(expected_staged_registry_version) = query.expected_staged_registry_version else {
        return service_error(ManagementServiceError::Conflict(
            "expected_staged_registry_version is required for model route upsert".to_string(),
        ));
    };
    match registry_model_route_upsert_response_for_state(
        &state,
        actor,
        public_model,
        expected_staged_registry_version,
        payload,
    )
    .await
    {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn policy_profiles(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    Json(policy_profiles_response(&state)).into_response()
}

pub async fn upsert_registry_policy_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<PolicyProfileConfig>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_policy_profile_upsert_response_for_state(&state, actor, &id, payload).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn policy_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match policy_profile_response(&state, &id) {
        Ok(profile) => Json(profile).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn routing_profiles(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    Json(routing_profiles_response(&state)).into_response()
}

pub async fn upsert_registry_routing_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<RoutingProfileConfig>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    match registry_routing_profile_upsert_response_for_state(&state, actor, &id, payload).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn routing_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match routing_profile_response(&state, &id) {
        Ok(profile) => Json(profile).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn routing_preview(
    State(state): State<AppState>,
    Query(query): Query<RoutingPreviewQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match routing_preview_for_model(&state, &query.model, query.client_token.as_deref()).await {
        Ok(preview) => Json(preview).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn error_rules(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match channel_error_rules_response(&state, &id) {
        Ok(rules) => Json(rules).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn expire_channel_credential(
    State(state): State<AppState>,
    Path((channel_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual management action");
    match expire_credential_response_for_channel(
        &state,
        actor,
        &channel_id,
        CredentialId(credential_id),
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn restore_channel_credential(
    State(state): State<AppState>,
    Path((channel_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual management action");
    match restore_credential_response_for_channel(
        &state,
        actor,
        &channel_id,
        CredentialId(credential_id),
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn quota_exhaust_channel_credential(
    State(state): State<AppState>,
    Path((channel_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual quota exhaust");
    match quota_exhaust_credential_response_for_channel(
        &state,
        actor,
        &channel_id,
        CredentialId(credential_id),
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn reset_channel_credential_cooldown(
    State(state): State<AppState>,
    Path((channel_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual cooldown reset");
    match clear_credential_cooldown_response_for_channel(
        &state,
        actor,
        &channel_id,
        CredentialId(credential_id),
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn disable_channel_credential(
    State(state): State<AppState>,
    Path((channel_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual disable");
    match disable_credential_response_for_channel(
        &state,
        actor,
        &channel_id,
        CredentialId(credential_id),
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn enable_channel_credential(
    State(state): State<AppState>,
    Path((channel_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual enable");
    match enable_credential_response_for_channel(
        &state,
        actor,
        &channel_id,
        CredentialId(credential_id),
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn expire_credential_set_credential(
    State(state): State<AppState>,
    Path((credential_set_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual management action");
    let credential_id =
        match credential_id_from_path_segment_response(&state, &credential_set_id, credential_id)
            .await
        {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match expire_credential_response_for_set(
        &state,
        actor,
        &credential_set_id,
        credential_id,
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn restore_credential_set_credential(
    State(state): State<AppState>,
    Path((credential_set_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual management action");
    let credential_id =
        match credential_id_from_path_segment_response(&state, &credential_set_id, credential_id)
            .await
        {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match restore_credential_response_for_set(
        &state,
        actor,
        &credential_set_id,
        credential_id,
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn quota_exhaust_credential_set_credential(
    State(state): State<AppState>,
    Path((credential_set_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual quota exhaust");
    let credential_id =
        match credential_id_from_path_segment_response(&state, &credential_set_id, credential_id)
            .await
        {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match quota_exhaust_credential_response_for_set(
        &state,
        actor,
        &credential_set_id,
        credential_id,
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn reset_credential_set_credential_cooldown(
    State(state): State<AppState>,
    Path((credential_set_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual cooldown reset");
    let credential_id =
        match credential_id_from_path_segment_response(&state, &credential_set_id, credential_id)
            .await
        {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match clear_credential_cooldown_response_for_set(
        &state,
        actor,
        &credential_set_id,
        credential_id,
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn disable_credential_set_credential(
    State(state): State<AppState>,
    Path((credential_set_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual disable");
    let credential_id =
        match credential_id_from_path_segment_response(&state, &credential_set_id, credential_id)
            .await
        {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match disable_credential_response_for_set(
        &state,
        actor,
        &credential_set_id,
        credential_id,
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn enable_credential_set_credential(
    State(state): State<AppState>,
    Path((credential_set_id, credential_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ExpireCredentialRequest>,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);
    let reason = request.reason_or("manual enable");
    let credential_id =
        match credential_id_from_path_segment_response(&state, &credential_set_id, credential_id)
            .await
        {
            Ok(credential_id) => credential_id,
            Err(resp) => return resp,
        };
    match enable_credential_response_for_set(
        &state,
        actor,
        &credential_set_id,
        credential_id,
        reason,
    )
    .await
    {
        Ok(mutation) => Json(mutation).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let limit = query.bounded_limit(100, 1000);
    let offset = query.offset_or_zero();
    Json(events_snapshot_response(&state, offset, limit).await).into_response()
}

pub async fn alerts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match alerts_response_for_state(&state).await {
        Ok(alerts) => Json(alerts).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn readiness(State(state): State<AppState>) -> Response {
    let readiness = readiness_response(&state).await;
    if readiness.serving_channels == 0 {
        (StatusCode::SERVICE_UNAVAILABLE, Json(readiness)).into_response()
    } else {
        Json(readiness).into_response()
    }
}

pub async fn serving_health(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);

    let response = serving_health_response(&state).await;
    if response.serving {
        Json(response).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(response)).into_response()
    }
}

pub async fn resilience_health(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);

    Json(resilience_health_response(&state).await).into_response()
}

pub async fn routing_telemetry(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let limit = query.bounded_limit(100, 1000);
    let offset = query.offset_or_zero();
    Json(routing_telemetry_snapshot_response(&state, offset, limit)).into_response()
}

pub async fn response_filter_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    let limit = query.bounded_limit(100, 1000);
    let offset = query.offset_or_zero();
    Json(response_filter_events_snapshot_response(
        &state, offset, limit,
    ))
    .into_response()
}

pub async fn runtime(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);

    match runtime_response_for_state(&state).await {
        Ok(runtime) => Json(runtime).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn explain_runtime(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);

    match runtime_explain_response_for_state(&state).await {
        Ok(runtime) => Json(runtime).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn runtime_reload_diff(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let _principal_context = (&principal.id, &principal.name, &principal.role);
    match runtime_reload_diff_response_for_state(&state).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => service_error(err),
    }
}

pub async fn reload_runtime(
    State(state): State<AppState>,
    Query(query): Query<RuntimeReloadQuery>,
    headers: HeaderMap,
) -> Response {
    let principal = match authorize_management(&state, &headers) {
        Ok(principal) => principal,
        Err(resp) => return *resp,
    };
    let actor = management_actor(&principal);

    match reload_runtime_state(&state, actor, query.expected_staged_registry_version).await {
        Ok(runtime) => Json(runtime).into_response(),
        Err(err) => service_error(err),
    }
}

fn service_error(err: ManagementServiceError) -> Response {
    match err {
        ManagementServiceError::BadRequest(message) => json_error(StatusCode::BAD_REQUEST, message),
        ManagementServiceError::NotFound(message) => json_error(StatusCode::NOT_FOUND, message),
        ManagementServiceError::Conflict(message) => json_error(StatusCode::CONFLICT, message),
        ManagementServiceError::PreconditionFailed(message) => {
            json_error(StatusCode::CONFLICT, message)
        }
        ManagementServiceError::Persistence(message) => {
            json_error(StatusCode::INTERNAL_SERVER_ERROR, message)
        }
        ManagementServiceError::EventAppendFailed { .. } => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to record management event".to_string(),
        ),
    }
}

fn management_actor(principal: &AuthorizedManagementPrincipal) -> ManagementEventActor {
    ManagementEventActor {
        id: principal.id.clone(),
        name: principal.name.clone(),
        role: match principal.role {
            ManagementRole::Readonly => "readonly".to_string(),
            ManagementRole::Operator => "operator".to_string(),
            ManagementRole::Admin => "admin".to_string(),
        },
    }
}
