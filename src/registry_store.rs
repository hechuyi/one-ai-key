#![allow(dead_code)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::registry::{RegistryDocument, RegistryResources};
use rusqlite::{params, Connection, TransactionBehavior};

use crate::{
    config::{
        AccountConfig, CredentialSetConfig, ErrorRulesConfig, ModelRouteConfig,
        ModelRouteTargetConfig, PolicyProfileConfig, PoolConfig, ProviderConfig,
        RoutingProfileConfig,
    },
    provider::ProviderKind,
};

#[derive(Debug, Clone)]
pub enum RegistryStoreHandle {
    ReadOnly,
    Sqlite(SqliteRegistryStore),
}

impl RegistryStoreHandle {
    pub fn read_only() -> Self {
        Self::ReadOnly
    }

    pub fn sqlite(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        Ok(Self::Sqlite(SqliteRegistryStore::open(path)?))
    }

    pub async fn apply_command(
        &self,
        command: RegistryCommand,
        validate: impl Fn(&RegistryDocument) -> Result<(), RegistryStoreError> + Send + Sync + 'static,
    ) -> Result<RegistryStoreCommit, RegistryStoreError> {
        match self {
            Self::ReadOnly => Err(RegistryStoreError::NotWritable),
            Self::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || store.apply_command(command, &validate))
                    .await
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
            }
        }
    }

    pub async fn current_version(&self) -> Result<Option<u64>, RegistryStoreError> {
        match self {
            Self::ReadOnly => Ok(None),
            Self::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || store.current_version().map(Some))
                    .await
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
            }
        }
    }

    pub fn current_version_for_startup(&self) -> Result<Option<u64>, RegistryStoreError> {
        match self {
            Self::ReadOnly => Ok(None),
            Self::Sqlite(store) => store.current_version().map(Some),
        }
    }

    pub async fn load_registry_for_validation(
        &self,
    ) -> Result<Option<RegistryDocument>, RegistryStoreError> {
        match self {
            Self::ReadOnly => Ok(None),
            Self::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || {
                    store
                        .load_registry_for_validation()
                        .map(Some)
                        .map_err(|err| RegistryStoreError::Persistence(err.to_string()))
                })
                .await
                .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
            }
        }
    }
}

pub trait RegistryStore {
    /// Loads persisted, non-secret registry resources as a resolver input.
    ///
    /// The returned document is not process bootstrap configuration: client
    /// tokens, management credentials, listen address, and secret material live
    /// in their own stores or startup config.
    fn load_registry_for_validation(&self) -> anyhow::Result<RegistryDocument>;

    fn apply_command(
        &self,
        command: RegistryCommand,
        validate: &dyn Fn(&RegistryDocument) -> Result<(), RegistryStoreError>,
    ) -> Result<RegistryStoreCommit, RegistryStoreError>;
}

