use serde::Serialize;
use std::collections::HashMap;

use crate::{
    config::{
        AccountConfig, ModelRouteConfig, PolicyProfileConfig, PoolConfig, ProviderConfig,
        ResolvedConfig, RoutingProfileConfig,
    },
    events::{ManagementAuditEvent, ManagementEventActor},
    management_errors::{registry_store_error, ManagementServiceError},
    registry::RegistryDocument,
    registry_store::{
        overlay_registry_resources, AccountRegistryCommand, ChannelRegistryCommand,
        ModelRouteRegistryCommand, PolicyProfileRegistryCommand, ProviderRegistryCommand,
        RegistryCommand, RegistryStoreCommit, RegistryStoreError, RoutingProfileRegistryCommand,
    },
    state::AppState,
};

#[derive(Debug, Clone, Copy)]
pub struct RegistryMutationStatus {
    pub registry_version: u64,
    pub staged_registry_version: u64,
    pub active_registry_generation: u64,
    pub applied_to_runtime: bool,
    pub runtime_reload_required: bool,
}

pub fn staged_registry_mutation_status(
    registry_version: u64,
    active_registry_generation: u64,
) -> RegistryMutationStatus {
    RegistryMutationStatus {
        registry_version,
        staged_registry_version: registry_version,
        active_registry_generation,
        applied_to_runtime: false,
        runtime_reload_required: true,
    }
}

