use rusqlite::{Connection, TransactionBehavior, params};
use tmt_core::limits::MAX_JS_SAFE_INTEGER;

use super::errors::{StorageError, StorageErrorCode, classify, incompatible};

#[cfg(test)]
mod announcement_tests;
#[cfg(test)]
mod auto_name_tests;
#[cfg(test)]
mod board_scope_tests;
#[cfg(test)]
mod change_cursor_tests;
#[cfg(test)]
mod dispatch_tests;
mod host_names;
#[cfg(test)]
mod host_names_tests;
#[cfg(test)]
mod host_tests;
#[cfg(test)]
#[path = "identity_lifetime_tests.rs"]
mod identity_lifetime_tests;
#[cfg(test)]
mod local_block_tests;
#[cfg(test)]
mod notification_tests;
#[cfg(test)]
mod pane_incarnation_tests;
#[cfg(test)]
mod prop_pack_tests;
#[cfg(test)]
mod receipt_tests;
#[cfg(test)]
mod request_history_tests;
#[cfg(test)]
mod request_room_tests;
#[cfg(test)]
mod room_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod storage_cutover_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod whiteboard_snapshot_tests;
#[cfg(test)]
mod whiteboard_tests;
#[cfg(test)]
mod world_layout_tests;

/// Use the actual predecessor schemas, never a subtraction from today's inventory.
#[cfg(test)]
pub(super) fn seed_hook_predecessor(connection: &mut Connection) {
    test_support::seed_history(connection);
    apply_identity_lifetime(connection, &MIGRATIONS[8]).unwrap();
}

