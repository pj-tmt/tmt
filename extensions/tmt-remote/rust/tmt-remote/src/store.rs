//! Remote's durable SQLite state in `<dataRoot>/remote/remote.db`.
//! The machine identity (ID and `/r/` route prefix) is created once and is
//! stable across restarts; neither is a credential.
use crate::{error::RemoteError, state::Serving};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::time::Duration;

fn database(error: impl std::fmt::Display) -> RemoteError {
    RemoteError::new(
        "REMOTE_STATE_UNAVAILABLE",
        &format!("Remote state database failed: {error}."),
    )
}

pub struct Store {
    connection: Connection,
}
/// Stable per-machine identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    /// Canonical lowercase UUIDv4.
    pub id: String,
    /// `/r/<32 lowercase hex>`.
    pub route_prefix: String,
}
impl Store {
    /// Opening requires the held serve lock: there is never a second opener.
    pub fn open(serving: &Serving) -> Result<Self, RemoteError> {
        let layout = serving.layout();
        // Admit the 0600 file before SQLite opens it without following symlinks.
        layout.file("remote.db")?.sync_all().map_err(database)?;
        let mut connection = Connection::open_with_flags(
            layout.directory.join("remote.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(database)?;
        connection
            .busy_timeout(Duration::from_millis(5000))
            .map_err(database)?;
        // Serve is the only opener (pairing and management go through it under
        // the serve lock), so DELETE journaling adds no reader contention, and
        // FULL sync makes committed grants and receipts survive power loss.
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
            )
            .map_err(database)?;
        migrate(&mut connection)?;
        Ok(Self { connection })
    }
    /// The machine identity, created on first use inside one transaction.
    pub fn machine(&mut self) -> Result<Machine, RemoteError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let existing = transaction.query_row(
            "SELECT machine_id, route_prefix FROM machine WHERE singleton = 1",
            [],
            |row| {
                Ok(Machine {
                    id: row.get(0)?,
                    route_prefix: row.get(1)?,
                })
            },
        );
        let machine = match existing {
            Ok(machine) => machine,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let machine = Machine {
                    id: uuid_v4()?,
                    route_prefix: format!("/r/{}", crate::state::hex(&random::<16>()?)),
                };
                transaction
                    .execute(
                        "INSERT INTO machine VALUES (1, ?1, ?2)",
                        [&machine.id, &machine.route_prefix],
                    )
                    .map_err(database)?;
                machine
            }
            Err(e) => return Err(database(e)),
        };
        transaction.commit().map_err(database)?;
        // A tampered row fails closed instead of widening the route grammar.
        let prefix_valid = machine.route_prefix.strip_prefix("/r/").is_some_and(|h| {
            h.len() == 32 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
        if !prefix_valid || !uuid_shape(&machine.id) {
            return Err(database("invalid machine identity"));
        }
        Ok(machine)
    }
}
/// Ordered schema history in the core `_migrations` shape. Append only; a
/// recorded name must match, and a newer database than this build refuses.
const MIGRATIONS: [(&str, &str); 2] = [
    (
        "machine",
        "CREATE TABLE machine(
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
             machine_id TEXT NOT NULL,
             route_prefix TEXT NOT NULL)",
    ),
    (
        "grants",
        "CREATE TABLE grants(
             client_id TEXT PRIMARY KEY,
             public_key BLOB NOT NULL CHECK(length(public_key) = 32),
             kind TEXT NOT NULL CHECK(kind IN ('addon', 'browser', 'cli')),
             origin TEXT NOT NULL,
             name TEXT NOT NULL,
             agents TEXT NOT NULL,
             scopes TEXT NOT NULL,
             mode TEXT NOT NULL CHECK(mode IN ('direct', 'hold')),
             issued_at_ms INTEGER NOT NULL,
             expires_at_ms INTEGER,
             revision INTEGER NOT NULL CHECK(revision > 0),
             disabled INTEGER NOT NULL CHECK(disabled IN (0, 1)));
         -- One live grant per device key; a revoked key may pair again.
         CREATE UNIQUE INDEX grants_live_key ON grants(public_key) WHERE disabled = 0;",
    ),
];
/// Default scopes, sorted bytewise.
pub const DEFAULT_SCOPES: [&str; 5] = [
    "agents.read",
    "check.read",
    "results.read",
    "status.read",
    "talk",
];
/// A trusted device grant as the channel contract names its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub client_id: String,
    pub public_key: [u8; 32],
    pub kind: String,
    pub origin: String,
    pub name: String,
    /// `"all"`; owner-chosen allowlists are later work.
    pub agents: String,
    pub scopes: Vec<String>,
    pub mode: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: Option<u64>,
    pub revision: u64,
    pub disabled: bool,
}
impl Store {
    /// Insert one grant in its own transaction. A live grant for the same
    /// device key refuses, and a failure leaves no partial grant.
    pub fn insert_grant(&mut self, grant: &Grant) -> Result<(), RemoteError> {
        let millis = |value: u64| {
            i64::try_from(value)
                .ok()
                .filter(|v| *v < (1 << 53))
                .ok_or_else(|| database("time out of range"))
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        transaction
            .execute(
                "INSERT INTO grants VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    grant.client_id,
                    grant.public_key.as_slice(),
                    grant.kind,
                    grant.origin,
                    grant.name,
                    grant.agents,
                    grant.scopes.join(" "),
                    grant.mode,
                    millis(grant.issued_at_ms)?,
                    grant.expires_at_ms.map(millis).transpose()?,
                    i64::try_from(grant.revision).map_err(database)?,
                    grant.disabled,
                ],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _)
                    if f.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    RemoteError::new(
                        "REMOTE_ALREADY_PAIRED",
                        "This device key already has a live grant.",
                    )
                }
                other => database(other),
            })?;
        transaction.commit().map_err(database)
    }
    pub fn grant(&self, client_id: &str) -> Result<Option<Grant>, RemoteError> {
        match self.connection.query_row(
            &format!("SELECT {GRANT_COLUMNS} FROM grants WHERE client_id = ?1"),
            [client_id],
            grant_row,
        ) {
            Ok(grant) => Ok(Some(grant)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(database(e)),
        }
    }
    /// Every grant, revoked ones included, oldest first.
    pub fn grants(&self) -> Result<Vec<Grant>, RemoteError> {
        self.connection
            .prepare(&format!(
                "SELECT {GRANT_COLUMNS} FROM grants ORDER BY issued_at_ms, client_id"
            ))
            .and_then(|mut query| query.query_map([], grant_row)?.collect())
            .map_err(database)
    }
    /// Disable a device's grant and advance its revision, so every session and
    /// context bound to the old revision fails its recheck. Revoking an already
    /// revoked grant changes nothing; an unknown device is `None`.
    pub fn revoke(&mut self, client_id: &str) -> Result<Option<Grant>, RemoteError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        transaction
            .execute(
                "UPDATE grants SET disabled = 1, revision = revision + 1
                 WHERE client_id = ?1 AND disabled = 0",
                [client_id],
            )
            .map_err(database)?;
        transaction.commit().map_err(database)?;
        self.grant(client_id)
    }
}
const GRANT_COLUMNS: &str = "client_id, public_key, kind, origin, name, agents, scopes, mode,
    issued_at_ms, expires_at_ms, revision, disabled";