pub fn existing_staged_registry_mutation_status(
    registry_version: u64,
    active_registry_version: Option<u64>,
    active_registry_generation: u64,
) -> RegistryMutationStatus {
    RegistryMutationStatus {
        registry_version,
        staged_registry_version: registry_version,
        active_registry_generation,
        applied_to_runtime: false,
        runtime_reload_required: Some(registry_version) != active_registry_version,
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RegistryMutationStatusFields {
    pub registry_version: u64,
    pub staged_registry_version: u64,
    pub active_registry_generation: u64,
    pub applied_to_runtime: bool,
    pub runtime_reload_required: bool,
}

pub fn registry_mutation_status_fields(
    status: RegistryMutationStatus,
) -> RegistryMutationStatusFields {
    RegistryMutationStatusFields {
        registry_version: status.registry_version,
        staged_registry_version: status.staged_registry_version,
        active_registry_generation: status.active_registry_generation,
        applied_to_runtime: status.applied_to_runtime,
        runtime_reload_required: status.runtime_reload_required,
    }
}

pub fn staged_registry_validator(
    validation_bootstrap: RegistryDocument,
) -> impl Fn(&RegistryDocument) -> Result<(), RegistryStoreError> + Send + Sync + 'static {
    move |document| {
        resolve_staged_registry_document(validation_bootstrap.clone(), document.clone()).map(|_| ())
    }
}

pub fn resolve_staged_registry_document(
    validation_bootstrap: RegistryDocument,
    staged_registry_document: RegistryDocument,
) -> Result<ResolvedConfig, RegistryStoreError> {
    overlay_registry_resources(validation_bootstrap, staged_registry_document)
        .resolve()
        .map_err(|err| RegistryStoreError::Validation(format!("registry validation failed: {err}")))
}

pub async fn apply_staged_registry_command(
    state: &AppState,
    command: RegistryCommand,
) -> Result<RegistryStoreCommit, RegistryStoreError> {
    let validation_bootstrap = (*state.registry_validation_bootstrap).clone();
    state
        .registry_store
        .apply_command(command, staged_registry_validator(validation_bootstrap))
        .await
}

pub async fn apply_staged_registry_mutation(
    state: &AppState,
    command: RegistryCommand,
) -> Result<RegistryMutationStatus, RegistryStoreError> {
    let commit = apply_staged_registry_command(state, command).await?;
    Ok(staged_registry_mutation_status(
        commit.registry_version,
        state.channels.registry_generation(),
    ))
}

pub async fn apply_audited_staged_registry_mutation(
    state: &AppState,
    actor: ManagementEventActor,
    command: RegistryCommand,
    audit: RegistryMutationAudit,
) -> Result<RegistryMutationStatus, ManagementServiceError> {
    let expected_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?
        .ok_or_else(|| {
            ManagementServiceError::Conflict(
                "registry persistence requires a writable registry store".to_string(),
            )
        })?
        + 1;
    record_registry_mutation_audit_event(state, actor, &audit, expected_registry_version).await?;
    apply_staged_registry_mutation(state, command)
        .await
        .map_err(registry_store_error)
}

async fn record_registry_mutation_audit_event(
    state: &AppState,
    actor: ManagementEventActor,
    audit: &RegistryMutationAudit,
    generation: u64,
) -> Result<(), ManagementServiceError> {
    state
        .events
        .record_audit_event(ManagementAuditEvent {
            kind: audit.kind.to_string(),
            action: audit.kind.to_string(),
            resource_type: audit.resource_type.to_string(),
            resource_id: audit.resource_id.clone(),
            channel_id: String::new(),
            credential_id: String::new(),
            outcome: "applied".to_string(),
            request_id: None,
            generation: Some(generation),
            reason_code: audit.reason_code.to_string(),
            actor: Some(actor),
        })
        .await
        .map_err(|err| ManagementServiceError::EventAppendFailed {
            message: err.to_string(),
            stale_history_id: None,
        })
}

#[derive(Debug)]
pub struct RegistryMutationAudit {
    pub kind: &'static str,
    pub resource_type: &'static str,
    pub resource_id: String,
    pub reason_code: &'static str,
}

pub async fn staged_model_routes_for_state(
    state: &AppState,
) -> Result<(u64, HashMap<String, ModelRouteConfig>), ManagementServiceError> {
    let expected_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?
        .ok_or_else(|| {
            ManagementServiceError::Conflict(
                "model discovery sync requires a writable registry store".to_string(),
            )
        })?;
    let staged_registry_document = state
        .registry_store
        .load_registry_for_validation()
        .await
        .map_err(registry_store_error)?
        .ok_or_else(|| {
            ManagementServiceError::Conflict(
                "model discovery sync requires a writable registry store".to_string(),
            )
        })?;

    Ok((
        expected_registry_version,
        staged_registry_document.model_routes,
    ))
}

pub async fn apply_staged_model_route_batch_for_state(
    state: &AppState,
    expected_registry_version: u64,
    routes: Vec<(String, ModelRouteConfig)>,
) -> Result<RegistryStoreCommit, ManagementServiceError> {
    apply_staged_registry_command(
        state,
        RegistryCommand::ModelRoute(ModelRouteRegistryCommand::UpsertBatch {
            expected_registry_version: Some(expected_registry_version),
            routes,
        }),
    )
    .await
    .map_err(registry_store_error)
}

pub async fn apply_audited_staged_model_route_batch_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    expected_registry_version: u64,
    routes: Vec<(String, ModelRouteConfig)>,
    audit: RegistryMutationAudit,
) -> Result<RegistryStoreCommit, ManagementServiceError> {
    let current_registry_version = state
        .registry_store
        .current_version()
        .await
        .map_err(registry_store_error)?
        .ok_or_else(|| {
            ManagementServiceError::Conflict(
                "registry persistence requires a writable registry store".to_string(),
            )
        })?;
    if expected_registry_version != current_registry_version {
        return Err(ManagementServiceError::Conflict(
            "registry version changed during model route apply".to_string(),
        ));
    }
    let audit_generation = expected_registry_version.saturating_add(1);
    record_registry_mutation_audit_event(state, actor, &audit, audit_generation).await?;
    apply_staged_model_route_batch_for_state(state, expected_registry_version, routes).await
}

