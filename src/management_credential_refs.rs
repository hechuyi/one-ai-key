use crate::{
    credential_repository::{CredentialSetId, CredentialStoreHandle},
    credentials::CredentialId,
    management_errors::{credential_resource_store_error, ManagementServiceError},
};

const POSITION_CREDENTIAL_REF_PREFIX: &str = "cr:v1:pos:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialRef {
    pub position: usize,
}

pub fn credential_ref_for_position(position: usize) -> String {
    format!("{POSITION_CREDENTIAL_REF_PREFIX}{position}")
}

pub fn parse_credential_ref(value: &str) -> Result<Option<CredentialRef>, ManagementServiceError> {
    if !value.starts_with("cr:") {
        return Ok(None);
    }
    let Some(position) = value.strip_prefix(POSITION_CREDENTIAL_REF_PREFIX) else {
        return Err(invalid_credential_ref());
    };
    if position.is_empty()
        || (position.len() > 1 && position.starts_with('0'))
        || !position.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err(invalid_credential_ref());
    }
    let position = position
        .parse::<usize>()
        .map_err(|_| invalid_credential_ref())?;
    Ok(Some(CredentialRef { position }))
}

pub async fn resolve_credential_path_segment(
    credential_store: &CredentialStoreHandle,
    credential_set_id: &str,
    path_segment: String,
) -> Result<CredentialId, ManagementServiceError> {
    let Some(credential_ref) = parse_credential_ref(&path_segment)? else {
        return Ok(CredentialId(path_segment));
    };
    let credential_set_id = CredentialSetId(credential_set_id.to_string());
    let resource = credential_store
        .load_credential_resource_by_position(credential_set_id.clone(), credential_ref.position)
        .await
        .map_err(credential_resource_store_error)?
        .ok_or_else(|| {
            ManagementServiceError::NotFound(format!(
                "unknown credential_ref {} in credential_set {}",
                credential_ref_for_position(credential_ref.position),
                credential_set_id.0
            ))
        })?;
    Ok(resource.credential_id)
}

fn invalid_credential_ref() -> ManagementServiceError {
    ManagementServiceError::BadRequest(
        "credential_ref must match cr:v1:pos:<non-negative-position>".to_string(),
    )
}
