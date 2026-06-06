use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    collections::{HashMap, HashSet},
    fs,
    hash::{Hash, Hasher},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    time::Duration,
};

use sha2::{Digest, Sha256};

use crate::{
    credential_repository::{
        CredentialRepository, CredentialSetId, CredentialSetSource, KeyImport, KeyImportReport,
    },
    endpoint_capabilities::{EndpointCapabilitiesConfig, ResolvedEndpointCapabilities},
    error::{
        BalanceScope, ErrorAdaptationAction, ErrorAdaptationMatcher, ErrorAdaptationRule,
        ErrorClassifier, ErrorClassifierSpec, FailureKind, FailureScope, RelayProfile,
        StatusMatcher,
    },
    pool::{KeyPoolConfig, PoolCredentialInput},
    provider::ProviderKind,
    response_filter::{
        ResponseFilterAction, ResponseFilterPolicy, ResponseFilterRuleKind, ResponseFilterRuleSpec,
        ResponseFilterSpec,
    },
    route_plan::{ModelRoute, RouteStrategy, RouteTarget},
    routing::RoutingPolicy,
    state::ChannelId,
};

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    #[serde(default)]
    pub client_tokens: Vec<ClientTokenConfig>,
    pub management: Option<ManagementConfig>,
    #[serde(default = "default_max_request_body_bytes")]
    pub max_request_body_bytes: usize,
    #[serde(default = "default_max_model_catalog_body_bytes")]
    pub max_model_catalog_body_bytes: usize,
    #[serde(default = "default_max_error_body_bytes")]
    pub max_error_body_bytes: usize,
    #[serde(default)]
    pub timeouts: TimeoutConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
    pub default_pool: Option<String>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub accounts: HashMap<String, AccountConfig>,
    #[serde(default)]
    pub policy_profiles: HashMap<String, PolicyProfileConfig>,
    #[serde(default)]
    pub default_routing_profile: Option<String>,
    #[serde(default)]
    pub routing_profiles: HashMap<String, RoutingProfileConfig>,
    #[serde(default)]
    pub credential_sets: HashMap<String, CredentialSetConfig>,
    #[serde(default)]
    pub model_routes: HashMap<String, ModelRouteConfig>,
    pub pools: HashMap<String, PoolConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientTokenConfig {
    pub name: String,
    pub token: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_model_groups: Vec<String>,
    #[serde(default)]
    pub allowed_channels: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementConfig {
    pub admin_token: String,
    #[serde(default)]
    pub ip_allowlist: Option<Vec<IpAddr>>,
    #[serde(default)]
    pub principals: Vec<ManagementPrincipalConfig>,
    #[serde(default)]
    pub event_log_path: Option<PathBuf>,
    #[serde(default)]
    pub event_window_capacity: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementPrincipalConfig {
    pub name: String,
    pub token: String,
    pub role: ManagementRole,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeoutConfig {
    pub connect_seconds: Option<u64>,
    pub non_streaming_total_seconds: Option<u64>,
    pub streaming_idle_seconds: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ResolvedTimeoutProfile {
    pub connect: Duration,
    pub non_streaming_total: Duration,
    pub streaming_idle: Duration,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingConfig {
    pub max_route_candidates: Option<usize>,
    pub max_model_catalog_channels: Option<usize>,
    pub telemetry_buffer_capacity: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ResolvedRoutingConfig {
    pub max_route_candidates: usize,
    pub max_model_catalog_channels: usize,
    pub telemetry_buffer_capacity: usize,
}

#[derive(Debug, Clone)]
pub struct ResolvedResponseFilterConfig {
    pub policy: ResponseFilterPolicy,
    pub event_window_capacity: usize,
    pub alert_window: Duration,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResponseFilterConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub replacement: Option<String>,
    #[serde(default)]
    pub event_window_capacity: Option<usize>,
    #[serde(default)]
    pub alert_window_seconds: Option<u64>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub rules: Vec<ResponseFilterRuleConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResponseFilterRuleConfig {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub kind: ResponseFilterRuleKindConfig,
    #[serde(default)]
    pub action: ResponseFilterActionConfig,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFilterRuleKindConfig {
    Literal,
    Regex,
    RequiredLiteral,
    RequiredRegex,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFilterActionConfig {
    #[default]
    Redact,
    Reject,
    RejectAndExpireCredential,
    RejectAndCooldownChannel,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub provider_kind: ProviderKind,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    pub provider: String,
    pub api_base: String,
    #[serde(default = "default_auth_header")]
    pub auth_header: String,
    #[serde(default = "default_auth_prefix")]
    pub auth_prefix: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PoolConfig {
    #[serde(default)]
    pub endpoint_capabilities: EndpointCapabilitiesConfig,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub policy_profile: Option<String>,
    #[serde(default)]
    pub routing_profile: Option<String>,
    #[serde(default)]
    pub provider_kind: ProviderKind,
    pub api_base: String,
    pub credential_set: String,
    #[serde(default = "default_auth_header")]
    pub auth_header: String,
    #[serde(default = "default_auth_prefix")]
    pub auth_prefix: String,
    #[serde(default)]
    pub error_rules: ErrorRulesConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialSetConfig {
    pub keys_file: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelGroupConfig {
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyProfileConfig {
    #[serde(default)]
    pub error_rules: ErrorRulesConfig,
    #[serde(default)]
    pub probe_result_actions: ProbeResultActionConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProbeResultActionConfig {
    pub success: Option<ProbeResultActionKindConfig>,
    pub invalid: Option<ProbeResultActionKindConfig>,
    pub quota_exhausted: Option<ProbeResultActionKindConfig>,
    pub rate_limited: Option<ProbeResultActionKindConfig>,
    pub provider_unavailable: Option<ProbeResultActionKindConfig>,
    pub unsupported_model: Option<ProbeResultActionKindConfig>,
    pub unknown: Option<ProbeResultActionKindConfig>,
    pub cooldown_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeResultActionKindConfig {
    Noop,
    Expire,
    QuotaExhaust,
    Restore,
    Cooldown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResultActionKind {
    Noop,
    Expire,
    QuotaExhaust,
    Restore,
    Cooldown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResultPolicy {
    pub success: ProbeResultActionKind,
    pub invalid: ProbeResultActionKind,
    pub quota_exhausted: ProbeResultActionKind,
    pub rate_limited: ProbeResultActionKind,
    pub provider_unavailable: ProbeResultActionKind,
    pub unsupported_model: ProbeResultActionKind,
    pub unknown: ProbeResultActionKind,
    pub cooldown: Duration,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoutingProfileConfig {
    pub key_selection: KeySelectionStrategyConfig,
    pub default_credential_cooldown_seconds: u64,
    pub same_request_credential_retry: SameRequestCredentialRetryConfig,
    pub route_target_retry: RouteTargetRetryConfig,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeySelectionStrategyConfig {
    StickyUntilFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySelectionStrategy {
    StickyUntilFailure,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SameRequestCredentialRetryConfig {
    pub enabled: bool,
    pub max_retries: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteTargetRetryConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorRulesConfig {
    pub relay_profile: Option<RelayProfile>,
    pub balance_scope: Option<BalanceScope>,
    pub keep_codes: Option<Vec<String>>,
    pub switch_codes: Option<Vec<String>>,
    pub expire_codes: Option<Vec<String>>,
    pub keep_statuses: Option<Vec<String>>,
    pub switch_statuses: Option<Vec<String>>,
    pub expire_statuses: Option<Vec<String>>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub adaptation_rules: Vec<ErrorAdaptationRuleConfig>,
}

fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorAdaptationRuleConfig {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub matcher: ErrorAdaptationMatcherConfig,
    #[serde(default)]
    pub action: ErrorAdaptationActionConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorAdaptationMatcherConfig {
    #[serde(default)]
    pub codes: Vec<String>,
    #[serde(default)]
    pub limit_types: Vec<String>,
    #[serde(default)]
    pub statuses: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorAdaptationActionConfig {
    pub kind: Option<FailureKind>,
    pub primary_scope: Option<FailureScope>,
    pub retryable: Option<bool>,
    pub cooldown_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelRouteConfig {
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub targets: Vec<ModelRouteTargetConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelRouteTargetConfig {
    pub channel: String,
    #[serde(default)]
    pub upstream_model: Option<String>,
    #[serde(default = "default_route_target_priority")]
    pub priority: u16,
    #[serde(default = "default_route_target_weight")]
    pub weight: u16,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub listen: SocketAddr,
    pub client_tokens: Vec<ResolvedClientToken>,
    pub management_principals: Vec<ResolvedManagementPrincipal>,
    pub management_ip_allowlist: ResolvedManagementIpAllowlist,
    pub max_request_body_bytes: usize,
    pub max_model_catalog_body_bytes: usize,
    pub max_error_body_bytes: usize,
    pub timeout_profile: ResolvedTimeoutProfile,
    pub routing: ResolvedRoutingConfig,
    pub response_filter: ResponseFilterPolicy,
    pub response_filter_event_window_capacity: usize,
    pub response_filter_alert_window: Duration,
    pub management_event_log_path: Option<PathBuf>,
    pub management_event_window_capacity: usize,
    pub credential_store_path: Option<PathBuf>,
    pub policy_profiles: HashMap<String, ResolvedPolicyProfile>,
    pub routing_profiles: HashMap<String, ResolvedRoutingProfile>,
    pub default_pool: Option<String>,
    pub pools: HashMap<String, ResolvedPoolConfig>,
    pub model_groups: HashMap<String, ResolvedModelGroup>,
    pub model_routes: HashMap<String, ModelRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedManagementIpAllowlist {
    Explicit(Vec<IpAddr>),
    LoopbackOnly,
}

impl ResolvedManagementIpAllowlist {
    pub fn allows_ip(&self, ip: IpAddr) -> bool {
        match self {
            Self::Explicit(ips) => ips.contains(&ip),
            Self::LoopbackOnly => ip.is_loopback(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedModelGroup {
    pub id: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedPolicyProfile {
    pub id: String,
    pub error_rules: ErrorRulesConfig,
    pub probe_result_policy: ProbeResultPolicy,
}

#[derive(Debug, Clone)]
pub struct ResolvedRoutingProfile {
    pub id: String,
    pub key_selection: KeySelectionStrategy,
    pub default_credential_cooldown: Duration,
    pub same_request_credential_retry_enabled: bool,
    pub max_same_request_retries: usize,
    pub route_target_retry_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedClientToken {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub enabled: bool,
    pub allowed_model_groups: Vec<String>,
    pub allowed_channels: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedManagementPrincipal {
    pub id: String,
    pub name: String,
    pub role: ManagementRole,
    pub token_hash: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagementRole {
    Readonly,
    Operator,
    Admin,
}

impl ManagementRole {
    pub fn allows(self, minimum: Self) -> bool {
        self.rank() >= minimum.rank()
    }

    fn rank(self) -> u8 {
        match self {
            Self::Readonly => 0,
            Self::Operator => 1,
            Self::Admin => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedPoolConfig {
    pub config_generation: u64,
    pub configured_enabled: bool,
    pub provider_id: String,
    pub account_id: String,
    pub provider_enabled: bool,
    pub account_configured_enabled: bool,
    pub account_enabled: bool,
    pub error_policy_sources: ResolvedErrorPolicySources,
    pub probe_result_policy: ProbeResultPolicy,
    pub routing_profile_id: String,
    pub routing_policy: RoutingPolicy,
    pub routing_policy_sources: ResolvedRoutingPolicySources,
    pub credential_set_id: CredentialSetId,
    pub provider_kind: ProviderKind,
    pub endpoint_capabilities: ResolvedEndpointCapabilities,
    pub auth_header: String,
    pub auth_prefix: String,
    pub error_classifier: ErrorClassifier,
    pub key_import_report: KeyImportReport,
    pub key_pool: KeyPoolConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedErrorPolicySources {
    pub profile_id: Option<String>,
    pub has_pool_override: bool,
    pub adaptation_rule_sources: Vec<AdaptationRuleSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdaptationRuleSource {
    pub id: String,
    pub source: ErrorPolicyRuleSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorPolicyRuleSource {
    Profile { profile_id: String },
    PoolOverride,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoutingPolicySources {
    pub profile_id: String,
}

impl ErrorRulesConfig {
    fn into_classifier_with_context(self, context: &str) -> anyhow::Result<ErrorClassifier> {
        ErrorClassifierSpec {
            relay_profile: self.relay_profile.unwrap_or_default(),
            balance_scope: validate_balance_scope(context, self.balance_scope.unwrap_or_default())?,
            keep_codes: self.keep_codes,
            switch_codes: self.switch_codes,
            expire_codes: self.expire_codes,
            keep_statuses: self.keep_statuses,
            switch_statuses: self.switch_statuses,
            expire_statuses: self.expire_statuses,
            adaptation_rules: self
                .adaptation_rules
                .into_iter()
                .map(|rule| {
                    let rule_id = rule.id.trim().to_string();
                    rule.into_rule(&format!("{context}.adaptation_rules.{rule_id}"))
                })
                .collect::<anyhow::Result<Vec<_>>>()?,
        }
        .build()
    }
}

impl ProbeResultPolicy {
    fn from_config(config: &ProbeResultActionConfig) -> anyhow::Result<Self> {
        let policy = Self {
            success: probe_action_or_default(config.success, ProbeResultActionKind::Restore),
            invalid: probe_action_or_default(config.invalid, ProbeResultActionKind::Expire),
            quota_exhausted: probe_action_or_default(
                config.quota_exhausted,
                ProbeResultActionKind::Noop,
            ),
            rate_limited: probe_action_or_default(config.rate_limited, ProbeResultActionKind::Noop),
            provider_unavailable: probe_action_or_default(
                config.provider_unavailable,
                ProbeResultActionKind::Noop,
            ),
            unsupported_model: probe_action_or_default(
                config.unsupported_model,
                ProbeResultActionKind::Noop,
            ),
            unknown: probe_action_or_default(config.unknown, ProbeResultActionKind::Noop),
            cooldown: Duration::from_secs(config.cooldown_seconds.unwrap_or(20).max(1)),
        };
        validate_probe_result_policy(&policy)?;
        Ok(policy)
    }
}

impl Default for ProbeResultPolicy {
    fn default() -> Self {
        Self {
            success: ProbeResultActionKind::Restore,
            invalid: ProbeResultActionKind::Expire,
            quota_exhausted: ProbeResultActionKind::Noop,
            rate_limited: ProbeResultActionKind::Noop,
            provider_unavailable: ProbeResultActionKind::Noop,
            unsupported_model: ProbeResultActionKind::Noop,
            unknown: ProbeResultActionKind::Noop,
            cooldown: Duration::from_secs(20),
        }
    }
}

fn probe_action_or_default(
    action: Option<ProbeResultActionKindConfig>,
    default: ProbeResultActionKind,
) -> ProbeResultActionKind {
    match action {
        Some(ProbeResultActionKindConfig::Noop) => ProbeResultActionKind::Noop,
        Some(ProbeResultActionKindConfig::Expire) => ProbeResultActionKind::Expire,
        Some(ProbeResultActionKindConfig::QuotaExhaust) => ProbeResultActionKind::QuotaExhaust,
        Some(ProbeResultActionKindConfig::Restore) => ProbeResultActionKind::Restore,
        Some(ProbeResultActionKindConfig::Cooldown) => ProbeResultActionKind::Cooldown,
        None => default,
    }
}

fn validate_probe_result_policy(policy: &ProbeResultPolicy) -> anyhow::Result<()> {
    anyhow::ensure!(
        policy.provider_unavailable != ProbeResultActionKind::Expire,
        "probe_result_actions.provider_unavailable cannot expire credentials"
    );
    anyhow::ensure!(
        policy.provider_unavailable != ProbeResultActionKind::QuotaExhaust,
        "probe_result_actions.provider_unavailable cannot quota-exhaust credentials"
    );
    anyhow::ensure!(
        policy.unsupported_model != ProbeResultActionKind::Expire,
        "probe_result_actions.unsupported_model cannot expire credentials"
    );
    anyhow::ensure!(
        policy.unsupported_model != ProbeResultActionKind::QuotaExhaust,
        "probe_result_actions.unsupported_model cannot quota-exhaust credentials"
    );
    Ok(())
}

impl ErrorAdaptationRuleConfig {
    fn into_rule(self, context: &str) -> anyhow::Result<ErrorAdaptationRule> {
        let id = self.id.trim().to_string();
        anyhow::ensure!(!id.is_empty(), "adaptation rule id must not be empty");
        let raw_action = self.action;
        let action = raw_action.clone().into_action(context)?;
        let matcher = self.matcher.into_matcher()?;
        anyhow::ensure!(
            !matcher.codes.is_empty()
                || !matcher.limit_types.is_empty()
                || !matcher.statuses.is_empty(),
            "adaptation rule {id} matcher must not be empty"
        );
        validate_key_switch_cooldown_action_override(context, &matcher, &raw_action)?;
        Ok(ErrorAdaptationRule {
            id,
            enabled: self.enabled,
            matcher,
            action,
        })
    }
}

fn ensure_key_switch_cooldown_action_has_no_overrides(
    context: &str,
    retryable: Option<bool>,
    cooldown_seconds: Option<u64>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        retryable != Some(true),
        "{context}.action kind key_switch_cooldown with primary_scope credential cannot be retryable"
    );
    anyhow::ensure!(
        cooldown_seconds.is_none(),
        "{context}.action kind key_switch_cooldown with primary_scope credential cannot set cooldown_seconds"
    );
    Ok(())
}

fn validate_key_switch_cooldown_action_override(
    context: &str,
    matcher: &ErrorAdaptationMatcher,
    action: &ErrorAdaptationActionConfig,
) -> anyhow::Result<()> {
    if !matcher.codes.is_empty()
        && !matcher
            .codes
            .iter()
            .any(|code| code == "key_switch_cooldown")
    {
        return Ok(());
    }
    let effective_kind = action.kind.unwrap_or(FailureKind::KeySwitchCooldown);
    let effective_scope = action.primary_scope.unwrap_or(FailureScope::Credential);
    if !matches!(
        (effective_kind, effective_scope),
        (FailureKind::KeySwitchCooldown, FailureScope::Credential)
    ) {
        return Ok(());
    }
    ensure_key_switch_cooldown_action_has_no_overrides(
        context,
        action.retryable,
        action.cooldown_seconds,
    )
}

impl ErrorAdaptationMatcherConfig {
    fn into_matcher(self) -> anyhow::Result<ErrorAdaptationMatcher> {
        Ok(ErrorAdaptationMatcher {
            codes: self.codes,
            limit_types: self.limit_types,
            statuses: self
                .statuses
                .iter()
                .map(|status| StatusMatcher::parse(status))
                .collect::<anyhow::Result<Vec<_>>>()?,
        })
    }
}

impl ErrorAdaptationActionConfig {
    fn into_action(self, context: &str) -> anyhow::Result<ErrorAdaptationAction> {
        validate_adaptation_action_combination(context, self.kind, self.primary_scope)?;
        if matches!(
            (self.kind, self.primary_scope),
            (
                Some(FailureKind::KeySwitchCooldown),
                Some(FailureScope::Credential)
            )
        ) {
            ensure_key_switch_cooldown_action_has_no_overrides(
                context,
                self.retryable,
                self.cooldown_seconds,
            )?;
        }
        anyhow::ensure!(
            self.cooldown_seconds != Some(0),
            "{context}.action.cooldown_seconds must be greater than zero"
        );
        Ok(ErrorAdaptationAction {
            kind: self.kind,
            primary_scope: self.primary_scope,
            retryable: self.retryable,
            cooldown: self.cooldown_seconds.map(Duration::from_secs),
        })
    }
}

impl AppConfig {
    pub fn apply_compatibility_defaults(&mut self) {
        if self.default_pool.is_none() && self.pools.len() == 1 {
            self.default_pool = self.pools.keys().next().cloned();
        }
    }

    pub fn into_registry_document(self) -> crate::registry::RegistryDocument {
        crate::registry::RegistryDocument {
            listen: self.listen,
            client_tokens: self.client_tokens,
            management: self.management,
            max_request_body_bytes: self.max_request_body_bytes,
            max_model_catalog_body_bytes: self.max_model_catalog_body_bytes,
            max_error_body_bytes: self.max_error_body_bytes,
            timeouts: self.timeouts,
            routing: self.routing,
            response_filter: ResponseFilterConfig::default(),
            default_pool: self.default_pool,
            providers: self.providers,
            accounts: self.accounts,
            credential_sets: self.credential_sets,
            model_groups: HashMap::new(),
            policy_profiles: self.policy_profiles,
            default_routing_profile: self.default_routing_profile,
            routing_profiles: self.routing_profiles,
            model_routes: self.model_routes,
            pools: self.pools,
        }
    }

    // Compatibility entrypoints remain for legacy callers while startup uses RegistryDocument.
    #[allow(dead_code)]
    pub fn from_path(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let raw = fs::read_to_string(&path)?;
        let raw = crate::upstream_templates::expand_raw_yaml(&raw)?;
        reject_unknown_top_level_config_fields(&raw, &[])?;
        let mut cfg: AppConfig = serde_yaml::from_str(&raw)?;
        cfg.apply_compatibility_defaults();
        Ok(cfg)
    }

    #[allow(dead_code)]
    pub fn resolve(self) -> anyhow::Result<ResolvedConfig> {
        self.into_registry_document().resolve()
    }

    #[allow(dead_code)]
    pub fn resolve_with_credential_repository(
        self,
        credential_repository: &impl CredentialRepository,
    ) -> anyhow::Result<ResolvedConfig> {
        self.into_registry_document()
            .resolve_with_credential_repository(credential_repository)
    }

    #[allow(dead_code)]
    pub fn resolve_with_credential_repository_and_store_path(
        self,
        credential_repository: &impl CredentialRepository,
        credential_store_path: Option<PathBuf>,
    ) -> anyhow::Result<ResolvedConfig> {
        self.into_registry_document()
            .resolve_with_credential_repository_and_store_path(
                credential_repository,
                credential_store_path,
            )
    }

    fn resolve_inner_with_credential_repository_and_store_path(
        self,
        explicit_model_groups: HashMap<String, ModelGroupConfig>,
        credential_repository: &impl CredentialRepository,
        credential_store_path: Option<PathBuf>,
    ) -> anyhow::Result<ResolvedConfig> {
        let management = self
            .management
            .ok_or_else(|| anyhow::anyhow!("management.admin_token must be set"))?;
        let admin_token = management.admin_token.trim();
        anyhow::ensure!(
            !admin_token.is_empty(),
            "management.admin_token must not be empty"
        );
        let management_event_window_capacity = management
            .event_window_capacity
            .unwrap_or_else(default_management_event_window_capacity);
        anyhow::ensure!(
            management_event_window_capacity > 0,
            "management.event_window_capacity must be greater than zero"
        );
        let management_ip_allowlist =
            resolve_management_ip_allowlist(self.listen, management.ip_allowlist.clone())?;

        let mut client_tokens = Vec::new();
        let mut client_token_names = HashSet::new();
        let mut client_token_hashes = HashSet::new();
        for token_config in self.client_tokens {
            let token = token_config.token.trim();
            anyhow::ensure!(!token.is_empty(), "client token must not be empty");
            let name = token_config.name.trim();
            anyhow::ensure!(!name.is_empty(), "client token name must not be empty");
            anyhow::ensure!(
                client_token_names.insert(name.to_string()),
                "duplicate client token name {name}"
            );
            let token_hash = hash_token(token);
            anyhow::ensure!(
                client_token_hashes.insert(token_hash.clone()),
                "duplicate client token secret for {name}"
            );
            client_tokens.push(ResolvedClientToken {
                id: stable_id("client", name),
                name: name.to_string(),
                token_hash,
                enabled: token_config.enabled,
                allowed_model_groups: token_config.allowed_model_groups,
                allowed_channels: token_config.allowed_channels,
            });
        }

        if credential_store_path.is_none() {
            anyhow::ensure!(
                !client_tokens.is_empty(),
                "at least one client token must be configured"
            );
        }

        let mut pool_configs = HashMap::new();
        for (id, pool) in self.pools {
            let normalized_id = normalize_resource_id("pools", id)?;
            anyhow::ensure!(
                pool_configs.insert(normalized_id.clone(), pool).is_none(),
                "duplicate pools id {normalized_id}"
            );
        }
        for client_token in &mut client_tokens {
            let mut allowed_channels = Vec::new();
            let mut seen = HashSet::new();
            for channel in std::mem::take(&mut client_token.allowed_channels) {
                let channel_id = normalize_resource_id(
                    &format!("client token {} allowed_channels", client_token.name),
                    channel,
                )?;
                anyhow::ensure!(
                    pool_configs.contains_key(&channel_id),
                    "client token {} references unknown channel {channel_id}",
                    client_token.name
                );
                if seen.insert(channel_id.clone()) {
                    allowed_channels.push(channel_id);
                }
            }
            client_token.allowed_channels = allowed_channels;
        }
        let default_pool = self
            .default_pool
            .map(|pool| normalize_resource_id("default_pool", pool))
            .transpose()?;

        let timeout_profile = self.timeouts.resolve()?;
        let routing = self.routing.resolve()?;
        let mut pools = HashMap::new();
        let mut credential_sets: HashMap<String, KeyImport> = HashMap::new();
        for (id, credential_set) in self.credential_sets {
            let normalized_id = normalize_resource_id("credential_sets", id)?;
            let credential_set_id = CredentialSetId(normalized_id.clone());
            let imported = credential_repository.load_credential_set(
                &credential_set_id,
                &CredentialSetSource::File {
                    path: credential_set.keys_file,
                },
            )?;
            credential_sets.insert(normalized_id, imported);
        }

        let mut providers = HashMap::new();
        for (id, provider) in self.providers {
            let id = normalize_resource_id("providers", id)?;
            providers.insert(id, provider);
        }
        let mut accounts = HashMap::new();
        for (id, account) in self.accounts {
            let id = normalize_resource_id("accounts", id)?;
            let provider_id = normalize_resource_id("accounts.provider", account.provider.clone())?;
            anyhow::ensure!(
                providers.contains_key(&provider_id),
                "account {id} references unknown provider {provider_id}"
            );
            accounts.insert(
                id,
                AccountConfig {
                    provider: provider_id,
                    api_base: account.api_base,
                    auth_header: account.auth_header,
                    auth_prefix: account.auth_prefix,
                    enabled: account.enabled,
                },
            );
        }
        let mut policy_profiles = HashMap::new();
        for (id, profile) in self.policy_profiles {
            let id = normalize_resource_id("policy_profiles", id)?;
            validate_unique_adaptation_rule_ids(
                &format!("policy_profile {id}"),
                &profile.error_rules.adaptation_rules,
            )?;
            profile
                .error_rules
                .clone()
                .into_classifier_with_context(&format!("policy_profiles.{id}.error_rules"))?;
            anyhow::ensure!(
                policy_profiles
                    .insert(
                        id.clone(),
                        ResolvedPolicyProfile {
                            id: id.clone(),
                            probe_result_policy: ProbeResultPolicy::from_config(
                                &profile.probe_result_actions,
                            )?,
                            error_rules: profile.error_rules,
                        },
                    )
                    .is_none(),
                "duplicate policy_profiles id {id}"
            );
        }

        let mut routing_profiles = HashMap::new();
        for (id, profile) in self.routing_profiles {
            let id = normalize_resource_id("routing_profiles", id)?;
            let resolved = resolve_routing_profile(&id, profile)?;
            anyhow::ensure!(
                routing_profiles.insert(id.clone(), resolved).is_none(),
                "duplicate routing_profiles id {id}"
            );
        }
        let default_routing_profile = self
            .default_routing_profile
            .map(|profile| normalize_resource_id("default_routing_profile", profile))
            .transpose()?;
        if let Some(default_routing_profile) = &default_routing_profile {
            anyhow::ensure!(
                routing_profiles.contains_key(default_routing_profile),
                "default_routing_profile {default_routing_profile} does not exist"
            );
        }

        let mut pool_entries: Vec<(String, PoolConfig)> = pool_configs.into_iter().collect();
        pool_entries.sort_by(|a, b| a.0.cmp(&b.0));

        let explicit_model_routes = self.model_routes;
        let mut model_routes: HashMap<String, ModelRoute> = HashMap::new();
        for (name, pool) in pool_entries {
            let credential_set_id = resolve_pool_credential_set_id(&name, &pool)?;
            let resolved_account = resolve_pool_account(&name, &pool, &providers, &accounts)?;
            let endpoint_capabilities = pool.endpoint_capabilities.resolve_with_base(
                resolved_account
                    .provider_kind
                    .default_endpoint_capabilities(),
            )?;
            let routing_profile_id =
                resolve_pool_routing_profile_id(&name, &pool, default_routing_profile.as_deref())?;
            let routing_profile = routing_profiles.get(&routing_profile_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "pool {name} references unknown routing_profile {routing_profile_id}"
                )
            })?;
            let routing_policy = RoutingPolicy {
                retry_switched_key_in_same_request: routing_profile
                    .same_request_credential_retry_enabled,
                max_same_request_retries: routing_profile.max_same_request_retries,
                route_target_retry_enabled: routing_profile.route_target_retry_enabled,
                default_credential_cooldown: routing_profile.default_credential_cooldown,
            };
            let (effective_error_rules, error_policy_sources) =
                resolve_pool_error_policy(&name, &pool, &policy_profiles)?;
            let probe_result_policy = resolve_pool_probe_result_policy(&pool, &policy_profiles)?;
            let imported = credential_sets
                .get(&credential_set_id.0)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "pool {name} references unknown credential_set {}",
                        credential_set_id.0
                    )
                })?
                .clone();
            let credentials = imported
                .credentials
                .into_iter()
                .map(|credential| PoolCredentialInput {
                    secret: credential.secret,
                    source: credential.source,
                })
                .collect();
            pools.insert(
                name.clone(),
                ResolvedPoolConfig {
                    config_generation: channel_config_generation(ChannelConfigGenerationInput {
                        channel_id: &name,
                        configured_enabled: pool.enabled,
                        account: &resolved_account,
                        endpoint_capabilities: &endpoint_capabilities,
                        credential_set_id: &credential_set_id,
                        routing_profile_id: &routing_profile_id,
                        routing_policy: &routing_policy,
                        error_rules: &effective_error_rules,
                    }),
                    configured_enabled: pool.enabled,
                    provider_id: resolved_account.provider_id,
                    account_id: resolved_account.account_id,
                    provider_enabled: resolved_account.provider_enabled,
                    account_configured_enabled: resolved_account.account_configured_enabled,
                    account_enabled: resolved_account.account_enabled,
                    credential_set_id: credential_set_id.clone(),
                    error_policy_sources,
                    probe_result_policy,
                    routing_profile_id: routing_profile_id.clone(),
                    routing_policy,
                    routing_policy_sources: ResolvedRoutingPolicySources {
                        profile_id: routing_profile_id,
                    },
                    provider_kind: resolved_account.provider_kind,
                    endpoint_capabilities,
                    auth_header: resolved_account.endpoint.auth_header,
                    auth_prefix: resolved_account.endpoint.auth_prefix,
                    error_classifier: effective_error_rules.into_classifier_with_context(
                        &format!("pools.{name}.effective_error_rules"),
                    )?,
                    key_import_report: imported.report,
                    key_pool: KeyPoolConfig {
                        name,
                        credential_namespace: credential_set_id.0,
                        api_base: resolved_account.endpoint.api_base,
                        credentials,
                    },
                },
            );
        }

        for (public_model, route_config) in explicit_model_routes {
            let strategy = parse_route_strategy(route_config.strategy.as_deref())?;
            anyhow::ensure!(
                !route_config.targets.is_empty(),
                "model_routes.{public_model}.targets must not be empty"
            );
            let mut targets = Vec::new();
            for target in route_config.targets {
                let channel_id =
                    normalize_resource_id("model_routes.target.channel", target.channel)?;
                let pool = pools.get(&channel_id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "model_routes.{public_model} target channel {channel_id} does not exist"
                    )
                })?;
                targets.push(RouteTarget {
                    channel_id: ChannelId(channel_id),
                    provider_kind: pool.provider_kind,
                    upstream_model: target.upstream_model,
                    priority: target.priority,
                    weight: target.weight,
                    enabled: target.enabled,
                });
            }
            targets.sort_by_key(|target| target.priority);
            model_routes.insert(
                public_model.clone(),
                ModelRoute {
                    public_model,
                    targets,
                    strategy,
                },
            );
        }

        let mut model_groups = HashMap::new();
        for (group_id, group_config) in explicit_model_groups {
            let group_id = normalize_resource_id("model_groups", group_id)?;
            anyhow::ensure!(
                !group_config.models.is_empty(),
                "model_groups.{group_id}.models must not be empty"
            );
            let mut models = Vec::new();
            let mut seen = HashSet::new();
            for model in group_config.models {
                let model = model.trim().to_string();
                anyhow::ensure!(
                    !model.is_empty(),
                    "model_groups.{group_id}.models must not contain empty model ids"
                );
                anyhow::ensure!(
                    model_routes.contains_key(&model),
                    "model_groups.{group_id} references unknown public model {model}"
                );
                if seen.insert(model.clone()) {
                    models.push(model);
                }
            }
            anyhow::ensure!(
                model_groups
                    .insert(
                        group_id.clone(),
                        ResolvedModelGroup {
                            id: group_id.clone(),
                            models,
                        },
                    )
                    .is_none(),
                "duplicate model_groups id {group_id}"
            );
        }

        if let Some(default_pool) = &default_pool {
            anyhow::ensure!(
                pools.contains_key(default_pool),
                "default_pool {default_pool} does not exist"
            );
        }

        let admin_token_hash = hash_token(admin_token);
        let mut management_principal_names = HashSet::from(["local-admin".to_string()]);
        let mut management_principal_hashes = HashSet::from([admin_token_hash.clone()]);
        let bootstrap_management_principal_id = stable_id("management", "admin");
        let mut management_principal_ids =
            HashSet::from([bootstrap_management_principal_id.clone()]);
        let management_principal = ResolvedManagementPrincipal {
            id: bootstrap_management_principal_id,
            name: "local-admin".to_string(),
            role: ManagementRole::Admin,
            token_hash: admin_token_hash,
            enabled: true,
        };
        let mut management_principals = vec![management_principal.clone()];
        for principal in management.principals {
            let name = principal.name.trim().to_string();
            anyhow::ensure!(
                !name.is_empty(),
                "management.principals name must not be empty"
            );
            anyhow::ensure!(
                management_principal_names.insert(name.clone()),
                "duplicate management principal name {name}"
            );
            let token = principal.token.trim();
            anyhow::ensure!(
                !token.is_empty(),
                "management.principals token must not be empty"
            );
            let token_hash = hash_token(token);
            anyhow::ensure!(
                management_principal_hashes.insert(token_hash.clone()),
                "duplicate management principal secret for {name}"
            );
            let id = stable_id("management", &name);
            anyhow::ensure!(
                management_principal_ids.insert(id.clone()),
                "duplicate management principal id for {name}"
            );
            management_principals.push(ResolvedManagementPrincipal {
                id,
                name,
                role: principal.role,
                token_hash,
                enabled: principal.enabled,
            });
        }

        Ok(ResolvedConfig {
            listen: self.listen,
            client_tokens,
            management_principals,
            management_ip_allowlist,
            max_request_body_bytes: self.max_request_body_bytes,
            max_model_catalog_body_bytes: self.max_model_catalog_body_bytes,
            max_error_body_bytes: self.max_error_body_bytes,
            timeout_profile,
            routing,
            response_filter: ResponseFilterPolicy::disabled(),
            response_filter_event_window_capacity: default_response_filter_event_window_capacity(),
            response_filter_alert_window: Duration::from_secs(
                default_response_filter_alert_window_seconds(),
            ),
            management_event_log_path: management.event_log_path,
            management_event_window_capacity,
            credential_store_path,
            policy_profiles,
            routing_profiles,
            default_pool,
            pools,
            model_groups,
            model_routes,
        })
    }
}

pub(crate) fn reject_unknown_top_level_config_fields(
    raw: &str,
    extra_allowed_fields: &[&str],
) -> anyhow::Result<()> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(raw)?;
    let serde_yaml::Value::Mapping(mapping) = yaml else {
        return Ok(());
    };
    for key in mapping.keys() {
        let Some(field) = key.as_str() else {
            continue;
        };
        if !APP_CONFIG_TOP_LEVEL_FIELDS.contains(&field) && !extra_allowed_fields.contains(&field) {
            anyhow::bail!("unknown top-level config field `{field}`");
        }
    }
    Ok(())
}

const APP_CONFIG_TOP_LEVEL_FIELDS: &[&str] = &[
    "listen",
    "client_tokens",
    "management",
    "max_request_body_bytes",
    "max_model_catalog_body_bytes",
    "max_error_body_bytes",
    "timeouts",
    "routing",
    "default_pool",
    "providers",
    "accounts",
    "policy_profiles",
    "default_routing_profile",
    "routing_profiles",
    "credential_sets",
    "model_routes",
    "pools",
];

pub(crate) fn resolve_registry_document_with_credential_repository_and_store_path(
    document: crate::registry::RegistryDocument,
    credential_repository: &impl CredentialRepository,
    credential_store_path: Option<PathBuf>,
) -> anyhow::Result<ResolvedConfig> {
    let explicit_model_groups = document.model_groups.clone();
    let response_filter = document.response_filter.clone().resolve()?;
    document
        .into_app_config_for_legacy_resolver()
        .resolve_inner_with_credential_repository_and_store_path(
            explicit_model_groups,
            credential_repository,
            credential_store_path,
        )
        .map(|mut config| {
            config.response_filter = response_filter.policy;
            config.response_filter_event_window_capacity = response_filter.event_window_capacity;
            config.response_filter_alert_window = response_filter.alert_window;
            config
        })
}

fn resolve_management_ip_allowlist(
    listen: SocketAddr,
    ip_allowlist: Option<Vec<IpAddr>>,
) -> anyhow::Result<ResolvedManagementIpAllowlist> {
    match ip_allowlist {
        Some(ips) if ips.is_empty() => {
            anyhow::bail!("management.ip_allowlist must not be empty")
        }
        Some(ips) => Ok(ResolvedManagementIpAllowlist::Explicit(ips)),
        None if listen.ip().is_loopback() => Ok(ResolvedManagementIpAllowlist::LoopbackOnly),
        None => anyhow::bail!(
            "management.ip_allowlist must be set when management is enabled on non-loopback listen address {listen}"
        ),
    }
}

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn stable_id(prefix: &str, value: &str) -> String {
    format!("{prefix}_{}", hash_token(value))
}

pub(crate) fn normalize_resource_id(field: &str, id: String) -> anyhow::Result<String> {
    let id = id.trim().to_string();
    anyhow::ensure!(!id.is_empty(), "{field} id must not be empty");
    Ok(id)
}

fn normalize_api_base(field: &str, api_base: String) -> anyhow::Result<String> {
    let api_base = api_base.trim().trim_end_matches('/').to_string();
    anyhow::ensure!(!api_base.is_empty(), "{field} api_base must not be empty");
    Ok(api_base)
}

fn normalize_auth_header(field: &str, auth_header: String) -> anyhow::Result<String> {
    let auth_header = auth_header.trim().to_string();
    anyhow::ensure!(
        !auth_header.is_empty(),
        "{field} auth_header must not be empty"
    );
    Ok(auth_header)
}

fn normalize_auth_prefix(auth_prefix: String) -> String {
    auth_prefix
}

fn resolve_pool_credential_set_id(
    channel_id: &str,
    pool: &PoolConfig,
) -> anyhow::Result<CredentialSetId> {
    let id = normalize_resource_id("pool.credential_set", pool.credential_set.clone())?;
    anyhow::ensure!(
        !id.is_empty(),
        "pool {channel_id} must reference a credential_set"
    );
    Ok(CredentialSetId(id))
}

fn resolve_pool_error_policy(
    channel_id: &str,
    pool: &PoolConfig,
    policy_profiles: &HashMap<String, ResolvedPolicyProfile>,
) -> anyhow::Result<(ErrorRulesConfig, ResolvedErrorPolicySources)> {
    validate_unique_adaptation_rule_ids(
        &format!("pool {channel_id}"),
        &pool.error_rules.adaptation_rules,
    )?;
    let profile_id = pool
        .policy_profile
        .clone()
        .map(|id| normalize_resource_id("pool.policy_profile", id))
        .transpose()?;
    let profile = match &profile_id {
        Some(id) => Some(policy_profiles.get(id).ok_or_else(|| {
            anyhow::anyhow!("pool {channel_id} references unknown policy_profile {id}")
        })?),
        None => None,
    };
    let mut merged = profile
        .map(|profile| profile.error_rules.clone())
        .unwrap_or_default();
    merged.keep_codes = pool.error_rules.keep_codes.clone().or(merged.keep_codes);
    merged.switch_codes = pool
        .error_rules
        .switch_codes
        .clone()
        .or(merged.switch_codes);
    merged.expire_codes = pool
        .error_rules
        .expire_codes
        .clone()
        .or(merged.expire_codes);
    merged.keep_statuses = pool
        .error_rules
        .keep_statuses
        .clone()
        .or(merged.keep_statuses);
    merged.switch_statuses = pool
        .error_rules
        .switch_statuses
        .clone()
        .or(merged.switch_statuses);
    merged.expire_statuses = pool
        .error_rules
        .expire_statuses
        .clone()
        .or(merged.expire_statuses);
    merged.relay_profile = pool.error_rules.relay_profile.or(merged.relay_profile);
    merged.balance_scope = pool.error_rules.balance_scope.or(merged.balance_scope);

    normalize_adaptation_rule_ids(&mut merged.adaptation_rules);
    let pool_adaptation_rules = normalized_adaptation_rules(&pool.error_rules.adaptation_rules);
    validate_unique_merged_adaptation_rule_ids(
        channel_id,
        merged
            .adaptation_rules
            .iter()
            .chain(pool_adaptation_rules.iter()),
    )?;

    let mut adaptation_rule_sources = Vec::new();
    if let Some(profile_id) = &profile_id {
        adaptation_rule_sources.extend(merged.adaptation_rules.iter().map(|rule| {
            AdaptationRuleSource {
                id: rule.id.clone(),
                source: ErrorPolicyRuleSource::Profile {
                    profile_id: profile_id.clone(),
                },
            }
        }));
    }
    adaptation_rule_sources.extend(
        pool_adaptation_rules
            .iter()
            .map(|rule| AdaptationRuleSource {
                id: rule.id.clone(),
                source: ErrorPolicyRuleSource::PoolOverride,
            }),
    );
    merged.adaptation_rules.extend(pool_adaptation_rules);

    Ok((
        merged,
        ResolvedErrorPolicySources {
            profile_id,
            has_pool_override: error_rules_has_override(&pool.error_rules),
            adaptation_rule_sources,
        },
    ))
}

fn resolve_pool_probe_result_policy(
    pool: &PoolConfig,
    policy_profiles: &HashMap<String, ResolvedPolicyProfile>,
) -> anyhow::Result<ProbeResultPolicy> {
    let profile_id = pool
        .policy_profile
        .clone()
        .map(|id| normalize_resource_id("pool.policy_profile", id))
        .transpose()?;
    Ok(match profile_id {
        Some(id) => policy_profiles
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("pool references unknown policy_profile {id}"))?
            .probe_result_policy
            .clone(),
        None => ProbeResultPolicy::default(),
    })
}

fn resolve_routing_profile(
    id: &str,
    profile: RoutingProfileConfig,
) -> anyhow::Result<ResolvedRoutingProfile> {
    anyhow::ensure!(
        profile.default_credential_cooldown_seconds > 0,
        "routing_profiles.{id}.default_credential_cooldown_seconds must be greater than zero"
    );
    let (same_request_credential_retry_enabled, max_same_request_retries) = match (
        profile.same_request_credential_retry.enabled,
        profile.same_request_credential_retry.max_retries,
    ) {
        (false, 0) => (false, 0),
        (false, _) => anyhow::bail!(
            "routing_profiles.{id}.same_request_credential_retry.max_retries must be 0 when disabled"
        ),
        (true, 0) => anyhow::bail!(
            "routing_profiles.{id}.same_request_credential_retry.max_retries must be greater than zero when enabled"
        ),
        (true, max_retries) => (true, max_retries),
    };

    Ok(ResolvedRoutingProfile {
        id: id.to_string(),
        key_selection: match profile.key_selection {
            KeySelectionStrategyConfig::StickyUntilFailure => {
                KeySelectionStrategy::StickyUntilFailure
            }
        },
        default_credential_cooldown: Duration::from_secs(
            profile.default_credential_cooldown_seconds,
        ),
        same_request_credential_retry_enabled,
        max_same_request_retries,
        route_target_retry_enabled: profile.route_target_retry.enabled,
    })
}

fn resolve_pool_routing_profile_id(
    channel_id: &str,
    pool: &PoolConfig,
    default_routing_profile: Option<&str>,
) -> anyhow::Result<String> {
    match &pool.routing_profile {
        Some(id) => {
            normalize_resource_id(&format!("pool {channel_id} routing_profile"), id.clone())
        }
        None => default_routing_profile.map(str::to_string).ok_or_else(|| {
            anyhow::anyhow!("pool {channel_id} requires routing_profile or default_routing_profile")
        }),
    }
}

fn normalized_adaptation_rules(
    rules: &[ErrorAdaptationRuleConfig],
) -> Vec<ErrorAdaptationRuleConfig> {
    let mut rules = rules.to_vec();
    normalize_adaptation_rule_ids(&mut rules);
    rules
}

fn normalize_adaptation_rule_ids(rules: &mut [ErrorAdaptationRuleConfig]) {
    for rule in rules {
        rule.id = rule.id.trim().to_string();
    }
}

fn validate_unique_adaptation_rule_ids(
    context: &str,
    rules: &[ErrorAdaptationRuleConfig],
) -> anyhow::Result<()> {
    let mut seen = HashSet::new();
    for rule in rules {
        let id = rule.id.trim();
        anyhow::ensure!(
            !id.is_empty(),
            "{context} adaptation rule id must not be empty"
        );
        anyhow::ensure!(
            seen.insert(id.to_string()),
            "duplicate error adaptation rule id {id} in {context}"
        );
    }
    Ok(())
}

fn validate_unique_merged_adaptation_rule_ids<'a>(
    channel_id: &str,
    rules: impl IntoIterator<Item = &'a ErrorAdaptationRuleConfig>,
) -> anyhow::Result<()> {
    let mut seen = HashSet::new();
    for rule in rules {
        let id = rule.id.trim();
        anyhow::ensure!(
            !id.is_empty(),
            "pool {channel_id} adaptation rule id must not be empty"
        );
        anyhow::ensure!(
            seen.insert(id.to_string()),
            "duplicate merged error adaptation rule id {id} for pool {channel_id}"
        );
    }
    Ok(())
}

fn validate_adaptation_action_combination(
    context: &str,
    kind: Option<FailureKind>,
    scope: Option<FailureScope>,
) -> anyhow::Result<()> {
    match (kind, scope) {
        (Some(kind), None) => {
            anyhow::bail!(
                "{context}.action kind {} requires explicit supported primary_scope",
                kind_as_config(kind)
            );
        }
        (None, Some(scope)) => {
            anyhow::bail!(
                "{context}.action primary_scope {} requires explicit supported kind",
                scope_as_config(scope)
            );
        }
        _ => {}
    }
    let Some(scope) = scope else {
        return Ok(());
    };
    match scope {
        FailureScope::Account
        | FailureScope::Deployment
        | FailureScope::ProviderAdapter
        | FailureScope::ClientToken => {
            anyhow::bail!(
                "{context}.action.primary_scope {} is not implemented",
                scope_as_config(scope)
            );
        }
        _ => {}
    }
    let Some(kind) = kind else {
        return Ok(());
    };
    let supported = matches!(
        (kind, scope),
        (FailureKind::AuthInvalid, FailureScope::Credential)
            | (FailureKind::RateLimited, FailureScope::Credential)
            | (FailureKind::QuotaExhausted, FailureScope::Credential)
            | (FailureKind::RelayBalanceUnavailable, FailureScope::Channel)
            | (FailureKind::ProviderUnavailable, FailureScope::Channel)
            | (FailureKind::KeySwitchCooldown, FailureScope::Credential)
            | (FailureKind::ClientError, FailureScope::RequestOnly)
            | (FailureKind::ClientError, FailureScope::ModelGroup)
            | (FailureKind::Unknown, FailureScope::RequestOnly)
    );
    anyhow::ensure!(
        supported,
        "{context}.action kind {} with primary_scope {} is not implemented",
        kind_as_config(kind),
        scope_as_config(scope)
    );
    Ok(())
}

fn validate_balance_scope(
    context: &str,
    balance_scope: BalanceScope,
) -> anyhow::Result<BalanceScope> {
    match balance_scope {
        BalanceScope::Credential | BalanceScope::Channel => Ok(balance_scope),
        BalanceScope::Account | BalanceScope::Provider | BalanceScope::ClientToken => {
            anyhow::bail!(
                "{context}.balance_scope {} is not implemented",
                balance_scope_as_config(balance_scope)
            )
        }
    }
}

fn kind_as_config(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::RateLimited => "rate_limited",
        FailureKind::KeySwitchCooldown => "key_switch_cooldown",
        FailureKind::AuthInvalid => "auth_invalid",
        FailureKind::QuotaExhausted => "quota_exhausted",
        FailureKind::RelayBalanceUnavailable => "relay_balance_unavailable",
        FailureKind::ProviderUnavailable => "provider_unavailable",
        FailureKind::ResponseFilterRejected => "response_filter_rejected",
        FailureKind::ClientError => "client_error",
        FailureKind::Unknown => "unknown",
    }
}

fn scope_as_config(scope: FailureScope) -> &'static str {
    match scope {
        FailureScope::RequestOnly => "request_only",
        FailureScope::Credential => "credential",
        FailureScope::Account => "account",
        FailureScope::Channel => "channel",
        FailureScope::Deployment => "deployment",
        FailureScope::ModelGroup => "model_group",
        FailureScope::ProviderAdapter => "provider_adapter",
        FailureScope::ClientToken => "client_token",
    }
}

fn balance_scope_as_config(scope: BalanceScope) -> &'static str {
    match scope {
        BalanceScope::Credential => "credential",
        BalanceScope::Channel => "channel",
        BalanceScope::Account => "account",
        BalanceScope::Provider => "provider",
        BalanceScope::ClientToken => "client_token",
    }
}

fn error_rules_has_override(error_rules: &ErrorRulesConfig) -> bool {
    error_rules.relay_profile.is_some()
        || error_rules.balance_scope.is_some()
        || error_rules.keep_codes.is_some()
        || error_rules.switch_codes.is_some()
        || error_rules.expire_codes.is_some()
        || error_rules.keep_statuses.is_some()
        || error_rules.switch_statuses.is_some()
        || error_rules.expire_statuses.is_some()
        || !error_rules.adaptation_rules.is_empty()
}

struct ResolvedUpstreamAccount {
    provider_id: String,
    account_id: String,
    provider_enabled: bool,
    account_configured_enabled: bool,
    account_enabled: bool,
    provider_kind: ProviderKind,
    endpoint: ResolvedUpstreamEndpoint,
}

struct ResolvedUpstreamEndpoint {
    api_base: String,
    auth_header: String,
    auth_prefix: String,
}

fn resolve_upstream_endpoint(
    field: &str,
    api_base: String,
    auth_header: String,
    auth_prefix: String,
) -> anyhow::Result<ResolvedUpstreamEndpoint> {
    Ok(ResolvedUpstreamEndpoint {
        api_base: normalize_api_base(field, api_base)?,
        auth_header: normalize_auth_header(field, auth_header)?,
        auth_prefix: normalize_auth_prefix(auth_prefix),
    })
}

fn resolve_pool_account(
    channel_id: &str,
    pool: &PoolConfig,
    providers: &HashMap<String, ProviderConfig>,
    accounts: &HashMap<String, AccountConfig>,
) -> anyhow::Result<ResolvedUpstreamAccount> {
    let Some(account_ref) = &pool.account else {
        return Ok(ResolvedUpstreamAccount {
            provider_id: format!("provider:{}", pool.provider_kind.stable_id_fragment()),
            account_id: format!("account:{channel_id}"),
            provider_enabled: true,
            account_configured_enabled: true,
            account_enabled: true,
            provider_kind: pool.provider_kind,
            endpoint: resolve_upstream_endpoint(
                &format!("pool {channel_id}"),
                pool.api_base.clone(),
                pool.auth_header.clone(),
                pool.auth_prefix.clone(),
            )?,
        });
    };

    let account_key = normalize_resource_id("pool.account", account_ref.clone())?;
    let account = accounts.get(&account_key).ok_or_else(|| {
        anyhow::anyhow!("pool {channel_id} references unknown account {account_key}")
    })?;
    let provider = providers.get(&account.provider).ok_or_else(|| {
        anyhow::anyhow!(
            "account {account_key} references unknown provider {}",
            account.provider
        )
    })?;
    Ok(ResolvedUpstreamAccount {
        provider_id: format!("provider:{}", account.provider),
        account_id: format!("account:{account_key}"),
        provider_enabled: provider.enabled,
        account_configured_enabled: account.enabled,
        account_enabled: account.enabled && provider.enabled,
        provider_kind: provider.provider_kind,
        endpoint: resolve_upstream_endpoint(
            &format!("account {account_key}"),
            account.api_base.clone(),
            account.auth_header.clone(),
            account.auth_prefix.clone(),
        )?,
    })
}

struct ChannelConfigGenerationInput<'a> {
    channel_id: &'a str,
    configured_enabled: bool,
    account: &'a ResolvedUpstreamAccount,
    endpoint_capabilities: &'a ResolvedEndpointCapabilities,
    credential_set_id: &'a CredentialSetId,
    routing_profile_id: &'a str,
    routing_policy: &'a RoutingPolicy,
    error_rules: &'a ErrorRulesConfig,
}

fn channel_config_generation(input: ChannelConfigGenerationInput<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    input.channel_id.hash(&mut hasher);
    input.configured_enabled.hash(&mut hasher);
    input.account.provider_id.hash(&mut hasher);
    input.account.account_id.hash(&mut hasher);
    input.account.account_enabled.hash(&mut hasher);
    input
        .account
        .provider_kind
        .stable_id_fragment()
        .hash(&mut hasher);
    input.account.endpoint.api_base.hash(&mut hasher);
    input.account.endpoint.auth_header.hash(&mut hasher);
    input.account.endpoint.auth_prefix.hash(&mut hasher);
    input.endpoint_capabilities.hash(&mut hasher);
    input.credential_set_id.0.hash(&mut hasher);
    input.routing_profile_id.hash(&mut hasher);
    input
        .routing_policy
        .retry_switched_key_in_same_request
        .hash(&mut hasher);
    input
        .routing_policy
        .max_same_request_retries
        .hash(&mut hasher);
    input
        .routing_policy
        .route_target_retry_enabled
        .hash(&mut hasher);
    input
        .routing_policy
        .default_credential_cooldown
        .as_secs()
        .hash(&mut hasher);
    format!("{:?}", input.error_rules).hash(&mut hasher);
    match hasher.finish() {
        0 => 1,
        generation => generation,
    }
}

impl TimeoutConfig {
    fn resolve(self) -> anyhow::Result<ResolvedTimeoutProfile> {
        Ok(ResolvedTimeoutProfile {
            connect: seconds_duration(
                self.connect_seconds.unwrap_or(10),
                "timeouts.connect_seconds",
            )?,
            non_streaming_total: seconds_duration(
                self.non_streaming_total_seconds.unwrap_or(120),
                "timeouts.non_streaming_total_seconds",
            )?,
            streaming_idle: seconds_duration(
                self.streaming_idle_seconds.unwrap_or(300),
                "timeouts.streaming_idle_seconds",
            )?,
        })
    }
}

impl RoutingConfig {
    fn resolve(self) -> anyhow::Result<ResolvedRoutingConfig> {
        Ok(ResolvedRoutingConfig {
            max_route_candidates: positive_usize(
                self.max_route_candidates.unwrap_or(16),
                "routing.max_route_candidates",
            )?,
            max_model_catalog_channels: positive_usize(
                self.max_model_catalog_channels.unwrap_or(16),
                "routing.max_model_catalog_channels",
            )?,
            telemetry_buffer_capacity: positive_usize(
                self.telemetry_buffer_capacity.unwrap_or(1024),
                "routing.telemetry_buffer_capacity",
            )?,
        })
    }
}

impl ResponseFilterConfig {
    fn resolve(self) -> anyhow::Result<ResolvedResponseFilterConfig> {
        let mut rule_ids = HashSet::new();
        let mut rules = Vec::new();
        for rule in self.rules.into_iter().filter(|rule| rule.enabled) {
            let id = rule.id.trim().to_string();
            anyhow::ensure!(!id.is_empty(), "response_filter rule id must not be empty");
            anyhow::ensure!(
                rule_ids.insert(id.clone()),
                "duplicate response_filter rule id {id}"
            );
            let kind = match rule.kind {
                ResponseFilterRuleKindConfig::Literal => ResponseFilterRuleKind::Literal {
                    value: required_response_filter_value(&id, "value", rule.value)?,
                    case_sensitive: rule.case_sensitive,
                },
                ResponseFilterRuleKindConfig::Regex => ResponseFilterRuleKind::Regex {
                    pattern: required_response_filter_value(&id, "pattern", rule.pattern)?,
                },
                ResponseFilterRuleKindConfig::RequiredLiteral => {
                    ResponseFilterRuleKind::RequiredLiteral {
                        value: required_response_filter_value(&id, "value", rule.value)?,
                        case_sensitive: rule.case_sensitive,
                    }
                }
                ResponseFilterRuleKindConfig::RequiredRegex => {
                    ResponseFilterRuleKind::RequiredRegex {
                        pattern: required_response_filter_value(&id, "pattern", rule.pattern)?,
                    }
                }
            };
            rules.push(ResponseFilterRuleSpec {
                id,
                kind,
                action: rule.action.into_action(),
            });
        }
        let policy = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: self.enabled,
            replacement: self.replacement.unwrap_or_default(),
            rules,
        })?;
        Ok(ResolvedResponseFilterConfig {
            policy,
            event_window_capacity: positive_usize(
                self.event_window_capacity
                    .unwrap_or_else(default_response_filter_event_window_capacity),
                "response_filter.event_window_capacity",
            )?,
            alert_window: seconds_duration(
                self.alert_window_seconds
                    .unwrap_or_else(default_response_filter_alert_window_seconds),
                "response_filter.alert_window_seconds",
            )?,
        })
    }
}

impl ResponseFilterActionConfig {
    fn into_action(self) -> ResponseFilterAction {
        match self {
            Self::Redact => ResponseFilterAction::Redact,
            Self::Reject => ResponseFilterAction::Reject,
            Self::RejectAndExpireCredential => ResponseFilterAction::RejectAndExpireCredential,
            Self::RejectAndCooldownChannel => ResponseFilterAction::RejectAndCooldownChannel,
        }
    }
}

fn required_response_filter_value(
    rule_id: &str,
    field: &str,
    value: Option<String>,
) -> anyhow::Result<String> {
    let value = value.unwrap_or_default().trim().to_string();
    anyhow::ensure!(
        !value.is_empty(),
        "response_filter rule {rule_id} requires non-empty {field}"
    );
    Ok(value)
}

fn seconds_duration(seconds: u64, field: &str) -> anyhow::Result<Duration> {
    anyhow::ensure!(seconds > 0, "{field} must be greater than zero");
    Ok(Duration::from_secs(seconds))
}

fn positive_usize(value: usize, field: &str) -> anyhow::Result<usize> {
    anyhow::ensure!(value > 0, "{field} must be greater than zero");
    Ok(value)
}

pub(crate) fn default_enabled() -> bool {
    true
}

fn default_auth_header() -> String {
    "authorization".to_string()
}

fn default_auth_prefix() -> String {
    "Bearer ".to_string()
}

fn default_listen() -> SocketAddr {
    "127.0.0.1:4000".parse().expect("valid default listen")
}

fn default_max_request_body_bytes() -> usize {
    2 * 1024 * 1024
}

fn default_max_model_catalog_body_bytes() -> usize {
    512 * 1024
}

fn default_max_error_body_bytes() -> usize {
    1024 * 1024
}

fn default_management_event_window_capacity() -> usize {
    1024
}

fn default_response_filter_event_window_capacity() -> usize {
    1024
}

fn default_response_filter_alert_window_seconds() -> u64 {
    900
}

pub(crate) fn default_route_target_priority() -> u16 {
    100
}

pub(crate) fn default_route_target_weight() -> u16 {
    1
}

fn parse_route_strategy(raw: Option<&str>) -> anyhow::Result<RouteStrategy> {
    match raw.unwrap_or("priority_weighted_sticky") {
        "priority" => Ok(RouteStrategy::Priority),
        "priority_weighted_sticky" => Ok(RouteStrategy::PriorityWeightedSticky),
        other => anyhow::bail!("unsupported route strategy {other}"),
    }
}

#[cfg(test)]
mod relay_hardening_phase_1a_tests {
    use super::*;
    use crate::{
        credential_repository::FileCredentialRepository,
        error::{ClassifiedFailure, FailureKind, FailureScope},
    };
    use std::{
        fs,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    fn temp_keys_file(contents: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("litellm-proxy-relay-phase-1a-{suffix}.keys"));
        fs::write(&path, contents).unwrap();
        path
    }

    fn config_yaml(error_rules: &str) -> String {
        let keys_file = temp_keys_file("synthetic-upstream-key\n");
        format!(
            r#"
listen: 127.0.0.1:0
client_tokens:
  - name: local-client
    token: synthetic-client-token
management:
  admin_token: synthetic-management-token
default_pool: relay
credential_sets:
  relay_credentials:
    keys_file: {}
default_routing_profile: default-routing
routing_profiles:
  default-routing:
    key_selection: sticky_until_failure
    default_credential_cooldown_seconds: 20
    same_request_credential_retry:
      enabled: false
      max_retries: 0
    route_target_retry:
      enabled: true
pools:
  relay:
    provider_kind: openai_compatible
    api_base: https://relay.example.test/v1
    credential_set: relay_credentials
    error_rules:
{}"#,
            keys_file.display(),
            indent(error_rules, 6)
        )
    }

    fn config_yaml_with_policy_profile(
        profile_error_rules: &str,
        pool_error_rules: &str,
    ) -> String {
        let keys_file = temp_keys_file("synthetic-upstream-key\n");
        format!(
            r#"
listen: 127.0.0.1:0
client_tokens:
  - name: local-client
    token: synthetic-client-token
management:
  admin_token: synthetic-management-token
default_pool: relay
credential_sets:
  relay_credentials:
    keys_file: {}
policy_profiles:
  generic-profile:
    error_rules:
{}
default_routing_profile: default-routing
routing_profiles:
  default-routing:
    key_selection: sticky_until_failure
    default_credential_cooldown_seconds: 20
    same_request_credential_retry:
      enabled: false
      max_retries: 0
    route_target_retry:
      enabled: true
pools:
  relay:
    policy_profile: generic-profile
    provider_kind: openai_compatible
    api_base: https://relay.example.test/v1
    credential_set: relay_credentials
    error_rules:
{}"#,
            keys_file.display(),
            indent(profile_error_rules, 6),
            indent(pool_error_rules, 6)
        )
    }

    fn indent(raw: &str, spaces: usize) -> String {
        let prefix = " ".repeat(spaces);
        raw.trim()
            .lines()
            .map(|line| format!("{prefix}{line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn resolve_config(raw: &str) -> anyhow::Result<ResolvedConfig> {
        let config: AppConfig = serde_yaml::from_str(raw)?;
        config.resolve_with_credential_repository(&FileCredentialRepository::new())
    }

    fn resolve_document_with_response_filter(
        response_filter: ResponseFilterConfig,
    ) -> anyhow::Result<ResolvedConfig> {
        let app: AppConfig = serde_yaml::from_str(&config_yaml("{}"))?;
        let mut document = app.into_registry_document();
        document.response_filter = response_filter;
        document.resolve_with_credential_repository(&FileCredentialRepository::new())
    }

    fn classify(error_rules: &str, status: u16, body: &[u8]) -> ClassifiedFailure {
        let resolved = resolve_config(&config_yaml(error_rules)).unwrap();
        resolved.pools["relay"]
            .error_classifier
            .classify_failure(status, &[], body)
    }

    #[test]
    fn response_filter_event_settings_default_to_phase4_values() {
        let resolved = resolve_document_with_response_filter(ResponseFilterConfig::default())
            .expect("default response filter settings resolve");

        assert_eq!(resolved.response_filter_event_window_capacity, 1024);
        assert_eq!(
            resolved.response_filter_alert_window,
            Duration::from_secs(900)
        );
    }

    #[test]
    fn response_filter_event_settings_resolve_from_registry_document() {
        let resolved = resolve_document_with_response_filter(ResponseFilterConfig {
            enabled: false,
            replacement: None,
            event_window_capacity: Some(7),
            alert_window_seconds: Some(11),
            rules: Vec::new(),
        })
        .expect("configured response filter settings resolve");

        assert_eq!(resolved.response_filter_event_window_capacity, 7);
        assert_eq!(
            resolved.response_filter_alert_window,
            Duration::from_secs(11)
        );
    }

    #[test]
    fn response_filter_event_settings_reject_zero_values() {
        for (field, config) in [
            (
                "response_filter.event_window_capacity",
                ResponseFilterConfig {
                    enabled: false,
                    replacement: None,
                    event_window_capacity: Some(0),
                    alert_window_seconds: None,
                    rules: Vec::new(),
                },
            ),
            (
                "response_filter.alert_window_seconds",
                ResponseFilterConfig {
                    enabled: false,
                    replacement: None,
                    event_window_capacity: None,
                    alert_window_seconds: Some(0),
                    rules: Vec::new(),
                },
            ),
        ] {
            let err = resolve_document_with_response_filter(config).unwrap_err();
            assert!(
                err.to_string().contains(field) && err.to_string().contains("greater than zero"),
                "unexpected error for {field}: {err}"
            );
        }
    }

    #[test]
    fn response_filter_config_rejects_unknown_observability_fields() {
        let err = serde_yaml::from_str::<ResponseFilterConfig>("event_capacity: 1\n").unwrap_err();

        assert!(
            err.to_string().contains("unknown field"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn endpoint_capabilities_parse_static_config() {
        let keys_file = temp_keys_file("synthetic-upstream-key\n");
        let raw = format!(
            r#"
listen: 127.0.0.1:0
client_tokens:
  - name: local-client
    token: synthetic-client-token
management:
  admin_token: synthetic-management-token
default_pool: relay
credential_sets:
  relay_credentials:
    keys_file: {}
default_routing_profile: default-routing
routing_profiles:
  default-routing:
    key_selection: sticky_until_failure
    default_credential_cooldown_seconds: 20
    same_request_credential_retry:
      enabled: false
      max_retries: 0
    route_target_retry:
      enabled: true
pools:
  relay:
    provider_kind: openai_compatible
    api_base: https://relay.example.test/v1
    credential_set: relay_credentials
    endpoint_capabilities:
      chat_completions: supported
      responses: unsupported
      embeddings: unknown
      models: local_projection
      diagnostic_labels:
        - relay
        - no_responses
"#,
            keys_file.display()
        );

        let resolved = resolve_config(&raw).expect("static endpoint capabilities should resolve");
        let capabilities = &resolved.pools["relay"].endpoint_capabilities;

        assert_eq!(
            capabilities.chat_completions,
            crate::endpoint_capabilities::EndpointSupport::Supported
        );
        assert_eq!(
            capabilities.responses,
            crate::endpoint_capabilities::EndpointSupport::Unsupported
        );
        assert_eq!(
            capabilities.embeddings,
            crate::endpoint_capabilities::EndpointSupport::Unknown
        );
        assert_eq!(
            capabilities.models,
            crate::endpoint_capabilities::ModelsEndpointCapability::LocalProjection
        );
        assert_eq!(
            capabilities.diagnostic_labels,
            ["no_responses", "openai_compatible", "relay"]
        );
    }

    #[test]
    fn relay_profile_values_parse_and_resolve() {
        for relay_profile in ["official_openai", "generic_relay", "untrusted_relay"] {
            resolve_config(&config_yaml(&format!("relay_profile: {relay_profile}"))).unwrap();
        }
    }

    #[test]
    fn default_relay_profile_preserves_official_openai_bare_auth_semantics() {
        let failure = classify("{}", 401, b"{}");

        assert_eq!(failure.kind, FailureKind::AuthInvalid);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn pool_error_rules_override_policy_profile_relay_profile() {
        let resolved = resolve_config(&config_yaml_with_policy_profile(
            "relay_profile: generic_relay",
            "relay_profile: official_openai",
        ))
        .unwrap();

        let failure = resolved.pools["relay"]
            .error_classifier
            .classify_failure(401, &[], b"{}");

        assert_eq!(failure.kind, FailureKind::AuthInvalid);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
    }

    #[test]
    fn balance_scope_defaults_to_credential_for_structured_quota() {
        let failure = classify(
            "relay_profile: generic_relay",
            400,
            br#"{"error":{"code":"insufficient_quota"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::QuotaExhausted);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(!failure.retryable);
    }

    #[test]
    fn balance_scope_channel_resolves_in_phase_1b() {
        let raw = config_yaml("balance_scope: channel");
        let config: AppConfig = serde_yaml::from_str(&raw).unwrap();

        let resolved = config
            .resolve_with_credential_repository(&FileCredentialRepository::new())
            .unwrap();

        let failure = resolved.pools["relay"].error_classifier.classify_failure(
            400,
            &[],
            br#"{"error":{"code":"insufficient_quota"}}"#,
        );
        assert_eq!(failure.kind, FailureKind::RelayBalanceUnavailable);
        assert_eq!(failure.primary_scope, FailureScope::Channel);
    }

    #[test]
    fn unsupported_balance_scopes_reject_config_resolution() {
        for scope in ["account", "provider", "client_token"] {
            let err = resolve_config(&config_yaml(&format!("balance_scope: {scope}"))).unwrap_err();
            assert!(
                err.to_string().contains("balance_scope")
                    && err.to_string().contains("not implemented"),
                "unexpected error for {scope}: {err}"
            );
        }
    }

    #[test]
    fn unsupported_free_form_message_matcher_fields_are_rejected() {
        let err = resolve_config(&config_yaml(
            r#"
adaptation_rules:
  - id: unsupported-message-matcher
    matcher:
      message_contains:
        - synthetic marker
"#,
        ))
        .unwrap_err();

        assert!(
            err.to_string().contains("unknown field"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn generic_relay_bare_401_is_request_only() {
        let failure = classify("relay_profile: generic_relay", 401, b"{}");

        assert_eq!(failure.kind, FailureKind::ClientError);
        assert_eq!(failure.primary_scope, FailureScope::RequestOnly);
        assert!(!failure.retryable);
    }

    #[test]
    fn untrusted_relay_bare_403_is_request_only() {
        let failure = classify("relay_profile: untrusted_relay", 403, b"{}");

        assert_eq!(failure.kind, FailureKind::ClientError);
        assert_eq!(failure.primary_scope, FailureScope::RequestOnly);
        assert!(!failure.retryable);
    }

    #[test]
    fn structured_invalid_key_expires_credential_across_relay_profiles() {
        for relay_profile in ["official_openai", "generic_relay", "untrusted_relay"] {
            let failure = classify(
                &format!("relay_profile: {relay_profile}"),
                400,
                br#"{"error":{"code":"invalid_api_key"}}"#,
            );

            assert_eq!(failure.kind, FailureKind::AuthInvalid);
            assert_eq!(failure.primary_scope, FailureScope::Credential);
            assert!(!failure.retryable);
        }
    }

    #[test]
    fn bare_429_is_credential_rate_limited_across_relay_profiles() {
        for relay_profile in ["official_openai", "generic_relay", "untrusted_relay"] {
            let failure = classify(&format!("relay_profile: {relay_profile}"), 429, b"{}");

            assert_eq!(failure.kind, FailureKind::RateLimited);
            assert_eq!(failure.primary_scope, FailureScope::Credential);
            assert!(failure.retryable);
        }
    }

    #[test]
    fn code_less_top_level_error_object_is_request_only() {
        let failure = classify(
            "relay_profile: official_openai",
            401,
            br#"{"error":{"message":"synthetic client error"}}"#,
        );

        assert_eq!(failure.kind, FailureKind::ClientError);
        assert_eq!(failure.primary_scope, FailureScope::RequestOnly);
        assert!(!failure.retryable);
    }
}
