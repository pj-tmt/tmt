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
    path: &'static str,
    name: &'static str,
    sql: &'static str,
}

macro_rules! migration {
    ($name:literal, $path:literal) => {
        Migration {
            path: concat!("rust/crates/tmt-adapters/src/storage/", $path),
            name: $name,
            sql: include_str!($path),
        }
    };
}

const MIGRATIONS: &[Migration] = &[
    migration!(
        "create durable identities and transient tmux bindings",
        "schema/001.sql"
    ),
    migration!("create optional identity role profiles", "schema/002.sql"),
    migration!("create durable identity preambles", "schema/003.sql"),
    migration!(
        "create request attempts and preamble cadence counters",
        "schema/004.sql"
    ),
    migration!("create immutable request responses", "schema/005.sql"),
    migration!(
        "freeze exchange retention and response expiry horizons",
        "schema/006_columns.sql"
    ),
    migration!(
        "retain request provenance and bounded original prompts",
        "schema/007.sql"
    ),
    migration!(
        "add identity-scoped exchange attention revisions",
        "schema/008.sql"
    ),
    migration!(
        "add identity lifetimes and reusable retired names",
        "schema/009.sql"
    ),
    migration!("add durable identity retirement hooks", "schema/010.sql"),
    migration!(
        "add installation-owned local Office blocks",
        "schema/011.sql"
    ),
    migration!(
        "add durable inbox routes and recipient attention",
        "schema/012.sql"
    ),
    migration!("add searchable identity metadata", "schema/013.sql"),
    migration!("add local Office discussion board", "schema/014.sql"),
    migration!(
        "add identity-owned local Office presentation profiles",
        "schema/015.sql"
    ),
    migration!(
        "add installation-owned local Office prop catalog",
        "schema/016.sql"
    ),
    migration!(
        "add installation-owned local Office avatar catalog",
        "schema/017.sql"
    ),
    migration!(
        "admit bounded directional Office prop packs",
        "schema/018.sql"
    ),
    migration!(
        "add installation lobby target to local Office layouts",
        "schema/019.sql"
    ),
    migration!(
        "allow bounded local Office placement customization",
        "schema/020.sql"
    ),
    migration!(
        "add local whiteboard documents and operation receipts",
        "schema/021.sql"
    ),
    migration!(
        "capture immutable whiteboard revisions and annotations",
        "schema/022.sql"
    ),
    migration!(
        "retain Office request dispatch operation receipts",
        "schema/023.sql"
    ),
    migration!(
        "add explicit local Office meeting membership",
        "schema/024.sql"
    ),
    migration!(
        "distinguish inbox announcements from replyable requests",
        "schema/025.sql"
    ),
    migration!(
        "retain original room context on durable requests",
        "schema/026.sql"
    ),
    migration!("index retained request conversations", "schema/027.sql"),
    migration!(
        "add atomic user-built Office world layouts",
        "schema/028.sql"
    ),
    migration!(
        "add identity-owned expiring self-reported status",
        "schema/029.sql"
    ),
    migration!(
        "extend shared Office discussions with room scopes",
        "schema/030.sql"
    ),
    migration!(
        "retain retired meeting rooms without accepting new work",
        "schema/031.sql"
    ),
    migration!(
        "track advisory wake attempts on durable inbox requests",
        "schema/032.sql"
    ),
    migration!(
        "separate remembered harness preferences from binding runtime observations",
        "schema/033.sql"
    ),
    migration!(
        "retain foreground launch ownership for binding runtime observations",
        "schema/034.sql"
    ),
    migration!(
        "claim originator reply and detached timeout hints",
        "schema/035.sql"
    ),
    migration!(
        "record extension storage cutovers and fence moved Office rows",
        "schema/036.sql"
    ),
    migration!(
        "keep driver-owned resume state beside remembered sessions",
        "schema/037.sql"
    ),
    migration!(
        "mark resume launches pending until a provider start confirms them",
        "schema/038.sql"
    ),
    migration!(
        "admit a second terminal host in bindings, request fences and host servers",
        "schema/039.sql"
    ),
    migration!(
        "advance one change cursor on every change to core-owned records",
        "schema/040.sql"
    ),
    migration!(
        "admit any approved host driver in bindings, request fences and host servers",
        "schema/041.sql"
    ),
    migration!(
        "record the observed pane process incarnation beside each binding's pane pid",
        "schema/042.sql"
    ),
    migration!(
        "record explicit automatic identity name provenance",
        "schema/043.sql"
    ),
    migration!(
        "persist pane reply notice batches and one-shot worker claims",
        "schema/044.sql"
    ),
    migration!(
        "remember runtime channel preference for exact resume",
        "schema/045.sql"
    ),
    migration!(
        "retain bounded consumption sources and timestamped history",
        "schema/046.sql"
    ),
    migration!(
        "index originator results by final submission time",
        "schema/047.sql"
    ),
    migration!(
        "retain originator withdrawal of unanswered requests",
        "schema/048.sql"
    ),
];

