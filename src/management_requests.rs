use serde::Deserialize;

use crate::{
    credential_probe::CredentialProbeFilter, management_credentials::CredentialStateFilter,
};

#[derive(Debug, Deserialize)]
pub struct ModelDiscoverySyncPlanRequest {
    #[serde(default)]
    pub channel_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelHealthMutationRequest {
    pub reason: Option<String>,
}

impl ChannelHealthMutationRequest {
    pub(crate) fn reason_or(self, default: &str) -> String {
        self.reason
            .map(|reason| reason.trim().to_string())
            .filter(|reason| !reason.is_empty())
            .unwrap_or_else(|| default.to_string())
    }
}

#[derive(Debug, Deserialize)]
pub struct CredentialsQuery {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub state: Option<String>,
    pub probe: Option<String>,
}

impl CredentialsQuery {
    pub(crate) fn offset_or_zero(&self) -> usize {
        self.offset.unwrap_or(0)
    }

    pub(crate) fn bounded_limit(&self, default: usize, max: usize) -> usize {
        self.limit.unwrap_or(default).clamp(1, max)
    }

    pub(crate) fn state_filter(&self) -> Result<CredentialStateFilter, String> {
        match self.state.as_deref().unwrap_or("all") {
            "all" => Ok(CredentialStateFilter::All),
            "available" => Ok(CredentialStateFilter::Available),
            "cooling_down" => Ok(CredentialStateFilter::CoolingDown),
            "expired" => Ok(CredentialStateFilter::Expired),
            "quota_exhausted" => Ok(CredentialStateFilter::QuotaExhausted),
            "disabled" => Ok(CredentialStateFilter::Disabled),
            other => Err(format!(
                "invalid credential state filter {other}; expected all, available, cooling_down, expired, quota_exhausted, or disabled"
            )),
        }
    }

    pub(crate) fn probe_filter(&self) -> Result<CredentialProbeFilter, String> {
        match self.probe.as_deref().unwrap_or("all") {
            "all" => Ok(CredentialProbeFilter::All),
            "success" => Ok(CredentialProbeFilter::Success),
            "invalid" => Ok(CredentialProbeFilter::Invalid),
            "quota_exhausted" => Ok(CredentialProbeFilter::QuotaExhausted),
            "rate_limited" => Ok(CredentialProbeFilter::RateLimited),
            "provider_unavailable" => Ok(CredentialProbeFilter::ProviderUnavailable),
            "unsupported_model" => Ok(CredentialProbeFilter::UnsupportedModel),
            "unknown" => Ok(CredentialProbeFilter::Unknown),
            "unprobed" => Ok(CredentialProbeFilter::Unprobed),
            other => Err(format!(
                "invalid credential probe filter {other}; expected all, success, invalid, quota_exhausted, rate_limited, provider_unavailable, unsupported_model, unknown, or unprobed"
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateClientTokenRequest {
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub allowed_model_groups: Vec<String>,
    #[serde(default)]
    pub allowed_channels: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateClientTokenScopeRequest {
    pub allowed_model_groups: Option<Vec<String>>,
    pub allowed_channels: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct SetCredentialMetadataRequest {
    pub label: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProbeCredentialRequest {
    pub model: String,
    pub kind: Option<ProbeCredentialKindRequest>,
    pub expected_output: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCredentialKindRequest {
    ModelRetrieve,
    ChatCompletion,
}

#[derive(Debug, Deserialize)]
pub struct ApplyLatestProbeRequest {
    pub reason: Option<String>,
    pub probe_result_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportCredentialSetCredentialsRequest {
    pub keys: Vec<String>,
    pub batch_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RoutingPreviewQuery {
    pub model: String,
    pub client_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RuntimeReloadQuery {
    pub expected_staged_registry_version: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ExpireCredentialRequest {
    pub reason: Option<String>,
}

impl ExpireCredentialRequest {
    pub(crate) fn reason_or(self, default: &str) -> String {
        self.reason
            .map(|reason| reason.trim().to_string())
            .filter(|reason| !reason.is_empty())
            .unwrap_or_else(|| default.to_string())
    }
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

impl EventsQuery {
    pub(crate) fn offset_or_zero(&self) -> usize {
        self.offset.unwrap_or(0)
    }

    pub(crate) fn bounded_limit(&self, default: usize, max: usize) -> usize {
        self.limit.unwrap_or(default).clamp(1, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expire_credential_request_reason_or_trims_and_defaults() {
        assert_eq!(
            ExpireCredentialRequest {
                reason: Some("  operator requested  ".to_string()),
            }
            .reason_or("fallback"),
            "operator requested"
        );
        assert_eq!(
            ExpireCredentialRequest {
                reason: Some("   ".to_string()),
            }
            .reason_or("fallback"),
            "fallback"
        );
        assert_eq!(
            ExpireCredentialRequest { reason: None }.reason_or("fallback"),
            "fallback"
        );
    }

    #[test]
    fn credentials_query_page_bounds_match_handlers() {
        assert_eq!(
            CredentialsQuery {
                offset: None,
                limit: None,
                state: None,
                probe: None,
            }
            .offset_or_zero(),
            0
        );
        assert_eq!(
            CredentialsQuery {
                offset: Some(42),
                limit: None,
                state: None,
                probe: None,
            }
            .offset_or_zero(),
            42
        );
        assert_eq!(
            CredentialsQuery {
                offset: None,
                limit: None,
                state: None,
                probe: None,
            }
            .bounded_limit(usize::MAX, 1000),
            1000
        );
        assert_eq!(
            CredentialsQuery {
                offset: None,
                limit: Some(0),
                state: None,
                probe: None,
            }
            .bounded_limit(usize::MAX, 1000),
            1
        );
        assert_eq!(
            CredentialsQuery {
                offset: None,
                limit: Some(1001),
                state: None,
                probe: None,
            }
            .bounded_limit(usize::MAX, 1000),
            1000
        );
    }

    #[test]
    fn events_query_page_bounds_match_handlers() {
        assert_eq!(
            EventsQuery {
                offset: None,
                limit: None,
            }
            .offset_or_zero(),
            0
        );
        assert_eq!(
            EventsQuery {
                offset: Some(7),
                limit: None,
            }
            .offset_or_zero(),
            7
        );
        assert_eq!(
            EventsQuery {
                offset: None,
                limit: None,
            }
            .bounded_limit(100, 1000),
            100
        );
        assert_eq!(
            EventsQuery {
                offset: None,
                limit: Some(0),
            }
            .bounded_limit(100, 1000),
            1
        );
        assert_eq!(
            EventsQuery {
                offset: None,
                limit: Some(1001),
            }
            .bounded_limit(100, 1000),
            1000
        );
    }
}
