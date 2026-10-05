//! Bounded identity metadata/status admission and public operation composition.

use super::{Fault, REFERENCE_LIMIT, Request, invalid};
use crate::storage::Storage;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdentityStatusesInput {
    identity_ids: Vec<String>,
}

pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    Ok({
        let value: IdentityStatusesInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        if value.identity_ids.len() > REFERENCE_LIMIT
            || !value
                .identity_ids
                .iter()
                .all(|id| tmt_core::dispatch::canonical_id(id))
        {
            return Err(invalid());
        }
        Request::IdentityStatuses {
            identities: value.identity_ids,
        }
    })
}

pub(super) fn status(storage: &Storage, identities: Vec<String>) -> Result<Vec<u8>, Fault> {
    // One directory read; expiry (`stale`) is applied here, by core.
    let statuses = storage
        .list_active_identity_statuses()
        .map_err(|_| Fault::unavailable())?;
    let now = crate::request_runtime::wall_time_ms();
    let mut entries = Vec::with_capacity(identities.len());
    for id in &identities {
        entries.push(
            match storage
                .find_identity_by_id(id)
                .map_err(|_| Fault::unavailable())?
            {
                None => json!({"id": id, "found": false}),
                Some(_) => json!({"id": id, "found": true,
                    "status": crate::identity_status::status_value(statuses.get(id), now)}),
            },
        );
    }
    Ok(serde_json::to_vec(&json!({"identities": entries})).expect("identity statuses"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MetadataApplyInput {
    identity_id: String,
    changes: Box<serde_json::value::RawValue>,
}

pub(super) fn decode_metadata(input: &[u8]) -> Result<Request, Fault> {
    let wire: MetadataApplyInput = serde_json::from_slice(input).map_err(|_| invalid())?;
    if !tmt_core::dispatch::canonical_id(&wire.identity_id) {
        return Err(invalid());
    }
    let changes =
        crate::identity_metadata::decode_changes(wire.changes.get()).map_err(|_| invalid())?;
    Ok(Request::MetadataApply {
        identity_id: wire.identity_id,
        changes,
    })
}

pub(super) fn apply_metadata(
    storage: &mut Storage,
    identity_id: &str,
    changes: &tmt_core::identity_metadata::MetadataChanges,
) -> Result<Vec<u8>, Fault> {
    use tmt_core::identity_metadata::{MetadataError, apply_identity_metadata};
    let result =
        apply_identity_metadata(storage, identity_id, changes).map_err(|error| match error {
            MetadataError::Conflict(current) => Fault::new(
                "METADATA_CONFLICT",
                "Metadata expectations did not match current values; no changes were applied.",
            )
            .with_metadata_conflicts(current),
            MetadataError::IdentityNotFound => {
                Fault::new("NAME_NOT_FOUND", "The selected identity was not found.")
            }
            MetadataError::Invalid(error) => {
                Fault::detailed("IDENTITY_METADATA_INVALID", error.to_string())
            }
            MetadataError::Repository(_) => Fault::unavailable(),
            MetadataError::KeyNotFound => unreachable!("apply has no single-key lookup"),
        })?;
    serde_json::to_vec(&crate::identity_metadata::applied_value(&result))
        .map_err(|_| Fault::unavailable())
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use crate::{api, config::ConfigPaths, test_support::TestDirectory};
    use tmt_core::{
        binding::BindingRepository,
        identity::{Lifetime, create_or_resolve},
        identity_metadata::IdentityMetadataRepository,
    };

    #[test]
    fn metadata_api_checks_admission_active_uuid_and_structured_conflict() {
        let directory = TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path),
            None,
        );
        let mut storage = Storage::open(&paths.database).unwrap();
        let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
            .unwrap()
            .identity;
        storage.close().unwrap();
        let call = |id: &str, changes: serde_json::Value| {
            let request = api::decode(&json!({"version":1,"operation":"identity.meta.apply","input":{"identityId":id,"changes":changes}}).to_string())?;
            api::execute(&paths, request)
        };
        let change = json!([{"key":"state","expect":"absent","then":{"set":"working"}}]);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &call(&identity.id, change.clone()).unwrap()
            )
            .unwrap(),
            json!({"identityId":identity.id,"changed":true})
        );
        let error = call(&identity.id, change.clone()).unwrap_err();
        assert_eq!(error.status(), 5);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&error.encode()).unwrap()["error"]["current"],
            json!({"state":"working"})
        );
        assert_eq!(error.code(), "METADATA_CONFLICT");
        assert_eq!(
            call("Alice", change.clone()).unwrap_err().code(),
            "API_INPUT_INVALID"
        );
        let mut storage = Storage::open(&paths.database).unwrap();
        storage
            .with_binding_transaction(|rows| rows.retire_identity(&identity, false))
            .unwrap();
        let replacement = create_or_resolve(&mut storage, &identity.id, Lifetime::Saved)
            .unwrap()
            .identity;
        storage.close().unwrap();
        assert_eq!(
            call(&identity.id, change).unwrap_err().code(),
            "NAME_NOT_FOUND"
        );
        let storage = Storage::open(&paths.database).unwrap();
        assert_eq!(
            storage.list_metadata(&replacement.id).unwrap(),
            tmt_core::identity_metadata::MetadataCollection::Found(Default::default())
        );
        for body in [
            r#"{"version":1,"operation":"identity.meta.apply","identity":"Alice","input":{"identityId":"11111111-1111-4111-8111-111111111111","changes":[]}}"#,
            r#"{"version":1,"operation":"identity.meta.apply","originator":"anonymous","input":{"identityId":"11111111-1111-4111-8111-111111111111","changes":[]}}"#,
            r#"{"version":1,"operation":"identity.meta.apply","input":{"identityId":"11111111-1111-4111-8111-111111111111","changes":[],"extra":true}}"#,
        ] {
            assert!(api::decode(body).is_err());
        }
        assert!(serde_json::from_slice::<serde_json::Value>(&api::capabilities()).unwrap()["operations"].as_array().unwrap().contains(&json!("identity.meta.apply")));
    }
}