struct Migration {
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        name: "create durable identities and transient tmux bindings",
        sql: include_str!("schema/001.sql"),
    },
    Migration {
        name: "create optional identity role profiles",
        sql: include_str!("schema/002.sql"),
    },
    Migration {
        name: "create durable identity preambles",
        sql: include_str!("schema/003.sql"),
    },
    Migration {
        name: "create request attempts and preamble cadence counters",
        sql: include_str!("schema/004.sql"),
    },
    Migration {
        name: "create immutable request responses",
        sql: include_str!("schema/005.sql"),
    },
    Migration {
        name: "freeze exchange retention and response expiry horizons",
        sql: include_str!("schema/006_columns.sql"),
    },
    Migration {
        name: "retain request provenance and bounded original prompts",
        sql: include_str!("schema/007.sql"),
    },
    Migration {
        name: "add identity-scoped exchange attention revisions",
        sql: include_str!("schema/008.sql"),
    },
    Migration {
        name: "add identity lifetimes and reusable retired names",
        sql: include_str!("schema/009.sql"),
    },
    Migration {
        name: "add durable identity retirement hooks",
        sql: include_str!("schema/010.sql"),
    },
    Migration {
        name: "add installation-owned local Office blocks",
        sql: include_str!("schema/011.sql"),
    },
    Migration {
        name: "add durable inbox routes and recipient attention",
        sql: include_str!("schema/012.sql"),
    },
    Migration {
        name: "add searchable identity metadata",
        sql: include_str!("schema/013.sql"),
    },
    Migration {
        name: "add local Office discussion board",
        sql: include_str!("schema/014.sql"),
    },
    Migration {
        name: "add identity-owned local Office presentation profiles",
        sql: include_str!("schema/015.sql"),
    },
    Migration {
        name: "add installation-owned local Office prop catalog",
        sql: include_str!("schema/016.sql"),
    },
    Migration {
        name: "add installation-owned local Office avatar catalog",
        sql: include_str!("schema/017.sql"),
    },
    Migration {
        name: "admit bounded directional Office prop packs",
        sql: include_str!("schema/018.sql"),
    },
    Migration {
        name: "add installation lobby target to local Office layouts",
        sql: include_str!("schema/019.sql"),
    },
    Migration {
        name: "allow bounded local Office placement customization",
        sql: include_str!("schema/020.sql"),
    },
    Migration {
        name: "add local whiteboard documents and operation receipts",
        sql: include_str!("schema/021.sql"),
    },
    Migration {
        name: "capture immutable whiteboard revisions and annotations",
        sql: include_str!("schema/022.sql"),
    },
    Migration {
        name: "retain Office request dispatch operation receipts",
        sql: include_str!("schema/023.sql"),
    },
    Migration {
        name: "add explicit local Office meeting membership",
        sql: include_str!("schema/024.sql"),
    },
    Migration {
        name: "distinguish inbox announcements from replyable requests",
        sql: include_str!("schema/025.sql"),
    },
    Migration {
        name: "retain original room context on durable requests",
        sql: include_str!("schema/026.sql"),
    },
    Migration {
        name: "index retained request conversations",
        sql: include_str!("schema/027.sql"),
    },
    Migration {
        name: "add atomic user-built Office world layouts",
        sql: include_str!("schema/028.sql"),
    },
    Migration {
        name: "add identity-owned expiring self-reported status",
        sql: include_str!("schema/029.sql"),
    },
    Migration {
        name: "extend shared Office discussions with room scopes",
        sql: include_str!("schema/030.sql"),
    },
    Migration {
        name: "retain retired meeting rooms without accepting new work",
        sql: include_str!("schema/031.sql"),
    },
    Migration {
        name: "track advisory wake attempts on durable inbox requests",
        sql: include_str!("schema/032.sql"),
    },
    Migration {
        name: "separate remembered harness preferences from binding runtime observations",
        sql: include_str!("schema/033.sql"),
    },
    Migration {
        name: "retain foreground launch ownership for binding runtime observations",
        sql: include_str!("schema/034.sql"),
    },
    Migration {
        name: "claim originator reply and detached timeout hints",
        sql: include_str!("schema/035.sql"),
    },
    Migration {
        name: "record extension storage cutovers and fence moved Office rows",
        sql: include_str!("schema/036.sql"),
    },
    Migration {
        name: "keep driver-owned resume state beside remembered sessions",
        sql: include_str!("schema/037.sql"),
    },
    Migration {
        name: "mark resume launches pending until a provider start confirms them",
        sql: include_str!("schema/038.sql"),
    },
    Migration {
        name: "admit a second terminal host in bindings, request fences and host servers",
        sql: include_str!("schema/039.sql"),
    },
    Migration {
        name: "advance one change cursor on every change to core-owned records",
        sql: include_str!("schema/040.sql"),
    },
    Migration {
        name: "admit any approved host driver in bindings, request fences and host servers",
        sql: include_str!("schema/041.sql"),
    },
    Migration {
        name: "record the observed pane process incarnation beside each binding's pane pid",
        sql: include_str!("schema/042.sql"),
    },
    Migration {
        name: "record explicit automatic identity name provenance",
        sql: include_str!("schema/043.sql"),
    },
    Migration {
        name: "persist pane reply notice batches and one-shot worker claims",
        sql: include_str!("schema/044.sql"),
    },
    Migration {
        name: "remember runtime channel preference for exact resume",
        sql: include_str!("schema/045.sql"),
    },
    Migration {
        name: "retain bounded consumption sources and timestamped history",
        sql: include_str!("schema/046.sql"),
    },
];

pub(super) fn apply(connection: &mut Connection) -> Result<(), StorageError> {
    apply_through(connection, MIGRATIONS.len())
}

