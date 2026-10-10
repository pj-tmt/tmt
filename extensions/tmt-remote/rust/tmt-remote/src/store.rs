//! Remote's durable SQLite state in `<dataRoot>/remote/remote.db`.
//! The machine identity (ID and `/r/` route prefix) is created once and is
//! stable across restarts; neither is a credential.
use crate::{error::RemoteError, state::Serving};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

/// First schema containing the remembered door port.
const PORT_SCHEMA: usize = 5;

pub(crate) fn database(error: impl std::fmt::Display) -> RemoteError {
    RemoteError::new(
        "REMOTE_STATE_UNAVAILABLE",
        &format!("Remote state database failed: {error}."),
    )
}

/// Open an admitted private database file under the serve lease. Serve is the
/// only opener (pairing and management go through it under the serve lock), so
/// DELETE journaling adds no reader contention, and FULL sync makes committed
/// rows survive power loss. Each database owner keeps its own schema history.
pub(crate) fn open_connection(serving: &Serving, file: &str) -> Result<Connection, RemoteError> {
    let layout = serving.layout();
    // Admit the 0600 file before SQLite opens it without following symlinks.
    layout.file(file)?.sync_all().map_err(database)?;
    let connection = Connection::open_with_flags(
        layout.directory.join(file),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(database)?;
    connection
        .busy_timeout(crate::limits::AUTHORITY_WAIT)
        .map_err(database)?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
        )
        .map_err(database)?;
    Ok(connection)
}