pub async fn registry_provider_enabled_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    provider_id: &str,
    enabled: bool,
) -> Result<RegistryProviderMutationResponse, ManagementServiceError> {
    let (kind, reason_code) = if enabled {
        (
            "registry_provider_enabled",
            "manual_registry_provider_enable",
        )
    } else {
        (
            "registry_provider_disabled",
            "manual_registry_provider_disable",
        )
    };
    let status = apply_audited_staged_registry_mutation(
        state,
        actor,
        set_registry_provider_enabled_command(provider_id, enabled),
        RegistryMutationAudit {
            kind,
            resource_type: "registry_provider",
            resource_id: provider_id.to_string(),
            reason_code,
        },
    )
    .await?;
    Ok(registry_provider_mutation_response(
        provider_id,
        enabled,
        status,
    ))
}

pub async fn registry_provider_upsert_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    provider_id: &str,
    provider: ProviderConfig,
) -> Result<RegistryProviderUpsertResponse, ManagementServiceError> {
    let status = apply_audited_staged_registry_mutation(
        state,
        actor,
        upsert_registry_provider_command(provider_id, provider),
        RegistryMutationAudit {
            kind: "registry_provider_upserted",
            resource_type: "registry_provider",
            resource_id: provider_id.to_string(),
            reason_code: "manual_registry_provider_upsert",
        },
    )
    .await?;
    Ok(registry_provider_upsert_response(provider_id, status))
}

pub async fn registry_account_enabled_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    account_id: &str,
    enabled: bool,
) -> Result<RegistryAccountMutationResponse, ManagementServiceError> {
    let (kind, reason_code) = if enabled {
        ("registry_account_enabled", "manual_registry_account_enable")
    } else {
        (
            "registry_account_disabled",
            "manual_registry_account_disable",
        )
    };
    let status = apply_audited_staged_registry_mutation(
        state,
        actor,
        set_registry_account_enabled_command(account_id, enabled),
        RegistryMutationAudit {
            kind,
            resource_type: "registry_account",
            resource_id: account_id.to_string(),
            reason_code,
        },
    )
    .await?;
    Ok(registry_account_mutation_response(
        account_id, enabled, status,
    ))
}

pub async fn registry_account_upsert_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    account_id: &str,
    account: AccountConfig,
) -> Result<RegistryAccountUpsertResponse, ManagementServiceError> {
    let status = apply_audited_staged_registry_mutation(
        state,
        actor,
        upsert_registry_account_command(account_id, account),
        RegistryMutationAudit {
            kind: "registry_account_upserted",
            resource_type: "registry_account",
            resource_id: account_id.to_string(),
            reason_code: "manual_registry_account_upsert",
        },
    )
    .await?;
    Ok(registry_account_upsert_response(account_id, status))
}

pub async fn registry_channel_enabled_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    enabled: bool,
) -> Result<RegistryChannelMutationResponse, ManagementServiceError> {
    let (kind, reason_code) = if enabled {
        ("registry_channel_enabled", "manual_registry_channel_enable")
    } else {
        (
            "registry_channel_disabled",
            "manual_registry_channel_disable",
        )
    };
    let status = apply_audited_staged_registry_mutation(
        state,
        actor,
        set_registry_channel_enabled_command(channel_id, enabled),
        RegistryMutationAudit {
            kind,
            resource_type: "registry_channel",
            resource_id: channel_id.to_string(),
            reason_code,
        },
    )
    .await?;
    Ok(registry_channel_mutation_response(
        channel_id, enabled, status,
    ))
}

pub async fn registry_channel_upsert_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    channel_id: &str,
    channel: PoolConfig,
) -> Result<RegistryChannelUpsertResponse, ManagementServiceError> {
    let status = apply_audited_staged_registry_mutation(
        state,
        actor,
        upsert_registry_channel_command(channel_id, channel),
        RegistryMutationAudit {
            kind: "registry_channel_upserted",
            resource_type: "registry_channel",
            resource_id: channel_id.to_string(),
            reason_code: "manual_registry_channel_upsert",
        },
    )
    .await?;
    Ok(registry_channel_upsert_response(channel_id, status))
}

