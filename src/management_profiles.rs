use serde::Serialize;
use std::collections::HashMap;

use crate::{
    config::{
        ErrorAdaptationActionConfig, ErrorAdaptationMatcherConfig, ErrorAdaptationRuleConfig,
        ErrorPolicyRuleSource, ErrorRulesConfig, KeySelectionStrategy, ProbeResultActionKind,
        ProbeResultPolicy, ResolvedErrorPolicySources, ResolvedPolicyProfile,
        ResolvedRoutingProfile,
    },
    error::{ErrorClassifierSnapshot, FailureKind, FailureScope},
    management_errors::ManagementServiceError,
    management_resource_lookup::{
        channel_error_rules_lookup, policy_profile, policy_profile_channel_ids, policy_profiles,
        routing_profile, routing_profile_channel_ids, routing_profiles,
    },
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct ErrorRulesResponse {
    pub channel_id: String,
    pub policy_profile_id: Option<String>,
    pub has_pool_override: bool,
    pub classifier_id: String,
    pub classifier_version: String,
    pub keep_codes: Vec<String>,
    pub switch_codes: Vec<String>,
    pub expire_codes: Vec<String>,
    pub keep_statuses: Vec<String>,
    pub switch_statuses: Vec<String>,
    pub expire_statuses: Vec<String>,
    pub adaptation_rules: Vec<ErrorAdaptationRuleStatus>,
}

#[derive(Debug, Serialize)]
pub struct ErrorAdaptationRuleStatus {
    pub id: String,
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    pub enabled: bool,
    pub matcher: ErrorAdaptationMatcherStatus,
    pub action: ErrorAdaptationActionStatus,
}

#[derive(Debug, Serialize)]
pub struct ErrorAdaptationMatcherStatus {
    pub codes: Vec<String>,
    pub limit_types: Vec<String>,
    pub statuses: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorAdaptationActionStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<FailureKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_scope: Option<FailureScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct PolicyProfilesResponse {
    pub policy_profiles: Vec<PolicyProfileStatus>,
}

#[derive(Debug, Serialize)]
pub struct PolicyProfileStatus {
    pub id: String,
    pub channel_count: usize,
    pub channel_ids: Vec<String>,
    pub rule_counts: ErrorRuleCounts,
    pub error_rules: ErrorRulesConfigStatus,
    pub probe_result_policy: ProbeResultPolicyStatus,
}

#[derive(Debug, Serialize)]
pub struct ProbeResultPolicyStatus {
    pub success: &'static str,
    pub invalid: &'static str,
    pub quota_exhausted: &'static str,
    pub rate_limited: &'static str,
    pub provider_unavailable: &'static str,
    pub unsupported_model: &'static str,
    pub unknown: &'static str,
    pub cooldown_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct RoutingProfilesResponse {
    pub routing_profiles: Vec<RoutingProfileStatus>,
}

#[derive(Debug, Serialize)]
pub struct RoutingProfileStatus {
    pub id: String,
    pub key_selection: &'static str,
    pub default_credential_cooldown_seconds: u64,
    pub same_request_credential_retry_enabled: bool,
    pub max_same_request_retries: usize,
    pub route_target_retry_enabled: bool,
    pub channel_count: usize,
    pub channel_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorRuleCounts {
    pub keep_codes: usize,
    pub switch_codes: usize,
    pub expire_codes: usize,
    pub keep_statuses: usize,
    pub switch_statuses: usize,
    pub expire_statuses: usize,
    pub adaptation_rules: usize,
}

#[derive(Debug, Serialize)]
pub struct ErrorRulesConfigStatus {
    pub keep_codes: Vec<String>,
    pub switch_codes: Vec<String>,
    pub expire_codes: Vec<String>,
    pub keep_statuses: Vec<String>,
    pub switch_statuses: Vec<String>,
    pub expire_statuses: Vec<String>,
    pub adaptation_rules: Vec<ErrorAdaptationRuleConfigStatus>,
}

#[derive(Debug, Serialize)]
pub struct ErrorAdaptationRuleConfigStatus {
    pub id: String,
    pub enabled: bool,
    pub matcher: ErrorAdaptationMatcherConfigStatus,
    pub action: ErrorAdaptationActionConfigStatus,
}

#[derive(Debug, Serialize)]
pub struct ErrorAdaptationMatcherConfigStatus {
    pub codes: Vec<String>,
    pub limit_types: Vec<String>,
    pub statuses: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorAdaptationActionConfigStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<FailureKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_scope: Option<FailureScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_seconds: Option<u64>,
}

pub fn error_rules_response(
    channel_id: &str,
    snapshot: ErrorClassifierSnapshot,
    error_policy_sources: Option<ResolvedErrorPolicySources>,
) -> ErrorRulesResponse {
    let source_by_id: HashMap<&str, &ErrorPolicyRuleSource> = error_policy_sources
        .iter()
        .flat_map(|sources| sources.adaptation_rule_sources.iter())
        .map(|source| (source.id.as_str(), &source.source))
        .collect();
    let adaptation_rules = snapshot
        .adaptation_rules
        .iter()
        .map(|rule| {
            let (source, profile_id) = match source_by_id.get(rule.id.as_str()) {
                Some(ErrorPolicyRuleSource::Profile { profile_id }) => {
                    ("profile", Some(profile_id.clone()))
                }
                Some(ErrorPolicyRuleSource::PoolOverride) => ("pool_override", None),
                None => ("pool_override", None),
            };
            ErrorAdaptationRuleStatus {
                id: rule.id.clone(),
                source,
                profile_id,
                enabled: rule.enabled,
                matcher: ErrorAdaptationMatcherStatus {
                    codes: rule.matcher.codes.clone(),
                    limit_types: rule.matcher.limit_types.clone(),
                    statuses: rule
                        .matcher
                        .statuses
                        .iter()
                        .map(|status| status.as_config_string())
                        .collect(),
                },
                action: ErrorAdaptationActionStatus {
                    kind: rule.action.kind,
                    primary_scope: rule.action.primary_scope,
                    retryable: rule.action.retryable,
                    cooldown_seconds: rule.action.cooldown.map(|cooldown| cooldown.as_secs()),
                },
            }
        })
        .collect();
    ErrorRulesResponse {
        channel_id: channel_id.to_string(),
        policy_profile_id: error_policy_sources
            .as_ref()
            .and_then(|sources| sources.profile_id.clone()),
        has_pool_override: error_policy_sources.is_some_and(|sources| sources.has_pool_override),
        classifier_id: snapshot.classifier_id,
        classifier_version: snapshot.classifier_version,
        keep_codes: snapshot.keep_codes,
        switch_codes: snapshot.switch_codes,
        expire_codes: snapshot.expire_codes,
        keep_statuses: snapshot.keep_statuses,
        switch_statuses: snapshot.switch_statuses,
        expire_statuses: snapshot.expire_statuses,
        adaptation_rules,
    }
}

pub fn channel_error_rules_response(
    state: &AppState,
    channel_id: &str,
) -> Result<ErrorRulesResponse, ManagementServiceError> {
    let lookup = channel_error_rules_lookup(state, channel_id)?;
    Ok(error_rules_response(
        channel_id,
        lookup.snapshot,
        lookup.error_policy_sources,
    ))
}

pub fn policy_profile_status(
    profile: &ResolvedPolicyProfile,
    channel_ids: Vec<String>,
) -> PolicyProfileStatus {
    PolicyProfileStatus {
        id: profile.id.clone(),
        channel_count: channel_ids.len(),
        channel_ids,
        rule_counts: error_rule_counts(&profile.error_rules),
        error_rules: error_rules_config_status(&profile.error_rules),
        probe_result_policy: probe_result_policy_status(&profile.probe_result_policy),
    }
}

pub fn policy_profiles_response(state: &AppState) -> PolicyProfilesResponse {
    let mut profiles: Vec<PolicyProfileStatus> = policy_profiles(state)
        .iter()
        .map(|profile| {
            policy_profile_status(profile, policy_profile_channel_ids(state, &profile.id))
        })
        .collect();
    profiles.sort_by(|a, b| a.id.cmp(&b.id));
    PolicyProfilesResponse {
        policy_profiles: profiles,
    }
}

pub fn policy_profile_response(
    state: &AppState,
    profile_id: &str,
) -> Result<PolicyProfileStatus, ManagementServiceError> {
    let Some(profile) = policy_profile(state, profile_id) else {
        return Err(ManagementServiceError::NotFound(format!(
            "unknown policy_profile {profile_id}"
        )));
    };
    Ok(policy_profile_status(
        &profile,
        policy_profile_channel_ids(state, profile_id),
    ))
}

pub fn routing_profile_status(
    profile: &ResolvedRoutingProfile,
    channel_ids: Vec<String>,
) -> RoutingProfileStatus {
    RoutingProfileStatus {
        id: profile.id.clone(),
        key_selection: key_selection_strategy_status(profile.key_selection),
        default_credential_cooldown_seconds: profile.default_credential_cooldown.as_secs(),
        same_request_credential_retry_enabled: profile.same_request_credential_retry_enabled,
        max_same_request_retries: profile.max_same_request_retries,
        route_target_retry_enabled: profile.route_target_retry_enabled,
        channel_count: channel_ids.len(),
        channel_ids,
    }
}

pub fn routing_profiles_response(state: &AppState) -> RoutingProfilesResponse {
    let mut profiles: Vec<RoutingProfileStatus> = routing_profiles(state)
        .iter()
        .map(|profile| {
            routing_profile_status(profile, routing_profile_channel_ids(state, &profile.id))
        })
        .collect();
    profiles.sort_by(|a, b| a.id.cmp(&b.id));
    RoutingProfilesResponse {
        routing_profiles: profiles,
    }
}

pub fn routing_profile_response(
    state: &AppState,
    profile_id: &str,
) -> Result<RoutingProfileStatus, ManagementServiceError> {
    let Some(profile) = routing_profile(state, profile_id) else {
        return Err(ManagementServiceError::NotFound(format!(
            "unknown routing_profile {profile_id}"
        )));
    };
    Ok(routing_profile_status(
        &profile,
        routing_profile_channel_ids(state, profile_id),
    ))
}

pub fn probe_result_policy_status(policy: &ProbeResultPolicy) -> ProbeResultPolicyStatus {
    ProbeResultPolicyStatus {
        success: probe_result_action_status(policy.success),
        invalid: probe_result_action_status(policy.invalid),
        quota_exhausted: probe_result_action_status(policy.quota_exhausted),
        rate_limited: probe_result_action_status(policy.rate_limited),
        provider_unavailable: probe_result_action_status(policy.provider_unavailable),
        unsupported_model: probe_result_action_status(policy.unsupported_model),
        unknown: probe_result_action_status(policy.unknown),
        cooldown_seconds: policy.cooldown.as_secs(),
    }
}

pub fn error_rules_config_status(error_rules: &ErrorRulesConfig) -> ErrorRulesConfigStatus {
    ErrorRulesConfigStatus {
        keep_codes: error_rules.keep_codes.clone().unwrap_or_default(),
        switch_codes: error_rules.switch_codes.clone().unwrap_or_default(),
        expire_codes: error_rules.expire_codes.clone().unwrap_or_default(),
        keep_statuses: error_rules.keep_statuses.clone().unwrap_or_default(),
        switch_statuses: error_rules.switch_statuses.clone().unwrap_or_default(),
        expire_statuses: error_rules.expire_statuses.clone().unwrap_or_default(),
        adaptation_rules: error_rules
            .adaptation_rules
            .iter()
            .map(error_adaptation_rule_config_status)
            .collect(),
    }
}

fn error_rule_counts(error_rules: &ErrorRulesConfig) -> ErrorRuleCounts {
    ErrorRuleCounts {
        keep_codes: error_rules.keep_codes.as_ref().map_or(0, Vec::len),
        switch_codes: error_rules.switch_codes.as_ref().map_or(0, Vec::len),
        expire_codes: error_rules.expire_codes.as_ref().map_or(0, Vec::len),
        keep_statuses: error_rules.keep_statuses.as_ref().map_or(0, Vec::len),
        switch_statuses: error_rules.switch_statuses.as_ref().map_or(0, Vec::len),
        expire_statuses: error_rules.expire_statuses.as_ref().map_or(0, Vec::len),
        adaptation_rules: error_rules.adaptation_rules.len(),
    }
}

fn probe_result_action_status(action: ProbeResultActionKind) -> &'static str {
    match action {
        ProbeResultActionKind::Noop => "noop",
        ProbeResultActionKind::Expire => "expire",
        ProbeResultActionKind::QuotaExhaust => "quota_exhaust",
        ProbeResultActionKind::Restore => "restore",
        ProbeResultActionKind::Cooldown => "cooldown",
    }
}

fn key_selection_strategy_status(strategy: KeySelectionStrategy) -> &'static str {
    match strategy {
        KeySelectionStrategy::StickyUntilFailure => "sticky_until_failure",
    }
}

fn error_adaptation_rule_config_status(
    rule: &ErrorAdaptationRuleConfig,
) -> ErrorAdaptationRuleConfigStatus {
    ErrorAdaptationRuleConfigStatus {
        id: rule.id.clone(),
        enabled: rule.enabled,
        matcher: error_adaptation_matcher_config_status(&rule.matcher),
        action: error_adaptation_action_config_status(&rule.action),
    }
}

fn error_adaptation_matcher_config_status(
    matcher: &ErrorAdaptationMatcherConfig,
) -> ErrorAdaptationMatcherConfigStatus {
    ErrorAdaptationMatcherConfigStatus {
        codes: matcher.codes.clone(),
        limit_types: matcher.limit_types.clone(),
        statuses: matcher.statuses.clone(),
    }
}

fn error_adaptation_action_config_status(
    action: &ErrorAdaptationActionConfig,
) -> ErrorAdaptationActionConfigStatus {
    ErrorAdaptationActionConfigStatus {
        kind: action.kind,
        primary_scope: action.primary_scope,
        retryable: action.retryable,
        cooldown_seconds: action.cooldown_seconds,
    }
}
