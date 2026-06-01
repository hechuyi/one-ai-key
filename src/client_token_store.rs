use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::config::{hash_token, stable_id, ResolvedClientToken};

#[derive(Debug, Clone)]
pub enum ClientTokenStoreHandle {
    ReadOnlyBootstrap,
    Sqlite(Arc<SqliteClientTokenStore>),
}

#[derive(Debug)]
pub struct SqliteClientTokenStore {
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientTokenStoreError {
    NotWritable,
    Duplicate,
    NotFound,
    Persistence(String),
}

#[derive(Debug, Clone)]
pub struct ClientTokenCreate {
    pub name: String,
    pub token: String,
    pub allowed_model_groups: Vec<String>,
    pub allowed_channels: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClientTokenScopeUpdate {
    pub allowed_model_groups: Option<Vec<String>>,
    pub allowed_channels: Option<Vec<String>>,
}

impl ClientTokenStoreHandle {
    pub fn read_only_bootstrap() -> Self {
        Self::ReadOnlyBootstrap
    }

    pub fn sqlite(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let store = SqliteClientTokenStore { path };
        store.initialize()?;
        Ok(Self::Sqlite(Arc::new(store)))
    }

    pub fn load_or_bootstrap(
        &self,
        bootstrap_tokens: &[ResolvedClientToken],
    ) -> Result<Vec<ResolvedClientToken>, ClientTokenStoreError> {
        match self {
            ClientTokenStoreHandle::ReadOnlyBootstrap => Ok(bootstrap_tokens.to_vec()),
            ClientTokenStoreHandle::Sqlite(store) => store.load_or_bootstrap(bootstrap_tokens),
        }
    }

    pub async fn create_token(
        &self,
        create: ClientTokenCreate,
    ) -> Result<ResolvedClientToken, ClientTokenStoreError> {
        match self {
            ClientTokenStoreHandle::ReadOnlyBootstrap => Err(ClientTokenStoreError::NotWritable),
            ClientTokenStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || store.create_token(create))
                    .await
                    .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?
            }
        }
    }

    pub async fn set_enabled(
        &self,
        token_id: String,
        enabled: bool,
    ) -> Result<ResolvedClientToken, ClientTokenStoreError> {
        match self {
            ClientTokenStoreHandle::ReadOnlyBootstrap => Err(ClientTokenStoreError::NotWritable),
            ClientTokenStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || store.set_enabled(&token_id, enabled))
                    .await
                    .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?
            }
        }
    }

    pub async fn update_scope(
        &self,
        token_id: String,
        update: ClientTokenScopeUpdate,
    ) -> Result<ResolvedClientToken, ClientTokenStoreError> {
        match self {
            ClientTokenStoreHandle::ReadOnlyBootstrap => Err(ClientTokenStoreError::NotWritable),
            ClientTokenStoreHandle::Sqlite(store) => {
                let store = store.clone();
                tokio::task::spawn_blocking(move || store.update_scope(&token_id, update))
                    .await
                    .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?
            }
        }
    }
}

impl SqliteClientTokenStore {
    fn connection(&self) -> Result<Connection, ClientTokenStoreError> {
        Connection::open(&self.path)
            .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))
    }

    fn initialize(&self) -> anyhow::Result<()> {
        let connection = Connection::open(&self.path)?;
        migrate_client_token_schema(&connection)?;
        Ok(())
    }

    fn load_or_bootstrap(
        &self,
        bootstrap_tokens: &[ResolvedClientToken],
    ) -> Result<Vec<ResolvedClientToken>, ClientTokenStoreError> {
        let mut connection = self.connection()?;
        migrate_client_token_schema(&connection)
            .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM client_tokens", [], |row| row.get(0))
            .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
        if count == 0 {
            let tx = connection
                .transaction()
                .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
            for token in bootstrap_tokens {
                insert_resolved_token_tx(&tx, token)
                    .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
            }
            tx.commit()
                .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
        }
        load_tokens(&connection)
    }

    fn create_token(
        &self,
        create: ClientTokenCreate,
    ) -> Result<ResolvedClientToken, ClientTokenStoreError> {
        let connection = self.connection()?;
        migrate_client_token_schema(&connection)
            .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
        let name = create.name.trim();
        if name.is_empty() || create.token.trim().is_empty() {
            return Err(ClientTokenStoreError::Persistence(
                "client token name and token must not be empty".to_string(),
            ));
        }
        let token = ResolvedClientToken {
            id: stable_id("client", name),
            name: name.to_string(),
            token_hash: hash_token(create.token.trim()),
            enabled: true,
            allowed_model_groups: create.allowed_model_groups,
            allowed_channels: create.allowed_channels,
        };
        insert_resolved_token(&connection, &token).map_err(|err| {
            if is_sqlite_unique_constraint(&err) {
                ClientTokenStoreError::Duplicate
            } else {
                ClientTokenStoreError::Persistence(err.to_string())
            }
        })?;
        Ok(token)
    }

    fn set_enabled(
        &self,
        token_id: &str,
        enabled: bool,
    ) -> Result<ResolvedClientToken, ClientTokenStoreError> {
        let connection = self.connection()?;
        migrate_client_token_schema(&connection)
            .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
        let changed = connection
            .execute(
                "UPDATE client_tokens
                 SET enabled = ?2, updated_at_unix_seconds = ?3
                 WHERE id = ?1",
                params![token_id, enabled as i64, now_unix_seconds()],
            )
            .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
        if changed == 0 {
            return Err(ClientTokenStoreError::NotFound);
        }
        load_token_by_id(&connection, token_id)?.ok_or(ClientTokenStoreError::NotFound)
    }

    fn update_scope(
        &self,
        token_id: &str,
        update: ClientTokenScopeUpdate,
    ) -> Result<ResolvedClientToken, ClientTokenStoreError> {
        let connection = self.connection()?;
        migrate_client_token_schema(&connection)
            .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
        let Some(mut token) = load_token_by_id(&connection, token_id)? else {
            return Err(ClientTokenStoreError::NotFound);
        };
        if let Some(allowed_model_groups) = update.allowed_model_groups {
            token.allowed_model_groups = allowed_model_groups;
        }
        if let Some(allowed_channels) = update.allowed_channels {
            token.allowed_channels = allowed_channels;
        }
        connection
            .execute(
                "UPDATE client_tokens
                 SET allowed_model_groups_json = ?2,
                     allowed_channels_json = ?3,
                     updated_at_unix_seconds = ?4
                 WHERE id = ?1",
                params![
                    token_id,
                    serde_json::to_string(&token.allowed_model_groups)
                        .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?,
                    serde_json::to_string(&token.allowed_channels)
                        .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?,
                    now_unix_seconds(),
                ],
            )
            .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
        load_token_by_id(&connection, token_id)?.ok_or(ClientTokenStoreError::NotFound)
    }
}

