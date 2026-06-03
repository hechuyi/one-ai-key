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
    NoAvailableCredentials,
    RuntimeUnavailable,
    ChannelDegraded,
    DegradedLastResort,
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
            RoutePreviewReason::NoAvailableCredentials => "no_available_credentials",
            RoutePreviewReason::RuntimeUnavailable => "runtime_unavailable",
            RoutePreviewReason::ChannelDegraded => "channel_degraded",
            RoutePreviewReason::DegradedLastResort => "degraded_last_resort",
            RoutePreviewReason::UnknownChannel => "unknown_channel",
            RoutePreviewReason::CandidateLimit => "candidate_limit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRouteState {
    Available,
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
        ChannelRouteState::RuntimeUnavailable => 2,
        ChannelRouteState::CoolingDown => 3,
        ChannelRouteState::NoAvailableCredentials => 4,
        ChannelRouteState::Disabled => 5,
        ChannelRouteState::UnknownChannel => 6,
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

    let has_available_eligible = eligible.iter().any(|(_, target)| {
        !matches!(
            input.channel_states.get(&target.channel_id),
            Some(ChannelRouteState::Degraded)
        )
    });
    if has_available_eligible {
        let mut retained = Vec::with_capacity(eligible.len());
        for (index, target) in eligible {
            if matches!(
                input.channel_states.get(&target.channel_id),
                Some(ChannelRouteState::Degraded)
            ) {
                candidates[index]
                    .reasons
                    .push(RoutePreviewReason::ChannelDegraded);
            } else {
                retained.push((index, target));
            }
        }
        eligible = retained;
    } else {
        for (index, target) in &eligible {
            if matches!(
                input.channel_states.get(&target.channel_id),
                Some(ChannelRouteState::Degraded)
            ) {
                candidates[*index]
                    .reasons
                    .push(RoutePreviewReason::DegradedLastResort);
            }
        }
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

    fn route_with_targets(channels: Vec<(&str, u16, u16)>) -> ModelRoute {
        ModelRoute {
            public_model: "gpt-x".to_string(),
            strategy: RouteStrategy::Priority,
            targets: channels
                .into_iter()
                .map(|(channel, priority, weight)| RouteTarget {
                    channel_id: ChannelId(channel.to_string()),
                    provider_kind: ProviderKind::OpenAiCompatible,
                    upstream_model: None,
                    priority,
                    weight,
                    enabled: true,
                })
                .collect(),
        }
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