/// Applies the pending migrations up to and including version `last`.
fn apply_through(connection: &mut Connection, last: usize) -> Result<(), StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| classify(error, "Initialize migration history"))?;
    transaction.execute_batch("CREATE TABLE IF NOT EXISTS _migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL)")
        .map_err(|error| classify(error, "Initialize migration history"))?;
    validate_table(&transaction)?;
    transaction
        .commit()
        .map_err(|error| classify(error, "Commit migration history"))?;
    let current = validate_history(connection)?;
    for (index, migration) in MIGRATIONS.iter().enumerate().take(last).skip(current) {
        let version = index as u32 + 1;
        if version == 9 || version == 41 {
            apply_without_foreign_keys(connection, version, migration)?;
        } else {
            apply_version(connection, version, migration)?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn apply_identity_lifetime(
    connection: &mut Connection,
    migration: &Migration,
) -> Result<(), StorageError> {
    apply_without_foreign_keys(connection, 9, migration)
}

/// A table rebuild: dropping a table with foreign keys on would cascade into
/// the tables that reference it.
fn apply_without_foreign_keys(
    connection: &mut Connection,
    version: u32,
    migration: &Migration,
) -> Result<(), StorageError> {
    // SQLite requires foreign_keys to change outside a transaction.
    // The private opening connection cannot escape during this rebuild.
    let result = set_foreign_keys(connection, false)
        .map_err(|error| StorageError::migration(version, error))
        .and_then(|()| apply_version(connection, version, migration));
    // apply_version has committed or dropped its transaction before
    // restoration. Always attempt it, preserving a primary failure.
    // Attempt restoration even if disabling succeeded but its verification failed.
    let restore =
        set_foreign_keys(connection, true).map_err(|error| StorageError::migration(version, error));
    result.and(restore)
}

/// What migrations 1 through `last` make in an empty database, to compare a
/// real source with.
fn fresh_through(last: usize) -> Result<Connection, StorageError> {
    let mut connection =
        Connection::open_in_memory().map_err(|error| classify(error, "Open reference schema"))?;
    set_foreign_keys(&connection, true)?;
    apply_through(&mut connection, last)?;
    Ok(connection)
}

fn apply_version(
    connection: &mut Connection,
    version: u32,
    migration: &Migration,
) -> Result<(), StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| classify(error, "Acquire migration lock"))?;
    let apply_one = (|| {
        // Another opener may have migrated while this one waited for the lock.
        if validate_history(&transaction)? >= version as usize {
            return Ok(());
        }
        if version == 9 {
            validate_identity_source(&transaction)?;
            check_foreign_keys(&transaction)?;
        }
        if version == 39 {
            validate_bindings_source(&transaction, migration)?;
        }
        if version == 41 {
            host_names::validate_source(&transaction, &fresh_through(40)?)?;
            check_foreign_keys(&transaction)?;
        }
        transaction
            .execute_batch(migration.sql)
            .map_err(|error| classify(error, "Apply migration SQL"))?;
        if version == 6 {
            backfill_retention(&transaction)?;
        }
        if version == 41 {
            host_names::rebuild(&transaction)?;
        }
        transaction.execute(
            "INSERT INTO _migrations (version, name, applied_at) VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![version, migration.name],
        ).map_err(|error| classify(error, "Record migration"))?;
        if version == 9 || version == 39 || version == 41 {
            check_foreign_keys(&transaction)?;
        }
        Ok(())
    })();
    apply_one.map_err(|error| StorageError::migration(version, error))?;
    transaction
        .commit()
        .map_err(|error| classify(error, "Commit migration"))
}

fn set_foreign_keys(connection: &Connection, enabled: bool) -> Result<(), StorageError> {
    connection
        .pragma_update(None, "foreign_keys", enabled)
        .map_err(|error| classify(error, "Configure migration foreign keys"))?;
    let actual: bool = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| classify(error, "Verify migration foreign keys"))?;
    if actual != enabled {
        return Err(incompatible(
            "Migration foreign-key policy did not take effect",
        ));
    }
    Ok(())
}

fn check_foreign_keys(connection: &Connection) -> Result<(), StorageError> {
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|error| classify(error, "Inspect migration foreign keys"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| classify(error, "Inspect migration foreign keys"))?;
    if rows
        .next()
        .map_err(|error| classify(error, "Read migration foreign keys"))?
        .is_some()
    {
        return Err(incompatible("Migration requires consistent foreign keys"));
    }
    Ok(())
}

fn validate_identity_source(connection: &Connection) -> Result<(), StorageError> {
    // Compare only the frozen first CREATE TABLE statement, not arbitrary SQL.
    // Whitespace differs between historical TS and Rust migration formatting.
    // Refuse custom columns/constraints rather than discarding their data/rules.
    let original = MIGRATIONS[0].sql.split(';').next().unwrap_or_default();
    let actual: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'identities'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Inspect identity source schema"))?;
    if !actual.split_whitespace().eq(original.split_whitespace()) {
        return Err(incompatible(
            "Identity migration requires the historical table definition",
        ));
    }
    // Only the two implicit identity indexes are part of schema 8. Never drop
    // an unrecognized user index or trigger as a side effect of table replacement.
    let custom: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE tbl_name = 'identities' AND type IN ('index', 'trigger') AND sql IS NOT NULL)",
        [], |row| row.get(0),
    ).map_err(|error| classify(error, "Inspect identity schema extensions"))?;
    if custom {
        return Err(incompatible(
            "Identity migration cannot replace custom indexes or triggers",
        ));
    }
    Ok(())
}

