//! Private usage snapshots and valuation reads share the settlement store.

use super::{Store, io, number, optional_number, prune};
use crate::{
    core::Error,
    history::{
        MAX, invalid,
        usage::{MINUTE, UsagePage},
    },
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

pub(super) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS usage_state(id TEXT PRIMARY KEY, through INTEGER NOT NULL, model_id TEXT, observed INTEGER);
    CREATE TABLE IF NOT EXISTS usage_minutes(identity_id TEXT NOT NULL, minute INTEGER NOT NULL, collected INTEGER NOT NULL, input_tokens INTEGER, PRIMARY KEY(identity_id,minute));
    CREATE INDEX IF NOT EXISTS usage_expiry ON usage_minutes(minute);";
pub(super) const ROW_BYTES: u64 = 128;

impl Store {
    pub(in crate::history) fn record_usage(
        &mut self,
        id: &str,
        page: &UsagePage,
    ) -> Result<(), Error> {
        if !crate::settings::canonical_uuid(id)
            || page.observed > MAX
            || page.through > page.observed
            || page
                .model
                .as_ref()
                .is_some_and(|(model, time)| model.len() > 128 || *time > page.through)
            || page.minutes.iter().any(|minute| {
                minute.start % MINUTE != 0
                    || minute
                        .start
                        .checked_add(MINUTE)
                        .is_none_or(|end| end > page.through)
                    || minute.input.is_some_and(|n| n > MAX)
            })
        {
            return Err(invalid("Invalid usage snapshot"));
        }
        self.write(|tx| {
            let started: Option<u64> = tx
                .query_row("SELECT started FROM members WHERE id=?1", [id], |r| {
                    number(r, 0)
                })
                .optional()
                .map_err(io)?;
            let Some(started) = started else {
                return Ok(());
            };
            let previous: Option<u64> = tx
                .query_row("SELECT through FROM usage_state WHERE id=?1", [id], |r| {
                    number(r, 0)
                })
                .optional()
                .map_err(io)?;
            if previous.is_some_and(|time| time > page.through) {
                return Err(invalid("Usage watermark moved backwards"));
            }
            for minute in &page.minutes {
                if minute.start < started {
                    continue;
                }
                // A closed minute is a snapshot, so overlap/restart replaces rather
                // than adds it. Later source reads may establish missing coverage.
                tx.execute("INSERT INTO usage_minutes VALUES(?1,?2,?3,?4) ON CONFLICT(identity_id,minute) DO UPDATE SET input_tokens=excluded.input_tokens",
                    params![id,minute.start as i64,page.observed as i64,minute.input.map(|n|n as i64)]).map_err(io)?;
            }
            let (model, observed) = page.model.as_ref().map_or((None, None), |(model, time)| {
                (Some(model.as_str()), Some(*time as i64))
            });
            tx.execute("INSERT INTO usage_state VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET through=excluded.through,model_id=excluded.model_id,observed=excluded.observed",params![id,page.through as i64,model,observed]).map_err(io)?;
            prune(tx, page.observed, super::RAW_LIMIT)?;
            Ok(())
        })
    }

    pub(super) fn valuations(
        tx: &Transaction<'_>,
        id: &str,
        tokens: Option<u64>,
        from: u64,
        through: u64,
        now: u64,
        recent: bool,
    ) -> (Value, Value) {
        let cost = tokens
            .and_then(|tokens| {
                let model: Option<(Option<String>, Option<u64>)> = tx
                    .query_row(
                        "SELECT model_id,observed FROM usage_state WHERE id=?1",
                        [id],
                        |r| Ok((r.get(0)?, optional_number(r, 1)?)),
                    )
                    .optional()
                    .ok()?;
                let (Some(model), Some(observed)) = model? else {
                    return None;
                };
                if observed > now {
                    return None;
                }
                crate::pricing::quote(&model, observed, tokens)
            })
            .unwrap_or(Value::Null);
        let percent = if recent && tokens.is_some() {
            percentage(tx, id, from, through).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        (cost, percent)
    }
}

fn percentage(tx: &Transaction<'_>, id: &str, from: u64, through: u64) -> Result<Value, Error> {
    let from = from.div_ceil(MINUTE) * MINUTE;
    let through = through / MINUTE * MINUTE;
    if from >= through {
        return Ok(Value::Null);
    }
    let (count,known,observed): (u64,u64,Option<u64>) = tx.query_row("SELECT COUNT(*),COUNT(input_tokens),SUM(input_tokens) FROM usage_minutes WHERE identity_id=?1 AND minute>=?2 AND minute<?3",params![id,from as i64,through as i64],|r|Ok((number(r,0)?,number(r,1)?,optional_number(r,2)?))).map_err(io)?;
    if count != (through - from) / MINUTE || known != count {
        return Ok(Value::Null);
    }
    let Some(observed) = observed else {
        return Ok(Value::Null);
    };
    let avoided: Option<u64> = tx.query_row("SELECT CASE WHEN COUNT(tokens)=COUNT(*) THEN COALESCE(SUM(tokens),0) END FROM events WHERE identity_id=?1 AND settled>?2 AND settled<=?3",params![id,from as i64,through as i64],|r|optional_number(r,0)).map_err(io)?;
    Ok(avoided
        .and_then(|avoided| ratio(avoided, observed, from, through))
        .unwrap_or(Value::Null))
}

fn ratio(avoided: u64, observed: u64, from: u64, through: u64) -> Option<Value> {
    let denominator = avoided
        .checked_add(observed)
        .filter(|n| *n > 0 && *n <= MAX)?;
    let points =
        (u128::from(avoided) * 10_000 + u128::from(denominator) / 2) / u128::from(denominator);
    Some(
        json!({"basisPoints":points as u64,"estimatedInputTokensAvoided":avoided,
        "observedInputTokens":observed,"fromMs":from,"throughMs":through}),
    )
}

#[cfg(test)]
mod tests;
