//! Whole-world stdin/stdout boundary; no HTTP server or identity binding required.

use super::invoke_bytes_bounded;
use std::{io, path::Path, time::Instant};
use tmt_office_model::codec::office_world::SaveWorld;
use tmt_office_model::codec::office_world::WORLD_ENVELOPE_LIMIT;
use tmt_office_model::codec::office_world::WORLD_REPLY_LIMIT;
use tmt_office_model::codec::office_world::decode_save;
use tmt_office_model::codec::office_world::reply::WorldFailure;
use tmt_office_model::codec::office_world::reply::decode_reply;
use tmt_office_model::codec::office_world::save_value;
use tmt_office_model::codec::office_world::world_value;
use tmt_office_model::office_protocol::OfficeInvocation;
use tmt_office_model::office_world::LocalWorldSnapshot;

pub fn invoke_office_world(
    executable: &Path,
    edit: Option<&SaveWorld>,
    deadline: Instant,
) -> io::Result<Result<LocalWorldSnapshot, WorldFailure>> {
    let (operation, input) = match edit {
        Some(edit) => {
            let input = serde_json::to_vec(&save_value(edit))?;
            if let Err(error) = decode_save(&input) {
                return Ok(Err(error.into()));
            }
            (OfficeInvocation::LocalWorldApply, input)
        }
        None => (OfficeInvocation::LocalWorldShow, b"{}".to_vec()),
    };
    if input.len() > WORLD_ENVELOPE_LIMIT {
        return Err(io::Error::other("World input exceeds its bound."));
    }
    let bytes = invoke_bytes_bounded(executable, operation, &input, deadline, WORLD_REPLY_LIMIT)?;
    let result = decode_reply(&bytes).map_err(io::Error::other)?;
    if let Ok(snapshot) = &result {
        check_receipt(snapshot, edit)?;
    }
    Ok(result)
}

fn check_receipt(snapshot: &LocalWorldSnapshot, edit: Option<&SaveWorld>) -> io::Result<()> {
    let valid = match edit {
        Some(intent) => {
            intent
                .expected_revision
                .checked_add(u64::from(snapshot.changed))
                == Some(snapshot.revision)
                && snapshot.legacy_basis.is_none()
                && world_value(&snapshot.layout) == world_value(&intent.layout)
        }
        None => !snapshot.changed,
    };
    if valid {
        Ok(())
    } else {
        Err(io::Error::other(
            "Unexpected Office world operation result.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use tmt_office_model::codec::office_world::decode_world;

    #[test]
    fn success_must_confirm_the_requested_revision_and_complete_candidate() {
        let vectors: Value =
            serde_json::from_slice(include_bytes!("../../../../contracts/map-v1-vectors.json"))
                .unwrap();
        let layout = decode_world(
            &serde_json::to_vec(&json!({
                "version": 1, "map": vectors["lobby"], "objects": [],
            }))
            .unwrap(),
        )
        .unwrap();
        let intent = SaveWorld {
            expected_revision: 1,
            legacy_basis: None,
            layout: layout.clone(),
        };
        let mut snapshot = LocalWorldSnapshot {
            world_id: Some("10000000-0000-4000-8000-000000000001".into()),
            revision: 2,
            legacy_basis: None,
            layout,
            updated_at_ms: 1,
            changed: true,
        };
        assert!(check_receipt(&snapshot, Some(&intent)).is_ok());
        assert!(check_receipt(&snapshot, None).is_err());
        snapshot.changed = false;
        assert!(check_receipt(&snapshot, None).is_ok());
        assert!(check_receipt(&snapshot, Some(&intent)).is_err());
        snapshot.revision = 1;
        assert!(check_receipt(&snapshot, Some(&intent)).is_ok());
        snapshot.legacy_basis = Some("a".repeat(64));
        assert!(check_receipt(&snapshot, Some(&intent)).is_err());
        snapshot.legacy_basis = None;
        let mut other = world_value(&snapshot.layout);
        other["map"]["areas"][0]["name"] = json!("Different layout");
        snapshot.layout = decode_world(&serde_json::to_vec(&other).unwrap()).unwrap();
        assert!(check_receipt(&snapshot, Some(&intent)).is_err());
    }
}
