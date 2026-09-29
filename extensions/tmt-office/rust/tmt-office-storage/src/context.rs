//! Office's contribution to an identity's rehydration context: a short,
//! read-only line about the identity's desk in the local world.
//!
//! It never opens the store through `OfficeStore::open_configured`, which may
//! migrate core or upgrade and activate Office storage. Both databases are
//! opened read-only and query-only; nothing is reconciled, started or created.

use crate::{StorageLayout, migration};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use tmt_office_model::{codec::office_world::decode_world, office_map::AreaKind};

const NAME_LIMIT: usize = 80;

/// `Office: desk in "<area>"; <n> meeting areas.`, or `None` when the identity
/// has no personal area or the store cannot be read.
pub fn summary(layout: &StorageLayout, identity_id: &str) -> Option<String> {
    let path = if migration::switched(layout).ok()? {
        &layout.database
    } else {
        &layout.source
    };
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    connection.pragma_update(None, "query_only", true).ok()?;
    let encoded: Option<String> = connection
        .query_row(
            "SELECT layout_json FROM office_local_worlds WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()??;
    let world = decode_world(encoded?.as_bytes()).ok()?;
    let areas = &world.map().draft().areas;
    let desk = areas.iter().find(|area| {
        matches!(&area.kind, AreaKind::Personal { identity_id: Some(id) } if id == identity_id)
    })?;
    let meetings = areas
        .iter()
        .filter(|area| matches!(area.kind, AreaKind::Meeting { .. }))
        .count();
    let name: String = desk.name.chars().take(NAME_LIMIT).collect();
    Some(format!(
        "Office: desk in \"{}\"; {meetings} meeting area{}.",
        name.replace(['"', '\\'], ""),
        if meetings == 1 { "" } else { "s" }
    ))
}
