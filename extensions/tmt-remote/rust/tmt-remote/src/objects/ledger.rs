//! The one installation-wide object ledger: `<dataRoot>/remote/objects.db`, a
//! Remote-owned coordination database opened only under the serve lease. It is
//! the sole source of truth for original intents, phases, checkpoints, namespace
//! fences and every byte/entry/identity charge; physical files only follow it.
//! Each transition is one short IMMEDIATE transaction, so the hierarchy limits
//! and every phase change are serialized here, and no file I/O or other lock is
//! taken inside a transaction. Rows are never deleted or rewritten: the
//! immutable original columns are guarded by triggers.
use super::{
    BackendError, BackendResult, BeginSpec, BlobKey, IntentId, Limit, NamespaceId, OpaqueKey,
    Quotas, Scope, Usage, payload_charge,
};
use crate::{
    error::RemoteError,
    limits,
    state::{Serving, state_error},
    store::{database, open_connection},
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::{collections::BTreeMap, os::unix::fs::MetadataExt, sync::Mutex};

const FILE: &str = "objects.db";
const VERSION: i64 = 1;

const LIVE: &str =
    "'staging','committing','committed','discarding','expiring','removing','unknown'";
const TERMINAL: &str = "'discarded','expired','deleted'";
const CHUNK: i64 = limits::OBJECT_CHUNK_BYTES as i64;
/// Rows released per transaction, bounding the rollback journal (see the ledger test).
const RELEASE_BATCH: i64 = 8;

/// Exact statements; the stored SQL of an opened ledger must equal these.
fn schema() -> Vec<(&'static str, String)> {
    vec![
        (
            "namespaces",
            "CREATE TABLE namespaces(\
             extension TEXT NOT NULL, namespace BLOB NOT NULL CHECK(length(namespace)=32),\
             state TEXT NOT NULL CHECK(state IN ('open','removing','removed')),\
             PRIMARY KEY(extension,namespace)) WITHOUT ROWID"
                .into(),
        ),
        (
            "intents",
            format!(
                "CREATE TABLE intents(\
             extension TEXT NOT NULL, intent BLOB NOT NULL CHECK(length(intent)=32),\
             namespace BLOB NOT NULL CHECK(length(namespace)=32),\
             object BLOB NOT NULL CHECK(length(object)=32),\
             payload_sha256 BLOB NOT NULL CHECK(length(payload_sha256)=32),\
             payload_bytes INTEGER NOT NULL CHECK(payload_bytes BETWEEN 0 AND {max}),\
             binding BLOB NOT NULL CHECK(length(binding)<={binding}),\
             adopted_ms INTEGER NOT NULL CHECK(adopted_ms>=0),\
             expires_ms INTEGER NOT NULL CHECK(expires_ms>=adopted_ms),\
             phase TEXT NOT NULL CHECK(phase IN ({live},{terminal})),\
             next_index INTEGER NOT NULL CHECK(next_index>=0),\
             received INTEGER NOT NULL CHECK(received BETWEEN 0 AND payload_bytes),\
             payload_charged INTEGER NOT NULL CHECK(payload_charged>=0),\
             entry_held INTEGER NOT NULL CHECK(entry_held IN (0,1)),\
             staged INTEGER NOT NULL CHECK(staged IN (0,1)),\
             PRIMARY KEY(extension,intent)) WITHOUT ROWID",
                max = limits::OBJECT_PAYLOAD_BYTES,
                binding = limits::OBJECT_BINDING_BYTES,
                live = LIVE,
                terminal = TERMINAL
            ),
        ),
        (
            "intents_live_key",
            format!(
                "CREATE UNIQUE INDEX intents_live_key ON intents(extension,namespace,object) \
             WHERE phase IN ({LIVE})"
            ),
        ),
        (
            "intents_namespace",
            "CREATE INDEX intents_namespace ON intents(extension,namespace)".into(),
        ),
        (
            "intents_immutable",
            "CREATE TRIGGER intents_immutable BEFORE UPDATE OF extension,intent,namespace,object,\
             payload_sha256,payload_bytes,binding,adopted_ms,expires_ms ON intents \
             BEGIN SELECT RAISE(ABORT,'immutable original intent'); END"
                .into(),
        ),
        (
            "intents_retained",
            "CREATE TRIGGER intents_retained BEFORE DELETE ON intents \
             BEGIN SELECT RAISE(ABORT,'retained original intent'); END"
                .into(),
        ),
        (
            "namespaces_retained",
            "CREATE TRIGGER namespaces_retained BEFORE DELETE ON namespaces \
             BEGIN SELECT RAISE(ABORT,'retained namespace fence'); END"
                .into(),
        ),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Staging,
    Committing,
    Committed,
    Discarding,
    Expiring,
    Discarded,
    Expired,
    Removing,
    Deleted,
    Unknown,
}
impl Phase {
    fn text(self) -> &'static str {
        match self {
            Self::Staging => "staging",
            Self::Committing => "committing",
            Self::Committed => "committed",
            Self::Discarding => "discarding",
            Self::Expiring => "expiring",
            Self::Discarded => "discarded",
            Self::Expired => "expired",
            Self::Removing => "removing",
            Self::Deleted => "deleted",
            Self::Unknown => "unknown",
        }
    }
    fn parse(text: &str) -> Option<Self> {
        [
            Self::Staging,
            Self::Committing,
            Self::Committed,
            Self::Discarding,
            Self::Expiring,
            Self::Discarded,
            Self::Expired,
            Self::Removing,
            Self::Deleted,
            Self::Unknown,
        ]
        .into_iter()
        .find(|phase| phase.text() == text)
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub extension: String,
    pub spec: BeginSpec,
    pub adopted_ms: u64,
    pub expires_ms: u64,
    pub phase: Phase,
    pub next_index: u32,
    pub received: u64,
    /// The staging name may still exist; cleared only after confirmed unlink.
    pub staged: bool,
}
const COLUMNS: &str = "extension,intent,namespace,object,payload_sha256,payload_bytes,binding,\
adopted_ms,expires_ms,phase,next_index,received,staged";

fn array(value: Vec<u8>) -> rusqlite::Result<[u8; 32]> {
    <[u8; 32]>::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let number = |index: usize| -> rusqlite::Result<u64> {
        u64::try_from(r.get::<_, i64>(index)?).map_err(|_| rusqlite::Error::InvalidQuery)
    };
    Ok(Row {
        extension: r.get(0)?,
        spec: BeginSpec {
            intent: IntentId(array(r.get(1)?)?),
            key: BlobKey {
                namespace: NamespaceId(array(r.get(2)?)?),
                object: OpaqueKey(array(r.get(3)?)?),
            },
            payload_sha256: array(r.get(4)?)?,
            payload_bytes: number(5)?,
            binding: r.get(6)?,
        },
        adopted_ms: number(7)?,
        expires_ms: number(8)?,
        phase: Phase::parse(&r.get::<_, String>(9)?).ok_or(rusqlite::Error::InvalidQuery)?,
        next_index: u32::try_from(number(10)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
        received: number(11)?,
        staged: r.get::<_, i64>(12)? == 1,
    })
}
fn db(_: rusqlite::Error) -> BackendError {
    BackendError::Unavailable
}
fn fetch(tx: &Connection, ext: &str, intent: &IntentId) -> BackendResult<Option<Row>> {
    tx.query_row(
        &format!("SELECT {COLUMNS} FROM intents WHERE extension=?1 AND intent=?2"),
        params![ext, intent.0.as_slice()],
        read_row,
    )
    .optional()
    .map_err(db)
}
fn namespace_state(tx: &Connection, ext: &str, ns: &NamespaceId) -> BackendResult<Option<String>> {
    tx.query_row(
        "SELECT state FROM namespaces WHERE extension=?1 AND namespace=?2",
        params![ext, ns.0.as_slice()],
        |r| r.get(0),
    )
    .optional()
    .map_err(db)
}
/// `intent` currently in one of `from`, moved to `to`; a lost race is a conflict.
fn transition(
    tx: &Connection,
    ext: &str,
    intent: &IntentId,
    from: &[Phase],
    to: Phase,
) -> BackendResult<()> {
    let set = from
        .iter()
        .map(|phase| format!("'{}'", phase.text()))
        .collect::<Vec<_>>()
        .join(",");
    let changed = tx
        .execute(
            &format!(
                "UPDATE intents SET phase=?3 WHERE extension=?1 AND intent=?2 AND phase IN ({set})"
            ),
            params![ext, intent.0.as_slice(), to.text()],
        )
        .map_err(db)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(BackendError::Conflict)
    }
}

#[derive(Clone, Copy)]
enum Filter<'a> {
    Installation,
    Extension(&'a str),
    Namespace(&'a str, &'a [u8]),
}
#[derive(Clone, Copy, Debug, Default)]
struct Totals {
    bytes: u64,
    entries: u32,
    active: u32,
    retained: u32,
}
/// The conservative charge formula, recomputed from rows (never a cached counter):
/// payload (4 KiB-rounded, until its removal is confirmed) + `OBJECT_RECORD_BYTES`
/// per retained intent row + `OBJECT_FENCE_BYTES` per namespace row, plus the
/// ledger base for the installation scope.
fn totals(tx: &Connection, filter: Filter<'_>) -> BackendResult<Totals> {
    let select = "SELECT COALESCE(SUM(payload_charged),0),COALESCE(SUM(entry_held),0),\
         COALESCE(SUM(phase IN ('staging','committing')),0),COUNT(*) FROM intents";
    let read = |r: &rusqlite::Row<'_>| -> rusqlite::Result<(i64, i64, i64, i64)> {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    };
    let (rows, fences): ((i64, i64, i64, i64), i64) = match filter {
        Filter::Installation => (
            tx.query_row(select, [], read).map_err(db)?,
            tx.query_row("SELECT COUNT(*) FROM namespaces", [], |r| r.get(0))
                .map_err(db)?,
        ),
        Filter::Extension(ext) => (
            tx.query_row(&format!("{select} WHERE extension=?1"), [ext], read)
                .map_err(db)?,
            tx.query_row(
                "SELECT COUNT(*) FROM namespaces WHERE extension=?1",
                [ext],
                |r| r.get(0),
            )
            .map_err(db)?,
        ),
        Filter::Namespace(ext, ns) => (
            tx.query_row(
                &format!("{select} WHERE extension=?1 AND namespace=?2"),
                params![ext, ns],
                read,
            )
            .map_err(db)?,
            tx.query_row(
                "SELECT COUNT(*) FROM namespaces WHERE extension=?1 AND namespace=?2",
                params![ext, ns],
                |r| r.get(0),
            )
            .map_err(db)?,
        ),
    };
    let ((payload, entries, active, retained), fences) = (rows, fences);
    let base = if matches!(filter, Filter::Installation) {
        limits::OBJECT_LEDGER_BASE_BYTES
    } else {
        0
    };
    Ok(Totals {
        bytes: payload as u64
            + retained as u64 * limits::OBJECT_RECORD_BYTES
            + fences as u64 * limits::OBJECT_FENCE_BYTES
            + base,
        entries: entries as u32,
        active: active as u32,
        retained: retained as u32,
    })
}

/// Originals and namespace fences an interrupted run left for owned settlement.
pub struct Unsettled {
    pub rows: Vec<Row>,
    pub namespaces: Vec<(String, NamespaceId)>,
}
pub enum Adoption {
    Fresh(Row),
    Repeat(Row),
}
/// A mutation gate: proceed, or the staging deadline had passed and the row is now `expiring`.
pub enum Gate {
    Go(Row),
    Expired(Row),
}

pub struct Ledger {
    connection: Mutex<Connection>,
}
impl Ledger {
    /// Open or create the ledger under the serve lease. A damaged, foreign or
    /// newer ledger refuses; it is never reset.
    pub fn open(serving: &Serving) -> Result<Self, RemoteError> {
        admit_sidecars(serving)?;
        let mut connection = open_connection(serving, FILE)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(database)?;
        let actual: BTreeMap<String, Option<String>> = connection
            .prepare("SELECT name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")
            .and_then(|mut query| {
                query
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .map_err(database)?;
        let expected: BTreeMap<String, Option<String>> = schema()
            .into_iter()
            .map(|(name, sql)| (name.to_owned(), Some(sql)))
            .collect();
        if version > VERSION {
            return Err(RemoteError::new(
                "REMOTE_STATE_UNSUPPORTED",
                "Remote object state was written by a newer build.",
            ));
        }
        if version == 0 && actual.is_empty() {
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database)?;
            for (_, sql) in schema() {
                tx.execute_batch(&sql).map_err(database)?;
            }
            tx.execute_batch(&format!("PRAGMA user_version={VERSION}"))
                .map_err(database)?;
            tx.commit().map_err(database)?;
        } else if version != VERSION || actual != expected {
            return Err(database("damaged object ledger"));
        }
        let ledger = Self {
            connection: Mutex::new(connection),
        };
        ledger.validate()?;
        Ok(ledger)
    }
    /// Every row must satisfy the charge and checkpoint invariants; a violation
    /// refuses readiness rather than being repaired.
    fn validate(&self) -> Result<(), RemoteError> {
        let connection = self.connection.lock().map_err(|_| database("poisoned"))?;
        let bad: i64 = connection
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM intents WHERE NOT (\
                     next_index=(received+{c}-1)/{c} AND (received=payload_bytes OR received%{c}=0)\
                     AND (phase NOT IN ('committing','committed') OR received=payload_bytes)\
                     AND CASE WHEN phase IN ({TERMINAL}) THEN payload_charged=0 AND entry_held=0 AND staged=0 \
                     ELSE entry_held=1 AND payload_charged=((payload_bytes+{b}-1)/{b})*{b} END)",
                    c = CHUNK,
                    b = limits::OBJECT_BLOCK_BYTES
                ),
                [],
                |r| r.get(0),
            )
            .map_err(database)?;
        let orphan: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM intents i LEFT JOIN namespaces n \
                 ON n.extension=i.extension AND n.namespace=i.namespace \
                 WHERE n.state IS NULL OR (n.state='removed' AND i.phase NOT IN ('discarded','expired','deleted')) \
                 OR (n.state='open' AND i.phase IN ('removing','deleted'))",
                [],
                |r| r.get(0),
            )
            .map_err(database)?;
        if bad != 0 || orphan != 0 {
            return Err(database("inconsistent object ledger"));
        }
        Ok(())
    }
    fn run<R>(&self, f: impl FnOnce(&Transaction<'_>) -> BackendResult<R>) -> BackendResult<R> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| BackendError::Unavailable)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let value = f(&tx)?;
        tx.commit().map_err(db)?;
        Ok(value)
    }

    /// Adopt the original intent, reserving entry and bytes before any payload, or
    /// observe the original of an exact repeat. Limits are checked in a fixed order:
    /// active, retained identities, entries, then bytes (namespace, extension, installation).
    pub fn adopt(
        &self,
        ext: &str,
        spec: &BeginSpec,
        now: u64,
        quotas: &Quotas,
    ) -> BackendResult<Adoption> {
        self.run(|tx| {
            if let Some(mut row) = fetch(tx, ext, &spec.intent)? {
                if row.spec != *spec {
                    return Err(BackendError::Conflict);
                }
                if row.phase == Phase::Staging && now >= row.expires_ms {
                    transition(tx, ext, &spec.intent, &[Phase::Staging], Phase::Expiring)?;
                    row.phase = Phase::Expiring;
                }
                return Ok(Adoption::Repeat(row));
            }
            let ns = spec.key.namespace;
            let state = namespace_state(tx, ext, &ns)?;
            if state.as_deref().is_some_and(|state| state != "open") {
                return Err(BackendError::Conflict);
            }
            let live: Option<i64> = tx
                .query_row(
                    &format!(
                        "SELECT 1 FROM intents WHERE extension=?1 AND namespace=?2 AND object=?3 \
                         AND phase IN ({LIVE})"
                    ),
                    params![ext, ns.0.as_slice(), spec.key.object.0.as_slice()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db)?;
            if live.is_some() {
                return Err(BackendError::Conflict);
            }
            let payload = payload_charge(spec.payload_bytes);
            let added = payload
                + limits::OBJECT_RECORD_BYTES
                + if state.is_none() {
                    limits::OBJECT_FENCE_BYTES
                } else {
                    0
                };
            let installation = totals(tx, Filter::Installation)?;
            let extension = totals(tx, Filter::Extension(ext))?;
            let namespace = totals(tx, Filter::Namespace(ext, &ns.0))?;
            let refuse = |limit| Err(BackendError::Capacity(limit));
            if installation.active >= quotas.active_intents {
                return refuse(Limit::ActiveIntents);
            }
            if extension.retained >= quotas.retained_extension {
                return refuse(Limit::RetainedExtension);
            }
            if installation.retained >= quotas.retained_installation {
                return refuse(Limit::RetainedInstallation);
            }
            if namespace.entries >= quotas.namespace_entries {
                return refuse(Limit::NamespaceEntries);
            }
            if extension.entries >= quotas.extension_entries {
                return refuse(Limit::ExtensionEntries);
            }
            if installation.entries >= quotas.installation_entries {
                return refuse(Limit::InstallationEntries);
            }
            if namespace.bytes + added > quotas.namespace_bytes {
                return refuse(Limit::NamespaceBytes);
            }
            if extension.bytes + added > quotas.extension_bytes {
                return refuse(Limit::ExtensionBytes);
            }
            if installation.bytes + added > quotas.installation_bytes {
                return refuse(Limit::InstallationBytes);
            }
            if state.is_none() {
                tx.execute(
                    "INSERT INTO namespaces VALUES (?1,?2,'open')",
                    params![ext, ns.0.as_slice()],
                )
                .map_err(db)?;
            }
            let expires = now.saturating_add(limits::OBJECT_STAGING.as_millis() as u64);
            tx.execute(
                "INSERT INTO intents VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'staging',0,0,?10,1,1)",
                params![
                    ext,
                    spec.intent.0.as_slice(),
                    ns.0.as_slice(),
                    spec.key.object.0.as_slice(),
                    spec.payload_sha256.as_slice(),
                    spec.payload_bytes as i64,
                    spec.binding,
                    now as i64,
                    expires as i64,
                    payload as i64
                ],
            )
            .map_err(db)?;
            Ok(Adoption::Fresh(Row {
                extension: ext.to_owned(),
                spec: spec.clone(),
                adopted_ms: now,
                expires_ms: expires,
                phase: Phase::Staging,
                next_index: 0,
                received: 0,
                staged: true,
            }))
        })
    }

    /// Read-only lookup for `status`.
    pub fn row(&self, ext: &str, intent: &IntentId) -> BackendResult<Option<Row>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| BackendError::Unavailable)?;
        fetch(&connection, ext, intent)
    }
    /// The committed row of a key in an open namespace.
    pub fn blob(&self, ext: &str, key: &BlobKey) -> BackendResult<Option<Row>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| BackendError::Unavailable)?;
        if namespace_state(&connection, ext, &key.namespace)?.as_deref() != Some("open") {
            return Ok(None);
        }
        connection
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM intents WHERE extension=?1 AND namespace=?2 \
                     AND object=?3 AND phase='committed'"
                ),
                params![ext, key.namespace.0.as_slice(), key.object.0.as_slice()],
                read_row,
            )
            .optional()
            .map_err(db)
    }
    pub fn usage(&self, ext: &str, scope: Scope) -> BackendResult<Usage> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| BackendError::Unavailable)?;
        let totals = match scope {
            Scope::Extension => totals(&connection, Filter::Extension(ext))?,
            Scope::Namespace(ns) => totals(&connection, Filter::Namespace(ext, &ns.0))?,
        };
        Ok(usage(totals))
    }
    pub fn installation_usage(&self) -> BackendResult<Usage> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| BackendError::Unavailable)?;
        Ok(usage(totals(&connection, Filter::Installation)?))
    }

    /// Gate an append or commit: the intent must be staging in an open namespace.
    /// A staging row past its deadline becomes `expiring` (one durable result).
    pub fn gate(
        &self,
        ext: &str,
        intent: &IntentId,
        now: u64,
        complete: bool,
    ) -> BackendResult<Gate> {
        self.run(|tx| {
            let mut row = fetch(tx, ext, intent)?.ok_or(BackendError::Missing)?;
            if row.phase != Phase::Staging {
                return Err(BackendError::Conflict);
            }
            if now >= row.expires_ms {
                transition(tx, ext, intent, &[Phase::Staging], Phase::Expiring)?;
                row.phase = Phase::Expiring;
                return Ok(Gate::Expired(row));
            }
            if namespace_state(tx, ext, &row.spec.key.namespace)?.as_deref() != Some("open") {
                return Err(BackendError::Conflict);
            }
            if complete && row.received != row.spec.payload_bytes {
                return Err(BackendError::Conflict);
            }
            Ok(Gate::Go(row))
        })
    }
    /// Close a staging row whose deadline passed: `staging -> expiring`. Only staging
    /// is eligible; a possibly published original is never downgraded to expired.
    pub fn expire(&self, ext: &str, intent: &IntentId, now: u64) -> BackendResult<Option<Row>> {
        self.run(|tx| {
            let Some(mut row) = fetch(tx, ext, intent)? else {
                return Ok(None);
            };
            if row.phase != Phase::Staging || now < row.expires_ms {
                return Ok(None);
            }
            transition(tx, ext, intent, &[Phase::Staging], Phase::Expiring)?;
            row.phase = Phase::Expiring;
            Ok(Some(row))
        })
    }
    /// Durable checkpoint after the chunk bytes are synced.
    pub fn advance(&self, ext: &str, row: &Row, chunk: u64) -> BackendResult<Row> {
        self.run(|tx| {
            let received = row.received + chunk;
            let changed = tx
                .execute(
                    "UPDATE intents SET next_index=?3,received=?4 WHERE extension=?1 AND intent=?2 \
                     AND phase='staging' AND next_index=?5 AND received=?6 \
                     AND EXISTS(SELECT 1 FROM namespaces WHERE extension=?1 AND namespace=?7 AND state='open')",
                    params![
                        ext,
                        row.spec.intent.0.as_slice(),
                        row.next_index as i64 + 1,
                        received as i64,
                        row.next_index as i64,
                        row.received as i64,
                        row.spec.key.namespace.0.as_slice()
                    ],
                )
                .map_err(db)?;
            if changed != 1 {
                return Err(BackendError::Conflict);
            }
            let mut next = row.clone();
            next.next_index += 1;
            next.received = received;
            Ok(next)
        })
    }
    /// `staging -> committing`: the commit is adopted; from here the bytes may be published.
    pub fn commit_adopt(&self, ext: &str, intent: &IntentId, now: u64) -> BackendResult<Gate> {
        self.run(|tx| {
            let mut row = fetch(tx, ext, intent)?.ok_or(BackendError::Missing)?;
            if row.phase != Phase::Staging {
                return Err(BackendError::Conflict);
            }
            if now >= row.expires_ms {
                transition(tx, ext, intent, &[Phase::Staging], Phase::Expiring)?;
                row.phase = Phase::Expiring;
                return Ok(Gate::Expired(row));
            }
            if row.received != row.spec.payload_bytes
                || namespace_state(tx, ext, &row.spec.key.namespace)?.as_deref() != Some("open")
            {
                return Err(BackendError::Conflict);
            }
            transition(tx, ext, intent, &[Phase::Staging], Phase::Committing)?;
            row.phase = Phase::Committing;
            Ok(Gate::Go(row))
        })
    }
    /// The matching receipt is durable: `committing -> committed`.
    pub fn commit_receipt(&self, ext: &str, intent: &IntentId) -> BackendResult<()> {
        self.run(|tx| transition(tx, ext, intent, &[Phase::Committing], Phase::Committed))
    }
    /// A possibly published original that cannot be settled stays charged and closed.
    pub fn mark_unknown(&self, ext: &str, intent: &IntentId) -> BackendResult<()> {
        self.run(|tx| {
            transition(
                tx,
                ext,
                intent,
                &[Phase::Staging, Phase::Committing, Phase::Unknown],
                Phase::Unknown,
            )
        })
    }
    /// Close incomplete staging by creator discard.
    pub fn discard_adopt(&self, ext: &str, intent: &IntentId) -> BackendResult<Row> {
        self.run(|tx| {
            let mut row = fetch(tx, ext, intent)?.ok_or(BackendError::Missing)?;
            match row.phase {
                Phase::Staging => {
                    transition(tx, ext, intent, &[Phase::Staging], Phase::Discarding)?;
                    row.phase = Phase::Discarding;
                    Ok(row)
                }
                Phase::Discarding | Phase::Discarded | Phase::Expiring | Phase::Expired => Ok(row),
                _ => Err(BackendError::Conflict),
            }
        })
    }
    /// Release the payload charge and entry only after its removal is confirmed.
    pub fn close_confirmed(&self, ext: &str, intent: &IntentId) -> BackendResult<()> {
        self.run(|tx| {
            let row = fetch(tx, ext, intent)?.ok_or(BackendError::Missing)?;
            let terminal = match row.phase {
                Phase::Discarding => Phase::Discarded,
                Phase::Expiring => Phase::Expired,
                _ => return Err(BackendError::Conflict),
            };
            tx.execute(
                "UPDATE intents SET phase=?3,payload_charged=0,entry_held=0,staged=0 \
                 WHERE extension=?1 AND intent=?2",
                params![ext, intent.0.as_slice(), terminal.text()],
            )
            .map_err(db)?;
            Ok(())
        })
    }
    /// The staging name of a committed original is confirmed gone.
    pub fn unstage(&self, ext: &str, intent: &IntentId) -> BackendResult<()> {
        self.run(|tx| {
            tx.execute(
                "UPDATE intents SET staged=0 WHERE extension=?1 AND intent=?2 AND phase='committed'",
                params![ext, intent.0.as_slice()],
            )
            .map_err(db)?;
            Ok(())
        })
    }

    /// Commit the namespace fence (creating its tombstone row when it never had one).
    /// Returns `false` once the namespace is already removed.
    pub fn fence(&self, ext: &str, ns: &NamespaceId, quotas: &Quotas) -> BackendResult<bool> {
        self.run(|tx| match namespace_state(tx, ext, ns)?.as_deref() {
            Some("removed") => Ok(false),
            Some(_) => {
                tx.execute(
                    "UPDATE namespaces SET state='removing' WHERE extension=?1 AND namespace=?2",
                    params![ext, ns.0.as_slice()],
                )
                .map_err(db)?;
                Ok(true)
            }
            None => {
                let installation = totals(tx, Filter::Installation)?;
                let extension = totals(tx, Filter::Extension(ext))?;
                let namespace = totals(tx, Filter::Namespace(ext, &ns.0))?;
                let added = limits::OBJECT_FENCE_BYTES;
                if namespace.bytes + added > quotas.namespace_bytes {
                    return Err(BackendError::Capacity(Limit::NamespaceBytes));
                }
                if extension.bytes + added > quotas.extension_bytes {
                    return Err(BackendError::Capacity(Limit::ExtensionBytes));
                }
                if installation.bytes + added > quotas.installation_bytes {
                    return Err(BackendError::Capacity(Limit::InstallationBytes));
                }
                tx.execute(
                    "INSERT INTO namespaces VALUES (?1,?2,'removing')",
                    params![ext, ns.0.as_slice()],
                )
                .map_err(db)?;
                Ok(true)
            }
        })
    }
    /// Every row of a fenced namespace that still holds payload or possible effects.
    pub fn namespace_rows(&self, ext: &str, ns: &NamespaceId) -> BackendResult<Vec<Row>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| BackendError::Unavailable)?;
        let mut query = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM intents WHERE extension=?1 AND namespace=?2 \
                 AND phase NOT IN ({TERMINAL}) ORDER BY adopted_ms,intent"
            ))
            .map_err(db)?;
        query
            .query_map(params![ext, ns.0.as_slice()], read_row)
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)
    }
    /// A fenced namespace converts any live original to `removing` (its files go next).
    pub fn mark_removing(&self, ext: &str, intent: &IntentId) -> BackendResult<()> {
        self.run(|tx| {
            transition(
                tx,
                ext,
                intent,
                &[
                    Phase::Staging,
                    Phase::Committing,
                    Phase::Committed,
                    Phase::Discarding,
                    Phase::Expiring,
                    Phase::Unknown,
                    Phase::Removing,
                ],
                Phase::Removing,
            )
        })
    }
    /// After every payload of the namespace is confirmed removed, release the
    /// charges and close the namespace for good; originals stay as tombstones.
    /// Rows are released in small batches so no transaction (and so no rollback
    /// journal) grows with the namespace; an interrupted release resumes safely
    /// because the physical removal it records was already confirmed.
    pub fn removal_confirmed(&self, ext: &str, ns: &NamespaceId) -> BackendResult<()> {
        loop {
            let released = self.run(|tx| {
                tx.execute(
                    "UPDATE intents SET phase='deleted',payload_charged=0,entry_held=0,staged=0 \
                     WHERE extension=?1 AND phase='removing' AND intent IN (\
                       SELECT intent FROM intents WHERE extension=?1 AND namespace=?2 \
                       AND phase='removing' LIMIT ?3)",
                    params![ext, ns.0.as_slice(), RELEASE_BATCH],
                )
                .map_err(db)
            })?;
            if released < RELEASE_BATCH as usize {
                break;
            }
        }
        self.run(|tx| {
            let pending: i64 = tx
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM intents WHERE extension=?1 AND namespace=?2 \
                         AND phase NOT IN ({TERMINAL})"
                    ),
                    params![ext, ns.0.as_slice()],
                    |r| r.get(0),
                )
                .map_err(db)?;
            if pending != 0 {
                return Err(BackendError::Conflict);
            }
            tx.execute(
                "UPDATE namespaces SET state='removed' WHERE extension=?1 AND namespace=?2 AND state='removing'",
                params![ext, ns.0.as_slice()],
            )
            .map_err(db)?;
            Ok(())
        })
    }
    /// Work an interrupted run left behind, in a stable order.
    pub fn unsettled(&self) -> BackendResult<Unsettled> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| BackendError::Unavailable)?;
        let rows = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM intents WHERE phase IN \
                 ('staging','committing','discarding','expiring','removing') \
                 OR (phase='committed' AND staged=1) ORDER BY adopted_ms,extension,intent"
            ))
            .map_err(db)?
            .query_map([], read_row)
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)?;
        let namespaces = connection
            .prepare("SELECT extension,namespace FROM namespaces WHERE state='removing' ORDER BY extension,namespace")
            .map_err(db)?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, array(r.get(1)?)?)))
            .map_err(db)?
            .map(|r| r.map(|(ext, ns)| (ext, NamespaceId(ns))))
            .collect::<Result<_, _>>()
            .map_err(db)?;
        Ok(Unsettled { rows, namespaces })
    }
    /// Whether any original or fence still needs owned settlement.
    pub fn settled(&self) -> BackendResult<bool> {
        let unsettled = self.unsettled()?;
        Ok(unsettled.rows.iter().all(|row| row.phase == Phase::Staging)
            && unsettled.namespaces.is_empty())
    }
}
fn usage(totals: Totals) -> Usage {
    Usage {
        charged_bytes: totals.bytes,
        entries: totals.entries,
        active_uploads: totals.active,
        retained_identities: totals.retained,
    }
}

/// SQLite's derived files: no WAL or shared-memory file may exist, and a hot
/// rollback journal must be an owned regular 0600 file, never a link or foreign file.
fn admit_sidecars(serving: &Serving) -> Result<(), RemoteError> {
    use nix::unistd::Uid;
    let directory = &serving.layout().directory;
    for (suffix, journal) in [("-journal", true), ("-wal", false), ("-shm", false)] {
        let path = directory.join(format!("{FILE}{suffix}"));
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(state_error(error.into())),
            Ok(metadata)
                if journal
                    && metadata.is_file()
                    && metadata.uid() == Uid::effective().as_raw()
                    && metadata.mode() & 0o777 == 0o600
                    && metadata.nlink() == 1 => {}
            Ok(_) => return Err(state_error(tmt_extension_state::Error::UnsafeFile)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
