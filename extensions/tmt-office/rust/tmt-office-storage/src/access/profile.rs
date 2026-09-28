//! Local profile storage operations shared by companion execution and HTTP.

use serde::Deserialize;
use serde_json::{Value, json};
use tmt_office_model::office_profile::MAX_REVISION;
use tmt_office_model::office_protocol::OfficeError;
use tmt_office_model::office_protocol::OfficeInvocation;

use crate::{LocalProfileError, LocalProfileMutation, LocalProfileSnapshot, OfficeStore};
use tmt_adapters::config::ConfigPaths;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalProfileInput {
    identity_id: String,
    #[serde(default)]
    profile: Option<Value>,
    #[serde(default)]
    expected_revision: Option<u64>,
}

pub fn execute(operation: OfficeInvocation, input: &[u8]) -> Vec<u8> {
    serde_json::to_vec(
        &execute_inner(operation, input).unwrap_or_else(|error| json!({"error":error.code()})),
    )
    .unwrap_or_else(|_| br#"{"error":"OFFICE_CREDENTIALS_INVALID"}"#.to_vec())
}

fn execute_inner(operation: OfficeInvocation, input: &[u8]) -> Result<Value, OfficeError> {
    if input.len() > 4096 {
        return Err(OfficeError::ProfileInvalid);
    }
    let input: LocalProfileInput =
        serde_json::from_slice(input).map_err(|_| OfficeError::ProfileInvalid)?;
    let paths = ConfigPaths::discover().map_err(|_| OfficeError::CredentialsUnavailable)?;
    let mut storage =
        OfficeStore::open_configured(&crate::StorageLayout::new(&paths)).map_err(storage_error)?;
    let result = match operation {
        OfficeInvocation::LocalProfileShow
            if input.profile.is_none() && input.expected_revision.is_none() =>
        {
            storage
                .show_local_profile(&input.identity_id)
                .map(snapshot_value)
                .map_err(profile_error)
        }
        OfficeInvocation::LocalProfileApply => {
            let profile = tmt_office_model::codec::office_profile_wire::decode_value(
                input.profile.ok_or(OfficeError::ProfileInvalid)?,
            )
            .map_err(|_| OfficeError::ProfileInvalid)?;
            let revision = input
                .expected_revision
                .filter(|value| *value <= MAX_REVISION)
                .ok_or(OfficeError::ProfileInvalid)?;
            storage
                .apply_local_profile(&input.identity_id, revision, &profile)
                .map(mutation_value)
                .map_err(profile_error)
        }
        _ => Err(OfficeError::CredentialsInvalid),
    };
    let close = storage.close().map_err(storage_error);
    match (result, close) {
        (Err(error), _) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
    }
}

pub fn snapshot_value(snapshot: LocalProfileSnapshot) -> Value {
    json!({
        "identityId": snapshot.identity_id,
        "identityName": snapshot.identity_name,
        "exists": snapshot.exists,
        "revision": snapshot.revision,
        "profile": tmt_office_model::codec::office_profile_wire::encode_value(&snapshot.profile),
        "updatedAtMs": snapshot.updated_at_ms,
        "catalog": {
            "hairStyles": tmt_office_model::office_profile::HAIR_STYLES,
            "hairColors": tmt_office_model::office_profile::HAIR_COLORS,
            "skinTones": tmt_office_model::office_profile::SKIN_TONES,
            "shirtColors": tmt_office_model::office_profile::SHIRT_COLORS,
        }
    })
}

pub fn mutation_value(mutation: LocalProfileMutation) -> Value {
    let mut value = snapshot_value(mutation.snapshot);
    value["changed"] = json!(mutation.changed);
    value
}

fn profile_error(error: LocalProfileError) -> OfficeError {
    match error {
        LocalProfileError::IdentityInactive => OfficeError::IdentityInactive,
        LocalProfileError::RevisionConflict | LocalProfileError::RevisionExhausted => {
            OfficeError::RevisionConflict
        }
        LocalProfileError::ProfileInvalid => OfficeError::ProfileInvalid,
        LocalProfileError::AvatarUnavailable => OfficeError::AvatarNotFound,
        LocalProfileError::StoredProfileInvalid => OfficeError::CredentialsInvalid,
        LocalProfileError::Storage(error) => storage_error(error),
    }
}
fn storage_error(error: impl std::error::Error) -> OfficeError {
    let _ = error;
    OfficeError::CredentialsUnavailable
}
