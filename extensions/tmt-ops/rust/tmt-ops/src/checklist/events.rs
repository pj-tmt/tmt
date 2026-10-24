//! The checklist mutation log: one row per applied change, written in the transaction that
//! replaces the document and pruned by the same writer (#4882). It copies no title or body;
//! a reader that wants the content reads the list.
use super::model::{AppliedChange, Id};
use rusqlite::{OptionalExtension, Transaction, params};

/// Events older than this are pruned...
pub const RETAIN_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// ...and so is everything beyond the newest this many events of one room.
pub const RETAIN_PER_ROOM: u64 = 1000;
/// The most one read returns.
pub const MAX_PAGE: u64 = 500;

/// A row as it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub id: u64,
    pub room_id: String,
    pub member_id: Option<String>,
    pub checklist_id: String,
    pub item_id: Option<String>,
    pub actor_id: String,
    pub at_ms: u64,
    pub action: String,
    pub revision: u64,
}

/// Why a reader cannot trust that it saw every change after its cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// The list was changed before the log existed; asked from the beginning, history is unknown.
    Legacy,
    /// Retention removed events after the cursor.
    Pruned,
}
impl Gap {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Pruned => "pruned",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub events: Vec<Logged>,
    /// The id of the last event returned, else the cursor that was asked.
    pub next_cursor: u64,
    /// More events wait after `next_cursor`.
    pub more: bool,
    pub gap: Option<Gap>,
}

/// Where a change landed: the document's slot key and identity.
pub struct Target<'a> {
    pub key: &'a str,
    pub room_id: &'a Id,
    pub member_id: Option<&'a Id>,
    pub checklist_id: &'a Id,
}

fn stored(value: u64) -> rusqlite::Result<i64> {
    i64::try_from(value).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}
fn loaded(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Integer, Box::new(e))
    })
}

/// Records `event` and prunes the room. `existed` says the document was there before the change:
/// a list that already had changes the log never saw is marked `legacy` once.
pub fn append(
    transaction: &Transaction<'_>,
    target: &Target<'_>,
    event: &AppliedChange,
    existed: bool,
) -> rusqlite::Result<()> {
    if existed {
        transaction.execute(
            "INSERT OR IGNORE INTO checklist_event_floor(checklist_key, legacy)
             SELECT ?1, 1 WHERE NOT EXISTS (SELECT 1 FROM checklist_events WHERE checklist_key = ?1)",
            [target.key],
        )?;
    }
    transaction.execute(
        "INSERT INTO checklist_events(checklist_key, room_id, member_id, checklist_id, item_id, actor_id, at_ms, action, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            target.key,
            target.room_id.as_str(),
            target.member_id.map(Id::as_str),
            target.checklist_id.as_str(),
            event.item_id.as_ref().map(Id::as_str),
            event.by.as_str(),
            stored(event.at_ms)?,
            event.action.as_str(),
            stored(event.revision)?,
        ],
    )?;
    prune(transaction, target.room_id, event.at_ms)
}

/// Keeps the shorter of the two bounds. Each list that loses events records the newest id lost,
/// so a reader whose cursor predates it is told.
fn prune(transaction: &Transaction<'_>, room: &Id, now_ms: u64) -> rusqlite::Result<()> {
    let oldest_kept_ms = stored(now_ms.saturating_sub(RETAIN_MS))?;
    let beyond = "(at_ms < ?2 OR id <= COALESCE((SELECT id FROM checklist_events WHERE room_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?3), 0))";
    let args = params![room.as_str(), oldest_kept_ms, stored(RETAIN_PER_ROOM)?];
    transaction.execute(
        &format!(
            "INSERT INTO checklist_event_floor(checklist_key, legacy, pruned_through)
             SELECT checklist_key, 0, MAX(id) FROM checklist_events WHERE room_id = ?1 AND {beyond} GROUP BY checklist_key
             ON CONFLICT(checklist_key) DO UPDATE SET pruned_through = MAX(pruned_through, excluded.pruned_through)"
        ),
        args,
    )?;
    transaction.execute(
        &format!("DELETE FROM checklist_events WHERE room_id = ?1 AND {beyond}"),
        args,
    )?;
    Ok(())
}

/// Events of one list after `after`, oldest first, at most `limit` (capped at [`MAX_PAGE`]).
pub fn read(
    transaction: &Transaction<'_>,
    key: &str,
    after: u64,
    limit: u64,
) -> rusqlite::Result<Page> {
    let limit = limit.clamp(1, MAX_PAGE);
    let mut statement = transaction.prepare(
        "SELECT id, room_id, member_id, checklist_id, item_id, actor_id, at_ms, action, revision
         FROM checklist_events WHERE checklist_key = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
    )?;
    let mut events = statement
        .query_map(params![key, stored(after)?, stored(limit + 1)?], |row| {
            Ok(Logged {
                id: loaded(row.get(0)?)?,
                room_id: row.get(1)?,
                member_id: row.get(2)?,
                checklist_id: row.get(3)?,
                item_id: row.get(4)?,
                actor_id: row.get(5)?,
                at_ms: loaded(row.get(6)?)?,
                action: row.get(7)?,
                revision: loaded(row.get(8)?)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = events.len() as u64 > limit;
    events.truncate(limit as usize);
    let floor = transaction
        .query_row(
            "SELECT legacy, pruned_through FROM checklist_event_floor WHERE checklist_key = ?1",
            [key],
            |row| Ok((row.get::<_, i64>(0)? == 1, loaded(row.get(1)?)?)),
        )
        .optional()?;
    let gap = match floor {
        Some((_, pruned_through)) if pruned_through > after => Some(Gap::Pruned),
        Some((true, _)) if after == 0 => Some(Gap::Legacy),
        _ => None,
    };
    Ok(Page {
        next_cursor: events.last().map_or(after, |last| last.id),
        events,
        more,
        gap,
    })
}

#[cfg(test)]
mod tests;