pub async fn registry_model_route_upsert_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    public_model: &str,
    expected_registry_version: u64,
    route: ModelRouteConfig,
) -> Result<RegistryModelRouteMutationResponse, ManagementServiceError> {
    let commit = apply_audited_staged_model_route_batch_for_state(
        state,
        actor,
        expected_registry_version,
        vec![(public_model.to_string(), route)],
        RegistryMutationAudit {
            kind: "registry_model_route_upserted",
            resource_type: "registry_model_route",
            resource_id: public_model.to_string(),
            reason_code: "manual_registry_model_route_upsert",
        },
    )
    .await?;
    let status = staged_registry_mutation_status(
        commit.registry_version,
        state.channels.registry_generation(),
    );
    Ok(registry_model_route_mutation_response(public_model, status))
}

pub async fn registry_policy_profile_upsert_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    profile_id: &str,
    profile: PolicyProfileConfig,
) -> Result<RegistryPolicyProfileMutationResponse, ManagementServiceError> {
    let status = apply_audited_staged_registry_mutation(
        state,
        actor,
        upsert_registry_policy_profile_command(profile_id, profile),
        RegistryMutationAudit {
            kind: "registry_policy_profile_upserted",
            resource_type: "registry_policy_profile",
            resource_id: profile_id.to_string(),
            reason_code: "manual_registry_policy_profile_upsert",
        },
    )
    .await?;
    Ok(registry_policy_profile_mutation_response(
        profile_id, status,
    ))
}

pub async fn registry_routing_profile_upsert_response_for_state(
    state: &AppState,
    actor: ManagementEventActor,
    profile_id: &str,
    profile: RoutingProfileConfig,
) -> Result<RegistryRoutingProfileMutationResponse, ManagementServiceError> {
    let status = apply_audited_staged_registry_mutation(
        state,
        actor,
        upsert_registry_routing_profile_command(profile_id, profile),
        RegistryMutationAudit {
            kind: "registry_routing_profile_upserted",
            resource_type: "registry_routing_profile",
            resource_id: profile_id.to_string(),
            reason_code: "manual_registry_routing_profile_upsert",
        },
    )
    .await?;
    Ok(registry_routing_profile_mutation_response(
        profile_id, status,
    ))
}

pub fn set_registry_provider_enabled_command(provider_id: &str, enabled: bool) -> RegistryCommand {
    RegistryCommand::Provider(ProviderRegistryCommand::SetEnabled {
        provider_id: provider_id.to_string(),
        enabled,
    })
}

pub fn upsert_registry_provider_command(
    provider_id: &str,
    provider: ProviderConfig,
) -> RegistryCommand {
    RegistryCommand::Provider(ProviderRegistryCommand::Upsert {
        provider_id: provider_id.to_string(),
        provider,
    })
}

pub fn set_registry_account_enabled_command(account_id: &str, enabled: bool) -> RegistryCommand {
    RegistryCommand::Account(AccountRegistryCommand::SetEnabled {
        account_id: account_id.to_string(),
        enabled,
    })
}

pub fn upsert_registry_account_command(
    account_id: &str,
    account: AccountConfig,
) -> RegistryCommand {
    RegistryCommand::Account(AccountRegistryCommand::Upsert {
        account_id: account_id.to_string(),
        account,
    })
}

pub fn set_registry_channel_enabled_command(channel_id: &str, enabled: bool) -> RegistryCommand {
    RegistryCommand::Channel(ChannelRegistryCommand::SetEnabled {
        channel_id: channel_id.to_string(),
        enabled,
    })
}

pub fn upsert_registry_channel_command(channel_id: &str, channel: PoolConfig) -> RegistryCommand {
    RegistryCommand::Channel(ChannelRegistryCommand::Upsert {
        channel_id: channel_id.to_string(),
        channel: Box::new(channel),
    })
}

#[allow(dead_code)]
pub fn upsert_registry_model_route_command(
    public_model: &str,
    route: ModelRouteConfig,
) -> RegistryCommand {
    RegistryCommand::ModelRoute(ModelRouteRegistryCommand::Upsert {
        public_model: public_model.to_string(),
        route,
    })
}