/// Schema 39 rebuilds `bindings` from a verbatim copy of its schema-38
/// definition. Refuse a table that differs from that copy, or anything else
/// that depends on it, rather than dropping custom columns or rules.
fn validate_bindings_source(
    connection: &Connection,
    migration: &Migration,
) -> Result<(), StorageError> {
    let start = migration
        .sql
        .find("CREATE TABLE bindings_039")
        .expect("schema 39 creates bindings_039");
    let rebuilt = migration.sql[start..].split(';').next().unwrap_or_default();
    let expected = rebuilt
        .replace("CREATE TABLE bindings_039", "CREATE TABLE bindings")
        .replace(
            "CHECK (transport IN ('tmux', 'herdr'))",
            "CHECK (transport = 'tmux')",
        );
    let actual: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'bindings'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Inspect binding source schema"))?;
    if !actual.split_whitespace().eq(expected.split_whitespace()) {
        return Err(incompatible(
            "Host migration requires the schema-38 bindings definition",
        ));
    }
    let dependents: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE sql IS NOT NULL
             AND name NOT IN ('bindings', 'bindings_endpoint') AND sql LIKE '%bindings%')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Inspect binding dependents"))?;
    if dependents {
        return Err(incompatible(
            "Host migration cannot rebuild bindings with dependent schema objects",
        ));
    }
    Ok(())
}

fn validate_table(connection: &Connection) -> Result<(), StorageError> {
    let mut query = connection
        .prepare("PRAGMA table_info(_migrations)")
        .map_err(|error| classify(error, "Inspect migration schema"))?;
    let columns = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| classify(error, "Inspect migration schema"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| classify(error, "Read migration schema"))?;
    let expected = [
        ("version", "INTEGER", 0, 1),
        ("name", "TEXT", 1, 0),
        ("applied_at", "TEXT", 1, 0),
    ];
    if columns.len() != expected.len()
        || columns.iter().zip(expected).any(|(actual, expected)| {
            actual.0 != expected.0
                || actual.1.to_ascii_uppercase() != expected.1
                || actual.2 != expected.2
                || actual.3 != expected.3
        })
    {
        return Err(incompatible(
            "The _migrations table does not match the supported schema",
        ));
    }
    Ok(())
}

/// Read-only consumers must not interpret an unsupported or incomplete schema.
pub(super) fn require_current(connection: &Connection) -> Result<(), StorageError> {
    if validate_history(connection)? != MIGRATIONS.len() {
        return Err(incompatible(
            "Database requires migration before context can be read",
        ));
    }
    Ok(())
}