fn migrate_client_token_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS client_tokens (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            token_hash TEXT NOT NULL UNIQUE,
            enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
            allowed_model_groups_json TEXT NOT NULL,
            allowed_channels_json TEXT NOT NULL,
            created_at_unix_seconds INTEGER NOT NULL,
            updated_at_unix_seconds INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_client_tokens_enabled ON client_tokens(enabled);
        ",
    )
}

fn insert_resolved_token(
    connection: &Connection,
    token: &ResolvedClientToken,
) -> rusqlite::Result<()> {
    let now = now_unix_seconds();
    connection.execute(
        "INSERT INTO client_tokens (
             id, name, token_hash, enabled, allowed_model_groups_json,
             allowed_channels_json, created_at_unix_seconds, updated_at_unix_seconds
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            &token.id,
            &token.name,
            &token.token_hash,
            token.enabled as i64,
            serde_json::to_string(&token.allowed_model_groups)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?,
            serde_json::to_string(&token.allowed_channels)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?,
            now,
            now,
        ],
    )?;
    Ok(())
}

fn insert_resolved_token_tx(
    transaction: &Transaction<'_>,
    token: &ResolvedClientToken,
) -> rusqlite::Result<()> {
    let now = now_unix_seconds();
    transaction.execute(
        "INSERT INTO client_tokens (
             id, name, token_hash, enabled, allowed_model_groups_json,
             allowed_channels_json, created_at_unix_seconds, updated_at_unix_seconds
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            &token.id,
            &token.name,
            &token.token_hash,
            token.enabled as i64,
            serde_json::to_string(&token.allowed_model_groups)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?,
            serde_json::to_string(&token.allowed_channels)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?,
            now,
            now,
        ],
    )?;
    Ok(())
}

fn load_tokens(connection: &Connection) -> Result<Vec<ResolvedClientToken>, ClientTokenStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT id, name, token_hash, enabled, allowed_model_groups_json, allowed_channels_json
             FROM client_tokens
             ORDER BY name, id",
        )
        .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
    let rows = statement
        .query_map([], row_to_client_token)
        .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))
}

fn load_token_by_id(
    connection: &Connection,
    token_id: &str,
) -> Result<Option<ResolvedClientToken>, ClientTokenStoreError> {
    connection
        .query_row(
            "SELECT id, name, token_hash, enabled, allowed_model_groups_json, allowed_channels_json
             FROM client_tokens
             WHERE id = ?1",
            params![token_id],
            row_to_client_token,
        )
        .optional()
        .map_err(|err| ClientTokenStoreError::Persistence(err.to_string()))
}

fn row_to_client_token(row: &rusqlite::Row<'_>) -> rusqlite::Result<ResolvedClientToken> {
    let allowed_model_groups_json: String = row.get(4)?;
    let allowed_channels_json: String = row.get(5)?;
    Ok(ResolvedClientToken {
        id: row.get(0)?,
        name: row.get(1)?,
        token_hash: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
        allowed_model_groups: serde_json::from_str(&allowed_model_groups_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(err))
        })?,
        allowed_channels: serde_json::from_str(&allowed_channels_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(err))
        })?,
    })
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn is_sqlite_unique_constraint(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(code, _)
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                || code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
    )
}
