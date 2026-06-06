use crate::{
    client_token_store::ClientTokenStoreError, credential_repository::CredentialStoreError,
    registry_store::RegistryStoreError,
};

#[derive(Debug)]
pub enum ManagementServiceError {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    PreconditionFailed(String),
    Persistence(String),
    EventAppendFailed {
        message: String,
        stale_history_id: Option<i64>,
    },
}

impl ManagementServiceError {
    pub fn into_public_error(self) -> Self {
        match self {
            ManagementServiceError::EventAppendFailed { message, .. } => {
                ManagementServiceError::Persistence(format!(
                    "failed to record management event: {message}"
                ))
            }
            other => other,
        }
    }

    pub fn records_runtime_reload_failure(&self) -> bool {
        !matches!(self, ManagementServiceError::PreconditionFailed(_))
    }
}

pub fn credential_store_error(err: CredentialStoreError) -> ManagementServiceError {
    match err {
        CredentialStoreError::NotWritable => ManagementServiceError::Conflict(
            "credential import requires a writable credential store".to_string(),
        ),
        CredentialStoreError::Persistence(_) => {
            ManagementServiceError::Persistence("credential store persistence error".to_string())
        }
    }
}

pub fn credential_lifecycle_store_error(err: CredentialStoreError) -> ManagementServiceError {
    match err {
        CredentialStoreError::NotWritable => ManagementServiceError::Conflict(
            "credential lifecycle persistence requires a writable credential store".to_string(),
        ),
        CredentialStoreError::Persistence(_) => {
            ManagementServiceError::Persistence("credential store persistence error".to_string())
        }
    }
}

pub fn credential_resource_store_error(err: CredentialStoreError) -> ManagementServiceError {
    match err {
        CredentialStoreError::NotWritable => ManagementServiceError::Conflict(
            "credential resource persistence requires a writable credential store".to_string(),
        ),
        CredentialStoreError::Persistence(_) => {
            ManagementServiceError::Persistence("credential store persistence error".to_string())
        }
    }
}

pub fn client_token_store_error(err: ClientTokenStoreError) -> ManagementServiceError {
    match err {
        ClientTokenStoreError::NotWritable => ManagementServiceError::Conflict(
            "client token persistence requires a writable credential store".to_string(),
        ),
        ClientTokenStoreError::Duplicate => {
            ManagementServiceError::Conflict("client token already exists".to_string())
        }
        ClientTokenStoreError::NotFound => {
            ManagementServiceError::NotFound("unknown client token".to_string())
        }
        ClientTokenStoreError::Persistence(_) => {
            ManagementServiceError::Persistence("client token store persistence error".to_string())
        }
    }
}

pub fn registry_store_error(err: RegistryStoreError) -> ManagementServiceError {
    match err {
        RegistryStoreError::NotWritable => ManagementServiceError::Conflict(
            "registry persistence requires a writable registry store".to_string(),
        ),
        RegistryStoreError::Conflict(message) if message.starts_with("unknown ") => {
            ManagementServiceError::NotFound(message)
        }
        RegistryStoreError::Conflict(message) | RegistryStoreError::Validation(message) => {
            ManagementServiceError::Conflict(message)
        }
        RegistryStoreError::Persistence(_) => {
            ManagementServiceError::Persistence("registry store persistence error".to_string())
        }
    }
}
