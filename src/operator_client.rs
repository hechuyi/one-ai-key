#![allow(dead_code)]

use reqwest::{StatusCode, Url};
use serde::Serialize;
use serde_json::Value;
use std::{
    fmt,
    io::{IsTerminal, Read},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagementTokenSource {
    Env(String),
    Stdin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorClientOptions {
    pub management_url: Option<String>,
    pub deprecated_base_url: Option<String>,
    pub token_source: ManagementTokenSource,
    pub timeout_seconds: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ManagementToken(String);

impl ManagementToken {
    pub fn from_env<F>(env_name: &str, get_env: F) -> Result<Self, OperatorClientError>
    where
        F: FnOnce(&str) -> Option<String>,
    {
        let value = get_env(env_name).ok_or_else(|| {
            OperatorClientError::new(
                "management_token_missing",
                format!("management token env var {env_name} is not set"),
            )
        })?;
        Self::from_string(value, "management_token_empty")
    }

    pub fn from_stdin() -> Result<Self, OperatorClientError> {
        if std::io::stdin().is_terminal() {
            return Err(OperatorClientError::new(
                "management_token_stdin_requires_pipe",
                "management token stdin source requires piped input",
            ));
        }
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes).map_err(|_| {
            OperatorClientError::new(
                "management_token_stdin_error",
                "failed to read management token",
            )
        })?;
        Self::from_stdin_bytes(&bytes)
    }

    pub fn from_stdin_reader<R: Read>(
        mut reader: R,
        stdin_is_tty: bool,
    ) -> Result<Self, OperatorClientError> {
        if stdin_is_tty {
            return Err(OperatorClientError::new(
                "management_token_stdin_requires_pipe",
                "management token stdin source requires piped input",
            ));
        }
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|_| {
            OperatorClientError::new(
                "management_token_stdin_error",
                "failed to read management token",
            )
        })?;
        Self::from_stdin_bytes(&bytes)
    }

    pub fn from_stdin_bytes(bytes: &[u8]) -> Result<Self, OperatorClientError> {
        let value = String::from_utf8(bytes.to_vec()).map_err(|_| {
            OperatorClientError::new(
                "management_token_stdin_error",
                "management token from stdin must be utf-8",
            )
        })?;
        Self::from_string(value, "management_token_empty")
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn from_string(value: String, empty_reason: &'static str) -> Result<Self, OperatorClientError> {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            return Err(OperatorClientError::new(
                empty_reason,
                "management token is empty",
            ));
        }
        Ok(Self(trimmed))
    }
}

impl fmt::Debug for ManagementToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ManagementToken")
            .field(&"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct OperatorClientConfig {
    pub base_url: Url,
    pub bearer_token: ManagementToken,
    pub timeout_seconds: u64,
}

impl OperatorClientConfig {
    pub fn from_options<F>(
        options: OperatorClientOptions,
        get_env: F,
    ) -> Result<Self, OperatorClientError>
    where
        F: FnOnce(&str) -> Option<String>,
    {
        let base_url = resolve_management_url(
            options.management_url.as_deref(),
            options.deprecated_base_url.as_deref(),
        )?;
        let bearer_token = match options.token_source {
            ManagementTokenSource::Env(env_name) => ManagementToken::from_env(&env_name, get_env)?,
            ManagementTokenSource::Stdin => ManagementToken::from_stdin()?,
        };
        Ok(Self {
            base_url,
            bearer_token,
            timeout_seconds: options.timeout_seconds,
        })
    }
}

pub struct OperatorClient {
    config: OperatorClientConfig,
    http: reqwest::Client,
}

