use axum::{
    body::Body,
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use serde_json::json;

use crate::{config::hash_token, state::AppState};

#[derive(Debug, Clone)]
pub struct AuthorizedClient {
    pub id: String,
    pub name: String,
    pub allowed_model_groups: Vec<String>,
    pub allowed_channels: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AuthorizedManagementPrincipal {
    pub id: String,
    pub name: String,
    pub role: crate::config::ManagementRole,
}

pub fn authorize_client(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthorizedClient, Box<Response>> {
    let got = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if let Some(token) = got {
        let token_hash = hash_token(token);
        let client_tokens = state
            .client_tokens
            .read()
            .expect("client token registry lock poisoned");
        if let Some(client) = client_tokens
            .iter()
            .find(|client| client.enabled && client.token_hash == token_hash)
        {
            return Ok(AuthorizedClient {
                id: client.id.clone(),
                name: client.name.clone(),
                allowed_model_groups: client.allowed_model_groups.clone(),
                allowed_channels: client.allowed_channels.clone(),
            });
        }
    }

    Err(Box::new(json_error(
        StatusCode::UNAUTHORIZED,
        "invalid router api key",
    )))
}

pub fn authorize_management(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthorizedManagementPrincipal, Box<Response>> {
    let got = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if let Some(token) = got {
        let token_hash = hash_token(token);
        if let Some(principal) = state
            .management_principals
            .iter()
            .find(|principal| principal.enabled && principal.token_hash == token_hash)
        {
            return Ok(AuthorizedManagementPrincipal {
                id: principal.id.clone(),
                name: principal.name.clone(),
                role: principal.role,
            });
        }
    }

    Err(Box::new(json_error(
        StatusCode::UNAUTHORIZED,
        "invalid management api key",
    )))
}

pub fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    json_error_with_code(status, "key_pool_router_error", message)
}

pub fn json_error_with_code(
    status: StatusCode,
    code: impl Into<String>,
    message: impl Into<String>,
) -> Response {
    let body = json!({
        "error": {
            "message": message.into(),
            "type": "router_error",
            "code": code.into()
        }
    });
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("valid json error response")
}
