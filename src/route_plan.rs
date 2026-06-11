use std::collections::HashMap;

use crate::{provider::ProviderKind, state::ChannelId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRoute {
    pub public_model: String,
    pub targets: Vec<RouteTarget>,
    pub strategy: RouteStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteStrategy {
    Priority,
    PriorityWeightedSticky,
}

impl RouteStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteStrategy::Priority => "priority",
            RouteStrategy::PriorityWeightedSticky => "priority_weighted_sticky",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTarget {
    pub channel_id: ChannelId,
    pub provider_kind: ProviderKind,
    pub upstream_model: Option<String>,
    pub priority: u16,
    pub weight: u16,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePlan {
    pub request_id: String,
    pub registry_generation: u64,
    pub public_model: Option<String>,
    pub targets: Vec<RouteCandidate>,
    pub selected_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCandidate {
    pub target_index: usize,
    pub channel_id: ChannelId,
    pub provider_kind: ProviderKind,
    pub upstream_model: Option<String>,
    pub priority: u16,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePreview {
    pub request_id: String,
    pub registry_generation: u64,
    pub public_model: Option<String>,
    pub route_found: bool,
    pub candidate_limit: usize,
    pub selected_target_index: Option<usize>,
    pub candidates: Vec<RoutePreviewCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePreviewCandidate {
    pub target_index: usize,
    pub channel_id: ChannelId,
    pub provider_kind: ProviderKind,
    pub upstream_model: Option<String>,
    pub priority: u16,
    pub weight: u16,
    pub target_enabled: bool,
    pub included: bool,
    pub selected: bool,
    pub plan_position: Option<usize>,
    pub reasons: Vec<RoutePreviewReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutePreviewReason {
    TargetDisabled,
    ClientChannelScope,
    ChannelDisabled,
    ChannelCoolingDown,
    ProviderCoolingDown,
    CredentialCoolingDown,
    NoAvailableCredentials,
    RuntimeUnavailable,
    ChannelDegraded,
    DegradedLastResort,
    ProviderCoolingDownLastResort,
    CredentialCoolingDownLastResort,
    UnknownChannel,
    CandidateLimit,
}

impl RoutePreviewReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RoutePreviewReason::TargetDisabled => "target_disabled",
            RoutePreviewReason::ClientChannelScope => "client_channel_scope",
            RoutePreviewReason::ChannelDisabled => "channel_disabled",
            RoutePreviewReason::ChannelCoolingDown => "channel_cooling_down",
            RoutePreviewReason::ProviderCoolingDown => "provider_cooling_down",
            RoutePreviewReason::CredentialCoolingDown => "credential_cooling_down",
            RoutePreviewReason::NoAvailableCredentials => "no_available_credentials",
            RoutePreviewReason::RuntimeUnavailable => "runtime_unavailable",
            RoutePreviewReason::ChannelDegraded => "channel_degraded",
            RoutePreviewReason::DegradedLastResort => "degraded_last_resort",
            RoutePreviewReason::ProviderCoolingDownLastResort => {
                "provider_cooling_down_last_resort"
            }
            RoutePreviewReason::CredentialCoolingDownLastResort => {
                "credential_cooling_down_last_resort"
            }
            RoutePreviewReason::UnknownChannel => "unknown_channel",
            RoutePreviewReason::CandidateLimit => "candidate_limit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAdmissionStatus {
    Available,
    LastResort,
    Unavailable,
}

impl RouteAdmissionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteAdmissionStatus::Available => "available",
            RouteAdmissionStatus::LastResort => "last_resort",
            RouteAdmissionStatus::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteAdmissionSelectedTarget {
    pub channel_id: ChannelId,
    pub plan_position: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteAdmissionSummary {
    pub status: RouteAdmissionStatus,
    pub reason_code: &'static str,
    pub selected_target: Option<RouteAdmissionSelectedTarget>,
    pub candidate_count: usize,
    pub included_count: usize,
    pub blocked_count: usize,
    pub soft_suppressed_count: usize,
    pub hard_blocked_count: usize,
    pub last_resort_used: bool,
    pub last_resort_reason: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRouteState {
    Available,
    /// Provider/account failure-domain soft cooldowns are normalized to this
    /// state before route planning.
    ProviderCoolingDown,
    CredentialCoolingDown,
    CoolingDown,
    Degraded,
    Disabled,
    NoAvailableCredentials,
    RuntimeUnavailable,
    UnknownChannel,
}

impl ChannelRouteState {
    pub fn most_restrictive(self, other: Self) -> Self {
        if route_state_severity(other) > route_state_severity(self) {
            other
        } else {
            self
        }
    }
}

fn route_state_severity(state: ChannelRouteState) -> u8 {
    match state {
        ChannelRouteState::Available => 0,
        ChannelRouteState::Degraded => 1,
        ChannelRouteState::ProviderCoolingDown => 2,
        ChannelRouteState::CredentialCoolingDown => 3,
        ChannelRouteState::RuntimeUnavailable => 4,
        ChannelRouteState::CoolingDown => 5,
        ChannelRouteState::NoAvailableCredentials => 6,
        ChannelRouteState::Disabled => 7,
        ChannelRouteState::UnknownChannel => 8,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutePlanError {
    NoRoute,
    NoEnabledTargets,
}

pub struct RoutePlanInput<'a> {
    pub request_id: String,
    pub registry_generation: u64,
    pub public_model: Option<String>,
    pub route: Option<&'a ModelRoute>,
    pub channel_states: &'a HashMap<ChannelId, ChannelRouteState>,
    pub allowed_channels: &'a [String],
    pub candidate_limit: usize,
}

pub struct RoutePreviewInput<'a> {
    pub request_id: String,
    pub registry_generation: u64,
    pub public_model: Option<String>,
    pub route: Option<&'a ModelRoute>,
    pub channel_states: &'a HashMap<ChannelId, ChannelRouteState>,
    pub allowed_channels: &'a [String],
    pub candidate_limit: usize,
}

pub fn plan_route(input: RoutePlanInput<'_>) -> Result<RoutePlan, RoutePlanError> {
    if input.route.is_none() {
        return Err(RoutePlanError::NoRoute);
    }
    let preview = preview_route(RoutePreviewInput {
        request_id: input.request_id,
        registry_generation: input.registry_generation,
        public_model: input.public_model,
        route: input.route,
        channel_states: input.channel_states,
        allowed_channels: input.allowed_channels,
        candidate_limit: input.candidate_limit,
    });
    let mut included: Vec<&RoutePreviewCandidate> = preview
        .candidates
        .iter()
        .filter(|candidate| candidate.included)
        .collect();
    included.sort_by_key(|candidate| candidate.plan_position.unwrap_or(usize::MAX));
    let targets: Vec<RouteCandidate> = included
        .into_iter()
        .map(|candidate| RouteCandidate {
            target_index: candidate.target_index,
            channel_id: candidate.channel_id.clone(),
            provider_kind: candidate.provider_kind,
            upstream_model: candidate.upstream_model.clone(),
            priority: candidate.priority,
            weight: candidate.weight,
        })
        .collect();
    if targets.is_empty() {
        return Err(RoutePlanError::NoEnabledTargets);
    }

    Ok(RoutePlan {
        request_id: preview.request_id,
        registry_generation: preview.registry_generation,
        public_model: preview.public_model,
        targets,
        selected_index: 0,
    })
}

pub fn route_admission_summary(preview: &RoutePreview) -> RouteAdmissionSummary {
    let candidate_count = preview.candidates.len();
    let included_count = preview
        .candidates
        .iter()
        .filter(|candidate| candidate.included)
        .count();
    let blocked_count = candidate_count.saturating_sub(included_count);
    let soft_suppressed_count = preview
        .candidates
        .iter()
        .filter(|candidate| {
            candidate
                .reasons
                .iter()
                .any(|reason| route_preview_reason_is_soft_suppression(*reason))
        })
        .count();
    let hard_blocked_count = preview
        .candidates
        .iter()
        .filter(|candidate| {
            candidate
                .reasons
                .iter()
                .any(|reason| route_preview_reason_is_hard_blocker(*reason))
        })
        .count();
    let selected_candidate = preview
        .candidates
        .iter()
        .find(|candidate| candidate.selected);
    let selected_target = selected_candidate.and_then(|candidate| {
        candidate
            .plan_position
            .map(|plan_position| RouteAdmissionSelectedTarget {
                channel_id: candidate.channel_id.clone(),
                plan_position,
            })
    });
    let last_resort_reason = selected_candidate.and_then(|candidate| {
        candidate
            .reasons
            .iter()
            .copied()
            .find(|reason| route_preview_reason_is_last_resort(*reason))
            .map(RoutePreviewReason::as_str)
    });
    let status = if last_resort_reason.is_some() {
        RouteAdmissionStatus::LastResort
    } else if selected_target.is_some() {
        RouteAdmissionStatus::Available
    } else {
        RouteAdmissionStatus::Unavailable
    };
    let reason_code = last_resort_reason.unwrap_or_else(|| {
        if selected_target.is_some() {
            "available"
        } else {
            route_admission_unavailable_reason(preview)
        }
    });

    RouteAdmissionSummary {
        status,
        reason_code,
        selected_target,
        candidate_count,
        included_count,
        blocked_count,
        soft_suppressed_count,
        hard_blocked_count,
        last_resort_used: last_resort_reason.is_some(),
        last_resort_reason,
    }
}

fn route_admission_unavailable_reason(preview: &RoutePreview) -> &'static str {
    preview
        .candidates
        .iter()
        .flat_map(|candidate| candidate.reasons.iter().copied())
        .find(|reason| route_preview_reason_is_hard_blocker(*reason))
        .or_else(|| {
            preview
                .candidates
                .iter()
                .flat_map(|candidate| candidate.reasons.iter().copied())
                .find(|reason| route_preview_reason_is_soft_suppression(*reason))
        })
        .map(RoutePreviewReason::as_str)
        .unwrap_or("no_route_candidate")
}

pub fn route_preview_reason_is_hard_blocker(reason: RoutePreviewReason) -> bool {
    matches!(
        reason,
        RoutePreviewReason::TargetDisabled
            | RoutePreviewReason::ClientChannelScope
            | RoutePreviewReason::ChannelDisabled
            | RoutePreviewReason::ChannelCoolingDown
            | RoutePreviewReason::NoAvailableCredentials
            | RoutePreviewReason::RuntimeUnavailable
            | RoutePreviewReason::UnknownChannel
            | RoutePreviewReason::CandidateLimit
    )
}

pub fn route_preview_reason_is_soft_suppression(reason: RoutePreviewReason) -> bool {
    matches!(
        reason,
        RoutePreviewReason::ChannelDegraded
            | RoutePreviewReason::ProviderCoolingDown
            | RoutePreviewReason::CredentialCoolingDown
    )
}

pub fn route_preview_reason_is_last_resort(reason: RoutePreviewReason) -> bool {
    matches!(
        reason,
        RoutePreviewReason::DegradedLastResort
            | RoutePreviewReason::ProviderCoolingDownLastResort
            | RoutePreviewReason::CredentialCoolingDownLastResort
    )
}

pub fn preview_route(input: RoutePreviewInput<'_>) -> RoutePreview {
    let Some(route) = input.route else {
        return RoutePreview {
            request_id: input.request_id,
            registry_generation: input.registry_generation,
            public_model: input.public_model,
            route_found: false,
            candidate_limit: input.candidate_limit,
            selected_target_index: None,
            candidates: Vec::new(),
        };
    };

    let mut candidates: Vec<RoutePreviewCandidate> = route
        .targets
        .iter()
        .enumerate()
        .map(|(index, target)| {
            let mut reasons = Vec::new();
            if !target.enabled {
                reasons.push(RoutePreviewReason::TargetDisabled);
            }
            if !channel_is_allowed(&target.channel_id, input.allowed_channels) {
                reasons.push(RoutePreviewReason::ClientChannelScope);
            }
            if matches!(
                input.channel_states.get(&target.channel_id),
                Some(ChannelRouteState::Disabled)
            ) {
                reasons.push(RoutePreviewReason::ChannelDisabled);
            }
            if matches!(
                input.channel_states.get(&target.channel_id),
                Some(ChannelRouteState::CoolingDown)
            ) {
                reasons.push(RoutePreviewReason::ChannelCoolingDown);
            }
            if matches!(
                input.channel_states.get(&target.channel_id),
                Some(ChannelRouteState::NoAvailableCredentials)
            ) {
                reasons.push(RoutePreviewReason::NoAvailableCredentials);
            }
            if matches!(
                input.channel_states.get(&target.channel_id),
                Some(ChannelRouteState::RuntimeUnavailable)
            ) {
                reasons.push(RoutePreviewReason::RuntimeUnavailable);
            }
            if matches!(
                input.channel_states.get(&target.channel_id),
                Some(ChannelRouteState::UnknownChannel)
            ) {
                reasons.push(RoutePreviewReason::UnknownChannel);
            }
            RoutePreviewCandidate {
                target_index: index,
                channel_id: target.channel_id.clone(),
                provider_kind: target.provider_kind,
                upstream_model: target.upstream_model.clone(),
                priority: target.priority,
                weight: target.weight,
                target_enabled: target.enabled,
                included: false,
                selected: false,
                plan_position: None,
                reasons,
            }
        })
        .collect();

    let mut eligible: Vec<(usize, RouteCandidate)> = candidates
        .iter()
        .filter(|candidate| candidate.reasons.is_empty())
        .map(|candidate| {
            (
                candidate.target_index,
                RouteCandidate {
                    target_index: candidate.target_index,
                    channel_id: candidate.channel_id.clone(),
                    provider_kind: candidate.provider_kind,
                    upstream_model: candidate.upstream_model.clone(),
                    priority: candidate.priority,
                    weight: candidate.weight,
                },
            )
        })
        .collect();
    eligible.sort_by_key(|(index, target)| (target.priority, *index));

    if let Some(preferred_tier) = eligible
        .iter()
        .map(|(_, target)| route_state_admission_tier(input.channel_states.get(&target.channel_id)))
        .min()
    {
        let mut retained = Vec::with_capacity(eligible.len());
        for (index, target) in eligible {
            let route_state = input.channel_states.get(&target.channel_id);
            let tier = route_state_admission_tier(route_state);
            if tier > preferred_tier {
                match route_state {
                    Some(ChannelRouteState::Degraded) => {
                        candidates[index]
                            .reasons
                            .push(RoutePreviewReason::ChannelDegraded);
                    }
                    Some(ChannelRouteState::ProviderCoolingDown) => {
                        candidates[index]
                            .reasons
                            .push(RoutePreviewReason::ProviderCoolingDown);
                    }
                    Some(ChannelRouteState::CredentialCoolingDown) => {
                        candidates[index]
                            .reasons
                            .push(RoutePreviewReason::CredentialCoolingDown);
                    }
                    _ => {}
                }
                continue;
            }
            match input.channel_states.get(&target.channel_id) {
                Some(ChannelRouteState::Degraded) => {
                    candidates[index]
                        .reasons
                        .push(RoutePreviewReason::DegradedLastResort);
                }
                Some(ChannelRouteState::ProviderCoolingDown) => {
                    candidates[index]
                        .reasons
                        .push(RoutePreviewReason::ProviderCoolingDownLastResort);
                }
                Some(ChannelRouteState::CredentialCoolingDown) => {
                    candidates[index]
                        .reasons
                        .push(RoutePreviewReason::CredentialCoolingDownLastResort);
                }
                _ => {}
            }
            retained.push((index, target));
        }
        eligible = retained;
    }

    if route.strategy == RouteStrategy::PriorityWeightedSticky {
        order_weighted_priority_tier_with_indices(&mut eligible, &input.request_id);
    }

    let limited_len = if input.candidate_limit > 0 {
        input.candidate_limit.min(eligible.len())
    } else {
        eligible.len()
    };
    for (position, (index, _)) in eligible.iter().take(limited_len).enumerate() {
        candidates[*index].included = true;
        candidates[*index].plan_position = Some(position);
        if position == 0 {
            candidates[*index].selected = true;
        }
    }
    for (index, _) in eligible.iter().skip(limited_len) {
        candidates[*index]
            .reasons
            .push(RoutePreviewReason::CandidateLimit);
    }
    let selected_target_index = candidates
        .iter()
        .find(|candidate| candidate.selected)
        .map(|candidate| candidate.target_index);

    RoutePreview {
        request_id: input.request_id,
        registry_generation: input.registry_generation,
        public_model: input.public_model,
        route_found: true,
        candidate_limit: input.candidate_limit,
        selected_target_index,
        candidates,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RouteAdmissionTier {
    Available,
    Degraded,
    ProviderAccountCooling,
    CredentialCooling,
}

fn route_state_admission_tier(state: Option<&ChannelRouteState>) -> RouteAdmissionTier {
    match state {
        Some(ChannelRouteState::Degraded) => RouteAdmissionTier::Degraded,
        Some(ChannelRouteState::ProviderCoolingDown) => RouteAdmissionTier::ProviderAccountCooling,
        Some(ChannelRouteState::CredentialCoolingDown) => RouteAdmissionTier::CredentialCooling,
        _ => RouteAdmissionTier::Available,
    }
}

fn order_weighted_priority_tier_with_indices(
    targets: &mut [(usize, RouteCandidate)],
    request_id: &str,
) {
    let Some(first_priority) = targets.first().map(|(_, target)| target.priority) else {
        return;
    };
    let tier_len = targets
        .iter()
        .take_while(|(_, target)| target.priority == first_priority)
        .count();
    if tier_len <= 1 {
        return;
    }

    let total_weight: u64 = targets
        .iter()
        .take(tier_len)
        .map(|(_, target)| u64::from(target.weight.max(1)))
        .sum();
    let mut ticket = stable_hash(request_id) % total_weight;
    let mut selected_index = 0;
    for (index, (_, target)) in targets.iter().take(tier_len).enumerate() {
        let weight = u64::from(target.weight.max(1));
        if ticket < weight {
            selected_index = index;
            break;
        }
        ticket -= weight;
    }
    targets[..tier_len].rotate_left(selected_index);
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn channel_is_allowed(channel_id: &ChannelId, allowed_channels: &[String]) -> bool {
    allowed_channels.is_empty()
        || allowed_channels
            .iter()
            .any(|allowed| allowed == &channel_id.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{provider::ProviderKind, state::ChannelId};
    use std::collections::HashMap;

    fn route_target(channel: &str, priority: u16, weight: u16, enabled: bool) -> RouteTarget {
        RouteTarget {
            channel_id: ChannelId(channel.to_string()),
            provider_kind: ProviderKind::OpenAiCompatible,
            upstream_model: None,
            priority,
            weight,
            enabled,
        }
    }

    fn route_with_targets(channels: Vec<(&str, u16, u16)>) -> ModelRoute {
        ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: channels
                .into_iter()
                .map(|(channel, priority, weight)| route_target(channel, priority, weight, true))
                .collect(),
        }
    }

    fn preview_candidate(
        channel: &str,
        included: bool,
        selected: bool,
        plan_position: Option<usize>,
        reasons: Vec<RoutePreviewReason>,
    ) -> RoutePreviewCandidate {
        RoutePreviewCandidate {
            target_index: 0,
            channel_id: ChannelId(channel.to_string()),
            provider_kind: ProviderKind::OpenAiCompatible,
            upstream_model: None,
            priority: 0,
            weight: 1,
            target_enabled: true,
            included,
            selected,
            plan_position,
            reasons,
        }
    }

    #[test]
    fn route_preview_last_resort_reasons_have_stable_codes() {
        assert_eq!(
            RoutePreviewReason::DegradedLastResort.as_str(),
            "degraded_last_resort"
        );
        assert_eq!(
            RoutePreviewReason::ProviderCoolingDownLastResort.as_str(),
            "provider_cooling_down_last_resort"
        );
        assert_eq!(
            RoutePreviewReason::CredentialCoolingDownLastResort.as_str(),
            "credential_cooling_down_last_resort"
        );
    }

    #[test]
    fn route_preview_reason_classification_is_centralized_and_stable() {
        for reason in [
            RoutePreviewReason::TargetDisabled,
            RoutePreviewReason::ClientChannelScope,
            RoutePreviewReason::ChannelDisabled,
            RoutePreviewReason::ChannelCoolingDown,
            RoutePreviewReason::NoAvailableCredentials,
            RoutePreviewReason::RuntimeUnavailable,
            RoutePreviewReason::UnknownChannel,
            RoutePreviewReason::CandidateLimit,
        ] {
            assert!(route_preview_reason_is_hard_blocker(reason));
            assert!(!route_preview_reason_is_soft_suppression(reason));
            assert!(!route_preview_reason_is_last_resort(reason));
        }

        for reason in [
            RoutePreviewReason::ChannelDegraded,
            RoutePreviewReason::ProviderCoolingDown,
            RoutePreviewReason::CredentialCoolingDown,
        ] {
            assert!(!route_preview_reason_is_hard_blocker(reason));
            assert!(route_preview_reason_is_soft_suppression(reason));
            assert!(!route_preview_reason_is_last_resort(reason));
        }

        for reason in [
            RoutePreviewReason::DegradedLastResort,
            RoutePreviewReason::ProviderCoolingDownLastResort,
            RoutePreviewReason::CredentialCoolingDownLastResort,
        ] {
            assert!(!route_preview_reason_is_hard_blocker(reason));
            assert!(!route_preview_reason_is_soft_suppression(reason));
            assert!(route_preview_reason_is_last_resort(reason));
        }
    }

    #[test]
    fn route_admission_summary_counts_hard_soft_and_last_resort_reasons() {
        let preview = RoutePreview {
            request_id: "req-summary".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route_found: true,
            candidate_limit: 16,
            selected_target_index: Some(2),
            candidates: vec![
                preview_candidate(
                    "hard",
                    false,
                    false,
                    None,
                    vec![RoutePreviewReason::ChannelCoolingDown],
                ),
                preview_candidate(
                    "soft",
                    false,
                    false,
                    None,
                    vec![RoutePreviewReason::ProviderCoolingDown],
                ),
                preview_candidate(
                    "last-resort",
                    true,
                    true,
                    Some(0),
                    vec![RoutePreviewReason::ProviderCoolingDownLastResort],
                ),
            ],
        };

        let summary = route_admission_summary(&preview);

        assert_eq!(summary.status, RouteAdmissionStatus::LastResort);
        assert_eq!(summary.status.as_str(), "last_resort");
        assert_eq!(summary.reason_code, "provider_cooling_down_last_resort");
        assert_eq!(
            summary.selected_target,
            Some(RouteAdmissionSelectedTarget {
                channel_id: ChannelId("last-resort".to_string()),
                plan_position: 0,
            })
        );
        assert_eq!(summary.candidate_count, 3);
        assert_eq!(summary.included_count, 1);
        assert_eq!(summary.blocked_count, 2);
        assert_eq!(summary.hard_blocked_count, 1);
        assert_eq!(summary.soft_suppressed_count, 1);
        assert!(summary.last_resort_used);
        assert_eq!(
            summary.last_resort_reason,
            Some("provider_cooling_down_last_resort")
        );
    }

    #[test]
    fn route_admission_summary_prefers_hard_reason_for_unavailable_preview() {
        let preview = RoutePreview {
            request_id: "req-unavailable-summary".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route_found: true,
            candidate_limit: 16,
            selected_target_index: None,
            candidates: vec![
                preview_candidate(
                    "soft",
                    false,
                    false,
                    None,
                    vec![RoutePreviewReason::ProviderCoolingDown],
                ),
                preview_candidate(
                    "hard",
                    false,
                    false,
                    None,
                    vec![RoutePreviewReason::CandidateLimit],
                ),
            ],
        };

        let summary = route_admission_summary(&preview);

        assert_eq!(summary.status, RouteAdmissionStatus::Unavailable);
        assert_eq!(summary.status.as_str(), "unavailable");
        assert_eq!(summary.reason_code, "candidate_limit");
        assert_eq!(summary.selected_target, None);
        assert!(!summary.last_resort_used);
        assert_eq!(summary.last_resort_reason, None);
    }

    #[test]
    fn route_admission_taxonomy_prefers_available_over_degraded_and_provider_account_cooling() {
        let route = route_with_targets(vec![
            ("provider-account-cooling", 0, 1),
            ("degraded", 1, 1),
            ("available", 2, 1),
        ]);
        let states = HashMap::from([
            (
                ChannelId("provider-account-cooling".to_string()),
                // Provider and account failure-domain cooldowns are both
                // lowered to ProviderCoolingDown before route planning.
                ChannelRouteState::ProviderCoolingDown,
            ),
            (
                ChannelId("degraded".to_string()),
                ChannelRouteState::Degraded,
            ),
            (
                ChannelId("available".to_string()),
                ChannelRouteState::Available,
            ),
        ]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-taxonomy".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, Some(2));
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::ProviderCoolingDown]
        );
        assert_eq!(
            preview.candidates[1].reasons,
            vec![RoutePreviewReason::ChannelDegraded]
        );
        assert!(preview.candidates[2].included);
        assert!(preview.candidates[2].selected);
        assert!(preview.candidates[2].reasons.is_empty());
    }

    #[test]
    fn plan_route_prefers_available_over_degraded_and_provider_account_cooling() {
        let route = route_with_targets(vec![
            ("provider-account-cooling", 0, 1),
            ("degraded", 1, 1),
            ("available", 2, 1),
        ]);
        let states = HashMap::from([
            (
                ChannelId("provider-account-cooling".to_string()),
                ChannelRouteState::ProviderCoolingDown,
            ),
            (
                ChannelId("degraded".to_string()),
                ChannelRouteState::Degraded,
            ),
            (
                ChannelId("available".to_string()),
                ChannelRouteState::Available,
            ),
        ]);

        let plan = plan_route(RoutePlanInput {
            request_id: "req-plan-taxonomy".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        })
        .expect("available route target should be selected");

        assert_eq!(plan.selected_index, 0);
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].target_index, 2);
        assert_eq!(
            plan.targets[0].channel_id,
            ChannelId("available".to_string())
        );
    }

    #[test]
    fn degraded_soft_state_is_last_resort_after_hard_blockers_fail_closed() {
        let route = route_with_targets(vec![
            ("hard-cooling", 0, 1),
            ("runtime-unavailable", 1, 1),
            ("degraded", 2, 1),
        ]);
        let states = HashMap::from([
            (
                ChannelId("hard-cooling".to_string()),
                ChannelRouteState::CoolingDown,
            ),
            (
                ChannelId("runtime-unavailable".to_string()),
                ChannelRouteState::RuntimeUnavailable,
            ),
            (
                ChannelId("degraded".to_string()),
                ChannelRouteState::Degraded,
            ),
        ]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-degraded-hard-blockers".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 1,
        });

        assert_eq!(preview.selected_target_index, Some(2));
        assert!(!preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::ChannelCoolingDown]
        );
        assert!(!preview.candidates[1].included);
        assert_eq!(
            preview.candidates[1].reasons,
            vec![RoutePreviewReason::RuntimeUnavailable]
        );
        assert!(preview.candidates[2].included);
        assert_eq!(
            preview.candidates[2].reasons,
            vec![RoutePreviewReason::DegradedLastResort]
        );
    }

    #[test]
    fn provider_account_soft_cooling_is_last_resort_when_no_better_soft_tier_exists() {
        let route = route_with_targets(vec![
            ("provider-cooling", 0, 1),
            ("account-cooling", 1, 1),
            ("unknown", 2, 1),
        ]);
        let states = HashMap::from([
            (
                ChannelId("provider-cooling".to_string()),
                ChannelRouteState::ProviderCoolingDown,
            ),
            (
                ChannelId("account-cooling".to_string()),
                ChannelRouteState::ProviderCoolingDown,
            ),
            (
                ChannelId("unknown".to_string()),
                ChannelRouteState::UnknownChannel,
            ),
        ]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-provider-account-last-resort".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, Some(0));
        assert!(preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::ProviderCoolingDownLastResort]
        );
        assert!(preview.candidates[1].included);
        assert_eq!(
            preview.candidates[1].reasons,
            vec![RoutePreviewReason::ProviderCoolingDownLastResort]
        );
        assert!(!preview.candidates[2].included);
        assert_eq!(
            preview.candidates[2].reasons,
            vec![RoutePreviewReason::UnknownChannel]
        );
    }

    #[test]
    fn hard_blockers_fail_closed_and_never_become_last_resort() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                route_target("target-disabled", 0, 1, false),
                route_target("scope-denied", 1, 1, true),
                route_target("channel-disabled", 2, 1, true),
                route_target("hard-cooling", 3, 1, true),
                route_target("no-credentials", 4, 1, true),
                route_target("runtime-unavailable", 5, 1, true),
                route_target("unknown", 6, 1, true),
            ],
        };
        let allowed_channels = vec![
            "target-disabled".to_string(),
            "channel-disabled".to_string(),
            "hard-cooling".to_string(),
            "no-credentials".to_string(),
            "runtime-unavailable".to_string(),
            "unknown".to_string(),
        ];
        let states = HashMap::from([
            (
                ChannelId("channel-disabled".to_string()),
                ChannelRouteState::Disabled,
            ),
            (
                ChannelId("hard-cooling".to_string()),
                ChannelRouteState::CoolingDown,
            ),
            (
                ChannelId("no-credentials".to_string()),
                ChannelRouteState::NoAvailableCredentials,
            ),
            (
                ChannelId("runtime-unavailable".to_string()),
                ChannelRouteState::RuntimeUnavailable,
            ),
            (
                ChannelId("unknown".to_string()),
                ChannelRouteState::UnknownChannel,
            ),
        ]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-hard-blockers".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &allowed_channels,
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, None);
        assert!(preview
            .candidates
            .iter()
            .all(|candidate| !candidate.included));
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::TargetDisabled]
        );
        assert_eq!(
            preview.candidates[1].reasons,
            vec![RoutePreviewReason::ClientChannelScope]
        );
        assert_eq!(
            preview.candidates[2].reasons,
            vec![RoutePreviewReason::ChannelDisabled]
        );
        assert_eq!(
            preview.candidates[3].reasons,
            vec![RoutePreviewReason::ChannelCoolingDown]
        );
        assert_eq!(
            preview.candidates[4].reasons,
            vec![RoutePreviewReason::NoAvailableCredentials]
        );
        assert_eq!(
            preview.candidates[5].reasons,
            vec![RoutePreviewReason::RuntimeUnavailable]
        );
        assert_eq!(
            preview.candidates[6].reasons,
            vec![RoutePreviewReason::UnknownChannel]
        );
    }

    #[test]
    fn candidate_limit_applies_only_after_hard_blockers_and_lower_soft_tiers_are_removed() {
        let route = route_with_targets(vec![
            ("hard-blocked", 0, 1),
            ("provider-account-cooling", 1, 1),
            ("available", 2, 1),
            ("available-over-limit", 3, 1),
        ]);
        let states = HashMap::from([
            (
                ChannelId("hard-blocked".to_string()),
                ChannelRouteState::CoolingDown,
            ),
            (
                ChannelId("provider-account-cooling".to_string()),
                ChannelRouteState::ProviderCoolingDown,
            ),
            (
                ChannelId("available".to_string()),
                ChannelRouteState::Available,
            ),
            (
                ChannelId("available-over-limit".to_string()),
                ChannelRouteState::Available,
            ),
        ]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-limit-final-eligible".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 1,
        });

        assert_eq!(preview.selected_target_index, Some(2));
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::ChannelCoolingDown]
        );
        assert_eq!(
            preview.candidates[1].reasons,
            vec![RoutePreviewReason::ProviderCoolingDown]
        );
        assert!(preview.candidates[2].included);
        assert_eq!(preview.candidates[2].plan_position, Some(0));
        assert_eq!(
            preview.candidates[3].reasons,
            vec![RoutePreviewReason::CandidateLimit]
        );
    }

    #[test]
    fn route_plan_is_pure_snapshot_consumer() {
        let source = include_str!("route_plan.rs")
            .split_once("#[cfg(test)]")
            .map_or(include_str!("route_plan.rs"), |(production, _)| production);
        let required = [
            "pub struct RoutePlanInput",
            "pub fn plan_route",
            "pub fn preview_route",
        ];
        let forbidden = [
            "AppState",
            "ChannelRegistry",
            "PoolState",
            "CredentialStoreHandle",
            "RegistryStoreHandle",
            "Mutex",
            "RwLock",
            "Arc<",
            "tokio",
            "reqwest",
            ".await",
            ".lock()",
            ".read()",
            "registry_store",
            "credential_store",
        ];

        for token in required {
            assert!(
                source.contains(token),
                "route_plan production source must expose {token}"
            );
        }
        for token in forbidden {
            assert!(
                !source.contains(token),
                "route_plan production source must not depend on {token}"
            );
        }
    }

    #[test]
    fn active_channel_cooldown_is_filtered_even_when_it_is_the_only_target() {
        let route = route_with_targets(vec![("cooling", 0, 1)]);
        let states = HashMap::from([(
            ChannelId("cooling".to_string()),
            ChannelRouteState::CoolingDown,
        )]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-cooling".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, None);
        assert!(!preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::ChannelCoolingDown]
        );
        assert_eq!(
            plan_route(RoutePlanInput {
                request_id: "req-cooling".to_string(),
                registry_generation: 1,
                public_model: Some("gpt-x".to_string()),
                route: Some(&route),
                channel_states: &states,
                allowed_channels: &[],
                candidate_limit: 16,
            }),
            Err(RoutePlanError::NoEnabledTargets)
        );
    }

    #[test]
    fn degraded_target_remains_last_resort_when_no_available_target_exists() {
        let route = route_with_targets(vec![("degraded", 0, 1)]);
        let states = HashMap::from([(
            ChannelId("degraded".to_string()),
            ChannelRouteState::Degraded,
        )]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-degraded".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, Some(0));
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::DegradedLastResort]
        );
    }

    #[test]
    fn provider_cooling_down_target_remains_last_resort_when_no_available_target_exists() {
        let route = route_with_targets(vec![("provider-cooling", 0, 1)]);
        let states = HashMap::from([(
            ChannelId("provider-cooling".to_string()),
            ChannelRouteState::ProviderCoolingDown,
        )]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-provider-cooling".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, Some(0));
        assert!(preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::ProviderCoolingDownLastResort]
        );
    }

    #[test]
    fn credential_cooling_down_target_remains_last_resort_when_no_available_target_exists() {
        let route = route_with_targets(vec![("credential-cooling", 0, 1)]);
        let states = HashMap::from([(
            ChannelId("credential-cooling".to_string()),
            ChannelRouteState::CredentialCoolingDown,
        )]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-credential-cooling".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, Some(0));
        assert!(preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::CredentialCoolingDownLastResort]
        );
        let summary = route_admission_summary(&preview);
        assert_eq!(summary.status, RouteAdmissionStatus::LastResort);
        assert_eq!(summary.reason_code, "credential_cooling_down_last_resort");
        assert_eq!(
            summary.last_resort_reason,
            Some("credential_cooling_down_last_resort")
        );
    }

    #[test]
    fn route_plan_prefers_provider_cooling_targets_over_credential_cooling_targets() {
        let route = route_with_targets(vec![
            ("credential-cooling", 0, 1),
            ("provider-cooling", 1, 1),
        ]);
        let channel_states = HashMap::from([
            (
                ChannelId("credential-cooling".to_string()),
                ChannelRouteState::CredentialCoolingDown,
            ),
            (
                ChannelId("provider-cooling".to_string()),
                ChannelRouteState::ProviderCoolingDown,
            ),
        ]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-credential-cooling-suppressed".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &channel_states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert!(!preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::CredentialCoolingDown]
        );
        assert!(preview.candidates[1].included);
        assert_eq!(
            preview.candidates[1].reasons,
            vec![RoutePreviewReason::ProviderCoolingDownLastResort]
        );
        assert_eq!(preview.selected_target_index, Some(1));
    }

    #[test]
    fn unknown_channel_target_is_filtered_with_specific_reason() {
        let route = route_with_targets(vec![("missing", 0, 1)]);
        let states = HashMap::from([(
            ChannelId("missing".to_string()),
            ChannelRouteState::UnknownChannel,
        )]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-missing".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, None);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::UnknownChannel]
        );
    }

    #[test]
    fn runtime_unavailable_target_is_filtered_with_specific_reason() {
        let route = route_with_targets(vec![("locked", 0, 1)]);
        let states = HashMap::from([(
            ChannelId("locked".to_string()),
            ChannelRouteState::RuntimeUnavailable,
        )]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req-runtime-unavailable".to_string(),
            registry_generation: 1,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(preview.selected_target_index, None);
        assert!(!preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::RuntimeUnavailable]
        );
        assert_eq!(
            plan_route(RoutePlanInput {
                request_id: "req-runtime-unavailable".to_string(),
                registry_generation: 1,
                public_model: Some("gpt-x".to_string()),
                route: Some(&route),
                channel_states: &states,
                allowed_channels: &[],
                candidate_limit: 16,
            }),
            Err(RoutePlanError::NoEnabledTargets)
        );
    }

    #[test]
    fn route_plan_preserves_ordered_candidate_set() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("primary".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("fallback".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: Some("upstream-gpt-x".to_string()),
                    priority: 20,
                    weight: 1,
                    enabled: true,
                },
            ],
        };

        let plan = plan_route(RoutePlanInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &HashMap::new(),
            allowed_channels: &[],
            candidate_limit: 16,
        })
        .unwrap();

        assert_eq!(plan.registry_generation, 7);
        assert_eq!(plan.targets.len(), 2);
        assert_eq!(plan.targets[0].channel_id, ChannelId("primary".to_string()));
        assert_eq!(
            plan.targets[1].channel_id,
            ChannelId("fallback".to_string())
        );
        assert_eq!(
            plan.targets[1].upstream_model.as_deref(),
            Some("upstream-gpt-x")
        );
    }

    #[test]
    fn route_plan_filters_disabled_targets_and_clamps_limit() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("disabled".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 1,
                    weight: 1,
                    enabled: false,
                },
                RouteTarget {
                    channel_id: ChannelId("first".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("second".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 20,
                    weight: 1,
                    enabled: true,
                },
            ],
        };

        let plan = plan_route(RoutePlanInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &HashMap::new(),
            allowed_channels: &[],
            candidate_limit: 1,
        })
        .unwrap();

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].channel_id, ChannelId("first".to_string()));
    }

    #[test]
    fn route_plan_candidates_keep_source_target_index() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("disabled".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 1,
                    weight: 1,
                    enabled: false,
                },
                RouteTarget {
                    channel_id: ChannelId("selected".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                },
            ],
        };

        let plan = plan_route(RoutePlanInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &HashMap::new(),
            allowed_channels: &[],
            candidate_limit: 16,
        })
        .unwrap();

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(
            plan.targets[0].channel_id,
            ChannelId("selected".to_string())
        );
        assert_eq!(plan.targets[0].target_index, 1);
    }

    #[test]
    fn route_plan_prefers_available_targets_over_degraded_targets() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("primary".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("fallback".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 20,
                    weight: 1,
                    enabled: true,
                },
            ],
        };
        let channel_states = HashMap::from([
            (
                ChannelId("primary".to_string()),
                ChannelRouteState::Degraded,
            ),
            (
                ChannelId("fallback".to_string()),
                ChannelRouteState::Available,
            ),
        ]);

        let plan = plan_route(RoutePlanInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &channel_states,
            allowed_channels: &[],
            candidate_limit: 16,
        })
        .unwrap();

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(
            plan.targets[0].channel_id,
            ChannelId("fallback".to_string())
        );
    }

    #[test]
    fn route_plan_prefers_available_targets_over_provider_cooling_targets() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("primary".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("fallback".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 20,
                    weight: 1,
                    enabled: true,
                },
            ],
        };
        let channel_states = HashMap::from([
            (
                ChannelId("primary".to_string()),
                ChannelRouteState::ProviderCoolingDown,
            ),
            (
                ChannelId("fallback".to_string()),
                ChannelRouteState::Available,
            ),
        ]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &channel_states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert!(!preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::ProviderCoolingDown]
        );
        assert!(preview.candidates[1].included);
        assert_eq!(preview.selected_target_index, Some(1));
    }

    #[test]
    fn route_plan_prefers_degraded_targets_over_provider_cooling_targets() {
        let route = route_with_targets(vec![
            ("provider-cooling", 0, 1),
            ("degraded-fallback", 1, 1),
        ]);
        let channel_states = HashMap::from([
            (
                ChannelId("provider-cooling".to_string()),
                ChannelRouteState::ProviderCoolingDown,
            ),
            (
                ChannelId("degraded-fallback".to_string()),
                ChannelRouteState::Degraded,
            ),
        ]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &channel_states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert!(!preview.candidates[0].included);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::ProviderCoolingDown]
        );
        assert!(preview.candidates[1].included);
        assert_eq!(
            preview.candidates[1].reasons,
            vec![RoutePreviewReason::DegradedLastResort]
        );
        assert_eq!(preview.selected_target_index, Some(1));
    }

    #[test]
    fn route_plan_keeps_degraded_targets_as_last_resort() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("primary".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("fallback".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 20,
                    weight: 1,
                    enabled: true,
                },
            ],
        };
        let channel_states = HashMap::from([
            (
                ChannelId("primary".to_string()),
                ChannelRouteState::Degraded,
            ),
            (
                ChannelId("fallback".to_string()),
                ChannelRouteState::Degraded,
            ),
        ]);

        let plan = plan_route(RoutePlanInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &channel_states,
            allowed_channels: &[],
            candidate_limit: 16,
        })
        .unwrap();

        assert_eq!(plan.targets.len(), 2);
        assert_eq!(plan.targets[0].channel_id, ChannelId("primary".to_string()));
        assert_eq!(
            plan.targets[1].channel_id,
            ChannelId("fallback".to_string())
        );
    }

    #[test]
    fn route_plan_applies_client_channel_scope_before_candidate_limit() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("unauthorized".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 1,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("authorized".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 20,
                    weight: 1,
                    enabled: true,
                },
            ],
        };
        let allowed_channels = vec!["authorized".to_string()];

        let plan = plan_route(RoutePlanInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &HashMap::new(),
            allowed_channels: &allowed_channels,
            candidate_limit: 1,
        })
        .unwrap();

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(
            plan.targets[0].channel_id,
            ChannelId("authorized".to_string())
        );
    }

    #[test]
    fn priority_weighted_sticky_selects_within_best_priority_tier_by_weight() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::PriorityWeightedSticky,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("light".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("heavy".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 5,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("lower-priority".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 20,
                    weight: 100,
                    enabled: true,
                },
            ],
        };

        let mut light_first = 0;
        let mut heavy_first = 0;
        for index in 0..64 {
            let plan = plan_route(RoutePlanInput {
                request_id: format!("req_{index}"),
                registry_generation: 7,
                public_model: Some("gpt-x".to_string()),
                route: Some(&route),
                channel_states: &HashMap::new(),
                allowed_channels: &[],
                candidate_limit: 16,
            })
            .unwrap();

            match plan.targets[0].channel_id.0.as_str() {
                "light" => light_first += 1,
                "heavy" => heavy_first += 1,
                other => panic!("unexpected first weighted target {other}"),
            }
            assert_eq!(
                plan.targets.last().unwrap().channel_id,
                ChannelId("lower-priority".to_string())
            );
        }

        assert!(light_first > 0);
        assert!(heavy_first > light_first);
    }

    #[test]
    fn route_preview_explains_selected_filtered_and_last_resort_targets() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("disabled".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 1,
                    weight: 1,
                    enabled: false,
                },
                RouteTarget {
                    channel_id: ChannelId("unauthorized".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 2,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("cooling".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 3,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("selected".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: Some("upstream-gpt-x".to_string()),
                    priority: 4,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("over-limit".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 5,
                    weight: 1,
                    enabled: true,
                },
            ],
        };
        let allowed_channels = vec![
            "disabled".to_string(),
            "cooling".to_string(),
            "selected".to_string(),
            "over-limit".to_string(),
        ];
        let channel_states = HashMap::from([(
            ChannelId("cooling".to_string()),
            ChannelRouteState::Degraded,
        )]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req_preview".to_string(),
            registry_generation: 9,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &channel_states,
            allowed_channels: &allowed_channels,
            candidate_limit: 1,
        });

        assert_eq!(preview.registry_generation, 9);
        assert_eq!(preview.selected_target_index, Some(3));
        assert_eq!(preview.candidates.len(), 5);
        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::TargetDisabled]
        );
        assert_eq!(
            preview.candidates[1].reasons,
            vec![RoutePreviewReason::ClientChannelScope]
        );
        assert_eq!(
            preview.candidates[2].reasons,
            vec![RoutePreviewReason::ChannelDegraded]
        );
        assert!(preview.candidates[3].included);
        assert!(preview.candidates[3].selected);
        assert_eq!(
            preview.candidates[3].upstream_model.as_deref(),
            Some("upstream-gpt-x")
        );
        assert_eq!(
            preview.candidates[4].reasons,
            vec![RoutePreviewReason::CandidateLimit]
        );
    }

    #[test]
    fn route_plan_filters_targets_without_available_credentials() {
        let route = ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: vec![
                RouteTarget {
                    channel_id: ChannelId("empty".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 10,
                    weight: 1,
                    enabled: true,
                },
                RouteTarget {
                    channel_id: ChannelId("fallback".to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority: 20,
                    weight: 1,
                    enabled: true,
                },
            ],
        };
        let channel_states = HashMap::from([(
            ChannelId("empty".to_string()),
            ChannelRouteState::NoAvailableCredentials,
        )]);

        let preview = preview_route(RoutePreviewInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &channel_states,
            allowed_channels: &[],
            candidate_limit: 16,
        });

        assert_eq!(
            preview.candidates[0].reasons,
            vec![RoutePreviewReason::NoAvailableCredentials]
        );
        assert!(!preview.candidates[0].included);
        assert!(preview.candidates[1].included);
        assert!(preview.candidates[1].selected);

        let plan = plan_route(RoutePlanInput {
            request_id: "req_test".to_string(),
            registry_generation: 7,
            public_model: Some("gpt-x".to_string()),
            route: Some(&route),
            channel_states: &channel_states,
            allowed_channels: &[],
            candidate_limit: 16,
        })
        .unwrap();
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(
            plan.targets[0].channel_id,
            ChannelId("fallback".to_string())
        );
    }
}