pub struct Store {
    pub(crate) connection: Connection,
    pub(crate) data_root: std::path::PathBuf,
}
/// Stable per-machine identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    /// Canonical lowercase UUIDv4.
    pub id: String,
    /// `/r/<16 lowercase base32>`.
    pub route_prefix: String,
}
impl Store {
    /// Opening requires the held serve lock: there is never a second opener.
    pub fn open(serving: &Serving) -> Result<Self, RemoteError> {
        let layout = serving.layout();
        let mut connection = open_connection(serving, "remote.db")?;
        migrate(&mut connection)?;
        Ok(Self {
            connection,
            data_root: layout
                .directory
                .parent()
                .ok_or_else(|| database("missing Remote data root"))?
                .to_owned(),
        })
    }
    pub fn remembered_port(&self) -> Result<Option<u16>, RemoteError> {
        read_port(&self.connection)
    }
    /// Persist only an actual bound port; the foreground lease serializes writers.
    pub fn remember_port(&self, port: u16) -> Result<(), RemoteError> {
        if port == 0 {
            return Err(database("cannot remember an unbound port"));
        }
        self.connection.execute(
            "INSERT INTO door_port VALUES (1, ?1) ON CONFLICT(singleton) DO UPDATE SET port=excluded.port",
            [port],
        ).map_err(database)?;
        Ok(())
    }
    /// Read stopped state under the existing lease, without initialization,
    /// migrations, journal changes or a second opener beside a live serve.
    pub fn stopped_port(serving: &Serving) -> Result<Option<u16>, RemoteError> {
        let layout = serving.layout();
        let Some(_file) = layout.read_file("remote.db")? else {
            return Ok(None);
        };
        let connection = Connection::open_with_flags(
            layout.directory.join("remote.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(database)?;
        let count = migration_count(&connection)?;
        if count < PORT_SCHEMA {
            return Ok(None);
        }
        read_port(&connection)
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
                    route_prefix: new_route_prefix()?,
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
        if !crate::canonical::route_prefix(&machine.route_prefix) || !uuid_shape(&machine.id) {
            return Err(database("invalid machine identity"));
        }
        Ok(machine)
    }
}
/// Ordered schema history in the core `_migrations` shape. Append only; a
/// recorded name must match, and a newer database than this build refuses.
const MIGRATIONS: [(&str, &str); 8] = [
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
    (
        "sessions",
        "CREATE TABLE sessions(
             client_id TEXT PRIMARY KEY REFERENCES grants(client_id),
             session_id TEXT NOT NULL UNIQUE,
             window_id TEXT NOT NULL,
             grant_revision INTEGER NOT NULL,
             next_client_sequence TEXT NOT NULL,
             next_server_sequence TEXT NOT NULL)",
    ),
    (
        "journal",
        "CREATE TABLE streams(client_id TEXT PRIMARY KEY REFERENCES grants(client_id),
            incarnation TEXT NOT NULL, key BLOB NOT NULL, tip INTEGER NOT NULL DEFAULT 0,
            floor INTEGER NOT NULL DEFAULT 0, observed INTEGER NOT NULL DEFAULT 0,
            acknowledged INTEGER NOT NULL DEFAULT 0,last_ms INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE entries(client_id TEXT NOT NULL REFERENCES streams(client_id),
            position INTEGER NOT NULL, envelope BLOB NOT NULL, at_ms INTEGER NOT NULL,
            PRIMARY KEY(client_id,position));
         CREATE TABLE operations(id TEXT PRIMARY KEY, client_id TEXT NOT NULL REFERENCES grants(client_id),
            operation TEXT NOT NULL, digest BLOB NOT NULL, frozen BLOB, phase TEXT NOT NULL,
            receipt TEXT NOT NULL, references_json TEXT NOT NULL DEFAULT '[]', updated_ms INTEGER NOT NULL);
         CREATE INDEX operations_client ON operations(client_id);
         CREATE TABLE budgets(subject TEXT NOT NULL,kind TEXT NOT NULL,started_ms INTEGER NOT NULL,
            used INTEGER NOT NULL,PRIMARY KEY(subject,kind));
         CREATE TABLE audit(position INTEGER PRIMARY KEY,at_ms INTEGER NOT NULL,
            client_id TEXT NOT NULL,envelope_id TEXT NOT NULL,operation TEXT NOT NULL,operation_id TEXT,resources_json TEXT NOT NULL,
            digest BLOB NOT NULL,grant_revision INTEGER NOT NULL,decision TEXT NOT NULL,code TEXT NOT NULL);",
    ),
    (
        "door_port",
        "CREATE TABLE door_port(
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
             port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535))",
    ),
    ("short_route_prefix", ""),
    ("multi_session", "ALTER TABLE sessions RENAME TO single_sessions;
         CREATE TABLE sessions(
             client_id TEXT NOT NULL REFERENCES grants(client_id),
             session_id TEXT PRIMARY KEY,
             window_id TEXT NOT NULL,
             grant_revision INTEGER NOT NULL,
             next_client_sequence TEXT NOT NULL,
             next_server_sequence TEXT NOT NULL,
             ended_reason TEXT, ended_at_ms INTEGER, ended_limit INTEGER);
         INSERT INTO sessions SELECT client_id,session_id,window_id,grant_revision,
             next_client_sequence,next_server_sequence,NULL,NULL,NULL FROM single_sessions;
         DROP TABLE single_sessions;
         CREATE INDEX sessions_client ON sessions(client_id);
         ALTER TABLE operations ADD COLUMN session_id TEXT;
         ALTER TABLE operations ADD COLUMN grant_revision INTEGER;
         UPDATE operations SET grant_revision=(SELECT revision FROM grants WHERE grants.client_id=operations.client_id);"),
    ("management", "CREATE TABLE settings_designation(
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        machine_id TEXT NOT NULL, client_id TEXT NOT NULL REFERENCES grants(client_id),
        public_key BLOB NOT NULL CHECK(length(public_key)=32));
     CREATE TABLE management_receipts(
        id TEXT PRIMARY KEY, client_id TEXT NOT NULL REFERENCES grants(client_id),
        operation TEXT NOT NULL, digest BLOB NOT NULL CHECK(length(digest)=32),
        grant_revision INTEGER NOT NULL, adopted_ms INTEGER NOT NULL, deadline_ms INTEGER NOT NULL,
        outcome TEXT NOT NULL);
     CREATE INDEX management_receipts_client ON management_receipts(client_id);"),
];
fn read_port(connection: &Connection) -> Result<Option<u16>, RemoteError> {
    let port: Option<i64> = connection
        .query_row(
            "SELECT port FROM door_port WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(database)?;
    port.map(|port| {
        u16::try_from(port)
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| database("invalid remembered door port"))
    })
    .transpose()
}
/// Supported scopes, sorted bytewise; existing grants keep this vocabulary.
pub const SUPPORTED_SCOPES: [&str; 5] = [
    "agents.read",
    "check.read",
    "results.read",
    "status.read",
    "talk",
];
/// Read-only scopes issued by a bare pairing, sorted bytewise.
pub const DEFAULT_SCOPES: [&str; 4] = ["agents.read", "check.read", "results.read", "status.read"];
/// A trusted device grant as the channel contract names its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub client_id: String,
    pub public_key: [u8; 32],
    pub kind: String,
    pub origin: String,
    pub name: String,
    /// `"all"` or the owner-chosen canonical UUID allowlist.
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
        self.insert_grant_audited(grant, None)
    }
    pub(crate) fn insert_paired_grant(
        &mut self,
        grant: &Grant,
        offer: &str,
        digest: &[u8],
    ) -> Result<(), RemoteError> {
        self.insert_grant_audited(grant, Some((offer, digest)))
    }
    fn insert_grant_audited(
        &mut self,
        grant: &Grant,
        audit: Option<(&str, &[u8])>,
    ) -> Result<(), RemoteError> {
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
                "DELETE FROM settings_designation WHERE public_key=?1",
                [grant.public_key.as_slice()],
            )
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
        if let Some((offer, digest)) = audit {
            crate::audit::append(
                &transaction,
                crate::audit::AuditMetadata {
                    time: grant.issued_at_ms,
                    client: &grant.client_id,
                    request: offer,
                    operation: "remote.pair",
                    operation_id: None,
                    resources: std::slice::from_ref(&grant.client_id),
                    digest,
                    revision: grant.revision,
                    decision: "committed",
                    code: "",
                },
            )?;
        }
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
        let grant = revoke_in(&transaction, client_id)?;
        transaction.commit().map_err(database)?;
        Ok(grant)
    }
    /// Local owner uses the same transaction-local talk writer as browser management.
    pub fn talk(&mut self, client_id: &str, enabled: bool) -> Result<Option<Grant>, RemoteError> {
        use sha2::{Digest, Sha256};
        let request = uuid_v4()?;
        let time = crate::pairing::now_ms()?;
        let digest = Sha256::digest(
            serde_json::to_vec(&serde_json::json!({"clientId":client_id,"enabled":enabled}))
                .map_err(database)?,
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let grant = talk_in(&tx, client_id, enabled)?;
        if let Some(grant) = &grant {
            crate::audit::append(
                &tx,
                crate::audit::AuditMetadata {
                    time,
                    client: client_id,
                    request: &request,
                    operation: "remote.devices.talk",
                    operation_id: None,
                    resources: &[client_id.to_owned()],
                    digest: &digest,
                    revision: grant.revision,
                    decision: "committed",
                    code: "LOCAL_OWNER",
                },
            )?;
        }
        tx.commit().map_err(database)?;
        Ok(grant)
    }
    /// Presentation-only rename. Disabled grants remain tombstones; an exact
    /// repeat preserves the revision and never changes grant authority. The last
    /// JSON-safe revision is reserved for revocation.
    pub fn rename(&mut self, client_id: &str, name: &str) -> Result<Option<Grant>, RemoteError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let grant = rename_in(&transaction, client_id, name)?;
        transaction.commit().map_err(database)?;
        Ok(grant)
    }
}
/// Shared transaction-local grant writers. Browser receipts and local management
/// use the same SQL; callers own the transaction and post-commit Session cleanup.
pub(crate) fn grant_in(
    connection: &Connection,
    client_id: &str,
) -> Result<Option<Grant>, RemoteError> {
    connection
        .query_row(
            &format!("SELECT {GRANT_COLUMNS} FROM grants WHERE client_id=?1"),
            [client_id],
            grant_row,
        )
        .optional()
        .map_err(database)
}
pub(crate) fn revoke_in(
    connection: &Connection,
    client_id: &str,
) -> Result<Option<Grant>, RemoteError> {
    connection.execute("UPDATE grants SET disabled=1, revision=revision+1 WHERE client_id=?1 AND disabled=0 AND revision<9007199254740991", [client_id]).map_err(database)?;
    connection
        .execute(
            "DELETE FROM settings_designation WHERE client_id=?1",
            [client_id],
        )
        .map_err(database)?;
    let grant = grant_in(connection, client_id)?;
    if grant.as_ref().is_some_and(|grant| !grant.disabled) {
        return Err(database("grant revision exhausted"));
    }
    Ok(grant)
}
/// Change only the explicit sending scope. Other authority remains frozen.
pub(crate) fn talk_in(
    connection: &Connection,
    client_id: &str,
    enabled: bool,
) -> Result<Option<Grant>, RemoteError> {
    let Some(mut grant) = grant_in(connection, client_id)? else {
        return Ok(None);
    };
    if grant.disabled {
        return Err(RemoteError::new(
            "REMOTE_DEVICE_REVOKED",
            "A revoked device cannot grant sending.",
        ));
    }
    if grant.permits_scope("talk") != enabled {
        if grant.revision >= 9_007_199_254_740_990 {
            return Err(database("grant revision exhausted"));
        }
        grant.scopes.retain(|scope| scope != "talk");
        if enabled {
            grant.scopes.push("talk".into());
            grant.scopes.sort();
        }
        connection
            .execute(
                "UPDATE grants SET scopes=?2,revision=revision+1 WHERE client_id=?1",
                params![client_id, grant.scopes.join(" ")],
            )
            .map_err(database)?;
    }
    grant_in(connection, client_id)
}
pub(crate) fn rename_in(
    connection: &Connection,
    client_id: &str,
    name: &str,
) -> Result<Option<Grant>, RemoteError> {
    if !crate::canonical::device_name(name) {
        return Err(RemoteError::new(
            "REMOTE_INPUT_INVALID",
            "Device names must be 1–64 nonblank UTF-8 bytes without controls.",
        ));
    }
    if grant_in(connection, client_id)?.is_some_and(|grant| grant.disabled) {
        return Err(RemoteError::new(
            "REMOTE_DEVICE_REVOKED",
            "A revoked device cannot be renamed.",
        ));
    }
    connection.execute("UPDATE grants SET name=?2,revision=revision+1 WHERE client_id=?1 AND name!=?2 AND revision<9007199254740990", rusqlite::params![client_id,name]).map_err(database)?;
    let grant = grant_in(connection, client_id)?;
    if grant.as_ref().is_some_and(|grant| grant.name != name) {
        return Err(database("grant revision exhausted"));
    }
    Ok(grant)
}
/// Durable counters belong to one live session/run. A restart never adopts an
/// old row as a live session; each fresh signed open adds independent counters.
impl Store {
    pub fn start_session(
        &mut self,
        grant: &Grant,
        session_id: &str,
        window_id: &str,
        now_ms: u64,
    ) -> Result<(), RemoteError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let current = tx
            .query_row(
                &format!("SELECT {GRANT_COLUMNS} FROM grants WHERE client_id = ?1"),
                [&grant.client_id],
                grant_row,
            )
            .optional()
            .map_err(database)?;
        if !current
            .is_some_and(|current| current.revision == grant.revision && current.live_at(now_ms))
        {
            return Err(RemoteError::new("REMOTE_CLOSED", "Device authority ended."));
        }
        tx.execute(
            "INSERT INTO sessions(client_id,session_id,window_id,grant_revision,next_client_sequence,next_server_sequence)
             VALUES (?1, ?2, ?3, ?4, '1', '2')",
            rusqlite::params![
                grant.client_id,
                session_id,
                window_id,
                grant.revision as i64
            ],
        )
        .map_err(database)?;
        tx.commit().map_err(database)
    }
    /// Call only after exact signature/authority admission. One transaction
    /// consumes the expected sequence; invalid or competing sequences change nothing.
    pub fn consume_sequence(
        &mut self,
        client_id: &str,
        session_id: &str,
        window_id: &str,
        sequence: u64,
        now_ms: u64,
    ) -> Result<(), RemoteError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let revision: Option<i64> = tx.query_row(
            "SELECT grant_revision FROM sessions WHERE client_id=?1 AND session_id=?2 AND window_id=?3 AND ended_reason IS NULL",
            [client_id, session_id, window_id], |row| row.get(0),
        ).optional().map_err(database)?;
        let grant = tx
            .query_row(
                &format!("SELECT {GRANT_COLUMNS} FROM grants WHERE client_id=?1"),
                [client_id],
                grant_row,
            )
            .optional()
            .map_err(database)?;
        if !grant
            .is_some_and(|grant| revision == Some(grant.revision as i64) && grant.live_at(now_ms))
        {
            return Err(RemoteError::new("REMOTE_CLOSED", "Device session ended."));
        }
        // The exhausted sentinel cannot match any valid wire sequence.
        let next = sequence
            .checked_add(1)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "exhausted".into());
        let changed = tx
            .execute(
                "UPDATE sessions SET next_client_sequence=?4
             WHERE client_id=?1 AND session_id=?2 AND window_id=?3 AND next_client_sequence=?5",
                rusqlite::params![client_id, session_id, window_id, next, sequence.to_string()],
            )
            .map_err(database)?;
        if sequence == 0 || changed != 1 {
            return Err(RemoteError::new(
                "REMOTE_REPLAY",
                "Unexpected client sequence.",
            ));
        }
        tx.commit().map_err(database)
    }
    /// Reserve a fresh machine response sequence, including signed refusals.
    /// Session.open used 1; historical journal envelopes keep their old counters.
    pub fn response_sequence(
        &mut self,
        client_id: &str,
        session_id: &str,
        window_id: &str,
    ) -> Result<u64, RemoteError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        let sequence: Option<String> = tx.query_row(
            "SELECT next_server_sequence FROM sessions WHERE client_id=?1 AND session_id=?2 AND window_id=?3",
            [client_id, session_id, window_id], |row| row.get(0),
        ).optional().map_err(database)?;
        let sequence = sequence
            .and_then(|value| {
                value
                    .parse::<u64>()
                    .ok()
                    .filter(|sequence| *sequence >= 2 && sequence.to_string() == value)
            })
            .ok_or_else(|| {
                RemoteError::new("REMOTE_CLOSED", "Device response sequence is unavailable.")
            })?;
        let next = sequence
            .checked_add(1)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "exhausted".into());
        tx.execute("UPDATE sessions SET next_server_sequence=?4 WHERE client_id=?1 AND session_id=?2 AND window_id=?3",
            rusqlite::params![client_id, session_id, window_id, next]).map_err(database)?;
        tx.commit().map_err(database)?;
        Ok(sequence)
    }
}
pub(crate) const GRANT_COLUMNS: &str =
    "client_id, public_key, kind, origin, name, agents, scopes, mode,
    issued_at_ms, expires_at_ms, revision, disabled";