impl OperatorClient {
    pub fn new(config: OperatorClientConfig) -> Result<Self, OperatorClientError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds.max(1)))
            .build()
            .map_err(|_| {
                OperatorClientError::new(
                    "management_client_build_failed",
                    "failed to build management API client",
                )
            })?;
        Ok(Self { config, http })
    }

    pub async fn get_json(&self, endpoint: ReadOnlyEndpoint) -> Result<Value, OperatorClientError> {
        let request = endpoint.request()?;
        self.get_json_request(request).await
    }

    pub async fn post_json<T: Serialize>(
        &self,
        endpoint: ManagementMutationEndpoint,
        body: &T,
    ) -> Result<Value, OperatorClientError> {
        let request = endpoint.request()?;
        let path = request.path.as_str();
        if !is_management_mutation_path(Method::Post, path) {
            return Err(OperatorClientError::new(
                "management_mutation_path_rejected",
                "management path is not in the mutation allowlist",
            ));
        }
        let mut url = self.config.base_url.clone();
        url.set_path(path);
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &request.query {
                pairs.append_pair(key, value);
            }
        }
        let response = self
            .http
            .post(url.clone())
            .bearer_auth(self.config.bearer_token.as_str())
            .json(body)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    OperatorClientError::timeout(url.as_str())
                } else {
                    OperatorClientError::new(
                        "management_transport_error",
                        "management API request failed",
                    )
                }
            })?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = response.bytes().await.map_err(|_| {
            OperatorClientError::new(
                "management_body_error",
                "failed to read management API body",
            )
        })?;
        if !request.accepts_status(status) {
            return Err(classify_http_error(status, content_type.as_deref(), &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            OperatorClientError::new(
                "management_non_json_error",
                "management API returned a non-json response",
            )
        })
    }

    async fn get_json_request(
        &self,
        request: EndpointRequest,
    ) -> Result<Value, OperatorClientError> {
        let path = request.path.as_str();
        if !is_readonly_management_path(Method::Get, path) {
            return Err(OperatorClientError::new(
                "management_readonly_path_rejected",
                "management path is not in the read-only allowlist",
            ));
        }
        let mut url = self.config.base_url.clone();
        url.set_path(path);
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &request.query {
                pairs.append_pair(key, value);
            }
        }
        let response = self
            .http
            .get(url.clone())
            .bearer_auth(self.config.bearer_token.as_str())
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    OperatorClientError::timeout(url.as_str())
                } else {
                    OperatorClientError::new(
                        "management_transport_error",
                        "management API request failed",
                    )
                }
            })?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = response.bytes().await.map_err(|_| {
            OperatorClientError::new(
                "management_body_error",
                "failed to read management API body",
            )
        })?;
        if !request.accepts_status(status) {
            return Err(classify_http_error(status, content_type.as_deref(), &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            OperatorClientError::new(
                "management_non_json_error",
                "management API returned a non-json response",
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOnlyEndpoint {
    RoutingPreview {
        model: String,
        client_token_ref: Option<String>,
    },
    ClientTokens,
    ModelRoutes,
    Channel {
        channel_id: String,
    },
    CredentialSets,
    CredentialSetOperations {
        credential_set_id: String,
    },
    CredentialSetCredentials {
        credential_set_id: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    CredentialSetCredentialProbeApplyPlan {
        credential_set_id: String,
        credential_ref: String,
    },
    RoutingTelemetry {
        offset: Option<usize>,
        limit: Option<usize>,
    },
    ResponseFilterEvents {
        offset: Option<usize>,
        limit: Option<usize>,
    },
    ExplainRuntime,
    Runtime,
    RuntimeReloadDiff,
    ServingHealth,
    ResilienceHealth,
    Alerts,
    Events {
        offset: Option<usize>,
        limit: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum ManagementMutationEndpoint {
    RuntimeReload {
        expected_staged_registry_version: u64,
    },
    CredentialSetCredentialsImport {
        credential_set_id: String,
    },
    CredentialSetCredentialProbe {
        credential_set_id: String,
        credential_ref: String,
    },
    CredentialSetCredentialProbeApply {
        credential_set_id: String,
        credential_ref: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EndpointRequest {
    path: String,
    query: Vec<(String, String)>,
    accepted_statuses: &'static [StatusCode],
}

const SUCCESS_STATUSES: &[StatusCode] = &[];
const SERVING_HEALTH_STATUSES: &[StatusCode] = &[StatusCode::OK, StatusCode::SERVICE_UNAVAILABLE];

impl EndpointRequest {
    fn accepts_status(&self, status: StatusCode) -> bool {
        if self.accepted_statuses.is_empty() {
            status.is_success()
        } else {
            self.accepted_statuses.contains(&status)
        }
    }
}

impl ReadOnlyEndpoint {
    #[cfg(test)]
    pub fn test_request_parts(
        &self,
    ) -> Result<(String, Vec<(String, String)>), OperatorClientError> {
        let request = self.request()?;
        Ok((request.path, request.query))
    }

    fn request(&self) -> Result<EndpointRequest, OperatorClientError> {
        let mut request = match self {
            Self::RoutingPreview {
                model,
                client_token_ref,
            } => {
                let mut query = vec![("model".to_string(), model.clone())];
                if let Some(client_token_ref) = client_token_ref {
                    query.push(("client_token".to_string(), client_token_ref.clone()));
                }
                EndpointRequest::new("/management/routing/preview", query)
            }
            Self::ClientTokens => EndpointRequest::new("/management/client-tokens", Vec::new()),
            Self::ModelRoutes => EndpointRequest::new("/management/model-routes", Vec::new()),
            Self::Channel { channel_id } => EndpointRequest::new(
                format!(
                    "/management/channels/{}",
                    safe_path_segment(channel_id, "channel_id")?
                ),
                Vec::new(),
            ),
            Self::CredentialSets => EndpointRequest::new("/management/credential-sets", Vec::new()),
            Self::CredentialSetOperations { credential_set_id } => EndpointRequest::new(
                credential_set_path(credential_set_id, "operations")?,
                Vec::new(),
            ),
            Self::CredentialSetCredentials {
                credential_set_id,
                offset,
                limit,
            } => EndpointRequest::new(
                credential_set_path(credential_set_id, "credentials")?,
                pagination_query(*offset, *limit),
            ),
            Self::CredentialSetCredentialProbeApplyPlan {
                credential_set_id,
                credential_ref,
            } => EndpointRequest::new(
                format!(
                    "{}/{}/apply-latest-probe/plan",
                    credential_set_path(credential_set_id, "credentials")?,
                    safe_credential_ref_path_segment(credential_ref)?
                ),
                Vec::new(),
            ),
            Self::RoutingTelemetry { offset, limit } => EndpointRequest::new(
                "/management/routing-telemetry",
                pagination_query(*offset, *limit),
            ),
            Self::ResponseFilterEvents { offset, limit } => EndpointRequest::new(
                "/management/response-filter-events",
                pagination_query(*offset, *limit),
            ),
            Self::ExplainRuntime => EndpointRequest::new("/management/explain/runtime", Vec::new()),
            Self::Runtime => EndpointRequest::new("/management/runtime", Vec::new()),
            Self::RuntimeReloadDiff => {
                EndpointRequest::new("/management/runtime/reload-diff", Vec::new())
            }
            Self::ServingHealth => EndpointRequest::with_statuses(
                "/management/health/serving",
                Vec::new(),
                SERVING_HEALTH_STATUSES,
            ),
            Self::ResilienceHealth => {
                EndpointRequest::new("/management/health/resilience", Vec::new())
            }
            Self::Alerts => EndpointRequest::new("/management/alerts", Vec::new()),
            Self::Events { offset, limit } => {
                EndpointRequest::new("/management/events", pagination_query(*offset, *limit))
            }
        };
        request.query.retain(|(_, value)| !value.is_empty());
        Ok(request)
    }
}

impl ManagementMutationEndpoint {
    #[cfg(test)]
    pub fn test_request_parts(
        &self,
    ) -> Result<(String, Vec<(String, String)>), OperatorClientError> {
        let request = self.request()?;
        Ok((request.path, request.query))
    }

    fn request(&self) -> Result<EndpointRequest, OperatorClientError> {
        let request = match self {
            Self::RuntimeReload {
                expected_staged_registry_version,
            } => EndpointRequest::new(
                "/management/runtime/reload",
                vec![(
                    "expected_staged_registry_version".to_string(),
                    expected_staged_registry_version.to_string(),
                )],
            ),
            Self::CredentialSetCredentialsImport { credential_set_id } => EndpointRequest::new(
                credential_set_path(credential_set_id, "credentials/import")?,
                Vec::new(),
            ),
            Self::CredentialSetCredentialProbe {
                credential_set_id,
                credential_ref,
            } => EndpointRequest::new(
                format!(
                    "{}/{}",
                    credential_set_path(credential_set_id, "credentials")?,
                    safe_credential_ref_path_segment(credential_ref)?
                ) + "/probe",
                Vec::new(),
            ),
            Self::CredentialSetCredentialProbeApply {
                credential_set_id,
                credential_ref,
            } => EndpointRequest::new(
                format!(
                    "{}/{}",
                    credential_set_path(credential_set_id, "credentials")?,
                    safe_credential_ref_path_segment(credential_ref)?
                ) + "/apply-latest-probe",
                Vec::new(),
            ),
        };
        Ok(request)
    }
}

impl EndpointRequest {
    fn new(path: impl Into<String>, query: Vec<(String, String)>) -> Self {
        Self::with_statuses(path, query, SUCCESS_STATUSES)
    }

    fn with_statuses(
        path: impl Into<String>,
        query: Vec<(String, String)>,
        accepted_statuses: &'static [StatusCode],
    ) -> Self {
        Self {
            path: path.into(),
            query,
            accepted_statuses,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorClientError {
    reason_code: &'static str,
    safe_message: String,
}

impl OperatorClientError {
    pub fn new(reason_code: &'static str, safe_message: impl Into<String>) -> Self {
        Self {
            reason_code,
            safe_message: safe_message.into(),
        }
    }

    pub fn timeout(_url: &str) -> Self {
        Self::new("management_timeout", "management API request timed out")
    }

    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

impl fmt::Display for OperatorClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.reason_code, self.safe_message)
    }
}

impl std::error::Error for OperatorClientError {}

pub fn normalize_management_url(input: &str) -> Result<Url, OperatorClientError> {
    let mut url = Url::parse(input).map_err(|_| {
        OperatorClientError::new("management_url_invalid", "management URL is invalid")
    })?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(OperatorClientError::new(
            "management_url_invalid_scheme",
            "management URL must use http or https",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OperatorClientError::new(
            "management_url_contains_credentials",
            "management URL must not contain credentials",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(OperatorClientError::new(
            "management_url_contains_token_components",
            "management URL must not contain query or fragment components",
        ));
    }

    let path = url.path().trim_end_matches('/');
    match path {
        "" | "/" | "/management" => {
            url.set_path("/");
            Ok(url)
        }
        "/v1" => Err(OperatorClientError::new(
            "client_base_url_used_for_management",
            "management URL must be an origin or /management base, not a client /v1 base",
        )),
        _ => Err(OperatorClientError::new(
            "management_url_unsupported_path",
            "management URL must be an origin or management base",
        )),
    }
}

pub fn resolve_management_url(
    management_url: Option<&str>,
    deprecated_base_url: Option<&str>,
) -> Result<Url, OperatorClientError> {
    match (management_url, deprecated_base_url) {
        (Some(primary), Some(alias)) => {
            let primary = normalize_management_url(primary)?;
            let alias = normalize_management_url(alias)?;
            if primary == alias {
                Ok(primary)
            } else {
                Err(OperatorClientError::new(
                    "management_url_alias_conflict",
                    "--base-url is a deprecated alias and must not conflict with --management-url",
                ))
            }
        }
        (Some(primary), None) => normalize_management_url(primary),
        (None, Some(alias)) => normalize_management_url(alias),
        (None, None) => Err(OperatorClientError::new(
            "management_url_missing",
            "management URL is required for operator commands",
        )),
    }
}

fn pagination_query(offset: Option<usize>, limit: Option<usize>) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(offset) = offset {
        query.push(("offset".to_string(), offset.to_string()));
    }
    if let Some(limit) = limit {
        query.push(("limit".to_string(), limit.to_string()));
    }
    query
}

fn credential_set_path(
    credential_set_id: &str,
    suffix: &'static str,
) -> Result<String, OperatorClientError> {
    Ok(format!(
        "/management/credential-sets/{}/{}",
        safe_path_segment(credential_set_id, "credential_set_id")?,
        suffix
    ))
}

fn safe_path_segment<'a>(
    segment: &'a str,
    field: &'static str,
) -> Result<&'a str, OperatorClientError> {
    if segment.is_empty()
        || segment == "."
        || segment == ".."
        || !segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(OperatorClientError::new(
            "management_path_segment_invalid",
            format!("{field} contains characters unsafe for a management path segment"),
        ));
    }
    Ok(segment)
}

fn safe_credential_ref_path_segment(segment: &str) -> Result<&str, OperatorClientError> {
    let Some(position) = segment.strip_prefix("cr:v1:pos:") else {
        return Err(OperatorClientError::new(
            "management_path_segment_invalid",
            "credential_ref must use the non-secret cr:v1:pos reference form",
        ));
    };
    if position.is_empty() || !position.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(OperatorClientError::new(
            "management_path_segment_invalid",
            "credential_ref must use the non-secret cr:v1:pos reference form",
        ));
    }
    Ok(segment)
}

pub fn classify_http_error(
    status: StatusCode,
    content_type: Option<&str>,
    _body: &[u8],
) -> OperatorClientError {
    match status {
        StatusCode::UNAUTHORIZED => {
            return OperatorClientError::new(
                "management_unauthorized",
                "management API rejected the token",
            )
        }
        StatusCode::FORBIDDEN => {
            return OperatorClientError::new(
                "management_forbidden",
                "management API principal lacks permission",
            )
        }
        StatusCode::NOT_FOUND => {
            return OperatorClientError::new(
                "management_not_found",
                "management API endpoint or resource was not found",
            )
        }
        _ => {}
    }
    if !is_json_content_type(content_type) {
        return OperatorClientError::new(
            "management_non_json_error",
            "management API returned a non-json error response",
        );
    }
    OperatorClientError::new(
        "management_http_error",
        "management API returned an error status",
    )
}

pub fn is_readonly_management_path(method: Method, path: &str) -> bool {
    if method != Method::Get {
        return false;
    }
    if !path.starts_with("/management/") && path != "/management/runtime" {
        return false;
    }
    matches!(
        path,
        "/management/routing/preview"
            | "/management/client-tokens"
            | "/management/model-routes"
            | "/management/channels"
            | "/management/credential-sets"
            | "/management/routing-telemetry"
            | "/management/response-filter-events"
            | "/management/explain/runtime"
            | "/management/runtime"
            | "/management/runtime/reload-diff"
            | "/management/health/serving"
            | "/management/health/resilience"
            | "/management/alerts"
            | "/management/events"
    ) || channel_readonly_path(path)
        || credential_set_readonly_path(path)
}

pub fn is_management_mutation_path(method: Method, path: &str) -> bool {
    if method != Method::Post {
        return false;
    }
    if path == "/management/runtime/reload" {
        return true;
    }
    let parts = path.split('/').collect::<Vec<_>>();
    let credential_set_prefix = parts.len() >= 6
        && parts[0].is_empty()
        && parts[1] == "management"
        && parts[2] == "credential-sets"
        && safe_path_segment(parts[3], "credential_set_id").is_ok()
        && parts[4] == "credentials";
    credential_set_prefix
        && ((parts.len() == 6 && parts[5] == "import")
            || (parts.len() == 7
                && safe_credential_ref_path_segment(parts[5]).is_ok()
                && matches!(parts[6], "probe" | "apply-latest-probe")))
}

fn credential_set_readonly_path(path: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    let credential_set_prefix = parts.len() >= 5
        && parts[0].is_empty()
        && parts[1] == "management"
        && parts[2] == "credential-sets"
        && safe_path_segment(parts[3], "credential_set_id").is_ok()
        && matches!(parts[4], "operations" | "credentials");
    credential_set_prefix
        && (parts.len() == 5
            || (parts.len() == 8
                && parts[4] == "credentials"
                && safe_credential_ref_path_segment(parts[5]).is_ok()
                && parts[6] == "apply-latest-probe"
                && parts[7] == "plan"))
}

fn channel_readonly_path(path: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    parts.len() == 4
        && parts[0].is_empty()
        && parts[1] == "management"
        && parts[2] == "channels"
        && safe_path_segment(parts[3], "channel_id").is_ok()
}

fn is_json_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    mime == "application/json" || mime.ends_with("+json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn normalizes_management_url_from_origin_or_management_base() {
        assert_eq!(
            normalize_management_url("https://ai.example.com")
                .unwrap()
                .as_str(),
            "https://ai.example.com/"
        );
        assert_eq!(
            normalize_management_url("https://ai.example.com/management")
                .unwrap()
                .as_str(),
            "https://ai.example.com/"
        );
    }

    #[test]
    fn management_url_rejects_client_v1_base() {
        let error = normalize_management_url("https://ai.example.com/v1")
            .expect_err("client base URL must be rejected for management commands");

        assert_eq!(error.reason_code(), "client_base_url_used_for_management");
        assert!(!error.to_string().contains("ai.example.com"));
    }

    #[test]
    fn management_url_rejects_token_like_url_components() {
        let error = normalize_management_url("https://user:secret@ai.example.com/v1")
            .expect_err("userinfo must be rejected");

        assert_eq!(error.reason_code(), "management_url_contains_credentials");
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn management_url_accepts_matching_deprecated_alias() {
        let input = OperatorClientOptions {
            management_url: Some("https://primary.example".to_string()),
            deprecated_base_url: Some("https://primary.example/management".to_string()),
            token_source: ManagementTokenSource::Env("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
            timeout_seconds: 3,
        };

        let config = OperatorClientConfig::from_options(input, |name| {
            assert_eq!(name, "ONE_AI_KEY_MANAGEMENT_TOKEN");
            Some("opaque-management-fixture-value".to_string())
        })
        .expect("config should resolve");

        assert_eq!(config.base_url.as_str(), "https://primary.example/");
        assert_eq!(config.timeout_seconds, 3);
        assert_eq!(
            config.bearer_token.as_str(),
            "opaque-management-fixture-value"
        );
    }

    #[test]
    fn management_url_rejects_conflicting_deprecated_alias() {
        let error = resolve_management_url(
            Some("https://primary.example"),
            Some("https://deprecated.example/management"),
        )
        .expect_err("conflicting URL flags should fail closed");

        assert_eq!(error.reason_code(), "management_url_alias_conflict");
        assert!(!error.to_string().contains("primary.example"));
        assert!(!error.to_string().contains("deprecated.example"));
    }

    #[test]
    fn management_token_can_be_read_from_env_without_leaking_value() {
        let token = ManagementToken::from_env("ONE_AI_KEY_MANAGEMENT_TOKEN", |name| {
            assert_eq!(name, "ONE_AI_KEY_MANAGEMENT_TOKEN");
            Some("opaque-env-management-fixture".to_string())
        })
        .expect("env token should resolve");

        assert_eq!(token.as_str(), "opaque-env-management-fixture");
        assert!(!format!("{token:?}").contains("opaque-env-management-fixture"));
    }

    #[test]
    fn management_token_can_be_read_from_stdin_without_leaking_value() {
        let token = ManagementToken::from_stdin_bytes(b"opaque-stdin-management-fixture\n")
            .expect("stdin token should resolve");

        assert_eq!(token.as_str(), "opaque-stdin-management-fixture");
        assert!(!format!("{token:?}").contains("opaque-stdin-management-fixture"));
    }

    #[test]
    fn management_token_stdin_fails_closed_for_tty_input() {
        let error =
            ManagementToken::from_stdin_reader(std::io::Cursor::new(Vec::<u8>::new()), true)
                .expect_err("tty stdin should be rejected");

        assert_eq!(error.reason_code(), "management_token_stdin_requires_pipe");
    }

    #[test]
    fn maps_timeout_and_non_json_errors_to_reason_codes_without_secrets() {
        let timeout = OperatorClientError::timeout("https://ai.example.com/management/runtime");
        assert_eq!(timeout.reason_code(), "management_timeout");
        assert!(!timeout.to_string().contains("Bearer"));

        let non_json = classify_http_error(
            StatusCode::BAD_GATEWAY,
            Some("text/plain"),
            b"upstream included opaque-sensitive-fixture in a plain error",
        );
        assert_eq!(non_json.reason_code(), "management_non_json_error");
        assert!(!non_json.to_string().contains("opaque-sensitive-fixture"));
    }

    #[test]
    fn maps_auth_and_missing_endpoint_statuses_to_stable_reason_codes() {
        assert_eq!(
            classify_http_error(StatusCode::UNAUTHORIZED, Some("application/json"), b"{}")
                .reason_code(),
            "management_unauthorized"
        );
        assert_eq!(
            classify_http_error(StatusCode::FORBIDDEN, Some("application/json"), b"{}")
                .reason_code(),
            "management_forbidden"
        );
        assert_eq!(
            classify_http_error(StatusCode::NOT_FOUND, Some("application/json"), b"{}")
                .reason_code(),
            "management_not_found"
        );
    }

    #[test]
    fn read_only_allowlist_accepts_only_m2_get_management_paths() {
        for path in [
            "/management/routing/preview",
            "/management/client-tokens",
            "/management/model-routes",
            "/management/channels",
            "/management/channels/relay-a",
            "/management/credential-sets",
            "/management/credential-sets/relay/operations",
            "/management/credential-sets/relay/credentials",
            "/management/routing-telemetry",
            "/management/response-filter-events",
            "/management/explain/runtime",
            "/management/runtime",
            "/management/runtime/reload-diff",
            "/management/health/serving",
            "/management/health/resilience",
            "/management/alerts",
            "/management/events",
        ] {
            assert!(
                is_readonly_management_path(Method::Get, path),
                "{path} should be allowlisted"
            );
        }

        for (method, path) in [
            (Method::Post, "/management/runtime/reload"),
            (Method::Get, "/management/runtime/reload"),
            (Method::Get, "/v1/models"),
            (Method::Get, "/management/channels/relay-a/model-discovery"),
            (Method::Post, "/management/channels/relay-a/model-discovery"),
            (Method::Post, "/management/client-tokens"),
            (
                Method::Post,
                "/management/credential-sets/relay/credentials/import",
            ),
            (
                Method::Get,
                "/management/credential-sets/relay/credentials/internal-id/history",
            ),
            (Method::Get, "/management/credential-sets/../credentials"),
            (Method::Get, "/management/credential-sets/relay/unknown"),
        ] {
            assert!(
                !is_readonly_management_path(method, path),
                "{method:?} {path} should be rejected"
            );
        }
    }

    #[test]
    fn typed_endpoint_rejects_unsafe_credential_set_path_segments() {
        for credential_set_id in ["", ".", "..", "relay/keys", "relay%2Fkeys"] {
            let error = ReadOnlyEndpoint::CredentialSetCredentials {
                credential_set_id: credential_set_id.to_string(),
                offset: None,
                limit: Some(10),
            }
            .request()
            .expect_err("unsafe credential set id should be rejected");

            assert_eq!(error.reason_code(), "management_path_segment_invalid");
            if !credential_set_id.is_empty() {
                assert!(!error.to_string().contains(credential_set_id));
            }
        }
    }

    #[test]
    fn typed_endpoint_builds_bounded_query_and_serving_health_accepts_503() {
        let credentials = ReadOnlyEndpoint::CredentialSetCredentials {
            credential_set_id: "relay_keys".to_string(),
            offset: Some(5),
            limit: Some(20),
        }
        .request()
        .expect("credential endpoint should build");

        assert_eq!(
            credentials.path,
            "/management/credential-sets/relay_keys/credentials"
        );
        assert_eq!(
            credentials.query,
            vec![
                ("offset".to_string(), "5".to_string()),
                ("limit".to_string(), "20".to_string())
            ]
        );

        let serving = ReadOnlyEndpoint::ServingHealth
            .request()
            .expect("serving health endpoint should build");
        assert!(serving.accepts_status(StatusCode::OK));
        assert!(serving.accepts_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!serving.accepts_status(StatusCode::INTERNAL_SERVER_ERROR));
    }

    #[test]
    fn models_onboard_channel_projection_endpoint_uses_safe_readonly_path() {
        let endpoint = ReadOnlyEndpoint::Channel {
            channel_id: "relay-a".to_string(),
        };
        let (path, query) = endpoint.test_request_parts().unwrap();

        assert_eq!(path, "/management/channels/relay-a");
        assert!(query.is_empty());
        assert!(is_readonly_management_path(Method::Get, &path));

        let error = ReadOnlyEndpoint::Channel {
            channel_id: "../relay-a".to_string(),
        }
        .test_request_parts()
        .expect_err("unsafe channel ids must be rejected");
        assert_eq!(error.reason_code(), "management_path_segment_invalid");
    }

    #[test]
    fn typed_mutation_endpoint_builds_import_path_without_broadening_readonly_allowlist() {
        let import = ManagementMutationEndpoint::CredentialSetCredentialsImport {
            credential_set_id: "relay_keys".to_string(),
        }
        .test_request_parts()
        .expect("credential import mutation endpoint should build");

        assert_eq!(
            import,
            (
                "/management/credential-sets/relay_keys/credentials/import".to_string(),
                Vec::new()
            )
        );
        assert!(is_management_mutation_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/import"
        ));
        assert!(!is_management_mutation_path(
            Method::Get,
            "/management/credential-sets/relay_keys/credentials/import"
        ));
        assert!(!is_readonly_management_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/import"
        ));
    }

    #[test]
    fn typed_mutation_endpoint_builds_probe_path_with_credential_ref() {
        let probe = ManagementMutationEndpoint::CredentialSetCredentialProbe {
            credential_set_id: "relay_keys".to_string(),
            credential_ref: "cr:v1:pos:0".to_string(),
        }
        .test_request_parts()
        .expect("credential probe mutation endpoint should build");

        assert_eq!(
            probe,
            (
                "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/probe".to_string(),
                Vec::new()
            )
        );
        assert!(is_management_mutation_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/probe"
        ));
        assert!(!is_management_mutation_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/internal-id/probe"
        ));
        assert!(!is_readonly_management_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/probe"
        ));
    }

    #[test]
    fn keys_probe_apply_dry_run_endpoint_builds_readonly_plan_path() {
        let plan = ReadOnlyEndpoint::CredentialSetCredentialProbeApplyPlan {
            credential_set_id: "relay_keys".to_string(),
            credential_ref: "cr:v1:pos:0".to_string(),
        }
        .test_request_parts()
        .expect("probe apply plan endpoint should build");

        assert_eq!(
            plan,
            (
                "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/apply-latest-probe/plan".to_string(),
                Vec::new()
            )
        );
        assert!(is_readonly_management_path(
            Method::Get,
            "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/apply-latest-probe/plan"
        ));
        assert!(!is_readonly_management_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/apply-latest-probe/plan"
        ));
        assert!(!is_management_mutation_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/apply-latest-probe/plan"
        ));
        assert!(!is_readonly_management_path(
            Method::Get,
            "/management/credential-sets/relay_keys/credentials/internal-id/apply-latest-probe/plan"
        ));
    }

    #[test]
    fn typed_mutation_endpoint_builds_probe_apply_path_with_credential_ref() {
        let apply = ManagementMutationEndpoint::CredentialSetCredentialProbeApply {
            credential_set_id: "relay_keys".to_string(),
            credential_ref: "cr:v1:pos:0".to_string(),
        }
        .test_request_parts()
        .expect("credential probe apply mutation endpoint should build");

        assert_eq!(
            apply,
            (
                "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/apply-latest-probe"
                    .to_string(),
                Vec::new()
            )
        );
        assert!(is_management_mutation_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/apply-latest-probe"
        ));
        assert!(!is_management_mutation_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/internal-id/apply-latest-probe"
        ));
        assert!(!is_readonly_management_path(
            Method::Post,
            "/management/credential-sets/relay_keys/credentials/cr:v1:pos:0/apply-latest-probe"
        ));
    }

    #[test]
    fn typed_mutation_endpoint_builds_runtime_reload_precondition_query() {
        let reload = ManagementMutationEndpoint::RuntimeReload {
            expected_staged_registry_version: 4,
        }
        .test_request_parts()
        .expect("runtime reload mutation endpoint should build");

        assert_eq!(
            reload,
            (
                "/management/runtime/reload".to_string(),
                vec![(
                    "expected_staged_registry_version".to_string(),
                    "4".to_string()
                )]
            )
        );
        assert!(is_management_mutation_path(
            Method::Post,
            "/management/runtime/reload"
        ));
        assert!(!is_readonly_management_path(
            Method::Post,
            "/management/runtime/reload"
        ));
    }
}
