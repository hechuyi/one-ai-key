use std::collections::HashSet;

use serde::Serialize;

use crate::{
    client_token_store::{ClientTokenCreate, ClientTokenScopeUpdate},
    config::ResolvedClientToken,
    management_errors::{client_token_store_error, ManagementServiceError},
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct ClientTokensResponse {
    pub client_tokens: Vec<ClientTokenStatus>,
}

pub fn client_tokens_response(tokens: Vec<ResolvedClientToken>) -> ClientTokensResponse {
    let mut client_tokens: Vec<ClientTokenStatus> =
        tokens.iter().map(client_token_status).collect();
    client_tokens.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    ClientTokensResponse { client_tokens }
}

pub fn runtime_client_tokens_response(state: &AppState) -> ClientTokensResponse {
    let client_tokens_snapshot = state
        .client_tokens
        .read()
        .expect("client token registry lock poisoned")
        .clone();
    client_tokens_response(client_tokens_snapshot)
}

pub async fn create_client_token_response_for_state(
    state: &AppState,
    name: String,
    token: String,
    allowed_model_groups: Vec<String>,
    allowed_channels: Vec<String>,
) -> Result<ClientTokenMutationResponse, ManagementServiceError> {
    let create = client_token_create(state, name, token, allowed_model_groups, allowed_channels)?;
    let created = state
        .client_token_store
        .create_token(create)
        .await
        .map_err(client_token_store_error)?;
    replace_runtime_client_token(state, created.clone());
    Ok(client_token_mutation_response(&created))
}

pub async fn set_client_token_enabled_response_for_state(
    state: &AppState,
    token_id: &str,
    enabled: bool,
) -> Result<ClientTokenMutationResponse, ManagementServiceError> {
    let updated = state
        .client_token_store
        .set_enabled(token_id.to_string(), enabled)
        .await
        .map_err(client_token_store_error)?;
    replace_runtime_client_token(state, updated.clone());
    Ok(client_token_mutation_response(&updated))
}

pub async fn update_client_token_scope_response_for_state(
    state: &AppState,
    token_id: &str,
    allowed_model_groups: Option<Vec<String>>,
    allowed_channels: Option<Vec<String>>,
) -> Result<ClientTokenMutationResponse, ManagementServiceError> {
    let scope_update = client_token_scope_update(state, allowed_model_groups, allowed_channels)?;
    let updated = state
        .client_token_store
        .update_scope(token_id.to_string(), scope_update)
        .await
        .map_err(client_token_store_error)?;
    replace_runtime_client_token(state, updated.clone());
    Ok(client_token_mutation_response(&updated))
}

pub fn replace_runtime_client_token(state: &AppState, updated: ResolvedClientToken) {
    let mut tokens = state
        .client_tokens
        .write()
        .expect("client token registry lock poisoned");
    match tokens.iter_mut().find(|token| token.id == updated.id) {
        Some(existing) => *existing = updated.clone(),
        None => tokens.push(updated),
    }
    tokens.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
}

#[derive(Debug, Serialize)]
pub struct ClientTokenStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub unrestricted_model_groups: bool,
    pub unrestricted_channels: bool,
    pub allowed_model_groups: Vec<String>,
    pub allowed_channels: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ClientTokenMutationResponse {
    pub token: ClientTokenStatus,
}

pub fn client_token_mutation_response(token: &ResolvedClientToken) -> ClientTokenMutationResponse {
    ClientTokenMutationResponse {
        token: client_token_status(token),
    }
}

pub fn client_token_status(token: &ResolvedClientToken) -> ClientTokenStatus {
    ClientTokenStatus {
        id: token.id.clone(),
        name: token.name.clone(),
        enabled: token.enabled,
        unrestricted_model_groups: token.allowed_model_groups.is_empty(),
        unrestricted_channels: token.allowed_channels.is_empty(),
        allowed_model_groups: token.allowed_model_groups.clone(),
        allowed_channels: token.allowed_channels.clone(),
    }
}

pub fn normalize_unique_non_empty(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() && seen.insert(value.to_string()) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

pub fn normalize_allowed_channels(
    state: &AppState,
    allowed_channels: Vec<String>,
) -> Result<Vec<String>, ManagementServiceError> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for channel in allowed_channels {
        let channel = channel.trim();
        if channel.is_empty() {
            continue;
        }
        if state.channels.get(channel).is_none() {
            return Err(ManagementServiceError::NotFound(format!(
                "unknown channel {channel}"
            )));
        }
        if seen.insert(channel.to_string()) {
            normalized.push(channel.to_string());
        }
    }
    Ok(normalized)
}

pub fn client_token_create(
    state: &AppState,
    name: String,
    token: String,
    allowed_model_groups: Vec<String>,
    allowed_channels: Vec<String>,
) -> Result<ClientTokenCreate, ManagementServiceError> {
    Ok(ClientTokenCreate {
        name,
        token,
        allowed_model_groups: normalize_unique_non_empty(allowed_model_groups),
        allowed_channels: normalize_allowed_channels(state, allowed_channels)?,
    })
}

pub fn client_token_scope_update(
    state: &AppState,
    allowed_model_groups: Option<Vec<String>>,
    allowed_channels: Option<Vec<String>>,
) -> Result<ClientTokenScopeUpdate, ManagementServiceError> {
    let allowed_model_groups = allowed_model_groups.map(normalize_unique_non_empty);
    let allowed_channels = match allowed_channels {
        Some(allowed_channels) => Some(normalize_allowed_channels(state, allowed_channels)?),
        None => None,
    };
    if allowed_model_groups.is_none() && allowed_channels.is_none() {
        return Err(ManagementServiceError::Conflict(
            "client token scope update must include allowed_model_groups or allowed_channels"
                .to_string(),
        ));
    }
    Ok(ClientTokenScopeUpdate {
        allowed_model_groups,
        allowed_channels,
    })
}
