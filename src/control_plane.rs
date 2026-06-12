use std::{collections::BTreeSet, fs, path::PathBuf};

use anyhow::Context;
use serde::Deserialize;

use crate::{
    config::{
        reject_unknown_top_level_config_fields, AppConfig, ModelGroupConfig, ResolvedClientToken,
        ResolvedConfig, ResolvedModelGroup, ResponseFilterConfig,
    },
    credential_repository::{
        CredentialRepository, FileCredentialRepository, SqliteCredentialRepository,
    },
    provider::ProviderAdapter,
    registry::RegistryDocument,
    registry_store::{overlay_registry_resources, RegistryStoreHandle},
    state::AppState,
};

#[derive(Debug, Clone)]
pub struct ConfigSource {
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct RegistryYamlDocument {
    #[serde(flatten)]
    app: AppConfig,
    #[serde(default)]
    model_groups: std::collections::HashMap<String, ModelGroupConfig>,
    #[serde(default)]
    response_filter: ResponseFilterConfig,
}

impl ConfigSource {
    pub fn yaml_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load_registry_document(&self) -> anyhow::Result<RegistryDocument> {
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("read registry config {}", self.path.display()))?;
        let raw = crate::upstream_templates::expand_raw_yaml(&raw)
            .with_context(|| format!("expand upstream templates {}", self.path.display()))?;
        reject_unknown_top_level_config_fields(&raw, &["model_groups", "response_filter"])
            .with_context(|| format!("validate registry config YAML {}", self.path.display()))?;
        let mut cfg: RegistryYamlDocument = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse registry config YAML {}", self.path.display()))?;
        cfg.app.apply_compatibility_defaults();
        let mut document = cfg.app.into_registry_document();
        document.model_groups = cfg.model_groups;
        document.response_filter = cfg.response_filter;
        Ok(document)
    }
}

pub struct RegistryOverlay;

impl RegistryOverlay {
    pub fn overlay_staged_resources(
        bootstrap: RegistryDocument,
        staged_resources: RegistryDocument,
    ) -> RegistryDocument {
        overlay_registry_resources(bootstrap, staged_resources)
    }
}

pub struct ConfigCompiler;

#[derive(Debug, Clone)]
pub enum CredentialRepositorySelection {
    File(FileCredentialRepository),
    Sqlite {
        repository: SqliteCredentialRepository,
        credential_store_path: PathBuf,
    },
}

impl CredentialRepositorySelection {
    pub fn from_process_environment() -> anyhow::Result<Self> {
        if let Some(database_path) = std::env::var_os("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE") {
            let credential_store_path = PathBuf::from(database_path);
            let repository = SqliteCredentialRepository::open(credential_store_path.clone())?;
            return Ok(Self::Sqlite {
                repository,
                credential_store_path,
            });
        }

        Ok(Self::File(FileCredentialRepository::new()))
    }
}

impl ConfigCompiler {
    pub fn compile_with_credential_repository(
        document: RegistryDocument,
        credential_repository: &impl CredentialRepository,
        credential_store_path: Option<PathBuf>,
    ) -> anyhow::Result<ResolvedConfig> {
        document.resolve_with_credential_repository_and_store_path(
            credential_repository,
            credential_store_path,
        )
    }

    pub fn compile_with_credential_selection(
        document: RegistryDocument,
        selection: &CredentialRepositorySelection,
    ) -> anyhow::Result<ResolvedConfig> {
        match selection {
            CredentialRepositorySelection::File(repository) => {
                Self::compile_with_credential_repository(document, repository, None)
            }
            CredentialRepositorySelection::Sqlite {
                repository,
                credential_store_path,
            } => Self::compile_with_credential_repository(
                document,
                repository,
                Some(credential_store_path.clone()),
            ),
        }
    }

    pub fn compile_with_process_credential_source(
        document: RegistryDocument,
    ) -> anyhow::Result<ResolvedConfig> {
        let selection = CredentialRepositorySelection::from_process_environment()?;
        Self::compile_with_credential_selection(document, &selection)
    }
}

pub struct RuntimeAssembler;

impl RuntimeAssembler {
    pub fn assemble_startup(
        config: ResolvedConfig,
        registry_store: RegistryStoreHandle,
        registry_validation_bootstrap: Option<RegistryDocument>,
    ) -> anyhow::Result<AppState> {
        AppState::new_with_registry_store_and_validation_bootstrap(
            config,
            registry_store,
            registry_validation_bootstrap,
        )
    }

