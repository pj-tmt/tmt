//! Remote's durable SQLite state in `<dataRoot>/remote/remote.db`.
//! The machine identity (ID and `/r/` route prefix) is created once and is
//! stable across restarts; neither is a credential.
use crate::{error::RemoteError, state::Layout};
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
    pub fn open(layout: &Layout) -> Result<Self, RemoteError> {
        // Admit the 0600 file before SQLite opens it without following symlinks.
        layout.file("remote.db")?.sync_all().map_err(database)?;
        let connection = Connection::open_with_flags(
            layout.directory.join("remote.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(database)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(database)?;
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(database)?;
        if version > 1 {
            return Err(RemoteError::new(
                "REMOTE_STATE_UNSUPPORTED",
                &format!("Remote state schema {version} is newer than this build."),
            ));
        }
        connection
            .execute_batch(
                "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
                 BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS machine(
                     singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                     machine_id TEXT NOT NULL,
                     route_prefix TEXT NOT NULL);
                 PRAGMA user_version=1; COMMIT;",
            )
            .map_err(database)?;
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
