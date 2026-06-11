use axum::http::{HeaderMap, Method};
use bytes::Bytes;
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::endpoint_capabilities::ResolvedEndpointCapabilities;
use crate::error::{ClassifiedFailure, ErrorClassifier};
use crate::model_catalog::{openai_model_catalog_from_body, ParsedModelCatalog};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum ProviderKind {
    #[default]
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    #[serde(rename = "generic_http")]
    GenericHttp,
}

impl ProviderKind {
    pub fn stable_id_fragment(self) -> &'static str {
        match self {
            ProviderKind::OpenAiCompatible => "openai_compatible",
            ProviderKind::GenericHttp => "generic_http",
        }
    }

    pub fn default_endpoint_capabilities(self) -> ResolvedEndpointCapabilities {
        match self {
            ProviderKind::OpenAiCompatible => {
                ResolvedEndpointCapabilities::openai_compatible_default()
            }
            ProviderKind::GenericHttp => ResolvedEndpointCapabilities::generic_http_default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Models,
    ChatCompletions,
    Responses,
    Embeddings,
    Generic,
}

impl EndpointKind {
    pub fn family_code(self) -> &'static str {
        match self {
            Self::Models => "models",
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
            Self::Embeddings => "embeddings",
            Self::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestContext {
    pub endpoint: EndpointKind,
    pub requested_model: Option<String>,
    pub body_replayable: bool,
    pub streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamRequestParts {
    pub path: String,
    pub body: Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedPoolRequestMode {
    ReplayableWithModelContext,
    StreamingPassThrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundProtocol {
    OpenAiCompatible,
}

impl InboundProtocol {
    pub fn request_context(&self, method: &Method, path: &str, body: &[u8]) -> RequestContext {
        match self {
            InboundProtocol::OpenAiCompatible => openai_request_context(method, path, body),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialProbeTarget {
    ModelRetrieve { path: String },
    ChatCompletion { path: String, body: Value },
    UnsupportedModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatProbeSuccessOutcome {
    MatchesExpectedOutput,
    UnexpectedOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCatalogTarget {
    OpenAiModels { path: String },
}

impl ModelCatalogTarget {
    pub fn path(&self) -> &str {
        match self {
            ModelCatalogTarget::OpenAiModels { path } => path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCatalogCapability {
    pub target: ModelCatalogTarget,
}

impl ModelCatalogCapability {
    pub fn parse(&self, body: &[u8]) -> anyhow::Result<ParsedModelCatalog> {
        match self.target {
            ModelCatalogTarget::OpenAiModels { .. } => openai_model_catalog_from_body(body)
                .ok_or_else(|| anyhow::anyhow!("malformed OpenAI model catalog")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderAdapter {
    kind: ProviderKind,
}

impl ProviderAdapter {
    pub fn new(kind: ProviderKind) -> Self {
        Self { kind }
    }

    pub fn request_context(&self, method: &Method, path: &str, body: &[u8]) -> RequestContext {
        match self.kind {
            ProviderKind::OpenAiCompatible => openai_request_context(method, path, body),
            ProviderKind::GenericHttp => RequestContext {
                endpoint: EndpointKind::Generic,
                requested_model: None,
                body_replayable: false,
                streaming: false,
            },
        }
    }

    pub fn named_pool_request_mode(&self) -> NamedPoolRequestMode {
        match self.kind {
            ProviderKind::OpenAiCompatible => NamedPoolRequestMode::ReplayableWithModelContext,
            ProviderKind::GenericHttp => NamedPoolRequestMode::StreamingPassThrough,
        }
    }

    pub fn model_catalog_capability(&self) -> Option<ModelCatalogCapability> {
        match self.kind {
            ProviderKind::OpenAiCompatible => Some(ModelCatalogCapability {
                target: ModelCatalogTarget::OpenAiModels {
                    path: "/v1/models".to_string(),
                },
            }),
            ProviderKind::GenericHttp => None,
        }
    }

    pub fn upstream_url(&self, api_base: &str, path: &str, query: Option<&str>) -> String {
        let base = api_base.trim_end_matches('/');
        let path = match self.kind {
            ProviderKind::OpenAiCompatible => normalize_openai_upstream_path(base, path),
            ProviderKind::GenericHttp => path,
        };
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        match query {
            Some(query) if !query.is_empty() => format!("{base}{path}?{query}"),
            _ => format!("{base}{path}"),
        }
    }

    pub fn transform_request_for_target(
        &self,
        method: &Method,
        path: &str,
        body: &Bytes,
        upstream_model: Option<&str>,
    ) -> UpstreamRequestParts {
        match self.kind {
            ProviderKind::OpenAiCompatible => {
                openai_transform_request_for_target(method, path, body, upstream_model)
            }
            ProviderKind::GenericHttp => UpstreamRequestParts {
                path: path.to_string(),
                body: body.clone(),
            },
        }
    }

    pub fn credential_probe_target(&self, model: &str) -> CredentialProbeTarget {
        match self.kind {
            ProviderKind::OpenAiCompatible => CredentialProbeTarget::ModelRetrieve {
                path: format!("/v1/models/{}", encode_path_segment(model)),
            },
            ProviderKind::GenericHttp => CredentialProbeTarget::UnsupportedModel,
        }
    }

    pub fn credential_chat_probe_target(
        &self,
        model: &str,
        expected_output: Option<&str>,
    ) -> CredentialProbeTarget {
        match self.kind {
            ProviderKind::OpenAiCompatible => CredentialProbeTarget::ChatCompletion {
                path: "/v1/chat/completions".to_string(),
                body: serde_json::json!({
                    "model": model,
                    "messages": [{
                        "role": "user",
                        "content": chat_probe_prompt(expected_output)
                    }],
                    "temperature": 0,
                    "stream": false,
                }),
            },
            ProviderKind::GenericHttp => CredentialProbeTarget::UnsupportedModel,
        }
    }

    pub fn credential_chat_probe_success_outcome(
        &self,
        body: &[u8],
        expected_output: Option<&str>,
    ) -> ChatProbeSuccessOutcome {
        match self.kind {
            ProviderKind::OpenAiCompatible => {
                openai_chat_probe_success_outcome(body, expected_output)
            }
            ProviderKind::GenericHttp => ChatProbeSuccessOutcome::MatchesExpectedOutput,
        }
    }

    pub fn apply_upstream_auth_and_headers(
        &self,
        builder: RequestBuilder,
        inbound_headers: &HeaderMap,
        auth_header: &str,
        auth_prefix: &str,
        credential_secret: &str,
    ) -> RequestBuilder {
        copy_allowed_upstream_headers(
            builder,
            inbound_headers,
            auth_header,
            auth_prefix,
            credential_secret,
        )
    }

    pub fn classify_failure(
        &self,
        classifier: &ErrorClassifier,
        status: u16,
        headers: &HeaderMap,
        body: &[u8],
    ) -> ClassifiedFailure {
        let headers = classifier_headers(headers);
        let header_refs = header_refs(&headers);
        classifier.classify_failure(status, &header_refs, body)
    }
}

fn normalize_openai_upstream_path<'a>(base: &str, path: &'a str) -> &'a str {
    if !base.ends_with("/v1") {
        return path;
    }
    match path.strip_prefix("/v1") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => rest,
        _ => path,
    }
}

fn chat_probe_prompt(expected_output: Option<&str>) -> String {
    match expected_output {
        Some(expected_output) => {
            format!("Return exactly the following text, with no extra text:\n{expected_output}")
        }
        None => "Return any short response. This is an API availability probe.".to_string(),
    }
}

fn openai_chat_probe_success_outcome(
    body: &[u8],
    expected_output: Option<&str>,
) -> ChatProbeSuccessOutcome {
    match expected_output {
        Some(expected_output)
            if !openai_chat_probe_matches_expected_output(body, expected_output) =>
        {
            ChatProbeSuccessOutcome::UnexpectedOutput
        }
        _ => ChatProbeSuccessOutcome::MatchesExpectedOutput,
    }
}

fn openai_chat_probe_matches_expected_output(body: &[u8], expected_output: &str) -> bool {
    openai_chat_probe_message_content(body)
        .map(|content| content == expected_output)
        .unwrap_or(false)
}

fn openai_chat_probe_message_content(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "authorization",
    "x-api-key",
    "content-length",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderForwardPolicy {
    connection_blocked: Vec<String>,
}

impl HeaderForwardPolicy {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            connection_blocked: connection_header_tokens(headers),
        }
    }

    pub fn allows(&self, name: &str) -> bool {
        !is_blocked_header(name, &self.connection_blocked)
    }
}

fn copy_allowed_upstream_headers(
    mut builder: RequestBuilder,
    headers: &HeaderMap,
    auth_header: &str,
    auth_prefix: &str,
    key: &str,
) -> RequestBuilder {
    let policy = HeaderForwardPolicy::from_headers(headers);
    for (name, value) in headers {
        if !policy.allows(name.as_str()) || name.as_str().eq_ignore_ascii_case(auth_header) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder.header(auth_header, format!("{auth_prefix}{key}"))
}

fn connection_header_tokens(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn is_blocked_header(name: &str, connection_blocked: &[String]) -> bool {
    HOP_BY_HOP
        .iter()
        .any(|blocked| name.eq_ignore_ascii_case(blocked))
        || connection_blocked
            .iter()
            .any(|blocked| name.eq_ignore_ascii_case(blocked))
}

fn classifier_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect()
}

fn header_refs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn openai_request_context(method: &Method, path: &str, body: &[u8]) -> RequestContext {
    let endpoint = match (method, path) {
        (&Method::GET, "/v1/models") | (&Method::GET, "/models") => EndpointKind::Models,
        (_, "/v1/chat/completions") | (_, "/chat/completions") => EndpointKind::ChatCompletions,
        (_, "/v1/responses") | (_, "/responses") => EndpointKind::Responses,
        (_, "/v1/embeddings") | (_, "/embeddings") => EndpointKind::Embeddings,
        _ => EndpointKind::Generic,
    };
    let value: Option<Value> = serde_json::from_slice(body).ok();
    let requested_model = value
        .as_ref()
        .and_then(|value| value.get("model"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| openai_model_from_retrieve_path(method, path));
    let streaming = value
        .as_ref()
        .and_then(|value| value.get("stream"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    RequestContext {
        endpoint,
        requested_model,
        body_replayable: true,
        streaming,
    }
}

fn openai_model_from_retrieve_path(method: &Method, path: &str) -> Option<String> {
    if method != Method::GET {
        return None;
    }
    path.strip_prefix("/v1/models/")
        .or_else(|| path.strip_prefix("/models/"))
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
}

fn openai_transform_request_for_target(
    method: &Method,
    path: &str,
    body: &Bytes,
    upstream_model: Option<&str>,
) -> UpstreamRequestParts {
    let Some(upstream_model) = upstream_model else {
        return UpstreamRequestParts {
            path: path.to_string(),
            body: body.clone(),
        };
    };
    UpstreamRequestParts {
        path: rewrite_openai_model_retrieve_path(method, path, upstream_model)
            .unwrap_or_else(|| path.to_string()),
        body: rewrite_openai_model_body(body, upstream_model).unwrap_or_else(|| body.clone()),
    }
}

fn rewrite_openai_model_retrieve_path(
    method: &Method,
    path: &str,
    upstream_model: &str,
) -> Option<String> {
    if method != Method::GET {
        return None;
    }
    if path.strip_prefix("/v1/models/").is_some() {
        return Some(format!(
            "/v1/models/{}",
            encode_path_segment(upstream_model)
        ));
    }
    if path.strip_prefix("/models/").is_some() {
        return Some(format!("/models/{}", encode_path_segment(upstream_model)));
    }
    None
}

fn rewrite_openai_model_body(body: &Bytes, upstream_model: &str) -> Option<Bytes> {
    let mut value = serde_json::from_slice::<Value>(body).ok()?;
    let model = value.get_mut("model")?;
    if !model.is_string() {
        return None;
    }
    *model = Value::String(upstream_model.to_string());
    serde_json::to_vec(&value).ok().map(Bytes::from)
}

pub fn bytes_from_body_for_context(body: &Bytes) -> &[u8] {
    body.as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_uses_canonical_names() {
        let kind: ProviderKind = serde_yaml::from_str("openai_compatible").unwrap();
        assert_eq!(kind, ProviderKind::OpenAiCompatible);

        assert!(serde_yaml::from_str::<ProviderKind>("open_ai_compatible").is_err());
    }

    #[test]
    fn openai_url_does_not_duplicate_v1() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);
        assert_eq!(
            adapter.upstream_url("https://example.com/v1", "/v1/chat/completions", None),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_preserves_v1_when_base_has_no_version_segment() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);
        assert_eq!(
            adapter.upstream_url("https://example.com", "/v1/models", None),
            "https://example.com/v1/models"
        );
    }

    #[test]
    fn openai_url_only_strips_complete_v1_segment() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);
        assert_eq!(
            adapter.upstream_url("https://example.com/v1", "/v10/models", None),
            "https://example.com/v1/v10/models"
        );
    }

    #[test]
    fn generic_url_preserves_raw_path() {
        let adapter = ProviderAdapter::new(ProviderKind::GenericHttp);
        assert_eq!(
            adapter.upstream_url("https://example.com/base", "/v1/models", Some("a=b")),
            "https://example.com/base/v1/models?a=b"
        );
    }

    #[test]
    fn openai_context_identifies_models_without_body_model_extraction() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);
        let context = adapter.request_context(&Method::GET, "/v1/models", b"");
        assert_eq!(context.endpoint, EndpointKind::Models);
        assert_eq!(context.requested_model, None);
        assert!(context.body_replayable);
    }

    #[test]
    fn openai_context_extracts_model_and_streaming() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);
        let context = adapter.request_context(
            &Method::POST,
            "/v1/chat/completions",
            br#"{"model":"gpt-test","stream":true}"#,
        );
        assert_eq!(context.endpoint, EndpointKind::ChatCompletions);
        assert_eq!(context.requested_model.as_deref(), Some("gpt-test"));
        assert!(context.streaming);
    }

    #[test]
    fn openai_context_extracts_responses_model_and_streaming() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let streaming_context = adapter.request_context(
            &Method::POST,
            "/v1/responses",
            br#"{"model":"response-model","stream":true}"#,
        );
        assert_eq!(streaming_context.endpoint, EndpointKind::Responses);
        assert_eq!(
            streaming_context.requested_model.as_deref(),
            Some("response-model")
        );
        assert!(streaming_context.body_replayable);
        assert!(streaming_context.streaming);

        let non_streaming_context = adapter.request_context(
            &Method::POST,
            "/responses",
            br#"{"model":"response-model","stream":false}"#,
        );
        assert_eq!(non_streaming_context.endpoint, EndpointKind::Responses);
        assert_eq!(
            non_streaming_context.requested_model.as_deref(),
            Some("response-model")
        );
        assert!(non_streaming_context.body_replayable);
        assert!(!non_streaming_context.streaming);
    }

    #[test]
    fn openai_context_extracts_embeddings_model_as_replayable_non_streaming() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let context = adapter.request_context(
            &Method::POST,
            "/embeddings",
            br#"{"model":"embedding-model","input":"hello"}"#,
        );

        assert_eq!(context.endpoint, EndpointKind::Embeddings);
        assert_eq!(context.requested_model.as_deref(), Some("embedding-model"));
        assert!(context.body_replayable);
        assert!(!context.streaming);
    }

    #[test]
    fn inbound_openai_context_matches_provider_openai_models_context() {
        let inbound =
            InboundProtocol::OpenAiCompatible.request_context(&Method::GET, "/v1/models", b"");
        let provider = ProviderAdapter::new(ProviderKind::OpenAiCompatible).request_context(
            &Method::GET,
            "/v1/models",
            b"",
        );

        assert_eq!(inbound, provider);
    }

    #[test]
    fn inbound_openai_context_matches_provider_openai_chat_context() {
        let body = br#"{"model":"gpt-test","stream":true}"#;
        let inbound = InboundProtocol::OpenAiCompatible.request_context(
            &Method::POST,
            "/v1/chat/completions",
            body,
        );
        let provider = ProviderAdapter::new(ProviderKind::OpenAiCompatible).request_context(
            &Method::POST,
            "/v1/chat/completions",
            body,
        );

        assert_eq!(inbound, provider);
    }

    #[test]
    fn inbound_openai_context_matches_provider_openai_responses_context() {
        let body = br#"{"model":"response-model","stream":true}"#;
        let inbound =
            InboundProtocol::OpenAiCompatible.request_context(&Method::POST, "/responses", body);
        let provider = ProviderAdapter::new(ProviderKind::OpenAiCompatible).request_context(
            &Method::POST,
            "/responses",
            body,
        );

        assert_eq!(inbound, provider);
        assert_eq!(provider.endpoint, EndpointKind::Responses);
    }

    #[test]
    fn inbound_openai_context_matches_provider_openai_embeddings_context() {
        let body = br#"{"model":"embedding-model","input":"hello"}"#;
        let inbound = InboundProtocol::OpenAiCompatible.request_context(
            &Method::POST,
            "/v1/embeddings",
            body,
        );
        let provider = ProviderAdapter::new(ProviderKind::OpenAiCompatible).request_context(
            &Method::POST,
            "/v1/embeddings",
            body,
        );

        assert_eq!(inbound, provider);
        assert_eq!(provider.endpoint, EndpointKind::Embeddings);
    }

    #[test]
    fn generic_context_is_not_replayable() {
        let adapter = ProviderAdapter::new(ProviderKind::GenericHttp);
        let context = adapter.request_context(&Method::POST, "/anything", b"{}");
        assert_eq!(context.endpoint, EndpointKind::Generic);
        assert!(!context.body_replayable);
    }

    #[test]
    fn named_pool_mode_is_provider_capability() {
        assert_eq!(
            ProviderAdapter::new(ProviderKind::OpenAiCompatible).named_pool_request_mode(),
            NamedPoolRequestMode::ReplayableWithModelContext
        );
        assert_eq!(
            ProviderAdapter::new(ProviderKind::GenericHttp).named_pool_request_mode(),
            NamedPoolRequestMode::StreamingPassThrough
        );
    }

    #[test]
    fn openai_provider_exposes_model_catalog_capability() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let capability = adapter.model_catalog_capability().unwrap();

        assert_eq!(
            capability.target,
            ModelCatalogTarget::OpenAiModels {
                path: "/v1/models".to_string()
            }
        );
    }

    #[test]
    fn generic_provider_has_no_model_catalog_capability() {
        let adapter = ProviderAdapter::new(ProviderKind::GenericHttp);

        assert_eq!(adapter.model_catalog_capability(), None);
    }

    #[test]
    fn openai_provider_adapter_parses_model_catalog() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let catalog = adapter
            .model_catalog_capability()
            .expect("openai provider should expose a model catalog capability")
            .parse(
                br#"{
                    "object": "list",
                    "data": [{"id": "gpt-4.1", "object": "model"}]
                }"#,
            )
            .expect("valid OpenAI catalog should parse");

        assert_eq!(catalog.model_ids, vec!["gpt-4.1"]);
    }

    #[test]
    fn openai_provider_adapter_reports_malformed_model_catalog() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let result = adapter
            .model_catalog_capability()
            .expect("openai provider should expose a model catalog capability")
            .parse(br#"{"object":"chat.completion","choices":[]}"#);

        assert!(result.is_err());
    }

    #[test]
    fn openai_credential_probe_targets_encoded_model_retrieve_path() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        assert_eq!(
            adapter.credential_probe_target("vendor/probe model"),
            CredentialProbeTarget::ModelRetrieve {
                path: "/v1/models/vendor%2Fprobe%20model".to_string()
            }
        );
    }

    #[test]
    fn openai_chat_probe_uses_expected_output_in_prompt() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let target = adapter.credential_chat_probe_target("probe-model", Some("READY"));

        let CredentialProbeTarget::ChatCompletion { path, body } = target else {
            panic!("expected chat completion probe target");
        };
        assert_eq!(path, "/v1/chat/completions");
        assert_eq!(body["model"], "probe-model");
        assert_eq!(body["stream"], false);
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("READY"));
    }

    #[test]
    fn openai_chat_probe_without_expected_output_uses_availability_prompt() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let target = adapter.credential_chat_probe_target("probe-model", None);

        let CredentialProbeTarget::ChatCompletion { body, .. } = target else {
            panic!("expected chat completion probe target");
        };
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Return any short response"));
    }

    #[test]
    fn openai_chat_probe_success_outcome_matches_expected_output() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let outcome = adapter.credential_chat_probe_success_outcome(
            br#"{"choices":[{"message":{"content":"READY"}}]}"#,
            Some("READY"),
        );

        assert_eq!(outcome, ChatProbeSuccessOutcome::MatchesExpectedOutput);
    }

    #[test]
    fn openai_chat_probe_success_outcome_rejects_unexpected_output() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let outcome = adapter.credential_chat_probe_success_outcome(
            br#"{"choices":[{"message":{"content":"NOT READY"}}]}"#,
            Some("READY"),
        );

        assert_eq!(outcome, ChatProbeSuccessOutcome::UnexpectedOutput);
    }

    #[test]
    fn openai_transform_rewrites_json_model_for_route_target() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let transformed = adapter.transform_request_for_target(
            &Method::POST,
            "/v1/chat/completions",
            &Bytes::from_static(br#"{"model":"public","messages":[]}"#),
            Some("provider-real"),
        );

        assert_eq!(transformed.path, "/v1/chat/completions");
        let body: Value = serde_json::from_slice(&transformed.body).unwrap();
        assert_eq!(body["model"], "provider-real");
    }

    #[test]
    fn openai_transform_rewrites_model_retrieve_path_for_route_target() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);

        let transformed = adapter.transform_request_for_target(
            &Method::GET,
            "/v1/models/public",
            &Bytes::new(),
            Some("vendor/provider model"),
        );

        assert_eq!(transformed.path, "/v1/models/vendor%2Fprovider%20model");
        assert!(transformed.body.is_empty());
    }

    #[test]
    fn generic_transform_does_not_rewrite_model_target() {
        let adapter = ProviderAdapter::new(ProviderKind::GenericHttp);

        let transformed = adapter.transform_request_for_target(
            &Method::POST,
            "/v1/chat/completions",
            &Bytes::from_static(br#"{"model":"public"}"#),
            Some("provider-real"),
        );

        assert_eq!(transformed.path, "/v1/chat/completions");
        assert_eq!(
            transformed.body,
            Bytes::from_static(br#"{"model":"public"}"#)
        );
    }

    #[test]
    fn generic_credential_probe_reports_unsupported_model_without_target() {
        let adapter = ProviderAdapter::new(ProviderKind::GenericHttp);

        assert_eq!(
            adapter.credential_probe_target("probe-model"),
            CredentialProbeTarget::UnsupportedModel
        );
    }

    #[test]
    fn header_forward_policy_blocks_client_auth_and_hop_by_hop_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("connection", "x-session-hop".parse().unwrap());

        let policy = HeaderForwardPolicy::from_headers(&headers);

        assert!(!policy.allows("authorization"));
        assert!(!policy.allows("x-api-key"));
        assert!(!policy.allows("host"));
        assert!(!policy.allows("x-session-hop"));
        assert!(policy.allows("content-type"));
    }

    #[test]
    fn adapter_classifies_failure_with_response_headers() {
        let adapter = ProviderAdapter::new(ProviderKind::OpenAiCompatible);
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "7".parse().unwrap());

        let failure = adapter.classify_failure(
            &ErrorClassifier::default(),
            429,
            &headers,
            br#"{"error":{"code":"rate_limit_exceeded"}}"#,
        );

        assert_eq!(failure.cooldown, Some(std::time::Duration::from_secs(7)));
        assert_eq!(failure.upstream_status, Some(429));
    }
}
