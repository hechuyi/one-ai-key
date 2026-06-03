use std::{collections::HashMap, fs, net::SocketAddr, path::PathBuf};

use anyhow::Context;
use serde::Deserialize;

use crate::config::{
    reject_unknown_top_level_config_fields, AccountConfig, AppConfig, ClientTokenConfig,
    CredentialSetConfig, ManagementConfig, ModelGroupConfig, ModelRouteConfig, PolicyProfileConfig,
    PoolConfig, ProviderConfig, ResolvedConfig, ResponseFilterConfig, RoutingConfig,
    RoutingProfileConfig, TimeoutConfig,
};
use crate::credential_repository::{
    CredentialRepository, FileCredentialRepository, SqliteCredentialRepository,
};

pub trait RegistryRepository {
    fn load_registry(&self) -> anyhow::Result<RegistryDocument>;
}

#[derive(Debug, Clone)]
pub struct YamlRegistryRepository {
    path: PathBuf,
}

impl YamlRegistryRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl RegistryRepository for YamlRegistryRepository {
    fn load_registry(&self) -> anyhow::Result<RegistryDocument> {
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

#[derive(Debug, Deserialize)]
struct RegistryYamlDocument {
    #[serde(flatten)]
    app: AppConfig,
    #[serde(default)]
    model_groups: HashMap<String, ModelGroupConfig>,
    #[serde(default)]
    response_filter: ResponseFilterConfig,
}

#[derive(Clone)]
pub struct RegistryDocument {
    pub listen: SocketAddr,
    pub client_tokens: Vec<ClientTokenConfig>,
    pub management: Option<ManagementConfig>,
    pub max_request_body_bytes: usize,
    pub max_model_catalog_body_bytes: usize,
    pub max_error_body_bytes: usize,
    pub timeouts: TimeoutConfig,
    pub routing: RoutingConfig,
    pub response_filter: ResponseFilterConfig,
    pub default_pool: Option<String>,
    pub providers: HashMap<String, ProviderConfig>,
    pub accounts: HashMap<String, AccountConfig>,
    pub credential_sets: HashMap<String, CredentialSetConfig>,
    pub model_groups: HashMap<String, ModelGroupConfig>,
    pub policy_profiles: HashMap<String, PolicyProfileConfig>,
    pub default_routing_profile: Option<String>,
    pub routing_profiles: HashMap<String, RoutingProfileConfig>,
    pub model_routes: HashMap<String, ModelRouteConfig>,
    pub pools: HashMap<String, PoolConfig>,
}

#[derive(Clone, Debug)]
pub struct RegistryResources {
    pub default_pool: Option<String>,
    pub providers: HashMap<String, ProviderConfig>,
    pub accounts: HashMap<String, AccountConfig>,
    pub credential_sets: HashMap<String, CredentialSetConfig>,
    pub policy_profiles: HashMap<String, PolicyProfileConfig>,
    pub default_routing_profile: Option<String>,
    pub routing_profiles: HashMap<String, RoutingProfileConfig>,
    pub model_routes: HashMap<String, ModelRouteConfig>,
    pub pools: HashMap<String, PoolConfig>,
}

impl std::fmt::Debug for RegistryDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut provider_ids: Vec<&str> = self.providers.keys().map(String::as_str).collect();
        let mut account_ids: Vec<&str> = self.accounts.keys().map(String::as_str).collect();
        let mut credential_set_ids: Vec<&str> =
            self.credential_sets.keys().map(String::as_str).collect();
        let mut policy_profile_ids: Vec<&str> =
            self.policy_profiles.keys().map(String::as_str).collect();
        let mut routing_profile_ids: Vec<&str> =
            self.routing_profiles.keys().map(String::as_str).collect();
        let mut model_group_ids: Vec<&str> = self.model_groups.keys().map(String::as_str).collect();
        let mut model_route_ids: Vec<&str> = self.model_routes.keys().map(String::as_str).collect();
        let mut pool_ids: Vec<&str> = self.pools.keys().map(String::as_str).collect();

        provider_ids.sort_unstable();
        account_ids.sort_unstable();
        credential_set_ids.sort_unstable();
        policy_profile_ids.sort_unstable();
        routing_profile_ids.sort_unstable();
        model_group_ids.sort_unstable();
        model_route_ids.sort_unstable();
        pool_ids.sort_unstable();

        f.debug_struct("RegistryDocument")
            .field("listen", &self.listen)
            .field("client_token_count", &self.client_tokens.len())
            .field("management_configured", &self.management.is_some())
            .field("max_request_body_bytes", &self.max_request_body_bytes)
            .field(
                "max_model_catalog_body_bytes",
                &self.max_model_catalog_body_bytes,
            )
            .field("max_error_body_bytes", &self.max_error_body_bytes)
            .field("timeouts", &self.timeouts)
            .field("routing", &self.routing)
            .field("response_filter_enabled", &self.response_filter.enabled)
            .field("default_pool", &self.default_pool)
            .field("provider_ids", &provider_ids)
            .field("account_ids", &account_ids)
            .field("credential_set_ids", &credential_set_ids)
            .field("policy_profile_ids", &policy_profile_ids)
            .field("default_routing_profile", &self.default_routing_profile)
            .field("routing_profile_ids", &routing_profile_ids)
            .field("model_group_ids", &model_group_ids)
            .field("model_route_ids", &model_route_ids)
            .field("pool_ids", &pool_ids)
            .finish()
    }
}