#[derive(Debug, Clone)]
pub struct RegistryStoreCommit {
    pub registry_version: u64,
    pub document_for_validation: RegistryDocument,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryStoreError {
    NotWritable,
    Conflict(String),
    Validation(String),
    Persistence(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryCommand {
    Provider(ProviderRegistryCommand),
    Account(AccountRegistryCommand),
    Channel(ChannelRegistryCommand),
    ModelRoute(ModelRouteRegistryCommand),
    PolicyProfile(PolicyProfileRegistryCommand),
    RoutingProfile(RoutingProfileRegistryCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRegistryCommand {
    SetEnabled {
        provider_id: String,
        enabled: bool,
    },
    Upsert {
        provider_id: String,
        provider: ProviderConfig,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountRegistryCommand {
    SetEnabled {
        account_id: String,
        enabled: bool,
    },
    Upsert {
        account_id: String,
        account: AccountConfig,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelRegistryCommand {
    SetEnabled {
        channel_id: String,
        enabled: bool,
    },
    Upsert {
        channel_id: String,
        channel: Box<PoolConfig>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRouteRegistryCommand {
    Upsert {
        public_model: String,
        route: ModelRouteConfig,
    },
    UpsertBatch {
        expected_registry_version: Option<u64>,
        routes: Vec<(String, ModelRouteConfig)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyProfileRegistryCommand {
    Upsert {
        profile_id: String,
        profile: PolicyProfileConfig,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingProfileRegistryCommand {
    Upsert {
        profile_id: String,
        profile: RoutingProfileConfig,
    },
}

#[derive(Debug, Clone)]
pub struct ReadOnlyRegistryStore {
    document: RegistryDocument,
}

impl ReadOnlyRegistryStore {
    pub fn new(document: RegistryDocument) -> Self {
        Self { document }
    }
}

impl RegistryStore for ReadOnlyRegistryStore {
    fn load_registry_for_validation(&self) -> anyhow::Result<RegistryDocument> {
        Ok(self.document.clone())
    }

    fn apply_command(
        &self,
        _command: RegistryCommand,
        _validate: &dyn Fn(&RegistryDocument) -> Result<(), RegistryStoreError>,
    ) -> Result<RegistryStoreCommit, RegistryStoreError> {
        Err(RegistryStoreError::NotWritable)
    }
}

#[derive(Debug, Clone)]
pub struct SqliteRegistryStore {
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RegistryStartupDocument {
    pub document: RegistryDocument,
    pub store: RegistryStoreHandle,
}

impl SqliteRegistryStore {
    pub fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let store = Self { path: path.into() };
        let connection = store.connect()?;
        initialize_schema(&connection)?;
        Ok(store)
    }

    pub fn bootstrap_from_document(
        path: impl AsRef<Path>,
        document: RegistryDocument,
    ) -> anyhow::Result<()> {
        validate_store_supported_registry_document(&document)?;
        let store = Self::open(path.as_ref().to_path_buf())?;
        let mut connection = store.connect()?;
        let tx = connection.transaction()?;
        let existing_versions: i64 =
            tx.query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })?;
        anyhow::ensure!(
            existing_versions == 0,
            "registry store already contains resource rows"
        );
        insert_registry_version(&tx, 1)?;
        replace_document_rows(&tx, &document, 1)?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_or_bootstrap_startup_document(
        path: impl AsRef<Path>,
        bootstrap: RegistryDocument,
    ) -> anyhow::Result<RegistryStartupDocument> {
        let store = Self::open(path.as_ref().to_path_buf())?;
        if store.is_empty()? {
            validate_store_supported_registry_document(&bootstrap)?;
            let mut connection = store.connect()?;
            let tx = connection.transaction()?;
            insert_registry_version(&tx, 1)?;
            replace_document_rows(&tx, &bootstrap, 1)?;
            tx.commit()?;
            return Ok(RegistryStartupDocument {
                document: bootstrap,
                store: RegistryStoreHandle::Sqlite(store),
            });
        }

        let stored_resources = store.load_registry_resources()?;
        Ok(RegistryStartupDocument {
            document: overlay_registry_loaded_resources(bootstrap, stored_resources),
            store: RegistryStoreHandle::Sqlite(store),
        })
    }

    fn connect(&self) -> anyhow::Result<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(connection)
    }

    fn is_empty(&self) -> anyhow::Result<bool> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        let existing_versions: i64 =
            connection.query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })?;
        Ok(existing_versions == 0)
    }

    fn current_version(&self) -> Result<u64, RegistryStoreError> {
        let connection = self
            .connect()
            .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
        initialize_schema(&connection)
            .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
        current_registry_version(&connection)
            .map_err(|err| RegistryStoreError::Persistence(err.to_string()))
    }

    fn load_registry_resources(&self) -> anyhow::Result<RegistryResources> {
        let connection = self.connect()?;
        initialize_schema(&connection)?;
        load_registry_resources(&connection)
    }
}

impl RegistryStore for SqliteRegistryStore {
    fn load_registry_for_validation(&self) -> anyhow::Result<RegistryDocument> {
        self.load_registry_resources()?
            .into_document_for_validation()
    }

    fn apply_command(
        &self,
        command: RegistryCommand,
        validate: &dyn Fn(&RegistryDocument) -> Result<(), RegistryStoreError>,
    ) -> Result<RegistryStoreCommit, RegistryStoreError> {
        let mut connection = self
            .connect()
            .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
        initialize_schema(&connection)
            .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
        let mut document = load_registry_resources(&tx)
            .and_then(RegistryResources::into_document_for_validation)
            .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
        match command {
            RegistryCommand::Provider(ProviderRegistryCommand::SetEnabled {
                provider_id,
                enabled,
            }) => {
                let provider = document.providers.get_mut(&provider_id).ok_or_else(|| {
                    RegistryStoreError::Conflict(format!("unknown provider {provider_id}"))
                })?;
                provider.enabled = enabled;
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.execute(
                    "UPDATE providers SET enabled = ?2, version_id = ?3 WHERE id = ?1",
                    params![provider_id, enabled as i64, next_version as i64],
                )
                .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::Provider(ProviderRegistryCommand::Upsert {
                provider_id,
                provider,
            }) => {
                document
                    .providers
                    .insert(provider_id.clone(), provider.clone());
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                upsert_provider(&tx, &provider_id, &provider, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::Account(AccountRegistryCommand::SetEnabled {
                account_id,
                enabled,
            }) => {
                let account = document.accounts.get_mut(&account_id).ok_or_else(|| {
                    RegistryStoreError::Conflict(format!("unknown account {account_id}"))
                })?;
                account.enabled = enabled;
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.execute(
                    "UPDATE accounts SET enabled = ?2, version_id = ?3 WHERE id = ?1",
                    params![account_id, enabled as i64, next_version as i64],
                )
                .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::Account(AccountRegistryCommand::Upsert {
                account_id,
                account,
            }) => {
                document
                    .accounts
                    .insert(account_id.clone(), account.clone());
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                upsert_account(&tx, &account_id, &account, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::Channel(ChannelRegistryCommand::SetEnabled {
                channel_id,
                enabled,
            }) => {
                let channel = document.pools.get_mut(&channel_id).ok_or_else(|| {
                    RegistryStoreError::Conflict(format!("unknown channel {channel_id}"))
                })?;
                channel.enabled = enabled;
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.execute(
                    "UPDATE channels SET enabled = ?2, version_id = ?3 WHERE id = ?1",
                    params![channel_id, enabled as i64, next_version as i64],
                )
                .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::Channel(ChannelRegistryCommand::Upsert {
                channel_id,
                channel,
            }) => {
                let channel = *channel;
                document.pools.insert(channel_id.clone(), channel.clone());
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                upsert_channel(&tx, &channel_id, &channel, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::ModelRoute(ModelRouteRegistryCommand::Upsert {
                public_model,
                route,
            }) => {
                document
                    .model_routes
                    .insert(public_model.clone(), route.clone());
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                upsert_model_route(&tx, &public_model, &route, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::ModelRoute(ModelRouteRegistryCommand::UpsertBatch {
                expected_registry_version,
                routes,
            }) => {
                let current_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                if expected_registry_version.is_some_and(|expected| expected != current_version) {
                    return Err(RegistryStoreError::Conflict(
                        "registry version changed during model discovery sync apply".to_string(),
                    ));
                }
                for (public_model, route) in &routes {
                    document
                        .model_routes
                        .insert(public_model.clone(), route.clone());
                }
                validate(&document)?;
                let next_version = current_version + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                for (public_model, route) in &routes {
                    upsert_model_route(&tx, public_model, route, next_version)
                        .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                }
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::PolicyProfile(PolicyProfileRegistryCommand::Upsert {
                profile_id,
                profile,
            }) => {
                document
                    .policy_profiles
                    .insert(profile_id.clone(), profile.clone());
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                upsert_policy_profile(&tx, &profile_id, &profile, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
            RegistryCommand::RoutingProfile(RoutingProfileRegistryCommand::Upsert {
                profile_id,
                profile,
            }) => {
                document
                    .routing_profiles
                    .insert(profile_id.clone(), profile.clone());
                validate(&document)?;
                let next_version = current_registry_version(&tx)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?
                    + 1;
                insert_registry_version(&tx, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                upsert_routing_profile(&tx, &profile_id, &profile, next_version)
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                tx.commit()
                    .map_err(|err| RegistryStoreError::Persistence(err.to_string()))?;
                Ok(RegistryStoreCommit {
                    registry_version: next_version,
                    document_for_validation: document,
                })
            }
        }
    }
}

fn initialize_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS registry_versions (
            id INTEGER PRIMARY KEY,
            created_at_unix_seconds INTEGER NOT NULL,
            actor_id TEXT,
            reason TEXT
        );
        CREATE TABLE IF NOT EXISTS providers (
            id TEXT PRIMARY KEY,
            provider_kind TEXT NOT NULL,
            enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
            version_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS accounts (
            id TEXT PRIMARY KEY,
            provider_id TEXT NOT NULL,
            api_base TEXT NOT NULL,
            auth_header TEXT NOT NULL,
            auth_prefix TEXT NOT NULL,
            enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
            version_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS credential_sets (
            id TEXT PRIMARY KEY,
            source_kind TEXT NOT NULL,
            source_ref TEXT NOT NULL,
            version_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS channels (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL,
            credential_set_id TEXT NOT NULL,
            policy_profile_id TEXT,
            routing_profile_id TEXT,
            error_rules_json TEXT NOT NULL,
            enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
            version_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS model_routes (
            public_model TEXT PRIMARY KEY,
            strategy TEXT,
            version_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS model_route_targets (
            public_model TEXT NOT NULL,
            position INTEGER NOT NULL,
            channel_id TEXT NOT NULL,
            upstream_model TEXT,
            priority INTEGER NOT NULL CHECK (priority BETWEEN 0 AND 65535),
            weight INTEGER NOT NULL CHECK (weight BETWEEN 0 AND 65535),
            enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
            version_id INTEGER NOT NULL,
            PRIMARY KEY(public_model, position)
        );
        CREATE TABLE IF NOT EXISTS policy_profiles (
            id TEXT PRIMARY KEY,
            error_rules_json TEXT NOT NULL,
            version_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS routing_profiles (
            id TEXT PRIMARY KEY,
            routing_profile_json TEXT NOT NULL,
            version_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS registry_defaults (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            default_pool TEXT,
            default_routing_profile TEXT,
            version_id INTEGER NOT NULL
        );
        ",
    )
}

fn insert_registry_version(connection: &Connection, version: u64) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO registry_versions (id, created_at_unix_seconds, actor_id, reason)
         VALUES (?1, ?2, NULL, NULL)",
        params![version as i64, now_unix_seconds()?],
    )?;
    Ok(())
}

fn current_registry_version(connection: &Connection) -> anyhow::Result<u64> {
    let version: Option<i64> =
        connection.query_row("SELECT MAX(id) FROM registry_versions", [], |row| {
            row.get(0)
        })?;
    Ok(version.unwrap_or(0) as u64)
}

fn replace_document_rows(
    connection: &Connection,
    document: &RegistryDocument,
    version: u64,
) -> anyhow::Result<()> {
    let version = version as i64;
    for table in [
        "model_route_targets",
        "model_routes",
        "channels",
        "routing_profiles",
        "policy_profiles",
        "credential_sets",
        "accounts",
        "providers",
        "registry_defaults",
    ] {
        connection.execute(&format!("DELETE FROM {table}"), [])?;
    }
    connection.execute(
        "INSERT INTO registry_defaults (
            id, default_pool, default_routing_profile, version_id
         ) VALUES (1, ?1, ?2, ?3)",
        params![
            document.default_pool.as_deref(),
            document.default_routing_profile.as_deref(),
            version
        ],
    )?;

    for (id, provider) in &document.providers {
        connection.execute(
            "INSERT INTO providers (id, provider_kind, enabled, version_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                provider_kind_to_str(provider.provider_kind),
                provider.enabled as i64,
                version
            ],
        )?;
    }
    for (id, account) in &document.accounts {
        connection.execute(
            "INSERT INTO accounts (
                id, provider_id, api_base, auth_header, auth_prefix, enabled, version_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                &account.provider,
                &account.api_base,
                &account.auth_header,
                &account.auth_prefix,
                account.enabled as i64,
                version
            ],
        )?;
    }
    for (id, credential_set) in &document.credential_sets {
        connection.execute(
            "INSERT INTO credential_sets (id, source_kind, source_ref, version_id)
             VALUES (?1, 'file', ?2, ?3)",
            params![id, credential_set.keys_file.to_string_lossy(), version],
        )?;
    }
    for (id, profile) in &document.policy_profiles {
        connection.execute(
            "INSERT INTO policy_profiles (id, error_rules_json, version_id)
             VALUES (?1, ?2, ?3)",
            params![id, serde_json::to_string(profile)?, version],
        )?;
    }
    for (id, profile) in &document.routing_profiles {
        connection.execute(
            "INSERT INTO routing_profiles (id, routing_profile_json, version_id)
             VALUES (?1, ?2, ?3)",
            params![id, serde_json::to_string(profile)?, version],
        )?;
    }
    for (id, channel) in &document.pools {
        connection.execute(
            "INSERT INTO channels (
                id, account_id, credential_set_id, policy_profile_id, routing_profile_id,
                error_rules_json, enabled, version_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                channel.account.as_deref().unwrap_or(""),
                &channel.credential_set,
                channel.policy_profile.as_deref(),
                channel.routing_profile.as_deref(),
                serde_json::to_string(&channel.error_rules)?,
                channel.enabled as i64,
                version
            ],
        )?;
    }
    for (public_model, route) in &document.model_routes {
        connection.execute(
            "INSERT INTO model_routes (public_model, strategy, version_id)
             VALUES (?1, ?2, ?3)",
            params![public_model, route.strategy.as_deref(), version],
        )?;
        for (position, target) in route.targets.iter().enumerate() {
            connection.execute(
                "INSERT INTO model_route_targets (
                    public_model, position, channel_id, upstream_model, priority, weight,
                    enabled, version_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    public_model,
                    position as i64,
                    &target.channel,
                    target.upstream_model.as_deref(),
                    target.priority as i64,
                    target.weight as i64,
                    target.enabled as i64,
                    version
                ],
            )?;
        }
    }
    Ok(())
}

fn upsert_provider(
    connection: &Connection,
    provider_id: &str,
    provider: &ProviderConfig,
    version: u64,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO providers (id, provider_kind, enabled, version_id)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            provider_kind = excluded.provider_kind,
            enabled = excluded.enabled,
            version_id = excluded.version_id",
        params![
            provider_id,
            provider_kind_to_str(provider.provider_kind),
            provider.enabled as i64,
            version as i64
        ],
    )?;
    Ok(())
}

fn upsert_account(
    connection: &Connection,
    account_id: &str,
    account: &AccountConfig,
    version: u64,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO accounts (
            id, provider_id, api_base, auth_header, auth_prefix, enabled, version_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            provider_id = excluded.provider_id,
            api_base = excluded.api_base,
            auth_header = excluded.auth_header,
            auth_prefix = excluded.auth_prefix,
            enabled = excluded.enabled,
            version_id = excluded.version_id",
        params![
            account_id,
            &account.provider,
            &account.api_base,
            &account.auth_header,
            &account.auth_prefix,
            account.enabled as i64,
            version as i64
        ],
    )?;
    Ok(())
}

fn upsert_channel(
    connection: &Connection,
    channel_id: &str,
    channel: &PoolConfig,
    version: u64,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO channels (
            id, account_id, credential_set_id, policy_profile_id, routing_profile_id,
            error_rules_json, enabled, version_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            account_id = excluded.account_id,
            credential_set_id = excluded.credential_set_id,
            policy_profile_id = excluded.policy_profile_id,
            routing_profile_id = excluded.routing_profile_id,
            error_rules_json = excluded.error_rules_json,
            enabled = excluded.enabled,
            version_id = excluded.version_id",
        params![
            channel_id,
            channel.account.as_deref().unwrap_or(""),
            &channel.credential_set,
            channel.policy_profile.as_deref(),
            channel.routing_profile.as_deref(),
            serde_json::to_string(&channel.error_rules)?,
            channel.enabled as i64,
            version as i64
        ],
    )?;
    Ok(())
}

fn upsert_model_route(
    connection: &Connection,
    public_model: &str,
    route: &ModelRouteConfig,
    version: u64,
) -> anyhow::Result<()> {
    let version = version as i64;
    connection.execute(
        "DELETE FROM model_route_targets WHERE public_model = ?1",
        params![public_model],
    )?;
    connection.execute(
        "INSERT INTO model_routes (public_model, strategy, version_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(public_model) DO UPDATE SET
            strategy = excluded.strategy,
            version_id = excluded.version_id",
        params![public_model, route.strategy.as_deref(), version],
    )?;
    for (position, target) in route.targets.iter().enumerate() {
        connection.execute(
            "INSERT INTO model_route_targets (
                public_model, position, channel_id, upstream_model, priority, weight,
                enabled, version_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                public_model,
                position as i64,
                &target.channel,
                target.upstream_model.as_deref(),
                target.priority as i64,
                target.weight as i64,
                target.enabled as i64,
                version
            ],
        )?;
    }
    Ok(())
}

fn upsert_policy_profile(
    connection: &Connection,
    profile_id: &str,
    profile: &PolicyProfileConfig,
    version: u64,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO policy_profiles (id, error_rules_json, version_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
            error_rules_json = excluded.error_rules_json,
            version_id = excluded.version_id",
        params![profile_id, serde_json::to_string(profile)?, version as i64],
    )?;
    Ok(())
}

fn upsert_routing_profile(
    connection: &Connection,
    profile_id: &str,
    profile: &RoutingProfileConfig,
    version: u64,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO routing_profiles (id, routing_profile_json, version_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
            routing_profile_json = excluded.routing_profile_json,
            version_id = excluded.version_id",
        params![profile_id, serde_json::to_string(profile)?, version as i64],
    )?;
    Ok(())
}

fn load_registry_resources(connection: &Connection) -> anyhow::Result<RegistryResources> {
    let defaults = load_registry_defaults(connection)?;

    let mut providers = HashMap::new();
    let mut provider_rows =
        connection.prepare("SELECT id, provider_kind, enabled FROM providers ORDER BY id")?;
    let provider_iter = provider_rows.query_map([], |row| {
        let kind: String = row.get(1)?;
        Ok((
            row.get::<_, String>(0)?,
            ProviderConfig {
                provider_kind: provider_kind_from_str(&kind).map_err(|message| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        message.into(),
                    )
                })?,
                enabled: row.get::<_, i64>(2)? != 0,
            },
        ))
    })?;
    for row in provider_iter {
        let (id, provider) = row?;
        providers.insert(id, provider);
    }

    let mut accounts = HashMap::new();
    let mut account_rows = connection.prepare(
        "SELECT id, provider_id, api_base, auth_header, auth_prefix, enabled
         FROM accounts ORDER BY id",
    )?;
    let account_iter = account_rows.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            AccountConfig {
                provider: row.get(1)?,
                api_base: row.get(2)?,
                auth_header: row.get(3)?,
                auth_prefix: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
            },
        ))
    })?;
    for row in account_iter {
        let (id, account) = row?;
        accounts.insert(id, account);
    }

    let mut credential_sets = HashMap::new();
    let mut credential_set_rows = connection
        .prepare("SELECT id, source_kind, source_ref FROM credential_sets ORDER BY id")?;
    let credential_set_iter = credential_set_rows.query_map([], |row| {
        let source_kind: String = row.get(1)?;
        if source_kind != "file" {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok((
            row.get::<_, String>(0)?,
            CredentialSetConfig {
                keys_file: PathBuf::from(row.get::<_, String>(2)?),
            },
        ))
    })?;
    for row in credential_set_iter {
        let (id, credential_set) = row?;
        credential_sets.insert(id, credential_set);
    }

    let mut policy_profiles = HashMap::new();
    let mut policy_rows =
        connection.prepare("SELECT id, error_rules_json FROM policy_profiles ORDER BY id")?;
    let policy_iter = policy_rows.query_map([], |row| {
        let json: String = row.get(1)?;
        let profile = decode_policy_profile_json(&json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    err.to_string(),
                )),
            )
        })?;
        Ok((row.get::<_, String>(0)?, profile))
    })?;
    for row in policy_iter {
        let (id, profile) = row?;
        policy_profiles.insert(id, profile);
    }

    let mut routing_profiles = HashMap::new();
    let mut routing_rows =
        connection.prepare("SELECT id, routing_profile_json FROM routing_profiles ORDER BY id")?;
    let routing_iter = routing_rows.query_map([], |row| {
        let json: String = row.get(1)?;
        let profile: RoutingProfileConfig = serde_json::from_str(&json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(err))
        })?;
        Ok((row.get::<_, String>(0)?, profile))
    })?;
    for row in routing_iter {
        let (id, profile) = row?;
        routing_profiles.insert(id, profile);
    }

    let mut pools = HashMap::new();
    let mut channel_rows = connection.prepare(
        "SELECT id, account_id, credential_set_id, policy_profile_id, routing_profile_id,
                error_rules_json, enabled
         FROM channels ORDER BY id",
    )?;
    let channel_iter = channel_rows.query_map([], |row| {
        let account_id: String = row.get(1)?;
        let account = accounts
            .get(&account_id)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let provider = providers
            .get(&account.provider)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        let error_rules_json: String = row.get(5)?;
        let error_rules: ErrorRulesConfig =
            serde_json::from_str(&error_rules_json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })?;
        Ok((
            row.get::<_, String>(0)?,
            PoolConfig {
                enabled: row.get::<_, i64>(6)? != 0,
                account: Some(account_id),
                policy_profile: row.get(3)?,
                routing_profile: row.get(4)?,
                provider_kind: provider.provider_kind,
                api_base: account.api_base.clone(),
                credential_set: row.get(2)?,
                auth_header: account.auth_header.clone(),
                auth_prefix: account.auth_prefix.clone(),
                error_rules,
            },
        ))
    })?;
    for row in channel_iter {
        let (id, channel) = row?;
        pools.insert(id, channel);
    }

    let mut model_routes = HashMap::new();
    let mut route_rows = connection
        .prepare("SELECT public_model, strategy FROM model_routes ORDER BY public_model")?;
    let route_iter = route_rows.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in route_iter {
        let (public_model, strategy) = row?;
        let mut target_rows = connection.prepare(
            "SELECT channel_id, upstream_model, priority, weight, enabled
             FROM model_route_targets
             WHERE public_model = ?1
             ORDER BY position",
        )?;
        let target_iter = target_rows.query_map(params![&public_model], |row| {
            Ok(ModelRouteTargetConfig {
                channel: row.get(0)?,
                upstream_model: row.get(1)?,
                priority: checked_u16(row.get(2)?, 2, "priority")?,
                weight: checked_u16(row.get(3)?, 3, "weight")?,
                enabled: row.get::<_, i64>(4)? != 0,
            })
        })?;
        let mut targets = Vec::new();
        for target in target_iter {
            targets.push(target?);
        }
        model_routes.insert(public_model, ModelRouteConfig { strategy, targets });
    }

    Ok(RegistryResources {
        default_pool: defaults.default_pool,
        providers,
        accounts,
        credential_sets,
        policy_profiles,
        default_routing_profile: defaults.default_routing_profile,
        routing_profiles,
        model_routes,
        pools,
    })
}

fn decode_policy_profile_json(json: &str) -> anyhow::Result<PolicyProfileConfig> {
    match serde_json::from_str::<PolicyProfileConfig>(json) {
        Ok(profile) => Ok(profile),
        Err(profile_err) => match serde_json::from_str::<ErrorRulesConfig>(json) {
            Ok(error_rules) => Ok(PolicyProfileConfig {
                error_rules,
                probe_result_actions: Default::default(),
            }),
            Err(_) => Err(profile_err.into()),
        },
    }
}

fn validate_store_supported_registry_document(document: &RegistryDocument) -> anyhow::Result<()> {
    anyhow::ensure!(
        document.model_groups.is_empty(),
        "sqlite registry store does not persist model_groups yet"
    );
    if let Some(default_pool) = &document.default_pool {
        anyhow::ensure!(
            document.pools.contains_key(default_pool),
            "default_pool {default_pool} does not exist"
        );
    }
    if let Some(default_routing_profile) = &document.default_routing_profile {
        anyhow::ensure!(
            document
                .routing_profiles
                .contains_key(default_routing_profile),
            "default_routing_profile {default_routing_profile} does not exist"
        );
    }
    for (id, account) in &document.accounts {
        anyhow::ensure!(
            document.providers.contains_key(&account.provider),
            "account {id} references unknown provider {}",
            account.provider
        );
    }
    for (id, channel) in &document.pools {
        let account_id = channel
            .account
            .as_deref()
            .filter(|account| !account.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("channel {id} requires canonical account reference"))?;
        anyhow::ensure!(
            document.accounts.contains_key(account_id),
            "channel {id} references unknown account {account_id}"
        );
        anyhow::ensure!(
            document
                .credential_sets
                .contains_key(&channel.credential_set),
            "channel {id} references unknown credential_set {}",
            channel.credential_set
        );
        if let Some(policy_profile) = &channel.policy_profile {
            anyhow::ensure!(
                document.policy_profiles.contains_key(policy_profile),
                "channel {id} references unknown policy_profile {policy_profile}"
            );
        }
        if let Some(routing_profile) = &channel.routing_profile {
            anyhow::ensure!(
                document.routing_profiles.contains_key(routing_profile),
                "channel {id} references unknown routing_profile {routing_profile}"
            );
        }
    }
    for (public_model, route) in &document.model_routes {
        for target in &route.targets {
            anyhow::ensure!(
                document.pools.contains_key(&target.channel),
                "model route {public_model} references unknown channel {}",
                target.channel
            );
        }
    }
    Ok(())
}

fn checked_u16(value: i64, column: usize, name: &'static str) -> rusqlite::Result<u16> {
    u16::try_from(value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            format!("{name} out of u16 range: {err}").into(),
        )
    })
}

struct RegistryDefaults {
    default_pool: Option<String>,
    default_routing_profile: Option<String>,
}

fn load_registry_defaults(connection: &Connection) -> anyhow::Result<RegistryDefaults> {
    let defaults = connection.query_row(
        "SELECT default_pool, default_routing_profile FROM registry_defaults WHERE id = 1",
        [],
        |row| {
            Ok(RegistryDefaults {
                default_pool: row.get(0)?,
                default_routing_profile: row.get(1)?,
            })
        },
    );
    match defaults {
        Ok(defaults) => Ok(defaults),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(RegistryDefaults {
            default_pool: None,
            default_routing_profile: None,
        }),
        Err(err) => Err(err.into()),
    }
}

pub(crate) fn overlay_registry_resources(
    mut bootstrap: RegistryDocument,
    resources: RegistryDocument,
) -> RegistryDocument {
    bootstrap.apply_resources(resources.into_resources());
    bootstrap
}

fn overlay_registry_loaded_resources(
    mut bootstrap: RegistryDocument,
    resources: RegistryResources,
) -> RegistryDocument {
    bootstrap.apply_resources(resources);
    bootstrap
}

fn provider_kind_to_str(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAiCompatible => "openai_compatible",
        ProviderKind::GenericHttp => "generic_http",
    }
}

fn provider_kind_from_str(kind: &str) -> Result<ProviderKind, String> {
    match kind {
        "openai_compatible" => Ok(ProviderKind::OpenAiCompatible),
        "generic_http" => Ok(ProviderKind::GenericHttp),
        other => Err(format!("unknown provider kind {other}")),
    }
}

fn now_unix_seconds() -> anyhow::Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{
            AccountConfig, CredentialSetConfig, ModelRouteConfig, ModelRouteTargetConfig,
            PolicyProfileConfig, PoolConfig, ProviderConfig, RouteTargetRetryConfig,
            RoutingProfileConfig, SameRequestCredentialRetryConfig, TimeoutConfig,
        },
        provider::ProviderKind,
        test_fixtures::{credential_lines, fixtures},
    };
    use std::{collections::HashMap, net::SocketAddr};
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn minimal_registry_document() -> RegistryDocument {
        RegistryDocument {
            listen: SocketAddr::from(([127, 0, 0, 1], 0)),
            client_tokens: Vec::new(),
            management: None,
            max_request_body_bytes: 1024 * 1024,
            max_model_catalog_body_bytes: 512 * 1024,
            max_error_body_bytes: 1024,
            timeouts: Default::default(),
            routing: Default::default(),
            response_filter: Default::default(),
            default_pool: None,
            providers: HashMap::new(),
            accounts: HashMap::new(),
            credential_sets: HashMap::new(),
            model_groups: HashMap::new(),
            policy_profiles: HashMap::new(),
            default_routing_profile: None,
            routing_profiles: HashMap::new(),
            model_routes: HashMap::new(),
            pools: HashMap::new(),
        }
    }

    #[test]
    fn registry_store_read_only_rejects_writes() {
        let document = minimal_registry_document();
        let store = ReadOnlyRegistryStore::new(document);

        assert!(store
            .load_registry_for_validation()
            .unwrap()
            .providers
            .is_empty());
        let err = store
            .apply_command(
                RegistryCommand::Provider(ProviderRegistryCommand::SetEnabled {
                    provider_id: "openai".to_string(),
                    enabled: false,
                }),
                &|_| Ok(()),
            )
            .unwrap_err();

        assert_eq!(err, RegistryStoreError::NotWritable);
    }

    fn temp_sqlite_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{suffix}.sqlite"))
    }

    fn representative_non_secret_registry_document() -> RegistryDocument {
        let mut document = minimal_registry_document();
        document.providers.insert(
            "openai".to_string(),
            ProviderConfig {
                provider_kind: ProviderKind::OpenAiCompatible,
                enabled: true,
            },
        );
        document.accounts.insert(
            "primary".to_string(),
            AccountConfig {
                provider: "openai".to_string(),
                api_base: "https://relay.example.test/v1".to_string(),
                auth_header: "Authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                enabled: true,
            },
        );
        document.credential_sets.insert(
            "primary-keys".to_string(),
            CredentialSetConfig {
                keys_file: PathBuf::from("/tmp/key-pool-router-test-keys.txt"),
            },
        );
        document.policy_profiles.insert(
            "relay-cooldown".to_string(),
            PolicyProfileConfig {
                error_rules: Default::default(),
                probe_result_actions: Default::default(),
            },
        );
        document.routing_profiles.insert(
            "sticky".to_string(),
            RoutingProfileConfig {
                key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                default_credential_cooldown_seconds: 20,
                same_request_credential_retry: SameRequestCredentialRetryConfig {
                    enabled: false,
                    max_retries: 0,
                },
                route_target_retry: RouteTargetRetryConfig { enabled: true },
            },
        );
        document.pools.insert(
            "primary-channel".to_string(),
            PoolConfig {
                enabled: true,
                account: Some("primary".to_string()),
                policy_profile: Some("relay-cooldown".to_string()),
                routing_profile: Some("sticky".to_string()),
                provider_kind: ProviderKind::OpenAiCompatible,
                api_base: "https://legacy-ignored.example.test/v1".to_string(),
                credential_set: "primary-keys".to_string(),
                auth_header: "Authorization".to_string(),
                auth_prefix: "Bearer ".to_string(),
                error_rules: crate::config::ErrorRulesConfig {
                    switch_codes: Some(vec!["custom_pool_switch".to_string()]),
                    ..Default::default()
                },
            },
        );
        document.model_routes.insert(
            "gpt-public".to_string(),
            ModelRouteConfig {
                strategy: Some("priority".to_string()),
                targets: vec![ModelRouteTargetConfig {
                    channel: "primary-channel".to_string(),
                    upstream_model: Some("gpt-upstream".to_string()),
                    priority: 10,
                    weight: 1,
                    enabled: true,
                }],
            },
        );
        document.default_routing_profile = Some("sticky".to_string());
        document
    }

    fn assert_non_secret_registry_equivalent(
        loaded: &RegistryDocument,
        expected: &RegistryDocument,
    ) {
        assert_eq!(loaded.providers.len(), expected.providers.len());
        assert_eq!(
            loaded.providers["openai"].provider_kind,
            expected.providers["openai"].provider_kind
        );
        assert_eq!(
            loaded.providers["openai"].enabled,
            expected.providers["openai"].enabled
        );
        assert_eq!(loaded.accounts.len(), expected.accounts.len());
        assert_eq!(
            loaded.accounts["primary"].provider,
            expected.accounts["primary"].provider
        );
        assert_eq!(
            loaded.accounts["primary"].api_base,
            expected.accounts["primary"].api_base
        );
        assert_eq!(
            loaded.credential_sets["primary-keys"].keys_file,
            expected.credential_sets["primary-keys"].keys_file
        );
        assert_eq!(
            loaded.pools["primary-channel"].account,
            expected.pools["primary-channel"].account
        );
        assert_eq!(
            loaded.pools["primary-channel"].credential_set,
            expected.pools["primary-channel"].credential_set
        );
        assert_eq!(
            loaded.pools["primary-channel"].policy_profile,
            expected.pools["primary-channel"].policy_profile
        );
        assert_eq!(
            loaded.pools["primary-channel"].routing_profile,
            expected.pools["primary-channel"].routing_profile
        );
        assert_eq!(
            serde_json::to_value(&loaded.pools["primary-channel"].error_rules).unwrap(),
            serde_json::to_value(&expected.pools["primary-channel"].error_rules).unwrap()
        );
        assert_eq!(
            loaded.default_routing_profile,
            expected.default_routing_profile
        );
        assert_eq!(
            loaded.routing_profiles["sticky"].default_credential_cooldown_seconds,
            expected.routing_profiles["sticky"].default_credential_cooldown_seconds
        );
        assert_eq!(
            loaded.model_routes["gpt-public"].strategy,
            expected.model_routes["gpt-public"].strategy
        );
        assert_eq!(
            loaded.model_routes["gpt-public"].targets[0].upstream_model,
            expected.model_routes["gpt-public"].targets[0].upstream_model
        );
        assert!(loaded.client_tokens.is_empty());
        assert!(loaded.management.is_none());
    }

    #[test]
    fn registry_document_apply_resources_replaces_only_registry_resources() {
        let mut bootstrap = minimal_registry_document();
        bootstrap.listen = "127.0.0.1:4101".parse().unwrap();
        bootstrap.client_tokens = vec![crate::config::ClientTokenConfig {
            name: "bootstrap-client".to_string(),
            token: "bootstrap-token".to_string(),
            enabled: false,
            allowed_model_groups: vec!["bootstrap-models".to_string()],
            allowed_channels: vec!["bootstrap-channel".to_string()],
        }];
        bootstrap.management = Some(crate::config::ManagementConfig {
            admin_token: "bootstrap-admin".to_string(),
            ip_allowlist: None,
            principals: Vec::new(),
            event_log_path: Some(PathBuf::from("/tmp/bootstrap-events.jsonl")),
            event_window_capacity: Some(17),
        });
        bootstrap.max_request_body_bytes = 1234;
        bootstrap.max_model_catalog_body_bytes = 2345;
        bootstrap.max_error_body_bytes = 3456;
        bootstrap.model_groups.insert(
            "bootstrap-models".to_string(),
            crate::config::ModelGroupConfig {
                models: vec!["bootstrap-model".to_string()],
            },
        );

        let mut resources_source = representative_non_secret_registry_document();
        resources_source.default_pool = Some("primary-channel".to_string());
        resources_source.listen = "127.0.0.1:4202".parse().unwrap();
        resources_source.client_tokens = vec![crate::config::ClientTokenConfig {
            name: "resource-client".to_string(),
            token: "resource-token".to_string(),
            enabled: true,
            allowed_model_groups: vec!["resource-models".to_string()],
            allowed_channels: vec!["resource-channel".to_string()],
        }];
        resources_source.management = Some(crate::config::ManagementConfig {
            admin_token: "resource-admin".to_string(),
            ip_allowlist: None,
            principals: Vec::new(),
            event_log_path: Some(PathBuf::from("/tmp/resource-events.jsonl")),
            event_window_capacity: Some(29),
        });
        resources_source.model_groups.insert(
            "resource-models".to_string(),
            crate::config::ModelGroupConfig {
                models: vec!["resource-model".to_string()],
            },
        );

        let resources = resources_source.into_resources();
        bootstrap.apply_resources(resources.clone());

        assert_eq!(bootstrap.default_pool, resources.default_pool);
        assert_eq!(bootstrap.providers, resources.providers);
        assert_eq!(bootstrap.accounts, resources.accounts);
        assert_eq!(
            bootstrap.credential_sets["primary-keys"].keys_file,
            resources.credential_sets["primary-keys"].keys_file
        );
        assert_eq!(bootstrap.policy_profiles, resources.policy_profiles);
        assert_eq!(
            bootstrap.default_routing_profile,
            resources.default_routing_profile
        );
        assert_eq!(bootstrap.routing_profiles, resources.routing_profiles);
        assert_eq!(bootstrap.model_routes, resources.model_routes);
        assert_eq!(bootstrap.pools, resources.pools);

        assert_eq!(bootstrap.listen, "127.0.0.1:4101".parse().unwrap());
        assert_eq!(bootstrap.client_tokens[0].name, "bootstrap-client");
        assert_eq!(
            bootstrap.management.as_ref().unwrap().admin_token,
            "bootstrap-admin"
        );
        assert_eq!(bootstrap.max_request_body_bytes, 1234);
        assert_eq!(bootstrap.max_model_catalog_body_bytes, 2345);
        assert_eq!(bootstrap.max_error_body_bytes, 3456);
        assert!(bootstrap.model_groups.contains_key("bootstrap-models"));
        assert!(!bootstrap.model_groups.contains_key("resource-models"));
    }

    #[test]
    fn overlay_registry_resources_preserves_control_plane_bootstrap_fields() {
        let mut bootstrap = minimal_registry_document();
        bootstrap.listen = "127.0.0.1:4101".parse().unwrap();
        bootstrap.client_tokens = vec![crate::config::ClientTokenConfig {
            name: "bootstrap-client".to_string(),
            token: "bootstrap-token".to_string(),
            enabled: false,
            allowed_model_groups: vec!["bootstrap-models".to_string()],
            allowed_channels: vec!["bootstrap-channel".to_string()],
        }];
        bootstrap.management = Some(crate::config::ManagementConfig {
            admin_token: "bootstrap-admin".to_string(),
            ip_allowlist: None,
            principals: Vec::new(),
            event_log_path: Some(PathBuf::from("/tmp/bootstrap-events.jsonl")),
            event_window_capacity: Some(17),
        });
        bootstrap.max_request_body_bytes = 1234;
        bootstrap.max_model_catalog_body_bytes = 2345;
        bootstrap.max_error_body_bytes = 3456;
        bootstrap.timeouts = TimeoutConfig {
            connect_seconds: Some(1),
            non_streaming_total_seconds: Some(2),
            streaming_idle_seconds: Some(3),
        };
        bootstrap.routing = crate::config::RoutingConfig {
            max_route_candidates: Some(4),
            max_model_catalog_channels: Some(5),
            telemetry_buffer_capacity: Some(6),
        };
        bootstrap.response_filter = crate::config::ResponseFilterConfig {
            enabled: true,
            replacement: Some("[filtered]".to_string()),
            event_window_capacity: None,
            alert_window_seconds: None,
            rules: vec![crate::config::ResponseFilterRuleConfig {
                id: "bootstrap-filter".to_string(),
                enabled: true,
                kind: crate::config::ResponseFilterRuleKindConfig::Literal,
                action: crate::config::ResponseFilterActionConfig::Reject,
                case_sensitive: true,
                value: Some("secret".to_string()),
                pattern: None,
            }],
        };
        bootstrap.model_groups.insert(
            "bootstrap-models".to_string(),
            crate::config::ModelGroupConfig {
                models: vec!["bootstrap-model".to_string()],
            },
        );
        bootstrap.default_pool = Some("bootstrap-pool".to_string());
        bootstrap.providers.insert(
            "bootstrap-provider".to_string(),
            ProviderConfig {
                provider_kind: ProviderKind::GenericHttp,
                enabled: false,
            },
        );
        bootstrap.accounts.insert(
            "bootstrap-account".to_string(),
            AccountConfig {
                provider: "bootstrap-provider".to_string(),
                api_base: "https://bootstrap.example.test/v1".to_string(),
                auth_header: "X-Bootstrap-Key".to_string(),
                auth_prefix: "Token ".to_string(),
                enabled: false,
            },
        );

        let mut resources = representative_non_secret_registry_document();
        resources.default_pool = Some("primary-channel".to_string());
        resources.listen = "127.0.0.1:4202".parse().unwrap();
        resources.client_tokens = vec![crate::config::ClientTokenConfig {
            name: "resource-client".to_string(),
            token: "resource-token".to_string(),
            enabled: true,
            allowed_model_groups: vec!["resource-models".to_string()],
            allowed_channels: vec!["resource-channel".to_string()],
        }];
        resources.management = Some(crate::config::ManagementConfig {
            admin_token: "resource-admin".to_string(),
            ip_allowlist: None,
            principals: Vec::new(),
            event_log_path: Some(PathBuf::from("/tmp/resource-events.jsonl")),
            event_window_capacity: Some(29),
        });
        resources.max_request_body_bytes = 9876;
        resources.max_model_catalog_body_bytes = 8765;
        resources.max_error_body_bytes = 7654;
        resources.timeouts = TimeoutConfig {
            connect_seconds: Some(7),
            non_streaming_total_seconds: Some(8),
            streaming_idle_seconds: Some(9),
        };
        resources.routing = crate::config::RoutingConfig {
            max_route_candidates: Some(10),
            max_model_catalog_channels: Some(11),
            telemetry_buffer_capacity: Some(12),
        };
        resources.response_filter = crate::config::ResponseFilterConfig {
            enabled: false,
            replacement: Some("[resource-filtered]".to_string()),
            event_window_capacity: None,
            alert_window_seconds: None,
            rules: Vec::new(),
        };
        resources.model_groups.insert(
            "resource-models".to_string(),
            crate::config::ModelGroupConfig {
                models: vec!["resource-model".to_string()],
            },
        );

        let overlaid = overlay_registry_resources(bootstrap.clone(), resources.clone());

        assert_eq!(overlaid.default_pool, resources.default_pool);
        assert_eq!(overlaid.providers, resources.providers);
        assert_eq!(overlaid.accounts, resources.accounts);
        assert_eq!(
            overlaid.credential_sets["primary-keys"].keys_file,
            resources.credential_sets["primary-keys"].keys_file
        );
        assert_eq!(overlaid.policy_profiles, resources.policy_profiles);
        assert_eq!(
            overlaid.default_routing_profile,
            resources.default_routing_profile
        );
        assert_eq!(overlaid.routing_profiles, resources.routing_profiles);
        assert_eq!(overlaid.model_routes, resources.model_routes);
        assert_eq!(overlaid.pools, resources.pools);

        assert_eq!(overlaid.listen, bootstrap.listen);
        assert_eq!(overlaid.client_tokens.len(), 1);
        assert_eq!(overlaid.client_tokens[0].name, "bootstrap-client");
        assert_eq!(overlaid.client_tokens[0].token, "bootstrap-token");
        assert!(!overlaid.client_tokens[0].enabled);
        assert_eq!(
            overlaid.client_tokens[0].allowed_model_groups,
            vec!["bootstrap-models".to_string()]
        );
        assert_eq!(
            overlaid.client_tokens[0].allowed_channels,
            vec!["bootstrap-channel".to_string()]
        );
        let management = overlaid.management.as_ref().unwrap();
        assert_eq!(management.admin_token, "bootstrap-admin");
        assert_eq!(
            management.event_log_path.as_deref(),
            Some(Path::new("/tmp/bootstrap-events.jsonl"))
        );
        assert_eq!(management.event_window_capacity, Some(17));
        assert_eq!(overlaid.max_request_body_bytes, 1234);
        assert_eq!(overlaid.max_model_catalog_body_bytes, 2345);
        assert_eq!(overlaid.max_error_body_bytes, 3456);
        assert_eq!(overlaid.timeouts.connect_seconds, Some(1));
        assert_eq!(overlaid.timeouts.non_streaming_total_seconds, Some(2));
        assert_eq!(overlaid.timeouts.streaming_idle_seconds, Some(3));
        assert_eq!(overlaid.routing.max_route_candidates, Some(4));
        assert_eq!(overlaid.routing.max_model_catalog_channels, Some(5));
        assert_eq!(overlaid.routing.telemetry_buffer_capacity, Some(6));
        assert_eq!(overlaid.response_filter, bootstrap.response_filter);
        assert_eq!(overlaid.model_groups, bootstrap.model_groups);
    }

    #[test]
    fn sqlite_registry_store_rejects_direct_pool_bootstrap() {
        let path = temp_sqlite_path("key-pool-router-registry-store-direct-pool");
        let mut document = representative_non_secret_registry_document();
        document.pools.get_mut("primary-channel").unwrap().account = None;

        let err = SqliteRegistryStore::bootstrap_from_document(&path, document).unwrap_err();

        assert!(err
            .to_string()
            .contains("requires canonical account reference"));
    }

    #[test]
    fn sqlite_registry_store_rejects_direct_pool_startup_bootstrap() {
        let path = temp_sqlite_path("key-pool-router-registry-store-direct-startup");
        let mut document = representative_non_secret_registry_document();
        document.pools.get_mut("primary-channel").unwrap().account = None;

        let err =
            SqliteRegistryStore::load_or_bootstrap_startup_document(&path, document).unwrap_err();

        assert!(err
            .to_string()
            .contains("requires canonical account reference"));
    }

    #[test]
    fn sqlite_registry_store_loads_bootstrapped_non_secret_document() {
        let path = temp_sqlite_path("key-pool-router-registry-store-load");
        let document = representative_non_secret_registry_document();

        SqliteRegistryStore::bootstrap_from_document(&path, document.clone()).unwrap();
        let loaded = SqliteRegistryStore::open(&path)
            .unwrap()
            .load_registry_for_validation()
            .unwrap();

        assert_non_secret_registry_equivalent(&loaded, &document);
    }

    #[test]
    fn sqlite_registry_store_set_provider_enabled_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-provider-enabled");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let commit = store
            .apply_command(
                RegistryCommand::Provider(ProviderRegistryCommand::SetEnabled {
                    provider_id: "openai".to_string(),
                    enabled: false,
                }),
                &|document| {
                    assert!(!document.providers["openai"].enabled);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        assert!(!commit.document_for_validation.providers["openai"].enabled);
        assert!(!store.load_registry_for_validation().unwrap().providers["openai"].enabled);
    }

    #[test]
    fn sqlite_registry_store_upsert_provider_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-provider-upsert");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        let provider = ProviderConfig {
            provider_kind: ProviderKind::OpenAiCompatible,
            enabled: true,
        };

        let commit = store
            .apply_command(
                RegistryCommand::Provider(ProviderRegistryCommand::Upsert {
                    provider_id: "relay-b".to_string(),
                    provider: provider.clone(),
                }),
                &|document| {
                    assert_eq!(
                        document.providers["relay-b"].provider_kind,
                        provider.provider_kind
                    );
                    assert!(document.providers["relay-b"].enabled);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        let stored = store.load_registry_for_validation().unwrap();
        assert_eq!(
            stored.providers["relay-b"].provider_kind,
            provider.provider_kind
        );
        assert!(stored.providers["relay-b"].enabled);
    }

    #[test]
    fn sqlite_registry_store_upsert_provider_rolls_back_when_validation_fails() {
        let path = temp_sqlite_path("key-pool-router-registry-store-provider-upsert-rollback");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let err = store
            .apply_command(
                RegistryCommand::Provider(ProviderRegistryCommand::Upsert {
                    provider_id: "relay-b".to_string(),
                    provider: ProviderConfig {
                        provider_kind: ProviderKind::OpenAiCompatible,
                        enabled: true,
                    },
                }),
                &|_| {
                    Err(RegistryStoreError::Validation(
                        "invalid provider".to_string(),
                    ))
                },
            )
            .unwrap_err();

        assert_eq!(
            err,
            RegistryStoreError::Validation("invalid provider".to_string())
        );
        let stored = store.load_registry_for_validation().unwrap();
        assert!(!stored.providers.contains_key("relay-b"));
        let connection = rusqlite::Connection::open(&path).unwrap();
        let version_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version_count, 1);
    }

    #[test]
    fn sqlite_registry_store_set_account_enabled_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-account-enabled");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let commit = store
            .apply_command(
                RegistryCommand::Account(AccountRegistryCommand::SetEnabled {
                    account_id: "primary".to_string(),
                    enabled: false,
                }),
                &|document| {
                    assert!(!document.accounts["primary"].enabled);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        assert!(!commit.document_for_validation.accounts["primary"].enabled);
        assert!(!store.load_registry_for_validation().unwrap().accounts["primary"].enabled);
    }

    #[test]
    fn sqlite_registry_store_upsert_account_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-account-upsert");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        let account = AccountConfig {
            provider: "openai".to_string(),
            api_base: "https://relay-b.example.test/v1".to_string(),
            auth_header: "X-Api-Key".to_string(),
            auth_prefix: "".to_string(),
            enabled: true,
        };

        let commit = store
            .apply_command(
                RegistryCommand::Account(AccountRegistryCommand::Upsert {
                    account_id: "secondary".to_string(),
                    account: account.clone(),
                }),
                &|document| {
                    assert_eq!(document.accounts["secondary"], account);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        let stored = store.load_registry_for_validation().unwrap();
        assert_eq!(stored.accounts["secondary"], account);
    }

    #[test]
    fn sqlite_registry_store_upsert_account_rolls_back_when_validation_fails() {
        let path = temp_sqlite_path("key-pool-router-registry-store-account-upsert-rollback");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let err = store
            .apply_command(
                RegistryCommand::Account(AccountRegistryCommand::Upsert {
                    account_id: "secondary".to_string(),
                    account: AccountConfig {
                        provider: "missing".to_string(),
                        api_base: "https://relay-b.example.test/v1".to_string(),
                        auth_header: "Authorization".to_string(),
                        auth_prefix: "Bearer ".to_string(),
                        enabled: true,
                    },
                }),
                &|_| {
                    Err(RegistryStoreError::Validation(
                        "invalid account".to_string(),
                    ))
                },
            )
            .unwrap_err();

        assert_eq!(
            err,
            RegistryStoreError::Validation("invalid account".to_string())
        );
        let stored = store.load_registry_for_validation().unwrap();
        assert!(!stored.accounts.contains_key("secondary"));
        let connection = rusqlite::Connection::open(&path).unwrap();
        let version_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version_count, 1);
    }

    #[test]
    fn sqlite_registry_store_set_channel_enabled_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-channel-enabled");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let commit = store
            .apply_command(
                RegistryCommand::Channel(ChannelRegistryCommand::SetEnabled {
                    channel_id: "primary-channel".to_string(),
                    enabled: false,
                }),
                &|document| {
                    assert!(!document.pools["primary-channel"].enabled);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        assert!(!commit.document_for_validation.pools["primary-channel"].enabled);
        assert!(!store.load_registry_for_validation().unwrap().pools["primary-channel"].enabled);
    }

    #[test]
    fn sqlite_registry_store_upsert_channel_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-channel-upsert");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        let channel = PoolConfig {
            enabled: true,
            account: Some("primary".to_string()),
            policy_profile: Some("relay-cooldown".to_string()),
            routing_profile: Some("sticky".to_string()),
            provider_kind: ProviderKind::OpenAiCompatible,
            api_base: "https://legacy.example.test/v1".to_string(),
            credential_set: "primary-keys".to_string(),
            auth_header: "Authorization".to_string(),
            auth_prefix: "Bearer ".to_string(),
            error_rules: ErrorRulesConfig::default(),
        };

        let commit = store
            .apply_command(
                RegistryCommand::Channel(ChannelRegistryCommand::Upsert {
                    channel_id: "secondary-channel".to_string(),
                    channel: Box::new(channel.clone()),
                }),
                &|document| {
                    assert_eq!(document.pools["secondary-channel"], channel);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        let stored = store.load_registry_for_validation().unwrap();
        assert_eq!(stored.pools["secondary-channel"].account, channel.account);
        assert_eq!(
            stored.pools["secondary-channel"].credential_set,
            channel.credential_set
        );
        assert_eq!(
            stored.pools["secondary-channel"].routing_profile,
            channel.routing_profile
        );
        assert!(stored.pools["secondary-channel"].enabled);
    }

    #[test]
    fn sqlite_registry_store_upsert_channel_rolls_back_when_validation_fails() {
        let path = temp_sqlite_path("key-pool-router-registry-store-channel-upsert-rollback");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let err = store
            .apply_command(
                RegistryCommand::Channel(ChannelRegistryCommand::Upsert {
                    channel_id: "secondary-channel".to_string(),
                    channel: Box::new(PoolConfig {
                        enabled: true,
                        account: Some("primary".to_string()),
                        policy_profile: None,
                        routing_profile: Some("missing-routing".to_string()),
                        provider_kind: ProviderKind::OpenAiCompatible,
                        api_base: "https://legacy.example.test/v1".to_string(),
                        credential_set: "primary-keys".to_string(),
                        auth_header: "Authorization".to_string(),
                        auth_prefix: "Bearer ".to_string(),
                        error_rules: ErrorRulesConfig::default(),
                    }),
                }),
                &|_| {
                    Err(RegistryStoreError::Validation(
                        "invalid channel".to_string(),
                    ))
                },
            )
            .unwrap_err();

        assert_eq!(
            err,
            RegistryStoreError::Validation("invalid channel".to_string())
        );
        let stored = store.load_registry_for_validation().unwrap();
        assert!(!stored.pools.contains_key("secondary-channel"));
        let connection = rusqlite::Connection::open(&path).unwrap();
        let version_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version_count, 1);
    }

    #[test]
    fn sqlite_registry_store_upsert_model_route_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-model-route");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        let route = ModelRouteConfig {
            strategy: Some("priority_weighted_sticky".to_string()),
            targets: vec![
                ModelRouteTargetConfig {
                    channel: "primary-channel".to_string(),
                    upstream_model: Some("gpt-upstream-a".to_string()),
                    priority: 5,
                    weight: 3,
                    enabled: true,
                },
                ModelRouteTargetConfig {
                    channel: "primary-channel".to_string(),
                    upstream_model: Some("gpt-upstream-b".to_string()),
                    priority: 9,
                    weight: 1,
                    enabled: false,
                },
            ],
        };

        let commit = store
            .apply_command(
                RegistryCommand::ModelRoute(ModelRouteRegistryCommand::Upsert {
                    public_model: "gpt-public".to_string(),
                    route: route.clone(),
                }),
                &|document| {
                    assert_eq!(document.model_routes["gpt-public"].strategy, route.strategy);
                    assert_eq!(document.model_routes["gpt-public"].targets.len(), 2);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        assert_eq!(
            commit.document_for_validation.model_routes["gpt-public"].targets[0].upstream_model,
            Some("gpt-upstream-a".to_string())
        );
        let stored = store.load_registry_for_validation().unwrap();
        assert_eq!(stored.model_routes["gpt-public"].strategy, route.strategy);
        assert_eq!(stored.model_routes["gpt-public"].targets.len(), 2);
        assert_eq!(
            stored.model_routes["gpt-public"].targets[1].upstream_model,
            Some("gpt-upstream-b".to_string())
        );
    }

    #[test]
    fn sqlite_registry_store_upsert_model_route_rolls_back_when_validation_fails() {
        let path = temp_sqlite_path("key-pool-router-registry-store-model-route-rollback");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        let invalid_route = ModelRouteConfig {
            strategy: Some("priority".to_string()),
            targets: vec![ModelRouteTargetConfig {
                channel: "missing-channel".to_string(),
                upstream_model: Some("gpt-invalid".to_string()),
                priority: 1,
                weight: 1,
                enabled: true,
            }],
        };

        let err = store
            .apply_command(
                RegistryCommand::ModelRoute(ModelRouteRegistryCommand::Upsert {
                    public_model: "gpt-public".to_string(),
                    route: invalid_route,
                }),
                &|document| {
                    assert_eq!(
                        document.model_routes["gpt-public"].targets[0].channel,
                        "missing-channel"
                    );
                    Err(RegistryStoreError::Validation(
                        "invalid model route".to_string(),
                    ))
                },
            )
            .unwrap_err();

        assert!(matches!(err, RegistryStoreError::Validation(_)));
        let stored = store.load_registry_for_validation().unwrap();
        assert_eq!(stored.model_routes["gpt-public"].targets.len(), 1);
        assert_eq!(
            stored.model_routes["gpt-public"].targets[0].channel,
            "primary-channel"
        );
        let connection = rusqlite::Connection::open(&path).unwrap();
        let version_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version_count, 1);
    }

    #[test]
    fn sqlite_registry_store_upsert_model_route_batch_rejects_stale_expected_version() {
        let path = temp_sqlite_path("key-pool-router-registry-store-model-route-stale-batch");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        store
            .apply_command(
                RegistryCommand::ModelRoute(ModelRouteRegistryCommand::Upsert {
                    public_model: "concurrent-model".to_string(),
                    route: ModelRouteConfig {
                        strategy: Some("priority".to_string()),
                        targets: vec![ModelRouteTargetConfig {
                            channel: "primary-channel".to_string(),
                            upstream_model: None,
                            priority: 0,
                            weight: 1,
                            enabled: true,
                        }],
                    },
                }),
                &|_| Ok(()),
            )
            .unwrap();

        let err = store
            .apply_command(
                RegistryCommand::ModelRoute(ModelRouteRegistryCommand::UpsertBatch {
                    expected_registry_version: Some(1),
                    routes: vec![(
                        "gpt-public".to_string(),
                        ModelRouteConfig {
                            strategy: Some("priority".to_string()),
                            targets: vec![ModelRouteTargetConfig {
                                channel: "primary-channel".to_string(),
                                upstream_model: None,
                                priority: 0,
                                weight: 1,
                                enabled: true,
                            }],
                        },
                    )],
                }),
                &|_| Ok(()),
            )
            .unwrap_err();

        assert_eq!(
            err,
            RegistryStoreError::Conflict(
                "registry version changed during model discovery sync apply".to_string()
            )
        );
        let stored = store.load_registry_for_validation().unwrap();
        assert!(stored.model_routes.contains_key("concurrent-model"));
        assert_eq!(stored.model_routes["gpt-public"].targets.len(), 1);
    }

    #[test]
    fn sqlite_registry_store_upsert_policy_profile_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-policy-profile");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        let profile = PolicyProfileConfig {
            error_rules: ErrorRulesConfig {
                switch_codes: Some(vec!["rate_limit_cooldown".to_string()]),
                adaptation_rules: vec![crate::config::ErrorAdaptationRuleConfig {
                    id: "relay-cooldown".to_string(),
                    enabled: true,
                    matcher: crate::config::ErrorAdaptationMatcherConfig {
                        limit_types: vec!["cooldown".to_string()],
                        ..Default::default()
                    },
                    action: crate::config::ErrorAdaptationActionConfig {
                        kind: Some(crate::error::FailureKind::RateLimited),
                        primary_scope: Some(crate::error::FailureScope::Credential),
                        retryable: Some(true),
                        cooldown_seconds: Some(20),
                    },
                }],
                ..Default::default()
            },
            probe_result_actions: Default::default(),
        };

        let commit = store
            .apply_command(
                RegistryCommand::PolicyProfile(PolicyProfileRegistryCommand::Upsert {
                    profile_id: "relay-cooldown".to_string(),
                    profile: profile.clone(),
                }),
                &|document| {
                    assert_eq!(
                        document.policy_profiles["relay-cooldown"].error_rules,
                        profile.error_rules
                    );
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        let stored = store.load_registry_for_validation().unwrap();
        assert_eq!(
            stored.policy_profiles["relay-cooldown"].error_rules,
            profile.error_rules
        );
    }

    #[test]
    fn sqlite_registry_store_upsert_policy_profile_rolls_back_when_validation_fails() {
        let path = temp_sqlite_path("key-pool-router-registry-store-policy-profile-rollback");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let err = store
            .apply_command(
                RegistryCommand::PolicyProfile(PolicyProfileRegistryCommand::Upsert {
                    profile_id: "relay-cooldown".to_string(),
                    profile: PolicyProfileConfig {
                        error_rules: ErrorRulesConfig {
                            switch_codes: Some(vec!["would-not-commit".to_string()]),
                            ..Default::default()
                        },
                        probe_result_actions: Default::default(),
                    },
                }),
                &|_| Err(RegistryStoreError::Validation("invalid policy".to_string())),
            )
            .unwrap_err();

        assert_eq!(
            err,
            RegistryStoreError::Validation("invalid policy".to_string())
        );
        let stored = store.load_registry_for_validation().unwrap();
        assert!(stored.policy_profiles["relay-cooldown"]
            .error_rules
            .switch_codes
            .is_none());
        let connection = rusqlite::Connection::open(&path).unwrap();
        let version_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version_count, 1);
    }

    #[test]
    fn sqlite_registry_store_upsert_routing_profile_commits_after_validation() {
        let path = temp_sqlite_path("key-pool-router-registry-store-routing-profile");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        let profile = RoutingProfileConfig {
            key_selection: crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
            default_credential_cooldown_seconds: 45,
            same_request_credential_retry: SameRequestCredentialRetryConfig {
                enabled: true,
                max_retries: 2,
            },
            route_target_retry: RouteTargetRetryConfig { enabled: false },
        };

        let commit = store
            .apply_command(
                RegistryCommand::RoutingProfile(RoutingProfileRegistryCommand::Upsert {
                    profile_id: "sticky".to_string(),
                    profile: profile.clone(),
                }),
                &|document| {
                    assert_eq!(document.routing_profiles["sticky"], profile);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(commit.registry_version, 2);
        let stored = store.load_registry_for_validation().unwrap();
        assert_eq!(stored.routing_profiles["sticky"], profile);
    }

    #[test]
    fn sqlite_registry_store_upsert_routing_profile_rolls_back_when_validation_fails() {
        let path = temp_sqlite_path("key-pool-router-registry-store-routing-profile-rollback");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let err = store
            .apply_command(
                RegistryCommand::RoutingProfile(RoutingProfileRegistryCommand::Upsert {
                    profile_id: "sticky".to_string(),
                    profile: RoutingProfileConfig {
                        key_selection:
                            crate::config::KeySelectionStrategyConfig::StickyUntilFailure,
                        default_credential_cooldown_seconds: 0,
                        same_request_credential_retry: SameRequestCredentialRetryConfig {
                            enabled: false,
                            max_retries: 1,
                        },
                        route_target_retry: RouteTargetRetryConfig { enabled: true },
                    },
                }),
                &|_| {
                    Err(RegistryStoreError::Validation(
                        "invalid routing".to_string(),
                    ))
                },
            )
            .unwrap_err();

        assert_eq!(
            err,
            RegistryStoreError::Validation("invalid routing".to_string())
        );
        let stored = store.load_registry_for_validation().unwrap();
        assert_eq!(
            stored.routing_profiles["sticky"].default_credential_cooldown_seconds,
            20
        );
        let connection = rusqlite::Connection::open(&path).unwrap();
        let version_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version_count, 1);
    }

    #[test]
    fn sqlite_registry_store_startup_loads_persisted_resources_over_bootstrap() {
        let path = temp_sqlite_path("key-pool-router-registry-store-startup-overlay");
        let mut initial = representative_non_secret_registry_document();
        initial.listen = "127.0.0.1:4111".parse().unwrap();
        SqliteRegistryStore::bootstrap_from_document(&path, initial).unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();
        store
            .apply_command(
                RegistryCommand::Provider(ProviderRegistryCommand::SetEnabled {
                    provider_id: "openai".to_string(),
                    enabled: false,
                }),
                &|_| Ok(()),
            )
            .unwrap();
        let mut bootstrap = representative_non_secret_registry_document();
        bootstrap.listen = "127.0.0.1:4222".parse().unwrap();
        bootstrap.providers.get_mut("openai").unwrap().enabled = true;

        let loaded = SqliteRegistryStore::load_or_bootstrap_startup_document(&path, bootstrap)
            .unwrap()
            .document;

        assert_eq!(loaded.listen, "127.0.0.1:4222".parse().unwrap());
        assert!(!loaded.providers["openai"].enabled);
    }

    #[test]
    fn sqlite_registry_store_rolls_back_when_validation_fails() {
        let path = temp_sqlite_path("key-pool-router-registry-store-rollback");
        SqliteRegistryStore::bootstrap_from_document(
            &path,
            representative_non_secret_registry_document(),
        )
        .unwrap();
        let store = SqliteRegistryStore::open(&path).unwrap();

        let err = store
            .apply_command(
                RegistryCommand::Provider(ProviderRegistryCommand::SetEnabled {
                    provider_id: "openai".to_string(),
                    enabled: false,
                }),
                &|_| {
                    Err(RegistryStoreError::Validation(
                        "invalid test document".to_string(),
                    ))
                },
            )
            .unwrap_err();

        assert_eq!(
            err,
            RegistryStoreError::Validation("invalid test document".to_string())
        );
        assert!(store.load_registry_for_validation().unwrap().providers["openai"].enabled);

        let connection = rusqlite::Connection::open(&path).unwrap();
        let version_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM registry_versions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version_count, 1);
    }

    #[test]
    fn sqlite_registry_store_does_not_persist_raw_secrets() {
        let path = temp_sqlite_path("key-pool-router-registry-store-no-secrets");
        let key_path = temp_sqlite_path("key-pool-router-upstream-secret").with_extension("txt");
        let fixture_values = fixtures();
        let upstream_secret = fixture_values.upstream_credentials.test_upstream.as_str();
        std::fs::write(&key_path, credential_lines(&[upstream_secret])).unwrap();
        let mut document = representative_non_secret_registry_document();
        document
            .credential_sets
            .get_mut("primary-keys")
            .unwrap()
            .keys_file = key_path;

        SqliteRegistryStore::bootstrap_from_document(&path, document).unwrap();

        let connection = rusqlite::Connection::open(&path).unwrap();
        for table in [
            "registry_versions",
            "providers",
            "accounts",
            "credential_sets",
            "channels",
            "model_routes",
            "model_route_targets",
            "policy_profiles",
            "routing_profiles",
        ] {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM {table}"))
                .unwrap();
            let column_count = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    let mut values = Vec::new();
                    for index in 0..column_count {
                        let value: rusqlite::types::Value = row.get(index)?;
                        values.push(format!("{value:?}"));
                    }
                    Ok(values.join("\n"))
                })
                .unwrap();
            for row in rows {
                let text = row.unwrap();
                assert!(!text.contains(upstream_secret));
                assert!(!text.contains(fixture_values.client_token.as_str()));
                assert!(!text.contains(fixture_values.admin_token.as_str()));
            }
        }
    }
}