fn grant_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Grant> {
    let key: Vec<u8> = r.get(1)?;
    let scopes: String = r.get(6)?;
    Ok(Grant {
        client_id: r.get(0)?,
        public_key: key.try_into().unwrap_or([0; 32]),
        kind: r.get(2)?,
        origin: r.get(3)?,
        name: r.get(4)?,
        agents: r.get(5)?,
        scopes: scopes.split(' ').map(str::to_owned).collect(),
        mode: r.get(7)?,
        issued_at_ms: r.get::<_, i64>(8)? as u64,
        expires_at_ms: r.get::<_, Option<i64>>(9)?.map(|v| v as u64),
        revision: r.get::<_, i64>(10)? as u64,
        disabled: r.get(11)?,
    })
}
fn migrate(connection: &mut Connection) -> Result<(), RemoteError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS _migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL)",
        )
        .map_err(database)?;
    let applied: Vec<(i64, String)> = connection
        .prepare("SELECT version, name FROM _migrations ORDER BY version")
        .and_then(|mut query| {
            query
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(database)?;
    if applied.len() > MIGRATIONS.len() {
        return Err(RemoteError::new(
            "REMOTE_STATE_UNSUPPORTED",
            "Remote state was written by a newer build.",
        ));
    }
    for (index, (version, name)) in applied.iter().enumerate() {
        if *version != index as i64 + 1 || name != MIGRATIONS[index].0 {
            return Err(database("unexpected migration history"));
        }
    }
    for (index, (name, sql)) in MIGRATIONS.iter().enumerate().skip(applied.len()) {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        transaction.execute_batch(sql).map_err(database)?;
        transaction
            .execute(
                "INSERT INTO _migrations (version, name, applied_at) VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                rusqlite::params![index as i64 + 1, name],
            )
            .map_err(database)?;
        transaction.commit().map_err(database)?;
    }
    Ok(())
}
fn random<const N: usize>() -> Result<[u8; N], RemoteError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes)
        .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain entropy."))?;
    Ok(bytes)
}
/// Canonical lowercase UUIDv4 from OS entropy.
pub fn uuid_v4() -> Result<String, RemoteError> {
    let mut b = random::<16>()?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = crate::state::hex(&b);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}
fn uuid_shape(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                matches!(b, b'0'..=b'9' | b'a'..=b'f')
            }
        })
        && value.as_bytes()[14] == b'4'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Layout;

    #[test]
    fn connection_enforces_foreign_keys_and_full_sync() {
        let root = std::env::temp_dir().join(format!("tmt-1039-store-{}", std::process::id()));
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let store = Store::open(&serving).unwrap();
        let pragma = |name: &str| -> i64 {
            store
                .connection
                .query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(pragma("foreign_keys"), 1);
        // FULL is 2 in SQLite's numbering.
        assert_eq!(pragma("synchronous"), 2);
        drop(store);
        drop(serving);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