pub(super) fn apply(connection: &mut Connection) -> Result<(), StorageError> {
    apply_through(connection, MIGRATIONS.len())
}

/// Compiled migration authority, with the actual Rust/SQL source closure.
/// This is not a filesystem scan or a count inferred by release tooling.
pub(super) fn compiled_schema() -> super::CompiledSchema {
    use tmt_core::content_digest::sha256;
    let mut sources = MIGRATIONS
        .iter()
        .map(|migration| super::SchemaSource {
            path: migration.path,
            sha256: sha256(migration.sql.as_bytes()),
        })
        .collect::<Vec<_>>();
    for (path, bytes) in [
        (
            "rust/crates/tmt-adapters/src/storage/migrations.rs",
            include_bytes!("migrations.rs").as_slice(),
        ),
        (
            "rust/crates/tmt-adapters/src/storage/migrations/host_names.rs",
            include_bytes!("migrations/host_names.rs").as_slice(),
        ),
        (
            "rust/crates/tmt-adapters/src/storage/schema/006_indexes.sql",
            include_bytes!("schema/006_indexes.sql").as_slice(),
        ),
    ] {
        sources.push(super::SchemaSource {
            path,
            sha256: sha256(bytes),
        });
    }
    sources.sort_by_key(|source| source.path);
    super::CompiledSchema {
        version: MIGRATIONS.len() as u32,
        sources,
    }
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

/// Observe the recorded application schema, including a future schema, without
/// interpreting its tables or admitting it for ordinary runtime use.
pub(super) fn observed_schema(connection: &Connection) -> Result<u32, StorageError> {
    validate_table(connection)?;
    let (count, minimum, maximum): (i64, Option<u32>, Option<u32>) = connection
        .query_row(
            "SELECT COUNT(*), MIN(version), MAX(version) FROM _migrations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| classify(error, "Observe application schema"))?;
    let version = maximum
        .filter(|maximum| minimum == Some(1) && count == i64::from(*maximum))
        .ok_or_else(|| incompatible("Application schema history is missing or incomplete"))?;
    let mut statement = connection
        .prepare("SELECT version, name FROM _migrations WHERE version <= ? ORDER BY version")
        .map_err(|error| classify(error, "Inspect known application migrations"))?;
    let rows = statement
        .query_map([MIGRATIONS.len() as u32], |row| {
            Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| classify(error, "Inspect known application migrations"))?;
    for row in rows {
        let (version, name) =
            row.map_err(|error| classify(error, "Read known application migration"))?;
        if MIGRATIONS
            .get(version as usize - 1)
            .is_none_or(|migration| migration.name != name)
        {
            return Err(incompatible(
                "Known application migration provenance has changed",
            ));
        }
    }
    Ok(version)
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