impl RegistryDocument {
    pub fn into_resources(self) -> RegistryResources {
        RegistryResources {
            default_pool: self.default_pool,
            providers: self.providers,
            accounts: self.accounts,
            credential_sets: self.credential_sets,
            policy_profiles: self.policy_profiles,
            default_routing_profile: self.default_routing_profile,
            routing_profiles: self.routing_profiles,
            model_routes: self.model_routes,
            pools: self.pools,
        }
    }

    pub fn apply_resources(&mut self, resources: RegistryResources) {
        self.default_pool = resources.default_pool;
        self.providers = resources.providers;
        self.accounts = resources.accounts;
        self.credential_sets = resources.credential_sets;
        self.policy_profiles = resources.policy_profiles;
        self.default_routing_profile = resources.default_routing_profile;
        self.routing_profiles = resources.routing_profiles;
        self.model_routes = resources.model_routes;
        self.pools = resources.pools;
    }

    pub fn resolve(self) -> anyhow::Result<ResolvedConfig> {
        if let Some(database_path) = std::env::var_os("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE") {
            let database_path = PathBuf::from(database_path);
            let repository = SqliteCredentialRepository::open(database_path.clone())?;
            return self.resolve_with_credential_repository_and_store_path(
                &repository,
                Some(database_path),
            );
        }

        let repository = FileCredentialRepository::new();
        self.resolve_with_credential_repository_and_store_path(&repository, None)
    }

    // Kept for tests and explicit repository injection; production resolve chooses the repository.
    #[allow(dead_code)]
    pub fn resolve_with_credential_repository(
        self,
        credential_repository: &impl CredentialRepository,
    ) -> anyhow::Result<ResolvedConfig> {
        self.resolve_with_credential_repository_and_store_path(credential_repository, None)
    }

    pub fn resolve_with_credential_repository_and_store_path(
        self,
        credential_repository: &impl CredentialRepository,
        credential_store_path: Option<PathBuf>,
    ) -> anyhow::Result<ResolvedConfig> {
        crate::config::resolve_registry_document_with_credential_repository_and_store_path(
            self,
            credential_repository,
            credential_store_path,
        )
    }

    pub(crate) fn into_app_config_for_legacy_resolver(self) -> AppConfig {
        AppConfig {
            listen: self.listen,
            client_tokens: self.client_tokens,
            management: self.management,
            max_request_body_bytes: self.max_request_body_bytes,
            max_model_catalog_body_bytes: self.max_model_catalog_body_bytes,
            max_error_body_bytes: self.max_error_body_bytes,
            timeouts: self.timeouts,
            routing: self.routing,
            default_pool: self.default_pool,
            providers: self.providers,
            accounts: self.accounts,
            credential_sets: self.credential_sets,
            policy_profiles: self.policy_profiles,
            default_routing_profile: self.default_routing_profile,
            routing_profiles: self.routing_profiles,
            model_routes: self.model_routes,
            pools: self.pools,
        }
    }
}

impl RegistryResources {
    pub fn into_document_for_validation(self) -> anyhow::Result<RegistryDocument> {
        Ok(RegistryDocument {
            listen: "127.0.0.1:0".parse()?,
            client_tokens: Vec::new(),
            management: None,
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: TimeoutConfig::default(),
            routing: Default::default(),
            response_filter: Default::default(),
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
        })
    }
}
