//! Canonical local profile JSON boundary shared by CLI and companion execution.

use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
use tmt_office_model::office_profile::LocalProfile;
use tmt_office_model::office_profile::MAX_PROFILE_FILE_BYTES;
use tmt_office_model::office_profile::MAX_REVISION;
use tmt_office_model::office_protocol::OfficeError;
use tmt_office_model::office_protocol::OfficeInvocation;

use crate::{
    config::ConfigPaths,
    storage::{LocalProfileError, LocalProfileMutation, LocalProfileSnapshot, Storage},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalProfileInput {
    identity_id: String,
    #[serde(default)]
    profile: Option<Value>,
    #[serde(default)]
    expected_revision: Option<u64>,
}

pub fn read_profile_file(path: &Path) -> Result<LocalProfile, OfficeError> {
    let bytes = crate::bounded_file::read(path, MAX_PROFILE_FILE_BYTES)
        .map_err(|_| OfficeError::ProfileInvalid)?;
    tmt_office_model::codec::office_profile_wire::decode_slice(&bytes)
        .map_err(|_| OfficeError::ProfileInvalid)
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
    let mut storage = Storage::open(paths.database).map_err(storage_error)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    #[test]
    fn profile_file_is_exact_bounded_and_never_rewritten() {
        let directory = TestDirectory::new();
        let file = directory.path.join("profile.json");
        let value = br#"{"displayLabel":"","description":"Architecture review","appearance":{"hairStyle":"short","hairColor":"ink","skinTone":"medium","shirtColor":"blue","shirtMark":"AI"}}"#;
        std::fs::write(&file, value).unwrap();
        assert_eq!(
            read_profile_file(&file).unwrap().description,
            "Architecture review"
        );
        assert_eq!(std::fs::read(&file).unwrap(), value);

        let mut oversized = value.to_vec();
        oversized.resize(MAX_PROFILE_FILE_BYTES + 1, b' ');
        std::fs::write(&file, &oversized).unwrap();
        assert_eq!(read_profile_file(&file), Err(OfficeError::ProfileInvalid));

        std::fs::write(&file, br#"{"displayLabel":"","description":"","appearance":{"hairStyle":"short","hairColor":"ink","skinTone":"medium","shirtColor":"blue","shirtMark":""},"asset":"https://example.test/avatar"}"#).unwrap();
        assert_eq!(read_profile_file(&file), Err(OfficeError::ProfileInvalid));
    }
}