pub fn upsert_registry_policy_profile_command(
    profile_id: &str,
    profile: PolicyProfileConfig,
) -> RegistryCommand {
    RegistryCommand::PolicyProfile(PolicyProfileRegistryCommand::Upsert {
        profile_id: profile_id.to_string(),
        profile,
    })
}

pub fn upsert_registry_routing_profile_command(
    profile_id: &str,
    profile: RoutingProfileConfig,
) -> RegistryCommand {
    RegistryCommand::RoutingProfile(RoutingProfileRegistryCommand::Upsert {
        profile_id: profile_id.to_string(),
        profile,
    })
}

#[derive(Debug, Serialize)]
pub struct RegistryProviderMutationResponse {
    pub provider_id: String,
    pub enabled: bool,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_provider_mutation_response(
    provider_id: &str,
    enabled: bool,
    status: RegistryMutationStatus,
) -> RegistryProviderMutationResponse {
    RegistryProviderMutationResponse {
        provider_id: provider_id.to_string(),
        enabled,
        status: registry_mutation_status_fields(status),
    }
}

#[derive(Debug, Serialize)]
pub struct RegistryProviderUpsertResponse {
    pub provider_id: String,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_provider_upsert_response(
    provider_id: &str,
    status: RegistryMutationStatus,
) -> RegistryProviderUpsertResponse {
    RegistryProviderUpsertResponse {
        provider_id: provider_id.to_string(),
        status: registry_mutation_status_fields(status),
    }
}

#[derive(Debug, Serialize)]
pub struct RegistryAccountMutationResponse {
    pub account_id: String,
    pub enabled: bool,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_account_mutation_response(
    account_id: &str,
    enabled: bool,
    status: RegistryMutationStatus,
) -> RegistryAccountMutationResponse {
    RegistryAccountMutationResponse {
        account_id: account_id.to_string(),
        enabled,
        status: registry_mutation_status_fields(status),
    }
}

#[derive(Debug, Serialize)]
pub struct RegistryAccountUpsertResponse {
    pub account_id: String,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_account_upsert_response(
    account_id: &str,
    status: RegistryMutationStatus,
) -> RegistryAccountUpsertResponse {
    RegistryAccountUpsertResponse {
        account_id: account_id.to_string(),
        status: registry_mutation_status_fields(status),
    }
}

#[derive(Debug, Serialize)]
pub struct RegistryChannelMutationResponse {
    pub channel_id: String,
    pub enabled: bool,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_channel_mutation_response(
    channel_id: &str,
    enabled: bool,
    status: RegistryMutationStatus,
) -> RegistryChannelMutationResponse {
    RegistryChannelMutationResponse {
        channel_id: channel_id.to_string(),
        enabled,
        status: registry_mutation_status_fields(status),
    }
}

#[derive(Debug, Serialize)]
pub struct RegistryChannelUpsertResponse {
    pub channel_id: String,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_channel_upsert_response(
    channel_id: &str,
    status: RegistryMutationStatus,
) -> RegistryChannelUpsertResponse {
    RegistryChannelUpsertResponse {
        channel_id: channel_id.to_string(),
        status: registry_mutation_status_fields(status),
    }
}

#[derive(Debug, Serialize)]
pub struct RegistryModelRouteMutationResponse {
    pub public_model: String,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_model_route_mutation_response(
    public_model: &str,
    status: RegistryMutationStatus,
) -> RegistryModelRouteMutationResponse {
    RegistryModelRouteMutationResponse {
        public_model: public_model.to_string(),
        status: registry_mutation_status_fields(status),
    }
}

#[derive(Debug, Serialize)]
pub struct RegistryPolicyProfileMutationResponse {
    pub profile_id: String,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_policy_profile_mutation_response(
    profile_id: &str,
    status: RegistryMutationStatus,
) -> RegistryPolicyProfileMutationResponse {
    RegistryPolicyProfileMutationResponse {
        profile_id: profile_id.to_string(),
        status: registry_mutation_status_fields(status),
    }
}

#[derive(Debug, Serialize)]
pub struct RegistryRoutingProfileMutationResponse {
    pub profile_id: String,
    #[serde(flatten)]
    pub status: RegistryMutationStatusFields,
}

pub fn registry_routing_profile_mutation_response(
    profile_id: &str,
    status: RegistryMutationStatus,
) -> RegistryRoutingProfileMutationResponse {
    RegistryRoutingProfileMutationResponse {
        profile_id: profile_id.to_string(),
        status: registry_mutation_status_fields(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_mutation_response_projects_shared_status_fields() {
        let status = RegistryMutationStatus {
            registry_version: 11,
            staged_registry_version: 13,
            active_registry_generation: 17,
            applied_to_runtime: false,
            runtime_reload_required: true,
        };

        let shared_status = registry_mutation_status_fields(status);
        let provider = registry_provider_mutation_response("openai", true, status);
        let account = registry_account_mutation_response("acct", false, status);
        let channel = registry_channel_mutation_response("primary", true, status);
        let model_route = registry_model_route_mutation_response("gpt-4o-mini", status);
        let routing_profile = registry_routing_profile_mutation_response("default", status);

        assert_eq!(
            provider.status.registry_version,
            shared_status.registry_version
        );
        assert_eq!(
            provider.status.staged_registry_version,
            shared_status.staged_registry_version
        );
        assert_eq!(
            provider.status.active_registry_generation,
            shared_status.active_registry_generation
        );
        assert_eq!(
            provider.status.applied_to_runtime,
            shared_status.applied_to_runtime
        );
        assert_eq!(
            provider.status.runtime_reload_required,
            shared_status.runtime_reload_required
        );

        assert_eq!(
            account.status.registry_version,
            shared_status.registry_version
        );
        assert_eq!(
            channel.status.registry_version,
            shared_status.registry_version
        );
        assert_eq!(
            model_route.status.registry_version,
            shared_status.registry_version
        );
        assert_eq!(
            routing_profile.status.registry_version,
            shared_status.registry_version
        );

        let provider_json = serde_json::to_value(provider).expect("serialize provider response");
        assert_eq!(provider_json["provider_id"], "openai");
        assert_eq!(provider_json["enabled"], true);
        assert_eq!(
            provider_json["registry_version"],
            shared_status.registry_version
        );
        assert_eq!(
            provider_json["staged_registry_version"],
            shared_status.staged_registry_version
        );
        assert_eq!(
            provider_json["active_registry_generation"],
            shared_status.active_registry_generation
        );
        assert_eq!(
            provider_json["applied_to_runtime"],
            shared_status.applied_to_runtime
        );
        assert_eq!(
            provider_json["runtime_reload_required"],
            shared_status.runtime_reload_required
        );
        assert!(provider_json.get("status").is_none());

        let account_json = serde_json::to_value(account).expect("serialize account response");
        let channel_json = serde_json::to_value(channel).expect("serialize channel response");
        let model_route_json =
            serde_json::to_value(model_route).expect("serialize model route response");
        let routing_profile_json =
            serde_json::to_value(routing_profile).expect("serialize routing profile response");

        for response in [
            account_json,
            channel_json,
            model_route_json,
            routing_profile_json,
        ] {
            assert_eq!(response["registry_version"], shared_status.registry_version);
            assert_eq!(
                response["staged_registry_version"],
                shared_status.staged_registry_version
            );
            assert_eq!(
                response["active_registry_generation"],
                shared_status.active_registry_generation
            );
            assert_eq!(
                response["applied_to_runtime"],
                shared_status.applied_to_runtime
            );
            assert_eq!(
                response["runtime_reload_required"],
                shared_status.runtime_reload_required
            );
            assert!(response.get("status").is_none());
        }
    }
}
