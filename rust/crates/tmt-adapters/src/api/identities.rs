//! Bounded identity rename/metadata/status admission and public composition.

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
                Some(_) => {
                    let mut value = json!({"id": id, "found": true,
                    "status": crate::identity_status::status_value(statuses.get(id), now),
                    "canPassthrough": crate::passthrough::can_identity(storage, id)
                        .map_err(|_| Fault::unavailable())?
                        // A retired identity has no pane to type into.
                        .unwrap_or(crate::passthrough::Can::no(crate::passthrough::Reason::NoPane))
                        .to_json()});
                    // The binding runs a plain program, not an agent: it receives nothing.
                    if let Some(program) = storage
                        .bound_plain_program(id)
                        .map_err(|_| Fault::unavailable())?
                    {
                        value["plainProgram"] = program.into();
                    }
                    value
                }
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RenameInput {
    identity_id: String,
    expected_name: String,
    name: String,
}

pub(super) fn decode_rename(input: &[u8]) -> Result<Request, Fault> {
    let wire: RenameInput = serde_json::from_slice(input).map_err(|_| invalid())?;
    if !tmt_core::dispatch::canonical_id(&wire.identity_id) {
        return Err(invalid());
    }
    Ok(Request::IdentityRename {
        identity_id: wire.identity_id,
        expected_name: wire.expected_name,
        name: wire.name,
    })
}