pub(crate) fn grant_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Grant> {
    let key: Vec<u8> = r.get(1)?;
    let scopes: String = r.get(6)?;
    Ok(Grant {
        client_id: r.get(0)?,
        public_key: key.try_into().unwrap_or([0; 32]),
        kind: r.get(2)?,
        origin: r.get(3)?,
        name: r.get(4)?,
        agents: r.get(5)?,
        scopes: if scopes.is_empty() {
            Vec::new()
        } else {
            scopes.split(' ').map(str::to_owned).collect()
        },
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
    let applied = migration_count(connection)?;
    for (index, (name, sql)) in MIGRATIONS.iter().enumerate().skip(applied) {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        transaction.execute_batch(sql).map_err(database)?;
        if *name == "short_route_prefix" {
            let old: Option<(String, String)> = transaction
                .query_row(
                    "SELECT machine_id, route_prefix FROM machine WHERE singleton=1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(database)?;
            if let Some((id, prefix)) = old {
                let legacy = prefix.strip_prefix("/r/").is_some_and(|hex| {
                    hex.len() == 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                });
                if !uuid_shape(&id) || !legacy {
                    return Err(database("invalid legacy machine identity"));
                }
                transaction
                    .execute(
                        "UPDATE machine SET route_prefix=?1 WHERE singleton=1",
                        [new_route_prefix()?],
                    )
                    .map_err(database)?;
            }
        }
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
fn migration_count(connection: &Connection) -> Result<usize, RemoteError> {
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
    Ok(applied.len())
}
fn new_route_prefix() -> Result<String, RemoteError> {
    Ok(format!(
        "/r/{}",
        crate::canonical::base32_text(&random::<10>()?).to_ascii_lowercase()
    ))
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

impl Store {
    /// Session lifetime is independent of grant-owned held work.
    pub(crate) fn end_session(
        &mut self,
        client: &str,
        session: &str,
        reason: &str,
        limit: Option<usize>,
        now: u64,
    ) -> Result<(), RemoteError> {
        self.connection.execute("UPDATE sessions SET ended_reason=?3,ended_at_ms=?4,ended_limit=?5 WHERE client_id=?1 AND session_id=?2 AND ended_reason IS NULL",rusqlite::params![client,session,reason,now as i64,limit.map(|n|n as i64)]).map_err(database)?;
        Ok(())
    }
    pub(crate) fn ended_reason(
        &self,
        client: &str,
        session: &str,
        window: &str,
    ) -> Result<Option<(String, Option<usize>)>, RemoteError> {
        self.connection.query_row("SELECT ended_reason,ended_limit FROM sessions WHERE client_id=?1 AND session_id=?2 AND window_id=?3",[client,session,window], |row| { let code: Option<String> = row.get(0)?; let limit = row.get::<_, Option<i64>>(1)?.map(|n|usize::try_from(n).map_err(|_|rusqlite::Error::IntegralValueOutOfRange(1,n))).transpose()?; Ok(code.map(|code| (code, limit))) }).optional().map(|value| value.flatten()).map_err(database)
    }
    pub(crate) fn operation_session(
        &self,
        client: &str,
        id: &str,
    ) -> Result<Option<String>, RemoteError> {
        self.connection
            .query_row(
                "SELECT session_id FROM operations WHERE client_id=?1 AND id=?2",
                [client, id],
                |row| row.get(0),
            )
            .optional()
            .map(|value| value.flatten())
            .map_err(database)
    }
    pub(crate) fn prune_sessions(&self, window: &str, now: u64) -> Result<(), RemoteError> {
        let cutoff = now.saturating_sub(crate::limits::SESSION_END_NOTICE.as_millis() as u64);
        let stale: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE window_id!=?1 OR ended_at_ms<=?2)",
                rusqlite::params![window, cutoff as i64],
                |row| row.get(0),
            )
            .map_err(database)?;
        // Avoid even acquiring SQLite's write lock when there is nothing to remove.
        if !stale {
            return Ok(());
        }
        self.connection
            .execute(
                "DELETE FROM sessions WHERE window_id!=?1 OR ended_at_ms<=?2",
                rusqlite::params![window, cutoff as i64],
            )
            .map_err(database)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Layout;

    #[test]
    fn existing_full_grant_reopens_without_narrowing_or_receipt_change() {
        let root = std::env::temp_dir().join(format!("tmt-talk-legacy-{}", uuid_v4().unwrap()));
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        let machine = store.machine().unwrap();
        let grant = Grant {
            client_id: uuid_v4().unwrap(),
            public_key: [7; 32],
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Existing device".into(),
            agents: "all".into(),
            scopes: SUPPORTED_SCOPES.map(str::to_owned).into(),
            mode: "direct".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        store.insert_grant(&grant).unwrap();
        let receipt = crate::pairing::receipt(&grant, &machine.id, &[9; 32]).unwrap();
        drop(store);
        let store = Store::open(&serving).unwrap();
        let reopened = store.grant(&grant.client_id).unwrap().unwrap();
        assert_eq!(reopened, grant);
        assert!(reopened.permits_scope("talk"));
        assert_eq!(
            crate::pairing::receipt(&reopened, &machine.id, &[9; 32]).unwrap(),
            receipt
        );
        drop(store);
        drop(serving);
        std::fs::remove_dir_all(root).unwrap();
    }

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

    #[test]
    fn session_pruning_writes_only_when_rows_need_removal() {
        let root = std::env::temp_dir().join(format!("tmt-1768-prune-{}", uuid_v4().unwrap()));
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let store = Store::open(&serving).unwrap();
        store
            .connection
            .busy_timeout(std::time::Duration::from_millis(10))
            .unwrap();
        let oracle = Connection::open(serving.layout().directory.join("remote.db")).unwrap();
        oracle
            .execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE")
            .unwrap();
        // An idle prune may read alongside another writer, but must not request a write lock.
        store.prune_sessions("current", 60000).unwrap();
        oracle.execute_batch("ROLLBACK").unwrap();
        oracle
            .execute_batch(
                "INSERT INTO grants VALUES ('device',zeroblob(32),'cli','cli','Test device','all','capabilities','direct',0,NULL,1,0);
            INSERT INTO sessions VALUES
            ('device','live','current',1,'1','2',NULL,NULL,NULL),
            ('device','notice','current',1,'1','2','REMOTE_SESSION_ENDED',1000,NULL),
            ('device','old-run','previous',1,'1','2',NULL,NULL,NULL)",
            )
            .unwrap();
        store.prune_sessions("current", 60999).unwrap();
        let count = || {
            store
                .connection
                .query_row("SELECT count(*) FROM sessions", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        };
        assert_eq!(count(), 2); // Old run gone; live and not-yet-expired notice remain.
        oracle.execute_batch("BEGIN IMMEDIATE").unwrap();
        store.prune_sessions("current", 60999).unwrap();
        // Positive control: an expired notice genuinely needs a write, so the writer blocks it.
        assert!(store.prune_sessions("current", 61000).is_err());
        oracle.execute_batch("ROLLBACK").unwrap();
        store.prune_sessions("current", 61000).unwrap();
        assert_eq!(count(), 1);
        let remaining: String = store
            .connection
            .query_row("SELECT session_id FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, "live");
        drop(oracle);
        drop(store);
        drop(serving);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rename_reserves_the_final_revision_for_revocation() {
        let root = std::env::temp_dir().join(format!("tmt-1100-revision-{}", std::process::id()));
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        let mut grant = Grant {
            client_id: uuid_v4().unwrap(),
            public_key: [7; 32],
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Laptop".into(),
            agents: "all".into(),
            scopes: SUPPORTED_SCOPES.iter().map(|s| (*s).into()).collect(),
            mode: "direct".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 9_007_199_254_740_989,
            disabled: false,
        };
        store.insert_grant(&grant).unwrap();
        grant.name = "Travel".into();
        grant.revision += 1;
        assert_eq!(
            store.rename(&grant.client_id, "Travel").unwrap(),
            Some(grant.clone())
        );
        assert_eq!(
            store.rename(&grant.client_id, "Laptop").unwrap_err().code,
            "REMOTE_STATE_UNAVAILABLE"
        );
        assert_eq!(store.grant(&grant.client_id).unwrap(), Some(grant.clone()));
        assert_eq!(
            store.rename(&grant.client_id, "Travel").unwrap(),
            Some(grant.clone())
        );
        grant.revision += 1;
        grant.disabled = true;
        assert_eq!(store.revoke(&grant.client_id).unwrap(), Some(grant.clone()));
        assert_eq!(store.revoke(&grant.client_id).unwrap(), Some(grant.clone()));
        assert_eq!(
            store.rename(&grant.client_id, "Laptop").unwrap_err().code,
            "REMOTE_DEVICE_REVOKED"
        );
        drop(store);
        let reopened = Store::open(&serving).unwrap();
        assert_eq!(reopened.grant(&grant.client_id).unwrap(), Some(grant));
        drop(reopened);
        drop(serving);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