fn validate_history(connection: &Connection) -> Result<usize, StorageError> {
    let mut statement = connection
        .prepare("SELECT version, name FROM _migrations ORDER BY version")
        .map_err(|error| classify(error, "Read migration history"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| classify(error, "Read migration history"))?;
    let mut count = 0;
    for row in rows {
        let (version, name) = row.map_err(|error| {
            incompatible("Migration history contains invalid values").caused_by(error)
        })?;
        if version != count as i64 + 1 {
            return Err(incompatible(format!(
                "Migration history is not contiguous at version {version}"
            )));
        }
        let Some(definition) = MIGRATIONS.get(count) else {
            return Err(incompatible(format!(
                "Database requires unsupported migration version {version}"
            )));
        };
        if definition.name != name {
            return Err(incompatible(format!(
                "Migration {version} has changed from {name} to {}",
                definition.name
            )));
        }
        count += 1;
    }
    Ok(count)
}

// Historical migration policy is frozen independently of today's settings.
const LEGACY_RETENTION_MS: i64 = 7 * 86_400_000;
const SETTLEMENT_FLOOR_MS: i64 = 86_400_000;

fn saturating_deadline(value: i64, delta: i64) -> Result<i64, StorageError> {
    let maximum = MAX_JS_SAFE_INTEGER as i64;
    if !(1..=maximum).contains(&value) || !(0..=maximum).contains(&delta) {
        return Err(StorageError::new(
            StorageErrorCode::Unknown,
            "Historical timestamp is outside the supported range",
        ));
    }
    Ok(value.saturating_add(delta).min(maximum))
}

fn backfill_retention(connection: &Connection) -> Result<(), StorageError> {
    let mut first = connection.prepare("SELECT attempt_id, prepared_at_ms, expires_at_ms, settled_at_ms FROM request_attempts ORDER BY attempt_id LIMIT 100")
        .map_err(|error| classify(error, "Prepare retention migration"))?;
    let mut next = connection.prepare("SELECT attempt_id, prepared_at_ms, expires_at_ms, settled_at_ms FROM request_attempts WHERE attempt_id > ? ORDER BY attempt_id LIMIT 100")
        .map_err(|error| classify(error, "Prepare retention migration"))?;
    let mut update = connection
        .prepare("UPDATE request_attempts SET retention_expires_at_ms = ? WHERE attempt_id = ?")
        .map_err(|error| classify(error, "Prepare retention migration"))?;
    let mut previous: Option<String> = None;
    loop {
        let read = |row: &rusqlite::Row<'_>| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        };
        let rows = match &previous {
            Some(previous) => next.query_map([previous], read),
            None => first.query_map([], read),
        }
        .map_err(|error| classify(error, "Read historical attempts"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| classify(error, "Decode historical attempts"))?;
        if rows.is_empty() {
            break;
        }
        for (id, prepared, expires, settled) in rows {
            // The historical reply-acceptance window is also seven days.
            let horizon = saturating_deadline(prepared, LEGACY_RETENTION_MS)?
                .max(saturating_deadline(expires, SETTLEMENT_FLOOR_MS)?)
                .max(
                    settled
                        .map(|time| saturating_deadline(time, SETTLEMENT_FLOOR_MS))
                        .transpose()?
                        .unwrap_or(0),
                );
            update
                .execute(params![horizon, id])
                .map_err(|error| classify(error, "Backfill attempt horizon"))?;
            previous = Some(id);
        }
    }

    let mut first = connection.prepare("SELECT request_id, submitted_at_ms FROM request_responses ORDER BY request_id LIMIT 100")
        .map_err(|error| classify(error, "Prepare response migration"))?;
    let mut next = connection.prepare("SELECT request_id, submitted_at_ms FROM request_responses WHERE request_id > ? ORDER BY request_id LIMIT 100")
        .map_err(|error| classify(error, "Prepare response migration"))?;
    let mut update = connection
        .prepare("UPDATE request_responses SET response_expires_at_ms = ? WHERE request_id = ?")
        .map_err(|error| classify(error, "Prepare response migration"))?;
    let mut previous: Option<String> = None;
    loop {
        let read = |row: &rusqlite::Row<'_>| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?));
        let rows = match &previous {
            Some(previous) => next.query_map([previous], read),
            None => first.query_map([], read),
        }
        .map_err(|error| classify(error, "Read historical responses"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| classify(error, "Decode historical responses"))?;
        if rows.is_empty() {
            break;
        }
        for (id, submitted) in rows {
            update
                .execute(params![
                    saturating_deadline(submitted, LEGACY_RETENTION_MS)?,
                    id
                ])
                .map_err(|error| classify(error, "Backfill response horizon"))?;
            previous = Some(id);
        }
    }
    connection.execute_batch("UPDATE request_attempts SET retention_expires_at_ms = MAX(retention_expires_at_ms, COALESCE((SELECT response_expires_at_ms FROM request_responses WHERE request_responses.request_id = request_attempts.request_id AND request_responses.attempt_id = request_attempts.attempt_id), retention_expires_at_ms))")
        .map_err(|error| classify(error, "Extend matching response horizons"))?;
    connection
        .execute_batch(include_str!("schema/006_indexes.sql"))
        .map_err(|error| classify(error, "Index retention horizons"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_deadlines_saturate_but_do_not_accept_invalid_anchors() {
        assert_eq!(
            saturating_deadline(1, LEGACY_RETENTION_MS).unwrap(),
            604_800_001
        );
        assert_eq!(
            saturating_deadline(MAX_JS_SAFE_INTEGER as i64 - 1, LEGACY_RETENTION_MS).unwrap(),
            MAX_JS_SAFE_INTEGER as i64
        );
        for value in [i64::MIN, -1, 0, MAX_JS_SAFE_INTEGER as i64 + 1, i64::MAX] {
            assert_eq!(
                saturating_deadline(value, 1).unwrap_err().code,
                StorageErrorCode::Unknown
            );
        }
    }
}
