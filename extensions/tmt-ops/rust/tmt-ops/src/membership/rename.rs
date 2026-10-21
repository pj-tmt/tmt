//! One repairable room rename. Core owns the room CAS; an Ops journal preserves
//! the exact source metadata/configuration until the explicit repair completes.
use super::*;
use nix::fcntl::{Flock, FlockArg, OFlag};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 16 * 1024 * 1024;

pub struct Rename {
    pub room_id: String,
    pub old: String,
    pub new: String,
    /// The board supplies its displayed revision. CLI preparation reads one.
    pub expected_revision: Option<u64>,
    pub repair: bool,
}

fn io(error: impl std::fmt::Display) -> SquadError {
    SquadError::new("SQUAD_RENAME_STORE_IO", format!("Rename journal: {error}"))
}
fn invalid(message: &str) -> SquadError {
    SquadError::new("SQUAD_RENAME_INVALID", message)
}
fn read_json(path: &Path) -> Result<Value, SquadError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
        .map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(io("Not a regular journal."));
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io("Journal exceeds 16 MiB."));
    }
    serde_json::from_slice(&bytes).map_err(io)
}
fn publish(path: &Path, document: &Value) -> Result<(), SquadError> {
    let bytes = serde_json::to_vec(document).map_err(io)?;
    if bytes.len() as u64 > LIMIT {
        return Err(io("Journal exceeds 16 MiB."));
    }
    let temporary = path.with_extension("tmp");
    // The stable per-room lock makes a retained temporary an interrupted writer.
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io(error)),
    }
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(path.parent().unwrap())?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(io)
}
fn journal_path(core: &Core, id: &str) -> Result<(PathBuf, Flock<File>), SquadError> {
    if !crate::config::uuid_like(id) {
        return Err(invalid("Rename requires a canonical room UUID."));
    }
    let directory = crate::migration::state_root(core)?.join("renames");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .map_err(io)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(directory.join(format!("{id}.lock")))
        .map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(io("Not a regular rename lock."));
    }
    let guard = Flock::lock(file, FlockArg::LockExclusiveNonblock)
        .map_err(|(_, error)| SquadError::new("SQUAD_RENAME_BUSY", error.to_string()))?;
    Ok((directory.join(format!("{id}.json")), guard))
}
fn room(core: &Core, id: &str) -> Result<Value, SquadError> {
    let shown = core.json(&["room", "show", id])?;
    let room = &shown["room"];
    if room["id"] != id
        || room["retired"] != false
        || room["name"].as_str().is_none()
        || room["revision"]
            .as_u64()
            .is_none_or(|revision| revision == 0)
        || room["memberIds"].as_array().is_none_or(|ids| {
            ids.iter()
                .any(|id| !id.as_str().is_some_and(crate::config::uuid_like))
        })
    {
        return Err(invalid("Core returned an incomplete active room snapshot."));
    }
    Ok(room.clone())
}
fn repair_command(request: &Rename) -> String {
    format!(
        "tmt ops squad rename {} {} --room-id {} --repair",
        request.old, request.new, request.room_id
    )
}
fn partial(request: &Rename, room_status: &str, stage: &str, error: SquadError) -> Outcome {
    Outcome {
        complete: false,
        document: json!({"roomId": request.room_id,
        "old": request.old, "new": request.new, "roomStatus": room_status,
        "ownedStatus": "Incomplete",
        "cronStatus": if matches!(stage, "metadata" | "configuration" | "completion record") { "Applied" } else { "Unknown" },
        "metadataStatus": if matches!(stage, "configuration" | "completion record") { "Applied" } else if stage == "metadata" { "Partial or unknown" } else { "Not applied" },
        "configStatus": if stage == "completion record" { "Applied" } else { "Not applied" },
        "failed": {"stage": stage, "error": {
            "code": error.code, "message": error.message}}, "repair": repair_command(request)}),
    }
}
fn prepare(
    core: &Core,
    config: &Config,
    request: &Rename,
    snapshot: &Value,
) -> Result<Value, SquadError> {
    if snapshot["name"] != squad::room_name(&request.old) {
        return Err(SquadError::new(
            "ROOM_REVISION_CONFLICT",
            "The displayed squad changed; refresh before renaming.",
        ));
    }
    let expected = snapshot["revision"].as_u64().unwrap();
    if request
        .expected_revision
        .is_some_and(|revision| revision != expected)
    {
        return Err(SquadError::new(
            "ROOM_REVISION_CONFLICT",
            "The displayed room revision changed; refresh before renaming.",
        ));
    }
    match core.json(&["room", "show", &squad::room_name(&request.new)]) {
        Err(error) if error.code == "ROOM_NOT_FOUND" => {}
        Err(error) => return Err(error),
        Ok(_) => return Err(invalid("The destination squad name already exists.")),
    }
    let scope = config.squad_config_text(&request.old);
    config.check_squad_rename(&request.old, &request.new, scope.as_deref())?;
    crate::cron_service::check_rename_namespace(
        core,
        &request.old,
        &request.new,
        &request.room_id,
    )?;
    let old_prefix = format!("squad.{}.", request.old);
    let new_prefix = format!("squad.{}.", request.new);
    let mut metadata = serde_json::Map::new();
    for id in snapshot["memberIds"].as_array().unwrap() {
        let id = id
            .as_str()
            .ok_or_else(|| invalid("Core returned a malformed member UUID."))?;
        let shown = core.json(&["identity", "meta", "list", "--identity", id])?;
        let current = shown["metadata"]
            .as_object()
            .ok_or_else(|| invalid("Core returned no metadata map."))?;
        let mut fields = serde_json::Map::new();
        for (key, value) in current
            .iter()
            .filter(|(key, _)| key.starts_with(&old_prefix))
        {
            let new = format!("{new_prefix}{}", &key[old_prefix.len()..]);
            if new.len() > 64 || current.contains_key(&new) || !value.is_string() {
                return Err(invalid(
                    "Renamed metadata would exceed its key limit or overwrite a destination field.",
                ));
            }
            fields.insert(key.clone(), value.clone());
        }
        metadata.insert(id.to_owned(), Value::Object(fields));
    }
    Ok(
        json!({"version":1,"roomId":request.room_id,"old":request.old,"new":request.new,
        "revision":expected,"memberIds":snapshot["memberIds"],"metadata":metadata,
        "configScope":scope,"complete":false}),
    )
}
fn apply_metadata(core: &Core, request: &Rename, journal: &Value) -> Result<(), SquadError> {
    let old_prefix = format!("squad.{}.", request.old);
    let new_prefix = format!("squad.{}.", request.new);
    let metadata = journal["metadata"]
        .as_object()
        .ok_or_else(|| invalid("Missing journal metadata."))?;
    for (id, fields) in metadata {
        let expected = fields
            .as_object()
            .ok_or_else(|| invalid("Malformed journal metadata."))?;
        if expected.is_empty() {
            continue;
        }
        let shown = core.json(&["identity", "meta", "list", "--identity", id])?;
        let current = shown["metadata"]
            .as_object()
            .ok_or_else(|| invalid("Core returned no metadata map."))?;
        let mut changes = Vec::new();
        for (old, value) in expected {
            let suffix = old
                .strip_prefix(&old_prefix)
                .ok_or_else(|| invalid("Journal metadata escaped its squad namespace."))?;
            let new = format!("{new_prefix}{suffix}");
            match (current.get(old), current.get(&new)) {
                (None, Some(already)) if already == value => continue,
                (Some(before), None) if before == value => {}
                _ => {
                    return Err(SquadError::new(
                        "METADATA_CONFLICT",
                        "A rename field changed; reconcile it before repairing.",
                    ));
                }
            }
            changes.push(json!({"key":old,"expect":{"value":value},"then":"remove"}));
            changes.push(json!({"key":new,"expect":"absent","then":{"set":value}}));
        }
        // Public atomic metadata admission permits 64 changes. Each exchange
        // needs two; a later failed batch remains an honest repairable partial.
        for batch in changes.chunks(64) {
            let applied = core.api(
                "identity.meta.apply",
                json!({"identityId":id,"changes":batch}),
            )?;
            if applied["identityId"] != *id || !applied["changed"].is_boolean() {
                return Err(invalid(
                    "Core did not confirm the metadata exchange; repair explicitly.",
                ));
            }
        }
    }
    Ok(())
}