    pub fn replace_runtime(
        state: &AppState,
        config: ResolvedConfig,
        active_registry_version: Option<u64>,
    ) -> anyhow::Result<()> {
        state.rebuild_runtime_from_resolved_config(config, active_registry_version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelVisibilityProjection {
    pub client_token_ref: String,
    pub visible_models: Vec<String>,
    pub reason_code: String,
}

pub struct Diagnostics;

impl Diagnostics {
    pub fn model_visibility_preview(config: &ResolvedConfig) -> Vec<ModelVisibilityProjection> {
        config
            .client_tokens
            .iter()
            .map(|client_token| model_visibility_for_client(config, client_token))
            .collect()
    }
}

fn model_visibility_for_client(
    config: &ResolvedConfig,
    client_token: &ResolvedClientToken,
) -> ModelVisibilityProjection {
    let visible_models = if client_token.enabled {
        visible_models_for_client(config, client_token)
    } else {
        Vec::new()
    };
    let reason_code = if !client_token.enabled {
        "client_token_disabled"
    } else if !visible_models.is_empty() {
        "models_visible"
    } else if config.model_routes.is_empty() {
        "model_route_missing"
    } else if !client_token.allowed_model_groups.is_empty() {
        "client_scope_empty"
    } else {
        "target_channel_disabled"
    };

    ModelVisibilityProjection {
        client_token_ref: client_token.name.clone(),
        visible_models,
        reason_code: reason_code.to_string(),
    }
}

fn visible_models_for_client(
    config: &ResolvedConfig,
    client_token: &ResolvedClientToken,
) -> Vec<String> {
    let mut visible = BTreeSet::new();
    for (public_model, route) in &config.model_routes {
        if !client_model_allowed(
            &config.model_groups,
            &client_token.allowed_model_groups,
            public_model,
        ) {
            continue;
        }
        if route.targets.iter().any(|target| {
            if !target.enabled {
                return false;
            }
            if !client_token.allowed_channels.is_empty()
                && !client_token
                    .allowed_channels
                    .iter()
                    .any(|allowed| allowed == &target.channel_id.0)
            {
                return false;
            }
            config.pools.get(&target.channel_id.0).is_some_and(|pool| {
                pool.configured_enabled
                    && pool.account_enabled
                    && ProviderAdapter::new(pool.provider_kind)
                        .model_catalog_capability()
                        .is_some()
            })
        }) {
            visible.insert(public_model.clone());
        }
    }
    visible.into_iter().collect()
}

fn client_model_allowed(
    model_groups: &std::collections::HashMap<String, ResolvedModelGroup>,
    allowed_model_groups: &[String],
    model: &str,
) -> bool {
    if allowed_model_groups.is_empty() {
        return true;
    }
    allowed_model_groups.iter().any(|allowed| {
        allowed == model
            || model_groups
                .get(allowed)
                .is_some_and(|group| group.models.iter().any(|member| member == model))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry_store::RegistryStoreHandle;
    use crate::{credential_repository::FileCredentialRepository, registry::RegistryRepository};
    use std::{
        env, fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("one-ai-key-control-plane-{suffix}"))
    }

    fn write_config(root: &Path, body: &str) -> PathBuf {
        fs::create_dir_all(root).unwrap();
        let path = root.join("config.yaml");
        fs::write(&path, body).unwrap();
        path
    }

    fn representative_config(keys_file: &Path) -> String {
        format!(
            r#"
listen: 127.0.0.1:4101
client_tokens:
  - name: local-client
    token: secret-client-token
management:
  admin_token: secret-management-token
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
response_filter:
  enabled: true
  replacement: "[filtered]"
  rules:
    - id: sample
      kind: literal
      value: unsafe-marker
      action: redact
model_groups:
  coding:
    models:
      - gpt-example
upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: https://relay.example/v1
    credential_set: relay_credentials
    keys_file: {}
    models:
      - public_model: gpt-example
        upstream_model: provider/gpt-example
"#,
            serde_yaml::to_string(&keys_file.to_string_lossy().to_string())
                .unwrap()
                .trim()
        )
    }

    #[test]
    fn config_source_yaml_matches_legacy_repository_loader() {
        let root = unique_temp_root();
        let keys = root.join("relay.keys");
        fs::create_dir_all(&root).unwrap();
        fs::write(&keys, "synthetic-upstream-key\n").unwrap();
        let config = write_config(&root, &representative_config(&keys));

        let source_document = ConfigSource::yaml_file(&config)
            .load_registry_document()
            .unwrap();
        let legacy_document = crate::registry::YamlRegistryRepository::new(&config)
            .load_registry()
            .unwrap();

        assert_eq!(
            format!("{source_document:?}"),
            format!("{legacy_document:?}")
        );
        assert!(source_document.response_filter.enabled);
        assert!(source_document.model_groups.contains_key("coding"));
    }

    #[test]
    fn config_compiler_matches_registry_document_resolver() {
        let root = unique_temp_root();
        let keys = root.join("relay.keys");
        fs::create_dir_all(&root).unwrap();
        fs::write(&keys, "synthetic-upstream-key\n").unwrap();
        let config = write_config(&root, &representative_config(&keys));
        let document = ConfigSource::yaml_file(&config)
            .load_registry_document()
            .unwrap();
        let repository = FileCredentialRepository::new();

        let compiled =
            ConfigCompiler::compile_with_credential_repository(document.clone(), &repository, None)
                .unwrap();
        let resolved = document
            .resolve_with_credential_repository_and_store_path(&repository, None)
            .unwrap();

        assert_eq!(compiled.model_routes.len(), resolved.model_routes.len());
        assert_eq!(compiled.pools.len(), resolved.pools.len());
        assert_eq!(
            compiled.response_filter_event_window_capacity,
            resolved.response_filter_event_window_capacity
        );
        assert_eq!(
            compiled.pools.get("relay").unwrap().config_generation,
            resolved.pools.get("relay").unwrap().config_generation
        );
    }

    #[test]
    fn registry_overlay_matches_existing_resource_overlay_semantics() {
        let root = unique_temp_root();
        let bootstrap_keys = root.join("bootstrap.keys");
        let staged_keys = root.join("staged.keys");
        fs::create_dir_all(&root).unwrap();
        fs::write(&bootstrap_keys, "bootstrap-key\n").unwrap();
        fs::write(&staged_keys, "staged-key\n").unwrap();
        let bootstrap_config = write_config(
            &root.join("bootstrap"),
            &representative_config(&bootstrap_keys),
        );
        let staged_config = write_config(
            &root.join("staged"),
            &representative_config(&staged_keys).replace("gpt-example", "gpt-staged"),
        );
        let bootstrap = ConfigSource::yaml_file(&bootstrap_config)
            .load_registry_document()
            .unwrap();
        let staged = ConfigSource::yaml_file(&staged_config)
            .load_registry_document()
            .unwrap();

        let overlay = RegistryOverlay::overlay_staged_resources(bootstrap.clone(), staged.clone());
        let existing = crate::registry_store::overlay_registry_resources(bootstrap.clone(), staged);

        assert_eq!(format!("{overlay:?}"), format!("{existing:?}"));
        assert_eq!(overlay.listen, bootstrap.listen);
        assert_eq!(
            overlay.client_tokens[0].name,
            bootstrap.client_tokens[0].name
        );
        assert!(overlay.model_routes.contains_key("gpt-staged"));
        assert!(!overlay.model_routes.contains_key("gpt-example"));
    }

    #[test]
    fn runtime_assembler_covers_startup_and_replacement_boundary() {
        let root = unique_temp_root();
        let keys = root.join("relay.keys");
        fs::create_dir_all(&root).unwrap();
        fs::write(&keys, "synthetic-upstream-key\n").unwrap();
        let config = write_config(&root, &representative_config(&keys));
        let document = ConfigSource::yaml_file(&config)
            .load_registry_document()
            .unwrap();
        let repository = FileCredentialRepository::new();
        let startup_config =
            ConfigCompiler::compile_with_credential_repository(document.clone(), &repository, None)
                .unwrap();
        let replacement_config =
            ConfigCompiler::compile_with_credential_repository(document.clone(), &repository, None)
                .unwrap();
        let state = RuntimeAssembler::assemble_startup(
            startup_config,
            RegistryStoreHandle::read_only(),
            Some(document),
        )
        .unwrap();

        let initial_generation = state.channels.registry_generation();
        assert_eq!(
            state.runtime_catalogs.registry_generation(),
            initial_generation
        );

        RuntimeAssembler::replace_runtime(&state, replacement_config, Some(7)).unwrap();

        let replacement_generation = state.channels.registry_generation();
        assert!(replacement_generation > initial_generation);
        assert_eq!(
            state.runtime_catalogs.registry_generation(),
            replacement_generation
        );
    }
}