pub(super) fn rename(
    storage: &mut Storage,
    identity_id: &str,
    expected_name: &str,
    name: &str,
) -> Result<Vec<u8>, Fault> {
    use tmt_core::binding::{BindingError, RenameOutcome, rename_identity_by_id};
    let result = rename_identity_by_id::<_, std::convert::Infallible>(
        storage,
        identity_id,
        expected_name,
        name,
    )
    .map_err(|error| match error {
        BindingError::InvalidName(error) => Fault::detailed("INVALID_NAME", error.to_string()),
        BindingError::NameTaken(name) => Fault::detailed(
            "NAME_ALREADY_ACTIVE",
            format!("Another identity is already named '{name}'."),
        ),
        _ => Fault::unavailable(),
    })?;
    let value = match result {
        RenameOutcome::Applied(result) => json!({"outcome":"applied", "identityId":identity_id,
            "name":result.identity.name, "changed":result.changed()}),
        RenameOutcome::Conflict(identity) => json!({"outcome":"conflict", "identityId":identity_id,
            "name":identity.name}),
        RenameOutcome::NotFound => json!({"outcome":"not-found", "identityId":identity_id}),
    };
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StopInput {
    identity_id: String,
    expected_name: String,
    close: bool,
}

pub(super) fn decode_stop(input: &[u8]) -> Result<Request, Fault> {
    let wire: StopInput = serde_json::from_slice(input).map_err(|_| invalid())?;
    if !tmt_core::dispatch::canonical_id(&wire.identity_id) {
        return Err(invalid());
    }
    Ok(Request::IdentityStop {
        identity_id: wire.identity_id,
        expected_name: wire.expected_name,
        close: wire.close,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoveInput {
    identity_id: String,
    expected_name: String,
}

pub(super) fn decode_remove(input: &[u8]) -> Result<Request, Fault> {
    let wire: RemoveInput = serde_json::from_slice(input).map_err(|_| invalid())?;
    if !tmt_core::dispatch::canonical_id(&wire.identity_id) {
        return Err(invalid());
    }
    Ok(Request::IdentityRemove {
        identity_id: wire.identity_id,
        expected_name: wire.expected_name,
    })
}

/// `identity.stop`: the verified stop behind `tmt stop`. Every fault means the
/// agent may still be running; a kept pane is still a stop.
pub(super) fn stop(
    storage: &mut Storage,
    paths: &crate::config::ConfigPaths,
    identity_id: &str,
    expected_name: &str,
    close: bool,
) -> Result<Vec<u8>, Fault> {
    use crate::verified_stop::{PaneOutcome, StopOutcome, stop_identity};
    // A retired UUID is not found, as in `identity.remove`.
    let Some(identity) = storage
        .find_active_identity_by_id(identity_id)
        .map_err(|_| Fault::unavailable())?
    else {
        return outcome(json!({"outcome":"not-found", "identityId":identity_id}));
    };
    if identity.name != expected_name {
        return outcome(
            json!({"outcome":"conflict", "identityId":identity_id, "name":identity.name}),
        );
    }
    let stopped = stop_identity(storage, paths, &identity, close).map_err(|refusal| {
        let fault = match refusal.cause.as_deref() {
            Some(crate::verified_stop::StopCause::Storage(_)) => Fault::unavailable(),
            _ => Fault::detailed(refusal.code, refusal.reason),
        };
        fault.with_signalled(refusal.signalled)
    })?;
    let mut value = json!({
        "outcome": match stopped.outcome {
            StopOutcome::Stopped => "stopped",
            StopOutcome::AlreadyEnded => "already-ended",
            StopOutcome::NotRunning => "not-running",
        },
        "identityId": identity_id,
        "name": identity.name,
    });
    match stopped.pane {
        None => (),
        Some(PaneOutcome::Closed) => value["pane"] = json!("closed"),
        Some(PaneOutcome::Gone) => value["pane"] = json!("gone"),
        Some(PaneOutcome::Kept(kept)) => {
            value["pane"] = json!("kept");
            value["paneReason"] = json!(kept.reason(&identity.name));
        }
    }
    outcome(value)
}

/// `identity.remove`: the guarded removal behind `tmt rm`, without the Saved
/// confirmation; the caller's own confirmation is the consent.
pub(super) fn remove(
    storage: &mut Storage,
    identity_id: &str,
    expected_name: &str,
) -> Result<Vec<u8>, Fault> {
    use tmt_core::binding::{BindingError, RemoveOutcome};
    let result = crate::verified_stop::remove_by_id(storage, identity_id, expected_name).map_err(
        |error| match &error {
            BindingError::AgentRunning(..) => Fault::detailed("AGENT_RUNNING", error.to_string()),
            BindingError::Changed => Fault::detailed("IDENTITY_CONFLICT", error.to_string()),
            BindingError::Endpoint(host) if host.socket_permission_denied() => Fault::new(
                "TMUX_PERMISSION_DENIED",
                "TMT cannot access the tmux socket. Nothing was removed.",
            ),
            BindingError::Endpoint(_) | BindingError::Unverified | BindingError::Deadline => {
                Fault::detailed("RECONCILIATION_FAILED", error.to_string())
            }
            _ => Fault::unavailable(),
        },
    )?;
    outcome(match result {
        RemoveOutcome::Removed(entry) => json!({"outcome":"removed", "identityId":identity_id,
            "name":entry.identity.name}),
        RemoveOutcome::Conflict(identity) => json!({"outcome":"conflict", "identityId":identity_id,
            "name":identity.name}),
        RemoveOutcome::NotFound => json!({"outcome":"not-found", "identityId":identity_id}),
    })
}

fn outcome(value: serde_json::Value) -> Result<Vec<u8>, Fault> {
    serde_json::to_vec(&value).map_err(|_| Fault::unavailable())
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

#[cfg(test)]
mod rename_tests {
    use super::*;
    use crate::{api, config::ConfigPaths, test_support::TestDirectory};
    use tmt_core::{
        binding::BindingRepository,
        identity::{Lifetime, create_or_resolve},
    };

    #[test]
    fn rename_api_pins_cas_outcomes_errors_and_exact_uuid_selection() {
        let directory = TestDirectory::new();
        let paths = ConfigPaths::resolve(
            &directory.path,
            &directory.path,
            Some(&directory.path),
            None,
        );
        let mut storage = Storage::open(&paths.database).unwrap();
        let alice = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
            .unwrap()
            .identity;
        create_or_resolve(&mut storage, "Bob", Lifetime::Saved).unwrap();
        storage.close().unwrap();
        let call = |id: &str, expected: &str, name: &str| {
            let request = api::decode(&json!({"version":1,"operation":"identity.rename","input":{"identityId":id,"expectedName":expected,"name":name}}).to_string())?;
            api::execute(&paths, request)
        };
        let resource =
            |bytes: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
        assert_eq!(
            resource(call(&alice.id, "Alice", "Ada").unwrap()),
            json!({"outcome":"applied","identityId":alice.id,"name":"Ada","changed":true})
        );
        assert_eq!(
            resource(call(&alice.id, "Alice", "Eve").unwrap()),
            json!({"outcome":"conflict","identityId":alice.id,"name":"Ada"})
        );
        assert_eq!(
            resource(call(&alice.id, "Ada", "Ada").unwrap()),
            json!({"outcome":"applied","identityId":alice.id,"name":"Ada","changed":false})
        );
        for (name, code) in [
            ("BOB", "NAME_ALREADY_ACTIVE"),
            ("%1", "INVALID_NAME"),
            ("", "INVALID_NAME"),
        ] {
            let fault = call(&alice.id, "Ada", name).unwrap_err();
            assert_eq!(fault.code(), code);
            assert_eq!(fault.status(), 1);
            assert_eq!(resource(fault.encode())["error"]["code"], code);
        }
        let mut storage = Storage::open(&paths.database).unwrap();
        let ada = storage.find_identity_by_id(&alice.id).unwrap().unwrap();
        assert_eq!(ada.name, "Ada");
        storage
            .with_binding_transaction(|rows| rows.retire_identity(&ada, false))
            .unwrap();
        let successor = create_or_resolve(&mut storage, &alice.id, Lifetime::Saved)
            .unwrap()
            .identity;
        storage.close().unwrap();
        assert_eq!(
            resource(call(&alice.id, "Ada", "Eve").unwrap()),
            json!({"outcome":"not-found","identityId":alice.id})
        );
        let mut storage = Storage::open(&paths.database).unwrap();
        assert_eq!(
            storage.find_identity_by_id(&successor.id).unwrap(),
            Some(successor)
        );
        storage.close().unwrap();
        for input in [
            json!({"identityId":"Alice","expectedName":"Alice","name":"Ada"}),
            json!({"identityId":alice.id,"name":"Ada"}),
            json!({"identityId":alice.id,"expectedName":42,"name":"Ada"}),
            json!({"identityId":alice.id,"expectedName":"Ada","name":"Eve","extra":true}),
        ] {
            assert_eq!(
                api::decode(
                    &json!({"version":1,"operation":"identity.rename","input":input}).to_string()
                )
                .err()
                .unwrap()
                .code(),
                "API_INPUT_INVALID"
            );
        }
        for field in ["identity", "originator"] {
            let mut envelope = json!({"version":1,"operation":"identity.rename","input":{"identityId":alice.id,"expectedName":"Ada","name":"Eve"}});
            envelope[field] = json!(if field == "identity" {
                "Alice"
            } else {
                "anonymous"
            });
            assert_eq!(
                api::decode(&envelope.to_string()).err().unwrap().code(),
                "API_INPUT_INVALID"
            );
        }
        assert!(
            resource(api::capabilities())["operations"]
                .as_array()
                .unwrap()
                .contains(&json!("identity.rename"))
        );
    }
}
