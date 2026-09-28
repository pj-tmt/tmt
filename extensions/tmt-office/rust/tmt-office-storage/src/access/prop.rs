//! Local prop catalog operations shared by companion execution and HTTP.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use tmt_office_model::codec::office_prop::*;
use tmt_office_model::office_protocol::{OfficeError, OfficeInvocation};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PropCommandInput {
    #[serde(default)]
    bytes: Option<String>,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    expected_revision: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    cursor: Option<String>,
}

pub fn execute(operation: OfficeInvocation, input: &[u8]) -> Vec<u8> {
    serde_json::to_vec(
        &execute_inner(operation, input).unwrap_or_else(|error| json!({"error":error.code()})),
    )
    .unwrap_or_else(|_| br#"{"error":"OFFICE_PROP_CORRUPT"}"#.to_vec())
}

fn execute_inner(operation: OfficeInvocation, input: &[u8]) -> Result<Value, OfficeError> {
    if input.len() > PROTOCOL_INPUT_LIMIT {
        return Err(OfficeError::PropInvalid);
    }
    let input: PropCommandInput =
        serde_json::from_slice(input).map_err(|_| OfficeError::PropInvalid)?;
    if operation == OfficeInvocation::LocalPropValidate {
        let candidate = command_pack(&input)?;
        return Ok(pack_projection(&candidate));
    }
    let paths = tmt_adapters::config::ConfigPaths::discover()
        .map_err(|_| OfficeError::CredentialsUnavailable)?;
    let mut storage = crate::OfficeStore::open_configured(&crate::StorageLayout::new(&paths))
        .map_err(storage_prop_error)?;
    let result = match operation {
        OfficeInvocation::LocalPropInstall => {
            let candidate = command_pack(&input)?;
            let revision = input.expected_revision.ok_or(OfficeError::PropInvalid)?;
            storage
                .install_local_prop_pack(revision, &candidate)
                .map(|mutation| mutation_projection(mutation, true))
                .map_err(catalog_error)
        }
        OfficeInvocation::LocalPropRemove => {
            let digest = input.digest.as_deref().ok_or(OfficeError::PropInvalid)?;
            let revision = input.expected_revision.ok_or(OfficeError::PropInvalid)?;
            storage
                .remove_local_prop_pack(revision, digest)
                .map(|mutation| mutation_projection(mutation, false))
                .map_err(catalog_error)
        }
        OfficeInvocation::LocalPropList => storage
            .list_local_prop_packs(input.limit.unwrap_or(20), input.cursor.as_deref())
            .map(list_projection)
            .map_err(catalog_error),
        OfficeInvocation::LocalPropShow => storage
            .show_local_prop_pack(input.digest.as_deref().ok_or(OfficeError::PropInvalid)?)
            .map(snapshot_projection)
            .map_err(catalog_error),
        _ => Err(OfficeError::CredentialsInvalid),
    };
    let close = storage.close().map_err(storage_prop_error);
    match (result, close) {
        (Err(error), _) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn command_pack(input: &PropCommandInput) -> Result<ValidatedPropPack, OfficeError> {
    let encoded = input.bytes.as_deref().ok_or(OfficeError::PropInvalid)?;
    if encoded.len() > ENCODED_INPUT_LIMIT {
        return Err(OfficeError::PropInvalid);
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| OfficeError::PropInvalid)?;
    validate_pack(&bytes).map_err(|_| OfficeError::PropInvalid)
}

fn snapshot_projection(snapshot: crate::LocalPropSnapshot) -> Value {
    let mut value = pack_projection(&snapshot.pack);
    value["builtin"] = json!(snapshot.builtin);
    value["catalogRevision"] = json!(snapshot.catalog_revision);
    value["installedAtMs"] = snapshot
        .installed_at_ms
        .map_or(Value::Null, |value| json!(value));
    value
}

fn mutation_projection(mutation: crate::LocalPropMutation, include_pack: bool) -> Value {
    if include_pack {
        let mut value = snapshot_projection(mutation.snapshot.expect("install returns a snapshot"));
        value["changed"] = json!(mutation.changed);
        value
    } else {
        json!({
            "digest":mutation.digest,
            "catalogRevision":mutation.catalog_revision,
            "changed":mutation.changed
        })
    }
}

fn list_projection(list: crate::LocalPropCatalogList) -> Value {
    json!({
        "catalogRevision":list.catalog_revision,
        "builtins":list.builtins.into_iter().map(snapshot_projection).collect::<Vec<_>>(),
        "packs":list.packs.into_iter().map(snapshot_projection).collect::<Vec<_>>(),
        "excluded":list.excluded.into_iter().map(|excluded| json!({
            "digest":excluded.digest,
            "reason":excluded.reason.code()
        })).collect::<Vec<_>>(),
        "nextCursor":list.next_cursor
    })
}

/// Stable catalog errors shared by CLI/companion and local browser adapters.
pub fn catalog_error(error: crate::LocalPropCatalogError) -> OfficeError {
    use crate::LocalPropCatalogError as Error;
    match error {
        Error::Invalid => OfficeError::PropInvalid,
        Error::Corrupt => OfficeError::PropCorrupt,
        Error::NotFound => OfficeError::PropNotFound,
        Error::Limit => OfficeError::PropLimit,
        Error::Builtin => OfficeError::PropBuiltin,
        Error::RevisionConflict | Error::RevisionExhausted => OfficeError::CatalogRevisionConflict,
        Error::CursorInvalid => OfficeError::CatalogCursorInvalid,
        Error::CursorStale => OfficeError::CatalogCursorStale,
        Error::Storage(error) => storage_prop_error(error),
    }
}

fn storage_prop_error(error: impl std::error::Error) -> OfficeError {
    let _ = error;
    OfficeError::CredentialsUnavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRECTIONAL_SAMPLE: &[u8] =
        include_bytes!("../../../../contracts/prop-pack-v2-sample.tmtprop.json");

    #[test]
    fn version_specific_bytes_and_base64_envelope_are_bounded() {
        let mut bytes = DIRECTIONAL_SAMPLE.to_vec();
        bytes.resize(PACK_INPUT_LIMIT, b' ');
        let pack = validate_pack(&bytes).unwrap();
        let input = command_pack_input(&pack);
        assert_eq!(input["bytes"].as_str().unwrap().len(), ENCODED_INPUT_LIMIT);
        let envelope = serde_json::to_vec(&input).unwrap();
        assert!(envelope.len() < PROTOCOL_INPUT_LIMIT);
        assert!(execute_inner(OfficeInvocation::LocalPropValidate, &envelope).is_ok());
        bytes.push(b' ');
        assert_eq!(validate_pack(&bytes), Err(PropPackError::TooLarge));
        let mut legacy = BUILTIN_BYTES.to_vec();
        legacy.resize(V1_PACK_INPUT_LIMIT + 1, b' ');
        assert_eq!(validate_pack(&legacy), Err(PropPackError::TooLarge));
    }
}
