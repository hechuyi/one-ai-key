use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EndpointSupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ModelsEndpointCapability {
    LocalProjection,
    Unsupported,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct EndpointCapabilitiesConfig {
    #[serde(default)]
    pub chat_completions: Option<EndpointSupport>,
    #[serde(default)]
    pub responses: Option<EndpointSupport>,
    #[serde(default)]
    pub embeddings: Option<EndpointSupport>,
    #[serde(default)]
    pub models: Option<ModelsEndpointCapability>,
    #[serde(default)]
    pub diagnostic_labels: Vec<String>,
}

impl EndpointCapabilitiesConfig {
    pub fn resolve_with_base(
        &self,
        base: ResolvedEndpointCapabilities,
    ) -> anyhow::Result<ResolvedEndpointCapabilities> {
        let mut labels = base.diagnostic_labels.into_iter().collect::<BTreeSet<_>>();
        for label in &self.diagnostic_labels {
            validate_diagnostic_label(label)?;
            labels.insert(label.clone());
        }
        Ok(ResolvedEndpointCapabilities {
            chat_completions: self.chat_completions.unwrap_or(base.chat_completions),
            responses: self.responses.unwrap_or(base.responses),
            embeddings: self.embeddings.unwrap_or(base.embeddings),
            models: self.models.unwrap_or(base.models),
            diagnostic_labels: labels.into_iter().collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedEndpointCapabilities {
    pub chat_completions: EndpointSupport,
    pub responses: EndpointSupport,
    pub embeddings: EndpointSupport,
    pub models: ModelsEndpointCapability,
    pub diagnostic_labels: Vec<String>,
}

impl ResolvedEndpointCapabilities {
    pub fn openai_compatible_default() -> Self {
        Self {
            chat_completions: EndpointSupport::Supported,
            responses: EndpointSupport::Unknown,
            embeddings: EndpointSupport::Unknown,
            models: ModelsEndpointCapability::LocalProjection,
            diagnostic_labels: vec!["openai_compatible".to_string()],
        }
    }

    pub fn generic_http_default() -> Self {
        Self {
            chat_completions: EndpointSupport::Unknown,
            responses: EndpointSupport::Unknown,
            embeddings: EndpointSupport::Unknown,
            models: ModelsEndpointCapability::Unknown,
            diagnostic_labels: vec!["generic_http".to_string()],
        }
    }

    pub fn to_config(&self) -> EndpointCapabilitiesConfig {
        EndpointCapabilitiesConfig {
            chat_completions: Some(self.chat_completions),
            responses: Some(self.responses),
            embeddings: Some(self.embeddings),
            models: Some(self.models),
            diagnostic_labels: self.diagnostic_labels.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EndpointCapabilitiesStatus {
    pub chat_completions: EndpointSupport,
    pub responses: EndpointSupport,
    pub embeddings: EndpointSupport,
    pub models: ModelsEndpointCapability,
    pub diagnostic_labels: Vec<String>,
}

impl From<&ResolvedEndpointCapabilities> for EndpointCapabilitiesStatus {
    fn from(capabilities: &ResolvedEndpointCapabilities) -> Self {
        Self {
            chat_completions: capabilities.chat_completions,
            responses: capabilities.responses,
            embeddings: capabilities.embeddings,
            models: capabilities.models,
            diagnostic_labels: capabilities.diagnostic_labels.clone(),
        }
    }
}

fn validate_diagnostic_label(label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !label.is_empty(),
        "endpoint capability diagnostic label must not be empty"
    );
    anyhow::ensure!(
        label.len() <= 64,
        "endpoint capability diagnostic label must be at most 64 bytes"
    );
    anyhow::ensure!(
        label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')),
        "endpoint capability diagnostic label contains unsupported characters"
    );
    Ok(())
}
