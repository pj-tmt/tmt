//! Private, bounded SQLite evidence; event and cursor changes share one transaction.

use super::{
    DAY, MAX, METHOD, RETENTION, invalid,
    model::{ChecklistSettlement, Window},
};
use crate::{core::Error, pricing::Table};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::OpenOptionsExt, path::Path};

mod decision;
mod settlement;
mod usage;

pub use settlement::Coverage;

// Reserve half the 64 MiB budget for SQLite's rollback journal. Raw payloads leave
// room for compact identity totals and indexes; exhausting that room fails closed.
// Journal page headers also fit within the reserved half.
const PAGES: u64 = 8160;
const RAW_LIMIT: u64 = 24 * 1024 * 1024;
pub struct Store(Connection);
struct Snapshot {
    started: u64,
    through: u64,
    counter: u64,
    pending: u64,
    tokens: Option<u64>,
    avoided: Option<u64>,
    gap: Option<String>,
    raw_from: u64,
}
fn io(error: impl std::fmt::Display) -> Error {
    Error::new("DIGEST_HISTORY_IO", error.to_string())
}

fn number(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value)
        .ok()
        .filter(|n| *n <= MAX)
        .ok_or_else(|| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn optional_number(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    if matches!(row.get_ref(index)?, rusqlite::types::ValueRef::Null) {
        Ok(None)
    } else {
        number(row, index).map(Some)
    }
}
fn private(path: &Path, directory: bool) -> Result<(), Error> {
    if crate::private::is_private(path, directory).map_err(io)? {
        return Ok(());
    }
    Err(not_private())
}
fn not_private() -> Error {
    invalid("History paths must be private regular files/directories without symlinks")
}
impl Store {
    pub fn open(root: &Path, write: bool) -> Result<Option<Self>, Error> {
        let digest = root.join("digest");
        let directory = digest.join("history");
        if write {
            for path in [&digest, &directory] {
                if !crate::private::ensure_dir(path).map_err(io)? {
                    return Err(not_private());
                }
            }
        } else {
            for path in [&digest, &directory] {
                if !path.try_exists().map_err(io)? {
                    return Ok(None);
                }
                private(path, true)?;
            }
        }
        let path = directory.join("history.sqlite");
        if write {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(nix::libc::O_NOFOLLOW)
                .open(&path)
            {
                Ok(file) => file.sync_all().map_err(io)?,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io(e)),
            }
        } else if !path.try_exists().map_err(io)? {
            return Ok(None);
        }
        private(&path, false)?;
        if fs::metadata(&path).map_err(io)?.len() > 32 * 1024 * 1024 {
            return Err(invalid("History store exceeds its budget"));
        }
        let journal = directory.join("history.sqlite-journal");
        if fs::symlink_metadata(&journal).is_ok() {
            private(&journal, false)?;
            if fs::metadata(&journal).map_err(io)?.len() > 32 * 1024 * 1024 {
                return Err(invalid("History journal exceeds its budget"));
            }
        }
        // SQLite's own no-follow admission fences the final file after the metadata check.
        let flags = OpenFlags::SQLITE_OPEN_NOFOLLOW
            | if write {
                OpenFlags::SQLITE_OPEN_READ_WRITE
            } else {
                OpenFlags::SQLITE_OPEN_READ_ONLY
            };
        let mut connection = Connection::open_with_flags(&path, flags).map_err(io)?;
        connection
            .busy_timeout(std::time::Duration::from_millis(100))
            .map_err(io)?;
        if connection
            .pragma_query_value(None, "page_size", |row| number(row, 0))
            .map_err(io)?
            != 4096
        {
            return Err(invalid("Unsupported history page size"));
        }
        let version: u64 = connection
            .pragma_query_value(None, "user_version", |row| number(row, 0))
            .map_err(io)?;
        if version > 3 {
            return Err(invalid("Unsupported history schema"));
        }
        if version == 0
            && connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| number(r, 0),
                )
                .map_err(io)?
                > 0
        {
            return Err(invalid("Unrecognized history store"));
        }
        if write {
            connection.execute_batch(&format!("PRAGMA journal_mode=DELETE; PRAGMA page_size=4096; PRAGMA max_page_count={PAGES}; PRAGMA synchronous=FULL;")).map_err(io)?;
            let tx = connection.transaction().map_err(io)?;
            tx.execute_batch("\
                CREATE TABLE IF NOT EXISTS members(id TEXT PRIMARY KEY, started INTEGER NOT NULL, through INTEGER NOT NULL, cursor INTEGER NOT NULL, counter INTEGER NOT NULL, pending INTEGER NOT NULL DEFAULT 0, tokens INTEGER, avoided INTEGER, gap TEXT, raw_from INTEGER NOT NULL);
                CREATE TABLE IF NOT EXISTS events(identity_id TEXT NOT NULL, checklist_id TEXT NOT NULL, sequence INTEGER NOT NULL, settled INTEGER, collected INTEGER NOT NULL, delivered INTEGER NOT NULL, tokens INTEGER, avoided INTEGER, gap TEXT, method TEXT NOT NULL, decision TEXT, inputs TEXT NOT NULL, PRIMARY KEY(identity_id,checklist_id));
                CREATE INDEX IF NOT EXISTS event_window ON events(identity_id,settled);
                CREATE TABLE IF NOT EXISTS gaps(identity_id TEXT NOT NULL, from_ms INTEGER NOT NULL, through_ms INTEGER NOT NULL, reason TEXT NOT NULL, PRIMARY KEY(identity_id,reason));
                ").map_err(io)?;
            tx.execute_batch(usage::SCHEMA).map_err(io)?;
            tx.execute_batch(decision::SCHEMA).map_err(io)?;
            tx.pragma_update(None, "user_version", 3).map_err(io)?;
            tx.commit().map_err(io)?;
        }
        if write
            && connection
                .pragma_query_value(None, "max_page_count", |row| number(row, 0))
                .map_err(io)?
                > PAGES
        {
            return Err(invalid("History store cannot fit its page budget"));
        }
        if !matches!(
            connection
                .pragma_query_value(None, "user_version", |row| number(row, 0))
                .map_err(io)?,
            1..=3
        ) {
            return Err(invalid("Unsupported history schema"));
        }
        Ok(Some(Self(connection)))
    }
    pub fn maintain(&mut self, now: u64) -> Result<(), Error> {
        if now > MAX {
            return Err(invalid("Unsafe collection timestamp"));
        }
        let tx = self.0.transaction().map_err(io)?;
        prune(&tx, now, RAW_LIMIT)?;
        tx.commit().map_err(io)
    }
    pub fn cursor(&self, id: &str) -> Result<Option<u64>, Error> {
        self.0
            .query_row("SELECT cursor FROM members WHERE id=?1", [id], |r| {
                number(r, 0)
            })
            .optional()
            .map_err(io)
    }
    /// First collection takes an atomic Core watermark as baseline, not historical savings.
    pub fn baseline(
        &mut self,
        id: &str,
        now: u64,
        sequence: u64,
        counter: u64,
    ) -> Result<(), Error> {
        if [now, sequence, counter].into_iter().any(|n| n > MAX) {
            return Err(invalid("Unsafe history watermark"));
        }
        self.0.execute("INSERT OR IGNORE INTO members(id,started,through,cursor,counter,tokens,avoided,raw_from) VALUES(?1,?2,?2,?3,?4,0,0,?2)",params![id,now as i64,sequence as i64,counter as i64]).map_err(io)?;
        Ok(())
    }
    pub fn record(&mut self, id: &str, event: &ChecklistSettlement, now: u64) -> Result<(), Error> {
        event.validate()?;
        if now > MAX {
            return Err(invalid("Unsafe collection timestamp"));
        }
        let inputs = serde_json::to_string(event).map_err(io)?;
        if inputs.len() > 1024 * 1024 {
            return Err(invalid("History event exceeds 1 MiB"));
        }
        let tx = self.0.transaction().map_err(io)?;
        let (cursor, through, tokens, avoided): (u64, u64, Option<u64>, Option<u64>) = tx
            .query_row(
                "SELECT cursor,through,tokens,avoided FROM members WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        number(r, 0)?,
                        number(r, 1)?,
                        optional_number(r, 2)?,
                        optional_number(r, 3)?,
                    ))
                },
            )
            .map_err(io)?;
        if event.settlement_sequence <= cursor {
            return Ok(());
        }
        let estimate = event.estimate();
        let clock_gap = event
            .settled_at_ms
            .is_none_or(|time| time < through || time > now || time < event.claimed_at_ms);
        let gap = if clock_gap {
            Some("clock_discontinuity")
        } else {
            estimate.gap
        };
        tx.execute("INSERT INTO events(identity_id,checklist_id,sequence,settled,collected,delivered,tokens,avoided,gap,method,decision,inputs) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,?11)", params![id,event.checklist_id,event.settlement_sequence as i64,event.settled_at_ms.map(|n|n as i64),now as i64,event.outcome=="delivered",estimate.tokens.map(|n|n as i64),estimate.avoided.map(|n|n as i64),gap,METHOD,inputs]).map_err(io)?;
        let sum = |a: Option<u64>, b: Option<u64>| {
            a.zip(b)
                .and_then(|(a, b)| a.checked_add(b))
                .filter(|n| *n <= MAX)
        };
        let tokens = sum(tokens, estimate.tokens);
        let avoided = sum(avoided, estimate.avoided);
        let gap = gap.or(if tokens.is_none() || avoided.is_none() {
            Some("unsafe_arithmetic")
        } else {
            None
        });
        tx.execute("UPDATE members SET cursor=?2,pending=pending+?3,tokens=?4,avoided=?5,gap=COALESCE(gap,?6) WHERE id=?1", params![id,event.settlement_sequence as i64,event.outcome=="delivered",tokens.map(|n|n as i64),avoided.map(|n|n as i64),gap]).map_err(io)?;
        if clock_gap {
            add_gap(&tx, id, through, now.max(through), "clock_discontinuity")?;
        }
        prune(&tx, now, RAW_LIMIT)?;
        tx.commit().map_err(io)
    }
    /// Checkpoint only after enumeration ends; the durable counter exposes missing deliveries.
    pub fn checkpoint(
        &mut self,
        id: &str,
        now: u64,
        sequence: u64,
        counter: u64,
    ) -> Result<(), Error> {
        if [now, sequence, counter].into_iter().any(|n| n > MAX) {
            return Err(invalid("Unsafe history watermark"));
        }
        let tx = self.0.transaction().map_err(io)?;
        let (through, cursor, previous, pending): (u64, u64, u64, u64) = tx
            .query_row(
                "SELECT through,cursor,counter,pending FROM members WHERE id=?1",
                [id],
                |r| Ok((number(r, 0)?, number(r, 1)?, number(r, 2)?, number(r, 3)?)),
            )
            .map_err(io)?;
        let gap = if now < through || sequence < cursor || counter < previous {
            Some("clock_or_store_discontinuity")
        } else if counter.checked_sub(previous) != Some(pending) {
            Some("missing_deliveries")
        } else {
            None
        };
        if let Some(reason) = gap {
            add_gap(&tx, id, through.min(now), through.max(now), reason)?;
            tx.execute(
                "UPDATE members SET tokens=NULL,avoided=NULL,gap=COALESCE(gap,?2) WHERE id=?1",
                params![id, reason],
            )
            .map_err(io)?;
        }
        tx.execute("UPDATE members SET through=MAX(through,?2),cursor=MAX(cursor,?3),counter=?4,pending=0 WHERE id=?1",params![id,now as i64,sequence as i64,counter as i64]).map_err(io)?;
        prune(&tx, now.max(through), RAW_LIMIT)?;
        tx.commit().map_err(io)
    }
    pub fn status(
        &self,
        id: &str,
        window: Window,
        now: u64,
        observed_counter: Option<u64>,
        prices: &Table,
    ) -> Result<Value, Error> {
        let tx = self.0.unchecked_transaction().map_err(io)?;
        let meta: Option<Snapshot> = tx.query_row("SELECT started,through,counter,pending,tokens,avoided,gap,raw_from FROM members WHERE id=?1",[id], |r| Ok(Snapshot {started:number(r,0)?,through:number(r,1)?,counter:number(r,2)?,pending:number(r,3)?,tokens:optional_number(r,4)?,avoided:optional_number(r,5)?,gap:r.get(6)?,raw_from:number(r,7)?})).optional().map_err(io)?;
        let Some(Snapshot {
            started,
            through,
            counter,
            pending,
            mut tokens,
            mut avoided,
            mut gap,
            raw_from,
        }) = meta
        else {
            return Ok(Value::Null);
        };
        let from = window
            .duration()
            .map_or(started, |duration| through.saturating_sub(duration));
        if window.duration().is_some() {
            let values: (Option<u64>,Option<u64>,Option<String>) = tx.query_row("SELECT CASE WHEN COUNT(tokens)=COUNT(*) THEN COALESCE(SUM(tokens),0) END, CASE WHEN COUNT(avoided)=COUNT(*) THEN COALESCE(SUM(avoided),0) END, MIN(gap) FROM events WHERE identity_id=?1 AND settled>?2 AND settled<=?3",params![id,from as i64,through as i64],|r| Ok((optional_number(r,0)?,optional_number(r,1)?,r.get(2)?))).map_err(io)?;
            (tokens, avoided, gap) = values;
            if from < started {
                gap = Some("before_collection".into());
            } else if from < raw_from {
                gap = Some("history_evicted".into());
            }
            let missing: Option<String> = tx.query_row("SELECT reason FROM gaps WHERE identity_id=?1 AND from_ms<=?3 AND through_ms>=?2 LIMIT 1",params![id,from as i64,through as i64],|r| r.get(0)).optional().map_err(io)?;
            gap = missing.or(gap);
        }
        if now < through {
            gap = Some("clock_discontinuity".into());
        }
        if pending > 0 || observed_counter.is_some_and(|value| value != counter) {
            gap = Some("uncollected_deliveries".into());
        }
        if tokens.is_some_and(|n| n > MAX) || avoided.is_some_and(|n| n > MAX) {
            gap = Some("unsafe_arithmetic".into());
        }
        if gap.is_some() {
            tokens = None;
        }
        if matches!(
            gap.as_deref(),
            Some(
                "missing_deliveries"
                    | "uncollected_deliveries"
                    | "before_collection"
                    | "history_evicted"
                    | "clock_or_store_discontinuity"
                    | "clock_discontinuity"
            )
        ) {
            avoided = None;
        }
        let (cost, percent) = Self::valuations(
            &tx,
            prices,
            id,
            tokens,
            (from, through),
            now,
            window.duration().is_some(),
        );
        Ok(
            json!({"method":METHOD,"window":window.text(),"fromMs":from,"throughMs":through,"estimatedInputTokensAvoided":tokens,"avoidedSeparateDeliveries":avoided,"complete":gap.is_none() && tokens.is_some() && avoided.is_some(),"gap":gap,"cost":cost,"percent":percent}),
        )
    }
}
fn add_gap(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    from: u64,
    through: u64,
    reason: &str,
) -> Result<(), Error> {
    tx.execute("INSERT INTO gaps VALUES(?1,?2,?3,?4) ON CONFLICT(identity_id,reason) DO UPDATE SET from_ms=MIN(from_ms,excluded.from_ms),through_ms=MAX(through_ms,excluded.through_ms)",params![id,from as i64,through as i64,reason]).map_err(io)?;
    Ok(())
}
fn prune(tx: &rusqlite::Transaction<'_>, now: u64, raw_limit: u64) -> Result<(), Error> {
    tx.execute(
        "DELETE FROM usage_minutes WHERE minute<?1",
        [now.saturating_sub(RETENTION) as i64],
    )
    .map_err(io)?;
    tx.execute(
        "UPDATE members SET raw_from=MAX(raw_from,?1)",
        [now.saturating_sub(RETENTION) as i64],
    )
    .map_err(io)?;
    tx.execute(
        "DELETE FROM events WHERE collected<?1",
        [now.saturating_sub(RETENTION) as i64],
    )
    .map_err(io)?;
    tx.execute(
        "DELETE FROM decisions WHERE at_ms<?1",
        [now.saturating_sub(RETENTION) as i64],
    )
    .map_err(io)?;
    let mut bytes: u64 = tx
        .query_row(
            "SELECT COALESCE(SUM(length(inputs)),0) FROM events",
            [],
            |r| number(r, 0),
        )
        .map_err(io)?;
    let mut usage_bytes = tx
        .query_row("SELECT COUNT(*) FROM usage_minutes", [], |r| number(r, 0))
        .map_err(io)?
        * usage::ROW_BYTES;
    let decisions = tx
        .query_row("SELECT COUNT(*) FROM decisions", [], |r| number(r, 0))
        .map_err(io)?;
    if decisions > decision::MAX_ROWS {
        tx.execute(
            "DELETE FROM decisions WHERE id NOT IN (SELECT id FROM decisions ORDER BY at_ms DESC,id DESC LIMIT ?1)",
            [decision::MAX_ROWS as i64],
        )
        .map_err(io)?;
    }
    let mut decision_bytes = decisions.min(decision::MAX_ROWS) * decision::ROW_BYTES;
    while bytes + usage_bytes + decision_bytes > raw_limit {
        let usage_oldest: Option<(String, u64)> = tx
            .query_row(
                "SELECT identity_id,minute FROM usage_minutes ORDER BY minute LIMIT 1",
                [],
                |r| Ok((r.get(0)?, number(r, 1)?)),
            )
            .optional()
            .map_err(io)?;
        let event_oldest: Option<u64> = tx
            .query_row(
                "SELECT collected FROM events ORDER BY collected,sequence LIMIT 1",
                [],
                |r| number(r, 0),
            )
            .optional()
            .map_err(io)?;
        let decision_oldest: Option<u64> = tx
            .query_row(
                "SELECT at_ms FROM decisions ORDER BY at_ms,id LIMIT 1",
                [],
                |r| number(r, 0),
            )
            .optional()
            .map_err(io)?;
        // Oldest evidence goes first, whichever table holds it.
        if let Some(at) = decision_oldest
            && usage_oldest
                .as_ref()
                .is_none_or(|(_, minute)| at <= *minute)
            && event_oldest.is_none_or(|time| at <= time)
        {
            tx.execute(
                "DELETE FROM decisions WHERE id=(SELECT id FROM decisions ORDER BY at_ms,id LIMIT 1)",
                [],
            )
            .map_err(io)?;
            decision_bytes = decision_bytes.saturating_sub(decision::ROW_BYTES);
            continue;
        }
        if let Some((id, minute)) = usage_oldest
            && event_oldest.is_none_or(|time| minute <= time)
        {
            tx.execute(
                "DELETE FROM usage_minutes WHERE identity_id=?1 AND minute=?2",
                params![id, minute as i64],
            )
            .map_err(io)?;
            usage_bytes = usage_bytes.saturating_sub(usage::ROW_BYTES);
            continue;
        }
        let (id,checklist,settled,size): (String,String,Option<u64>,u64) = tx.query_row("SELECT identity_id,checklist_id,settled,length(inputs) FROM events ORDER BY collected,sequence LIMIT 1",[],|r| Ok((r.get(0)?,r.get(1)?,optional_number(r,2)?,number(r,3)?))).map_err(io)?;
        tx.execute(
            "DELETE FROM events WHERE identity_id=?1 AND checklist_id=?2",
            params![id, checklist],
        )
        .map_err(io)?;
        tx.execute(
            "UPDATE members SET raw_from=MAX(raw_from,?2) WHERE id=?1",
            params![id, settled.unwrap_or(now).saturating_add(1).min(MAX) as i64],
        )
        .map_err(io)?;
        bytes = bytes.saturating_sub(size);
    }
    tx.execute(
        "DELETE FROM gaps WHERE through_ms<(SELECT MAX(0,through-?1) FROM members WHERE id=gaps.identity_id)",
        [(7 * DAY) as i64],
    )
    .map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
