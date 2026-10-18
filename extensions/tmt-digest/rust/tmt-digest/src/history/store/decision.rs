//! Decision records share the settlement store, retention and byte budget.

use super::{Store, io, number, optional_number};
use crate::{
    core::Error,
    history::{
        MAX,
        decision::{Decision, OUTCOMES, Rule},
        invalid,
    },
};
use rusqlite::{OptionalExtension, params};

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS decisions(id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, identity_id TEXT NOT NULL, interval_ms INTEGER, flush_count INTEGER NOT NULL, rule TEXT NOT NULL, held INTEGER NOT NULL, oldest_age_ms INTEGER, outcome TEXT);
    CREATE INDEX IF NOT EXISTS decision_time ON decisions(at_ms);
    CREATE INDEX IF NOT EXISTS decision_member ON decisions(identity_id,at_ms);";
pub(super) const ROW_BYTES: u64 = 160;
/// Decisions never hold more than this many rows (about 4.8 MB, a fifth of the
/// shared raw budget), however often a member's state changes.
pub(super) const MAX_ROWS: u64 = 30_000;

type Raw = (
    u64,
    String,
    Option<u64>,
    u64,
    String,
    u64,
    Option<u64>,
    Option<String>,
);
const COLUMNS: &str = "at_ms,identity_id,interval_ms,flush_count,rule,held,oldest_age_ms,outcome";
fn raw(r: &rusqlite::Row<'_>) -> rusqlite::Result<Raw> {
    Ok((
        number(r, 0)?,
        r.get::<_, String>(1)?,
        optional_number(r, 2)?,
        number(r, 3)?,
        r.get::<_, String>(4)?,
        number(r, 5)?,
        optional_number(r, 6)?,
        r.get::<_, Option<String>>(7)?,
    ))
}
fn decode(raw: Raw) -> Result<Decision, Error> {
    let (at_ms, identity_id, interval_ms, flush_count, rule, held_count, age, outcome) = raw;
    let decision = Decision {
        at_ms,
        identity_id,
        interval_ms,
        flush_count,
        rule: Rule::parse(&rule).ok_or_else(|| invalid("Unknown decision rule"))?,
        held_count,
        oldest_held_age_ms: age,
        outcome: outcome
            .map(|text| {
                OUTCOMES
                    .into_iter()
                    .find(|known| *known == text)
                    .ok_or_else(|| invalid("Unknown decision outcome"))
            })
            .transpose()?,
    };
    decision.validate()?;
    Ok(decision)
}

impl Store {
    /// A tick appends at most a few rows a minute; the next `maintain` enforces retention.
    pub fn record_decision(&mut self, decision: &Decision) -> Result<(), Error> {
        decision.validate()?;
        self.0
            .execute(
                "INSERT INTO decisions(at_ms,identity_id,interval_ms,flush_count,rule,held,oldest_age_ms,outcome) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    decision.at_ms as i64,
                    decision.identity_id,
                    decision.interval_ms.map(|n| n as i64),
                    decision.flush_count as i64,
                    decision.rule.text(),
                    decision.held_count as i64,
                    decision.oldest_held_age_ms.map(|n| n as i64),
                    decision.outcome,
                ],
            )
            .map_err(io)?;
        Ok(())
    }

    /// Records in `(from, through]`, oldest first. A store older than decision
    /// records has none.
    pub fn decisions(&self, from: u64, through: u64) -> Result<Vec<Decision>, Error> {
        if from > through || through > MAX {
            return Err(invalid("Invalid decision window"));
        }
        let present: Option<u64> = self
            .0
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='decisions'",
                [],
                |r| number(r, 0),
            )
            .optional()
            .map_err(io)?;
        if present.is_none() {
            return Ok(vec![]);
        }
        let mut statement = self
            .0
            .prepare(&format!(
                "SELECT {COLUMNS} FROM decisions WHERE at_ms>?1 AND at_ms<=?2 ORDER BY at_ms,id"
            ))
            .map_err(io)?;
        let rows = statement
            .query_map(params![from as i64, through as i64], raw)
            .map_err(io)?;
        let mut decisions = vec![];
        for row in rows {
            decisions.push(decode(row.map_err(io)?)?);
        }
        Ok(decisions)
    }

    /// The member's newest stored record: a tick writes a decision only when it
    /// differs, so a state that persists across ticks stays one row.
    pub fn last_decision(&self, id: &str) -> Result<Option<Decision>, Error> {
        self.0
            .query_row(
                &format!("SELECT {COLUMNS} FROM decisions WHERE identity_id=?1 ORDER BY at_ms DESC,id DESC LIMIT 1"),
                [id],
                raw,
            )
            .optional()
            .map_err(io)?
            .map(decode)
            .transpose()
    }
}
