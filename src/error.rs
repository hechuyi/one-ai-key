use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, SystemTime};

pub const DEFAULT_ERROR_CLASSIFIER_ID: &str = "openai-compatible-default";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    RateLimited,
    KeySwitchCooldown,
    AuthInvalid,
    QuotaExhausted,
    RelayBalanceUnavailable,
    ProviderUnavailable,
    ResponseFilterRejected,
    ClientError,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureScope {
    RequestOnly,
    Credential,
    Account,
    Channel,
    Deployment,
    ModelGroup,
    ProviderAdapter,
    ClientToken,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayProfile {
    #[default]
    #[serde(rename = "official_openai")]
    OfficialOpenAi,
    GenericRelay,
    UntrustedRelay,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceScope {
    #[default]
    Credential,
    Channel,
    Account,
    Provider,
    ClientToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAfterSource {
    DeltaSeconds,
    HttpDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedFailure {
    pub kind: FailureKind,
    pub primary_scope: FailureScope,
    pub retryable: bool,
    pub cooldown: Option<Duration>,
    pub retry_after_source: Option<RetryAfterSource>,
    pub confidence: FailureConfidence,
    pub upstream_status: Option<u16>,
    pub upstream_code: Option<String>,
    pub upstream_limit_type: Option<String>,
    pub classifier_id: String,
    pub classifier_version: String,
    pub adaptation_rule_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorClassifier {
    relay_profile: RelayProfile,
    balance_scope: BalanceScope,
    keep_codes: Vec<String>,
    switch_codes: Vec<String>,
    expire_codes: Vec<String>,
    keep_statuses: Vec<StatusMatcher>,
    switch_statuses: Vec<StatusMatcher>,
    expire_statuses: Vec<StatusMatcher>,
    adaptation_rules: Vec<ErrorAdaptationRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorClassifierSnapshot {
    pub classifier_id: String,
    pub classifier_version: String,
    pub relay_profile: RelayProfile,
    pub balance_scope: BalanceScope,
    pub keep_codes: Vec<String>,
    pub switch_codes: Vec<String>,
    pub expire_codes: Vec<String>,
    pub keep_statuses: Vec<String>,
    pub switch_statuses: Vec<String>,
    pub expire_statuses: Vec<String>,
    pub adaptation_rules: Vec<ErrorAdaptationRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorAdaptationRule {
    pub id: String,
    pub enabled: bool,
    pub matcher: ErrorAdaptationMatcher,
    pub action: ErrorAdaptationAction,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorAdaptationMatcher {
    pub codes: Vec<String>,
    pub limit_types: Vec<String>,
    pub statuses: Vec<StatusMatcher>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorAdaptationAction {
    pub kind: Option<FailureKind>,
    pub primary_scope: Option<FailureScope>,
    pub retryable: Option<bool>,
    pub cooldown: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusMatcher {
    Exact(u16),
    Range { start: u16, end: u16 },
}

impl ErrorClassifier {
    pub fn classifier_id(&self) -> &'static str {
        DEFAULT_ERROR_CLASSIFIER_ID
    }

    pub fn classifier_version(&self) -> &'static str {
        "1"
    }

    pub fn snapshot(&self) -> ErrorClassifierSnapshot {
        ErrorClassifierSnapshot {
            classifier_id: self.classifier_id().to_string(),
            classifier_version: self.classifier_version().to_string(),
            relay_profile: self.relay_profile,
            balance_scope: self.balance_scope,
            keep_codes: self.keep_codes.clone(),
            switch_codes: self.switch_codes.clone(),
            expire_codes: self.expire_codes.clone(),
            keep_statuses: status_matchers_as_config(&self.keep_statuses),
            switch_statuses: status_matchers_as_config(&self.switch_statuses),
            expire_statuses: status_matchers_as_config(&self.expire_statuses),
            adaptation_rules: self.adaptation_rules.clone(),
        }
    }

    pub fn classify_failure(
        &self,
        status: u16,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> ClassifiedFailure {
        let evidence = UpstreamErrorEvidence::from_response(status, body);
        let _reserved_scopes = (
            FailureScope::Account,
            FailureScope::Deployment,
            FailureScope::ModelGroup,
            FailureScope::ProviderAdapter,
            FailureScope::ClientToken,
        );
        let (kind, primary_scope, retryable, confidence) =
            if evidence.code.is_none() && evidence.has_top_level_error_object {
                (
                    FailureKind::ClientError,
                    FailureScope::RequestOnly,
                    false,
                    FailureConfidence::Medium,
                )
            } else {
                evidence
                    .code
                    .as_deref()
                    .and_then(|code| self.classify_code(code))
                    .unwrap_or_else(|| self.classify_status(status))
            };
        let (cooldown, retry_after_source) = retry_after(headers);
        let mut failure = ClassifiedFailure {
            kind,
            primary_scope,
            retryable,
            cooldown,
            retry_after_source,
            confidence,
            upstream_status: Some(status),
            upstream_code: evidence.code.clone(),
            upstream_limit_type: evidence.limit_type.clone(),
            classifier_id: self.classifier_id().to_string(),
            classifier_version: self.classifier_version().to_string(),
            adaptation_rule_id: None,
        };
        self.apply_adaptation_rules(&evidence, &mut failure);
        failure
    }

    pub fn classify_transport_failure(&self) -> ClassifiedFailure {
        ClassifiedFailure {
            kind: FailureKind::ProviderUnavailable,
            primary_scope: FailureScope::Channel,
            retryable: true,
            cooldown: None,
            retry_after_source: None,
            confidence: FailureConfidence::Medium,
            upstream_status: None,
            upstream_code: Some("transport_error".to_string()),
            upstream_limit_type: None,
            classifier_id: self.classifier_id().to_string(),
            classifier_version: self.classifier_version().to_string(),
            adaptation_rule_id: None,
        }
    }

    fn apply_adaptation_rules(
        &self,
        evidence: &UpstreamErrorEvidence,
        failure: &mut ClassifiedFailure,
    ) {
        let Some(rule) = self
            .adaptation_rules
            .iter()
            .find(|rule| rule.matches(evidence))
        else {
            return;
        };

        if let Some(kind) = rule.action.kind {
            failure.kind = kind;
        }
        if let Some(primary_scope) = rule.action.primary_scope {
            failure.primary_scope = primary_scope;
        }
        if let Some(retryable) = rule.action.retryable {
            failure.retryable = retryable;
        }
        if let Some(cooldown) = rule.action.cooldown {
            failure.cooldown.get_or_insert(cooldown);
        }
        failure.adaptation_rule_id = Some(rule.id.clone());
    }

    fn classify_code(
        &self,
        code: &str,
    ) -> Option<(FailureKind, FailureScope, bool, FailureConfidence)> {
        if contains_code(&self.keep_codes, code) {
            return Some(match code {
                "key_switch_cooldown" => (
                    FailureKind::KeySwitchCooldown,
                    FailureScope::Credential,
                    false,
                    FailureConfidence::High,
                ),
                "model_not_found" | "unsupported_endpoint" | "unsupported_parameter" => (
                    FailureKind::ClientError,
                    FailureScope::ModelGroup,
                    false,
                    FailureConfidence::Medium,
                ),
                _ => (
                    FailureKind::Unknown,
                    FailureScope::RequestOnly,
                    false,
                    FailureConfidence::Low,
                ),
            });
        }

        if contains_code(&self.expire_codes, code) {
            return Some((
                FailureKind::AuthInvalid,
                FailureScope::Credential,
                false,
                if code == "invalid_api_key" {
                    FailureConfidence::High
                } else {
                    FailureConfidence::Low
                },
            ));
        }

        if contains_code(&self.switch_codes, code) {
            return Some(match code {
                "insufficient_quota" | "quota_exceeded" | "billing_hard_limit_reached" => (
                    match self.balance_scope {
                        BalanceScope::Credential => FailureKind::QuotaExhausted,
                        BalanceScope::Channel => FailureKind::RelayBalanceUnavailable,
                        BalanceScope::Account
                        | BalanceScope::Provider
                        | BalanceScope::ClientToken => FailureKind::Unknown,
                    },
                    match self.balance_scope {
                        BalanceScope::Credential => FailureScope::Credential,
                        BalanceScope::Channel => FailureScope::Channel,
                        BalanceScope::Account => FailureScope::Account,
                        BalanceScope::Provider => FailureScope::ProviderAdapter,
                        BalanceScope::ClientToken => FailureScope::ClientToken,
                    },
                    matches!(self.balance_scope, BalanceScope::Channel),
                    FailureConfidence::Medium,
                ),
                "rate_limit_exceeded" | "rate_limit_cooldown" => (
                    FailureKind::RateLimited,
                    FailureScope::Credential,
                    true,
                    FailureConfidence::Medium,
                ),
                _ => (
                    FailureKind::RateLimited,
                    FailureScope::Credential,
                    true,
                    FailureConfidence::Low,
                ),
            });
        }

        Some(match code {
            "key_switch_cooldown" => (
                FailureKind::KeySwitchCooldown,
                FailureScope::Credential,
                false,
                FailureConfidence::High,
            ),
            "model_not_found" | "unsupported_endpoint" | "unsupported_parameter" => (
                FailureKind::ClientError,
                FailureScope::ModelGroup,
                false,
                FailureConfidence::Medium,
            ),
            _ => return None,
        })
    }

    fn classify_status(&self, status: u16) -> (FailureKind, FailureScope, bool, FailureConfidence) {
        if matches_status(&self.keep_statuses, status) {
            return (
                FailureKind::Unknown,
                FailureScope::RequestOnly,
                false,
                FailureConfidence::Low,
            );
        }

        if matches_status(&self.expire_statuses, status) {
            if matches!(
                self.relay_profile,
                RelayProfile::GenericRelay | RelayProfile::UntrustedRelay
            ) && matches!(status, 401 | 403)
            {
                return (
                    FailureKind::ClientError,
                    FailureScope::RequestOnly,
                    false,
                    FailureConfidence::Medium,
                );
            }
            return (
                FailureKind::AuthInvalid,
                FailureScope::Credential,
                false,
                FailureConfidence::Medium,
            );
        }

        if matches_status(&self.switch_statuses, status) {
            if (500..=599).contains(&status) {
                return (
                    FailureKind::ProviderUnavailable,
                    FailureScope::Channel,
                    true,
                    FailureConfidence::Low,
                );
            }
            return (
                FailureKind::RateLimited,
                FailureScope::Credential,
                true,
                FailureConfidence::Low,
            );
        }

        (
            FailureKind::Unknown,
            FailureScope::RequestOnly,
            false,
            FailureConfidence::Low,
        )
    }
}

fn retry_after(headers: &[(&str, &str)]) -> (Option<Duration>, Option<RetryAfterSource>) {
    retry_after_with_now(headers, SystemTime::now())
}

fn retry_after_with_now(
    headers: &[(&str, &str)],
    now: SystemTime,
) -> (Option<Duration>, Option<RetryAfterSource>) {
    let Some(value) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .map(|(_, value)| value.trim())
    else {
        return (None, None);
    };
    if let Ok(seconds) = value.parse::<u64>() {
        return (
            Some(Duration::from_secs(seconds)),
            Some(RetryAfterSource::DeltaSeconds),
        );
    }
    if let Ok(retry_at) = httpdate::parse_http_date(value) {
        return (
            Some(retry_at.duration_since(now).unwrap_or(Duration::ZERO)),
            Some(RetryAfterSource::HttpDate),
        );
    }
    (None, None)
}

impl Default for ErrorClassifier {
    fn default() -> Self {
        Self {
            relay_profile: RelayProfile::OfficialOpenAi,
            balance_scope: BalanceScope::Credential,
            keep_codes: vec!["key_switch_cooldown".to_string()],
            switch_codes: vec![
                "insufficient_quota".to_string(),
                "quota_exceeded".to_string(),
                "rate_limit_exceeded".to_string(),
                "rate_limit_cooldown".to_string(),
                "billing_hard_limit_reached".to_string(),
            ],
            expire_codes: vec!["invalid_api_key".to_string()],
            keep_statuses: Vec::new(),
            switch_statuses: vec![
                StatusMatcher::Exact(429),
                StatusMatcher::Range {
                    start: 500,
                    end: 599,
                },
            ],
            expire_statuses: vec![StatusMatcher::Exact(401), StatusMatcher::Exact(403)],
            adaptation_rules: Vec::new(),
        }
    }
}

impl StatusMatcher {
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        if let Some(prefix) = raw.strip_suffix("xx") {
            let hundreds: u16 = prefix.parse()?;
            anyhow::ensure!(
                (1..=5).contains(&hundreds),
                "invalid HTTP status range {raw}"
            );
            return Ok(Self::Range {
                start: hundreds * 100,
                end: hundreds * 100 + 99,
            });
        }
        let status: u16 = raw.parse()?;
        anyhow::ensure!(
            (100..=599).contains(&status),
            "invalid HTTP status {status}"
        );
        Ok(Self::Exact(status))
    }

    pub fn as_config_string(&self) -> String {
        match self {
            StatusMatcher::Exact(status) => status.to_string(),
            StatusMatcher::Range { start, end } if start % 100 == 0 && *end == *start + 99 => {
                format!("{}xx", start / 100)
            }
            StatusMatcher::Range { start, end } => format!("{start}-{end}"),
        }
    }

    fn matches(&self, status: u16) -> bool {
        match self {
            StatusMatcher::Exact(value) => *value == status,
            StatusMatcher::Range { start, end } => status >= *start && status <= *end,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpstreamErrorEvidence {
    status: u16,
    code: Option<String>,
    limit_type: Option<String>,
    has_top_level_error_object: bool,
}

impl UpstreamErrorEvidence {
    fn from_response(status: u16, body: &[u8]) -> Self {
        let value = serde_json::from_slice::<Value>(body).ok();
        Self {
            status,
            code: value.as_ref().and_then(error_code_from_value),
            limit_type: value.as_ref().and_then(limit_type_from_value),
            has_top_level_error_object: value
                .as_ref()
                .and_then(|value| value.get("error"))
                .is_some_and(Value::is_object),
        }
    }
}

impl ErrorAdaptationRule {
    fn matches(&self, evidence: &UpstreamErrorEvidence) -> bool {
        if !self.enabled || self.id.trim().is_empty() || self.matcher.is_empty() {
            return false;
        }
        self.matcher.matches(evidence)
    }
}

impl ErrorAdaptationMatcher {
    fn is_empty(&self) -> bool {
        self.codes.is_empty() && self.limit_types.is_empty() && self.statuses.is_empty()
    }

    fn matches(&self, evidence: &UpstreamErrorEvidence) -> bool {
        if !self.codes.is_empty()
            && !evidence
                .code
                .as_deref()
                .is_some_and(|code| contains_code(&self.codes, code))
        {
            return false;
        }
        if !self.limit_types.is_empty()
            && !evidence
                .limit_type
                .as_deref()
                .is_some_and(|limit_type| contains_code(&self.limit_types, limit_type))
        {
            return false;
        }
        if !self.statuses.is_empty() && !matches_status(&self.statuses, evidence.status) {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, Default)]
pub struct ErrorClassifierSpec {
    pub relay_profile: RelayProfile,
    pub balance_scope: BalanceScope,
    pub keep_codes: Option<Vec<String>>,
    pub switch_codes: Option<Vec<String>>,
    pub expire_codes: Option<Vec<String>>,
    pub keep_statuses: Option<Vec<String>>,
    pub switch_statuses: Option<Vec<String>>,
    pub expire_statuses: Option<Vec<String>>,
    pub adaptation_rules: Vec<ErrorAdaptationRule>,
}

impl ErrorClassifierSpec {
    pub fn build(self) -> anyhow::Result<ErrorClassifier> {
        let default = ErrorClassifier::default();
        Ok(ErrorClassifier {
            relay_profile: self.relay_profile,
            balance_scope: self.balance_scope,
            keep_codes: merge_or_default(self.keep_codes, default.keep_codes),
            switch_codes: merge_or_default(self.switch_codes, default.switch_codes),
            expire_codes: merge_or_default(self.expire_codes, default.expire_codes),
            keep_statuses: parse_statuses(self.keep_statuses, default.keep_statuses)?,
            switch_statuses: parse_statuses(self.switch_statuses, default.switch_statuses)?,
            expire_statuses: parse_statuses(self.expire_statuses, default.expire_statuses)?,
            adaptation_rules: self.adaptation_rules,
        })
    }
}

fn parse_statuses(
    raw: Option<Vec<String>>,
    default: Vec<StatusMatcher>,
) -> anyhow::Result<Vec<StatusMatcher>> {
    match raw {
        Some(items) => items
            .iter()
            .map(|item| StatusMatcher::parse(item))
            .collect(),
        None => Ok(default),
    }
}

fn merge_or_default(configured: Option<Vec<String>>, default: Vec<String>) -> Vec<String> {
    configured.unwrap_or(default)
}

fn contains_code(codes: &[String], code: &str) -> bool {
    codes.iter().any(|candidate| candidate == code)
}

fn matches_status(matchers: &[StatusMatcher], status: u16) -> bool {
    matchers.iter().any(|matcher| matcher.matches(status))
}

fn status_matchers_as_config(matchers: &[StatusMatcher]) -> Vec<String> {
    matchers
        .iter()
        .map(|matcher| matcher.as_config_string())
        .collect()
}

fn error_code_from_value(value: &Value) -> Option<String> {
    value
        .pointer("/error/code")
        .or_else(|| value.get("code"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn limit_type_from_value(value: &Value) -> Option<String> {
    value
        .pointer("/error/limit_type")
        .or_else(|| value.get("limit_type"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(status: u16, body: &[u8]) -> ClassifiedFailure {
        ErrorClassifier::default().classify_failure(status, &[], body)
    }

    #[test]
    fn key_switch_cooldown_keeps_current_key() {
        let body = br#"{"error":{"code":"key_switch_cooldown"}}"#;
        let failure = classify(400, body);

        assert_eq!(failure.kind, FailureKind::KeySwitchCooldown);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn rate_limit_cooldown_switches_key() {
        let body = br#"{"error":{"code":"rate_limit_cooldown"}}"#;
        let failure = classify(400, body);

        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(failure.retryable);
    }

    #[test]
    fn unauthorized_switches_key() {
        let failure = classify(401, b"{}");

        assert_eq!(failure.kind, FailureKind::AuthInvalid);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn explicit_invalid_key_expires_key() {
        let body = br#"{"error":{"code":"invalid_api_key"}}"#;
        let failure = classify(400, body);

        assert_eq!(failure.kind, FailureKind::AuthInvalid);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn structured_invalid_key_failure_targets_credential() {
        let failure = ErrorClassifier::default().classify_failure(
            400,
            &[],
            br#"{"error":{"code":"invalid_api_key"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::AuthInvalid);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn structured_key_switch_cooldown_is_explicit_rate_limit_scope() {
        let failure = ErrorClassifier::default().classify_failure(
            400,
            &[],
            br#"{"error":{"code":"key_switch_cooldown"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::KeySwitchCooldown);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn retry_after_delta_seconds_preserves_upstream_cooldown() {
        let failure = ErrorClassifier::default().classify_failure(
            429,
            &[("retry-after", "120")],
            br#"{"error":{"code":"rate_limit_exceeded"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert_eq!(failure.cooldown, Some(std::time::Duration::from_secs(120)));
        assert_eq!(
            failure.retry_after_source,
            Some(RetryAfterSource::DeltaSeconds)
        );
    }

    #[test]
    fn retry_after_http_date_uses_duration_until_future_date() {
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let target = now + std::time::Duration::from_secs(120);
        let header = httpdate::fmt_http_date(target);

        let (cooldown, source) = retry_after_with_now(&[("retry-after", header.as_str())], now);

        assert_eq!(cooldown, Some(std::time::Duration::from_secs(120)));
        assert_eq!(source, Some(RetryAfterSource::HttpDate));
    }

    #[test]
    fn retry_after_http_date_in_past_uses_zero_duration() {
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let target = now - std::time::Duration::from_secs(120);
        let header = httpdate::fmt_http_date(target);

        let (cooldown, source) = retry_after_with_now(&[("retry-after", header.as_str())], now);

        assert_eq!(cooldown, Some(std::time::Duration::ZERO));
        assert_eq!(source, Some(RetryAfterSource::HttpDate));
    }

    #[test]
    fn unknown_error_code_falls_back_to_status_rule() {
        let failure = ErrorClassifier::default().classify_failure(
            502,
            &[],
            br#"{"error":{"code":"upstream_unavailable"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::ProviderUnavailable);
        assert_eq!(failure.primary_scope, FailureScope::Channel);
        assert!(failure.retryable);
        assert_eq!(
            failure.upstream_code.as_deref(),
            Some("upstream_unavailable")
        );
    }

    #[test]
    fn transport_failure_is_channel_provider_unavailable() {
        let failure = ErrorClassifier::default().classify_transport_failure();

        assert_eq!(failure.kind, FailureKind::ProviderUnavailable);
        assert_eq!(failure.primary_scope, FailureScope::Channel);
        assert!(failure.retryable);
        assert_eq!(failure.upstream_status, None);
        assert_eq!(failure.upstream_code.as_deref(), Some("transport_error"));
    }

    #[test]
    fn model_not_found_is_client_model_evidence_not_credential_failure() {
        let failure = ErrorClassifier::default().classify_failure(
            400,
            &[],
            br#"{"error":{"code":"model_not_found"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::ClientError);
        assert_eq!(failure.primary_scope, FailureScope::ModelGroup);
        assert!(!failure.retryable);
    }

    #[test]
    fn configured_rules_override_defaults() {
        let classifier = ErrorClassifierSpec {
            relay_profile: RelayProfile::OfficialOpenAi,
            balance_scope: BalanceScope::Credential,
            keep_codes: Some(vec!["rate_limit_cooldown".to_string()]),
            switch_codes: Some(vec!["custom_switch".to_string()]),
            expire_codes: Some(vec!["custom_expire".to_string()]),
            keep_statuses: Some(vec!["429".to_string()]),
            switch_statuses: Some(vec!["5xx".to_string()]),
            expire_statuses: Some(vec!["401".to_string()]),
            adaptation_rules: Vec::new(),
        }
        .build()
        .unwrap();

        let kept_code =
            classifier.classify_failure(400, &[], br#"{"error":{"code":"rate_limit_cooldown"}}"#);
        let kept_status = classifier.classify_failure(429, &[], b"{}");
        let switched_status = classifier.classify_failure(503, &[], b"{}");

        assert_eq!(kept_code.kind, FailureKind::Unknown);
        assert_eq!(kept_code.primary_scope, FailureScope::RequestOnly);
        assert!(!kept_code.retryable);
        assert_eq!(kept_status.kind, FailureKind::Unknown);
        assert_eq!(kept_status.primary_scope, FailureScope::RequestOnly);
        assert_eq!(switched_status.kind, FailureKind::ProviderUnavailable);
        assert_eq!(switched_status.primary_scope, FailureScope::Channel);
        assert!(switched_status.retryable);
    }

    #[test]
    fn explicit_empty_rules_disable_defaults() {
        let classifier = ErrorClassifierSpec {
            switch_codes: Some(Vec::new()),
            switch_statuses: Some(Vec::new()),
            ..Default::default()
        }
        .build()
        .unwrap();

        let code =
            classifier.classify_failure(400, &[], br#"{"error":{"code":"rate_limit_cooldown"}}"#);
        let rate_limited_status = classifier.classify_failure(429, &[], b"{}");
        let provider_status = classifier.classify_failure(503, &[], b"{}");

        assert_eq!(code.kind, FailureKind::Unknown);
        assert_eq!(code.primary_scope, FailureScope::RequestOnly);
        assert_eq!(rate_limited_status.kind, FailureKind::Unknown);
        assert_eq!(rate_limited_status.primary_scope, FailureScope::RequestOnly);
        assert_eq!(provider_status.kind, FailureKind::Unknown);
        assert_eq!(provider_status.primary_scope, FailureScope::RequestOnly);
    }

    #[test]
    fn adaptation_rule_can_attach_structured_cooldown_from_limit_type() {
        let classifier = ErrorClassifierSpec {
            adaptation_rules: vec![ErrorAdaptationRule {
                id: "relay-cooldown".to_string(),
                enabled: true,
                matcher: ErrorAdaptationMatcher {
                    codes: vec!["rate_limit_cooldown".to_string()],
                    limit_types: vec!["cooldown".to_string()],
                    statuses: Vec::new(),
                },
                action: ErrorAdaptationAction {
                    kind: Some(FailureKind::RateLimited),
                    primary_scope: Some(FailureScope::Credential),
                    retryable: Some(true),
                    cooldown: Some(Duration::from_secs(20)),
                },
            }],
            ..Default::default()
        }
        .build()
        .unwrap();

        let failure = classifier.classify_failure(
            400,
            &[],
            br#"{"error":{"code":"rate_limit_cooldown","limit_type":"cooldown"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(failure.retryable);
        assert_eq!(failure.cooldown, Some(Duration::from_secs(20)));
        assert_eq!(failure.upstream_limit_type.as_deref(), Some("cooldown"));
        assert_eq!(
            failure.adaptation_rule_id.as_deref(),
            Some("relay-cooldown")
        );
    }

    #[test]
    fn disabled_adaptation_rule_does_not_change_default_classification() {
        let classifier = ErrorClassifierSpec {
            adaptation_rules: vec![ErrorAdaptationRule {
                id: "disabled-relay-cooldown".to_string(),
                enabled: false,
                matcher: ErrorAdaptationMatcher {
                    codes: vec!["rate_limit_cooldown".to_string()],
                    limit_types: vec!["cooldown".to_string()],
                    statuses: Vec::new(),
                },
                action: ErrorAdaptationAction {
                    kind: Some(FailureKind::RateLimited),
                    primary_scope: Some(FailureScope::Credential),
                    retryable: Some(true),
                    cooldown: Some(Duration::from_secs(20)),
                },
            }],
            switch_codes: Some(Vec::new()),
            ..Default::default()
        }
        .build()
        .unwrap();

        let failure = classifier.classify_failure(
            400,
            &[],
            br#"{"error":{"code":"rate_limit_cooldown","limit_type":"cooldown"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::Unknown);
        assert_eq!(failure.primary_scope, FailureScope::RequestOnly);
        assert!(!failure.retryable);
        assert_eq!(failure.cooldown, None);
        assert_eq!(failure.upstream_limit_type.as_deref(), Some("cooldown"));
        assert_eq!(failure.adaptation_rule_id, None);
    }

    #[test]
    fn invalid_status_matchers_fail_fast() {
        assert!(StatusMatcher::parse("7xx").is_err());
        assert!(StatusMatcher::parse("999").is_err());
        assert!(StatusMatcher::parse("99").is_err());
    }

    fn classifier_for_relay_profile(relay_profile: RelayProfile) -> ErrorClassifier {
        ErrorClassifierSpec {
            relay_profile,
            balance_scope: BalanceScope::Credential,
            ..Default::default()
        }
        .build()
        .unwrap()
    }

    #[test]
    fn phase_1a_official_bare_401_expires_credential() {
        let failure = classifier_for_relay_profile(RelayProfile::OfficialOpenAi).classify_failure(
            401,
            &[],
            b"{}",
        );

        assert_eq!(failure.kind, FailureKind::AuthInvalid);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn phase_1a_generic_bare_401_is_request_only() {
        let failure = classifier_for_relay_profile(RelayProfile::GenericRelay).classify_failure(
            401,
            &[],
            b"{}",
        );

        assert_eq!(failure.kind, FailureKind::ClientError);
        assert_eq!(failure.primary_scope, FailureScope::RequestOnly);
        assert!(!failure.retryable);
    }

    #[test]
    fn phase_1a_untrusted_bare_403_is_request_only() {
        let failure = classifier_for_relay_profile(RelayProfile::UntrustedRelay).classify_failure(
            403,
            &[],
            b"{}",
        );

        assert_eq!(failure.kind, FailureKind::ClientError);
        assert_eq!(failure.primary_scope, FailureScope::RequestOnly);
        assert!(!failure.retryable);
    }

    #[test]
    fn phase_1a_structured_invalid_key_expires_credential_for_all_profiles() {
        for relay_profile in [
            RelayProfile::OfficialOpenAi,
            RelayProfile::GenericRelay,
            RelayProfile::UntrustedRelay,
        ] {
            let failure = classifier_for_relay_profile(relay_profile).classify_failure(
                400,
                &[],
                br#"{"error":{"code":"invalid_api_key"}}"#,
            );

            assert_eq!(failure.kind, FailureKind::AuthInvalid);
            assert_eq!(failure.primary_scope, FailureScope::Credential);
            assert!(!failure.retryable);
        }
    }

    #[test]
    fn phase_1a_bare_429_is_credential_rate_limited_for_all_profiles() {
        for relay_profile in [
            RelayProfile::OfficialOpenAi,
            RelayProfile::GenericRelay,
            RelayProfile::UntrustedRelay,
        ] {
            let failure =
                classifier_for_relay_profile(relay_profile).classify_failure(429, &[], b"{}");

            assert_eq!(failure.kind, FailureKind::RateLimited);
            assert_eq!(failure.primary_scope, FailureScope::Credential);
            assert!(failure.retryable);
        }
    }

    #[test]
    fn phase_1a_structured_quota_with_credential_scope_exhausts_credential() {
        let failure = ErrorClassifierSpec {
            relay_profile: RelayProfile::GenericRelay,
            balance_scope: BalanceScope::Credential,
            ..Default::default()
        }
        .build()
        .unwrap()
        .classify_failure(400, &[], br#"{"error":{"code":"insufficient_quota"}}"#);

        assert_eq!(failure.kind, FailureKind::QuotaExhausted);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn phase_1b_structured_balance_with_channel_scope_suppresses_channel() {
        let failure = ErrorClassifierSpec {
            relay_profile: RelayProfile::GenericRelay,
            balance_scope: BalanceScope::Channel,
            ..Default::default()
        }
        .build()
        .unwrap()
        .classify_failure(
            400,
            &[],
            br#"{"error":{"code":"insufficient_quota","limit_type":"balance"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::RelayBalanceUnavailable);
        assert_eq!(failure.primary_scope, FailureScope::Channel);
        assert!(failure.retryable);
        assert_eq!(failure.upstream_limit_type.as_deref(), Some("balance"));
    }

    #[test]
    fn phase_1a_code_less_top_level_error_object_is_request_only() {
        let failure = classifier_for_relay_profile(RelayProfile::OfficialOpenAi).classify_failure(
            401,
            &[],
            br#"{"error":{"message":"synthetic client error"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::ClientError);
        assert_eq!(failure.primary_scope, FailureScope::RequestOnly);
        assert!(!failure.retryable);
        assert_eq!(failure.upstream_code, None);
    }
}
