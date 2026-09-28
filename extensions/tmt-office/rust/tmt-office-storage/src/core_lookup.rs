//! TRANSITIONAL: verbatim copies of core identity and room queries.
//!
//! Before the storage switch, Office repositories share the core database file,
//! and these reads keep today's in-transaction checks exact while the
//! repositories move out of core. The next #353 slice replaces every call site
//! with CoreAccess preflight and deletes this module; no storage switch may
//! start while it exists. Do not add call sites: the architecture guard freezes
//! the current list.

use rusqlite::{Connection, OptionalExtension, Row};
use tmt_adapters::storage::{StorageError, StorageErrorCode, classify};
use tmt_core::{
    identity::{Identity, Lifetime},
    room::MeetingRoom,
};

const COLUMNS: &str = "id, name, canonical_name, lifetime, created_at, updated_at";

fn identity_row_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<Identity> {
    let lifetime = match row.get::<_, String>(offset + 3)?.as_str() {
        "temporary" => Lifetime::Temporary,
        "saved" => Lifetime::Saved,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Identity {
        id: row.get(offset)?,
        name: row.get(offset + 1)?,
        canonical_name: row.get(offset + 2)?,
        lifetime,
        created_at: row.get(offset + 4)?,
        updated_at: row.get(offset + 5)?,
    })
}

fn identity_row(row: &Row<'_>) -> rusqlite::Result<Identity> {
    identity_row_at(row, 0)
}

pub(crate) fn identity_by_id(
    connection: &Connection,
    id: &str,
) -> Result<Option<Identity>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM identities WHERE id = ?"),
            [id],
            identity_row,
        )
        .optional()
        .map_err(|error| classify(error, "Find identity by ID"))
}

pub(crate) fn active_identity_by_id(
    connection: &Connection,
    id: &str,
) -> Result<Option<Identity>, StorageError> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM identities WHERE id = ? AND retired_at_ms IS NULL"),
            [id],
            identity_row,
        )
        .optional()
        .map_err(|error| classify(error, "Find active identity by ID"))
}

pub(crate) fn read_room(
    connection: &Connection,
    id: &str,
) -> Result<Option<MeetingRoom>, StorageError> {
    Ok(read_historical_room(connection, id)?.filter(|room| !room.retired))
}

pub(crate) fn read_historical_room(
    connection: &Connection,
    id: &str,
) -> Result<Option<MeetingRoom>, StorageError> {
    let Some((name, revision, retired)) = connection
        .query_row(
            "SELECT name,revision,retired FROM office_meeting_rooms WHERE room_id=?",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| classify(error, "Read meeting room"))?
    else {
        return Ok(None);
    };
    let revision = u64::try_from(revision)
        .ok()
        .filter(|value| *value > 0 && *value <= tmt_core::limits::MAX_JS_SAFE_INTEGER)
        .ok_or_else(|| StorageError::new(StorageErrorCode::Corrupt, "Invalid meeting revision"))?;
    let mut statement = connection.prepare(
        "SELECT m.identity_id FROM office_meeting_members m JOIN identities i ON i.id=m.identity_id WHERE m.room_id=? AND i.retired_at_ms IS NULL ORDER BY m.identity_id"
    ).map_err(|error| classify(error, "Read meeting membership"))?;
    let member_ids = statement
        .query_map([id], |row| row.get(0))
        .map_err(|error| classify(error, "Read meeting membership"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|error| classify(error, "Read meeting membership"))?;
    Ok(Some(MeetingRoom {
        id: id.into(),
        name,
        revision,
        retired,
        member_ids,
    }))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::Path};

    /// Production reads of core-owned tables, per file. PR2b removes every
    /// entry with this module; any new or extra read fails here first.
    const FROZEN: &[(&str, usize)] = &[
        ("office_board/mod.rs", 3),
        ("office_local.rs", 2),
        ("office_profile.rs", 4),
        ("office_world/layout.rs", 5),
        ("office_world/legacy.rs", 1),
    ];

    fn production(text: &str) -> &str {
        text.split("#[cfg(test)]").next().unwrap_or(text)
    }

    fn core_reads(text: &str) -> usize {
        let text = production(text);
        let mut count = 0;
        for pattern in [
            "FROM identities",
            "JOIN identities",
            "office_meeting_rooms",
            "office_meeting_members",
            "core_lookup::",
        ] {
            count += text.matches(pattern).count();
        }
        for call in ["active_identity_by_id(", "read_historical_room("] {
            count += text.matches(call).count();
        }
        // Exclude the longer names that contain these shorter ones.
        count += text.matches("identity_by_id(").count()
            - text.matches("active_identity_by_id(").count();
        count += text.matches("read_room(").count();
        count
    }

    fn visit(root: &Path, directory: &Path, found: &mut BTreeMap<String, usize>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, found);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let name = path.file_name().unwrap().to_string_lossy();
            if !relative.ends_with(".rs")
                || relative == "core_lookup.rs"
                || name == "tests.rs"
                || relative.contains("tests/")
                || name.ends_with("_tests.rs")
                || name == "test_support.rs"
            {
                continue;
            }
            let reads = core_reads(&fs::read_to_string(&path).unwrap());
            if reads > 0 {
                found.insert(relative, reads);
            }
        }
    }

    #[test]
    fn transitional_core_reads_are_frozen_until_preflight_replaces_them() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = BTreeMap::new();
        visit(&root, &root, &mut found);
        let expected: BTreeMap<String, usize> = FROZEN
            .iter()
            .map(|(file, count)| ((*file).to_owned(), *count))
            .collect();
        assert_eq!(found, expected);
    }
}