pub fn rename_squad(
    core: &Core,
    config: &mut Config,
    request: &Rename,
) -> Result<Outcome, SquadError> {
    if !valid_name(&request.old) || !valid_name(&request.new) || request.old == request.new {
        return Err(invalid("Choose two distinct valid squad names."));
    }
    let (path, _guard) = journal_path(core, &request.room_id)?;
    let snapshot = room(core, &request.room_id)?;
    let mut journal = if request.repair {
        read_json(&path)?
    } else {
        let retained = path
            .try_exists()
            .map_err(io)?
            .then(|| read_json(&path))
            .transpose()?;
        if let Some(retained) = retained.filter(|journal| journal["complete"] != true) {
            let old = retained["old"]
                .as_str()
                .filter(|name| valid_name(name))
                .ok_or_else(|| invalid("Malformed retained rename name."))?;
            let new = retained["new"]
                .as_str()
                .filter(|name| valid_name(name))
                .ok_or_else(|| invalid("Malformed retained rename name."))?;
            let pending = Rename {
                old: old.into(),
                new: new.into(),
                room_id: request.room_id.clone(),
                expected_revision: None,
                repair: true,
            };
            return Err(SquadError::hinted(
                "SQUAD_RENAME_PENDING",
                "This room has an incomplete rename",
                "; repair explicitly: ",
                &repair_command(&pending),
            ));
        }
        let prepared = prepare(core, config, request, &snapshot)?;
        publish(&path, &prepared)?;
        prepared
    };
    if journal["version"] != 1
        || journal["roomId"] != request.room_id
        || journal["old"] != request.old
        || journal["new"] != request.new
        || journal["cancelled"] == true
        || journal["revision"].as_u64().is_none_or(|value| value == 0)
        || journal["memberIds"].as_array().is_none_or(|ids| {
            ids.iter()
                .any(|id| !id.as_str().is_some_and(crate::config::uuid_like))
        })
        || !(journal["configScope"].is_null() || journal["configScope"].is_string())
        || journal["metadata"].as_object().is_none_or(|entries| {
            entries.keys().any(|id| {
                !journal["memberIds"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(id))
            })
        })
    {
        return Err(invalid(
            "Repair arguments do not match the retained rename.",
        ));
    }
    if snapshot["name"] == squad::room_name(&request.old) {
        let writer = match crate::me::caller(core)? {
            Some(caller) => Some(caller.me.id),
            None => crate::me::current(core)?.map(|person| person.id),
        };
        let written = core.api_write("rooms.write", json!({"roomId":request.room_id,
            "room":{"expectedRevision":journal["revision"],"name":squad::room_name(&request.new),"memberIds":journal["memberIds"]}}), writer.as_deref());
        if let Err(error) = written {
            if matches!(
                error.code.as_str(),
                "ROOM_REVISION_CONFLICT"
                    | "ROOM_INVALID"
                    | "ROOM_RETIRED"
                    | "API_INPUT_INVALID"
                    | "NAME_NOT_FOUND"
            ) {
                journal["complete"] = json!(true);
                journal["cancelled"] = json!(true);
                publish(&path, &journal)?;
                return Err(error);
            }
            return Ok(partial(request, "Unknown", "room CAS", error));
        }
        let written = written.unwrap();
        let expected_members = journal["memberIds"].as_array().unwrap();
        if written["id"] != request.room_id
            || written["name"] != squad::room_name(&request.new)
            || written["retired"] != false
            || written["revision"]
                .as_u64()
                .is_none_or(|revision| revision <= journal["revision"].as_u64().unwrap())
            || written["memberIds"].as_array().is_none_or(|ids| {
                ids.len() != expected_members.len()
                    || ids.iter().any(|id| !expected_members.contains(id))
            })
        {
            return Ok(partial(
                request,
                "Unknown",
                "room CAS",
                invalid("Core did not confirm the renamed room; repair explicitly."),
            ));
        }
    } else if snapshot["name"] != squad::room_name(&request.new) {
        return Err(SquadError::new(
            "ROOM_REVISION_CONFLICT",
            "The room no longer has either rename name; do not replay this operation.",
        ));
    }
    if let Err(error) =
        crate::cron_service::rename_namespace(core, &request.old, &request.new, &request.room_id)
    {
        return Ok(partial(request, "Applied", "cron namespace", error));
    }
    if let Err(error) = apply_metadata(core, request, &journal) {
        return Ok(partial(request, "Applied", "metadata", error));
    }
    if let Err(error) =
        config.rename_squad(&request.old, &request.new, journal["configScope"].as_str())
    {
        return Ok(partial(request, "Applied", "configuration", error));
    }
    journal["complete"] = json!(true);
    if let Err(error) = publish(&path, &journal) {
        return Ok(partial(request, "Applied", "completion record", error));
    }
    Ok(
        json!({"roomId":request.room_id,"old":request.old,"new":request.new,
        "roomStatus":"Applied","ownedStatus":"Applied","repair":null})
        .into(),
    )
}

#[cfg(test)]
mod tests;
