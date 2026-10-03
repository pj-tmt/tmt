//! Normalized consumption persistence. Provider parsing stays with the driver;
//! history, source coordinates and the opaque resume state commit in one CAS.
use super::{
    IdentityContextSnapshot, Storage, StorageError, StorageErrorCode, bindings::BindingRows,
    errors::classify, identities::with_immediate_transaction,
};
use crate::runtime::consumption::Consumption;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Instant;
use tmt_core::{
    binding::{BindingRecords, session::SessionPreferences},
    limits::MAX_JS_SAFE_INTEGER,
};

pub const BUCKET_MS: u64 = 5_000;
pub const HISTORY_MS: u64 = 2 * 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumptionLatest {
    pub driver: String,
    pub session: String,
    pub consumption: Consumption,
}

/// A coordinator already admitted this binding/session against live evidence.
/// `sampled` distinguishes a real read (including failure) from activity updates.
pub struct RuntimeObservation<'a> {
    pub expected: &'a IdentityContextSnapshot,
    pub preferences: &'a SessionPreferences,
    pub remember_source: bool,
    pub locator: Option<&'a str>,
    pub sampled: bool,
    pub consumption: Option<Consumption>,
    pub now_ms: u64,
    pub deadline: Instant,
}

fn integer(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}
fn optional_integer(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Integer,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

fn invalid() -> StorageError {
    StorageError::new(
        StorageErrorCode::Corrupt,
        "Invalid consumption history evidence",
    )
}

#[derive(Clone, Default)]
struct Bucket {
    from: u64,
    input: u64,
    output: u64,
    cached: u64,
    covered: u64,
    incomplete: bool,
    gap: bool,
    discontinuous: bool,
    sampled: Option<u64>,
    latest: Option<String>,
}

impl Bucket {
    fn read(connection: &Connection, id: &str, from: u64) -> Result<Self, StorageError> {
        connection.query_row("SELECT input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,sampled_at_ms,latest FROM consumption_buckets WHERE identity_id=? AND from_ms=?", params![id,from as i64], |row| Ok(Self {
            from, input:integer(row,0)?,output:integer(row,1)?,cached:integer(row,2)?,covered:integer(row,3)?,incomplete:!row.get::<_,bool>(4)?,gap:row.get(5)?,discontinuous:row.get(6)?,sampled:optional_integer(row,7)?,latest:row.get(8)?
        })).optional().map(|value| value.unwrap_or(Self { from, ..Self::default() })).map_err(|e| classify(e,"Read consumption bucket"))
    }

    fn write(&self, connection: &Connection, id: &str) -> Result<(), StorageError> {
        if self
            .input
            .checked_add(self.output)
            .is_none_or(|n| n > MAX_JS_SAFE_INTEGER)
            || self.cached > self.input
            || self.covered > BUCKET_MS
        {
            return Err(invalid());
        }
        connection.execute("INSERT INTO consumption_buckets(identity_id,from_ms,input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,sampled_at_ms,latest) VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(identity_id,from_ms) DO UPDATE SET input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cached_input_tokens=excluded.cached_input_tokens,covered_ms=excluded.covered_ms,complete=excluded.complete,gap=excluded.gap,discontinuous=excluded.discontinuous,sampled_at_ms=excluded.sampled_at_ms,latest=excluded.latest", params![id,self.from as i64,self.input as i64,self.output as i64,self.cached as i64,self.covered as i64,!self.incomplete,self.gap,self.discontinuous,self.sampled.map(|n|n as i64),self.latest]).map_err(|e|classify(e,"Write consumption bucket"))?;
        Ok(())
    }
}

impl Storage {
    /// No late write, renamed/replaced binding, session switch or losing race can
    /// advance the source cursor without committing its exact history delta.
    pub fn commit_runtime_observation(
        &mut self,
        observation: RuntimeObservation<'_>,
    ) -> Result<bool, StorageError> {
        let Some(binding) = observation.expected.entry.binding.as_ref() else {
            return Ok(false);
        };
        with_immediate_transaction(self, "runtime observation", |connection| {
            if Instant::now() >= observation.deadline {
                return Ok(false);
            }
            let mut records = BindingRows(connection);
            if records
                .entry_by_id(&binding.identity_id)?
                .and_then(|entry| entry.binding)
                .as_ref()
                != Some(binding)
                || records.session_preferences(&binding.identity_id)?
                    != observation.expected.preferences
            {
                return Ok(false);
            }
            if !records.set_session_preferences(&binding.identity_id, observation.preferences)? {
                return Ok(false);
            }
            let Some(remembered) = observation.preferences.remembered.as_ref() else {
                return Ok(true);
            };
            let id = &binding.identity_id;
            if observation.remember_source || observation.sampled {
                connection.execute("INSERT INTO consumption_sources(identity_id,binding_id,driver,session,locator) VALUES(?,?,?,?,?) ON CONFLICT(identity_id) DO UPDATE SET binding_id=excluded.binding_id,driver=excluded.driver,session=excluded.session,locator=CASE WHEN consumption_sources.binding_id=excluded.binding_id AND consumption_sources.driver=excluded.driver AND consumption_sources.session=excluded.session THEN COALESCE(excluded.locator,consumption_sources.locator) ELSE excluded.locator END",params![id,binding.id,remembered.harness.as_str(),remembered.provider_session.as_str(),observation.locator]).map_err(|e|classify(e,"Remember consumption source"))?;
            }
            if observation.sampled {
                let latest = observation
                    .consumption
                    .map(|consumption| ConsumptionLatest {
                        driver: remembered.harness.as_str().to_owned(),
                        session: remembered.provider_session.as_str().to_owned(),
                        consumption,
                    });
                record_sample(connection, id, latest, observation.now_ms)?;
            }
            if Instant::now() >= observation.deadline {
                return Err(invalid());
            }
            Ok(true)
        })
    }

    pub fn consumption_locator(
        &self,
        id: &str,
        binding: &str,
        driver: &str,
        session: &str,
    ) -> Result<Option<String>, StorageError> {
        self.connection()?.query_row("SELECT locator FROM consumption_sources WHERE identity_id=? AND binding_id=? AND driver=? AND session=?",params![id,binding,driver,session],|row|row.get::<_,Option<String>>(0)).optional().map(Option::flatten).map_err(|e|classify(e,"Read consumption locator"))
    }

    /// One deferred read snapshot; no source access or history renewal/pruning.
    pub fn consumption_history(
        &mut self,
        ids: &[String],
        windows: &[u64],
        max_buckets: u64,
        now: u64,
    ) -> Result<Value, StorageError> {
        if now > MAX_JS_SAFE_INTEGER
            || max_buckets == 0
            || windows
                .iter()
                .any(|window| *window < BUCKET_MS || *window > 3_600_000 || window % BUCKET_MS != 0)
        {
            return Err(invalid());
        }
        let transaction = self
            .connection_mut()?
            .transaction()
            .map_err(|e| classify(e, "Read consumption history snapshot"))?;
        let through = now / BUCKET_MS * BUCKET_MS;
        let retained = through.saturating_sub(HISTORY_MS);
        let mut identities = Vec::new();
        for id in ids {
            let found: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM identities WHERE id=? AND retired_at_ms IS NULL)",
                    [id],
                    |row| row.get(0),
                )
                .map_err(|e| classify(e, "Read consumption identity"))?;
            if !found {
                identities.push(json!({"id":id,"found":false}));
                continue;
            }
            let mut statement = transaction.prepare("SELECT from_ms FROM consumption_buckets WHERE identity_id=? AND from_ms>=? AND from_ms<? ORDER BY from_ms").map_err(|e|classify(e,"Read retained consumption buckets"))?;
            let starts: Vec<u64> = statement
                .query_map(params![id, retained as i64, through as i64], |row| {
                    integer(row, 0)
                })
                .and_then(|rows| rows.collect())
                .map_err(|e| classify(e, "Read retained consumption starts"))?;
            let rows: Vec<_> = starts
                .into_iter()
                .map(|from| Bucket::read(&transaction, id, from))
                .collect::<Result<_, _>>()?;
            let last = rows.iter().rev().find(|row| row.sampled.is_some());
            let latest: Option<ConsumptionLatest> = last
                .and_then(|row| row.latest.as_deref())
                .map(serde_json::from_str)
                .transpose()
                .map_err(|_| invalid())?;
            if latest
                .as_ref()
                .is_some_and(|value| !value.consumption.valid())
            {
                return Err(invalid());
            }
            let reporting = rows.iter().any(|row| row.latest.is_some());
            let available = rows
                .iter()
                .filter(|row| row.latest.is_some())
                .filter_map(|row| row.sampled)
                .min();
            let mut readings = Vec::new();
            for window in windows {
                let from = through.saturating_sub(*window);
                let slots = *window / BUCKET_MS;
                let width = slots.div_ceil(max_buckets) * BUCKET_MS;
                let mut buckets = Vec::new();
                let mut start = from;
                while start < through {
                    let end = (start + width).min(through);
                    let mut aggregate = Bucket {
                        from: start,
                        ..Bucket::default()
                    };
                    for row in rows
                        .iter()
                        .filter(|row| row.from >= start && row.from < end)
                    {
                        aggregate.input =
                            aggregate.input.checked_add(row.input).ok_or_else(invalid)?;
                        aggregate.output = aggregate
                            .output
                            .checked_add(row.output)
                            .ok_or_else(invalid)?;
                        aggregate.cached = aggregate
                            .cached
                            .checked_add(row.cached)
                            .ok_or_else(invalid)?;
                        aggregate.covered = aggregate
                            .covered
                            .checked_add(row.covered)
                            .ok_or_else(invalid)?;
                        aggregate.incomplete |= row.incomplete;
                        aggregate.gap |= row.gap;
                        aggregate.discontinuous |= row.discontinuous;
                    }
                    if aggregate
                        .input
                        .checked_add(aggregate.output)
                        .is_none_or(|n| n > MAX_JS_SAFE_INTEGER)
                    {
                        return Err(invalid());
                    }
                    let complete =
                        aggregate.covered == end - start && !aggregate.incomplete && !aggregate.gap;
                    buckets.push(json!({"fromMs":start,"toMs":end,"inputTokens":aggregate.input,"outputTokens":aggregate.output,"cachedInputTokens":aggregate.cached,"coveredMs":aggregate.covered,"complete":complete,"gap":aggregate.gap || aggregate.covered<end-start,"discontinuous":aggregate.discontinuous}));
                    start = end;
                }
                readings.push(json!({"windowMs":window,"fromMs":from,"toMs":through,"bucketMs":width,"buckets":buckets}));
            }
            identities.push(json!({"id":id,"found":true,"reporting":reporting,"availableFromMs":available,"lastSampleAtMs":last.filter(|row|row.latest.is_some()).and_then(|row|row.sampled),"latest":latest,"windows":readings}));
        }
        transaction
            .commit()
            .map_err(|e| classify(e, "Close consumption history snapshot"))?;
        Ok(
            json!({"asOfMs":now,"throughMs":through,"retainedFromMs":retained,"resolutionMs":BUCKET_MS,"identities":identities}),
        )
    }
}

fn record_sample(
    connection: &Connection,
    id: &str,
    next: Option<ConsumptionLatest>,
    now: u64,
) -> Result<(), StorageError> {
    if now == 0
        || now > MAX_JS_SAFE_INTEGER
        || next
            .as_ref()
            .is_some_and(|value| !value.consumption.valid())
    {
        return Err(invalid());
    }
    let (old_time, old_json): (Option<u64>, Option<String>) = connection
        .query_row(
            "SELECT sampled_at_ms,latest FROM consumption_sources WHERE identity_id=?",
            [id],
            |row| Ok((optional_integer(row, 0)?, row.get(1)?)),
        )
        .map_err(|e| classify(e, "Read previous consumption observation"))?;
    if old_time.is_some_and(|old| now < old) {
        return Err(invalid());
    }
    let old: Option<ConsumptionLatest> = old_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| invalid())?;
    if old.as_ref().is_some_and(|value| !value.consumption.valid()) {
        return Err(invalid());
    }
    let continuous = old.as_ref().zip(next.as_ref()).filter(|(old, next)| {
        old.driver == next.driver
            && old.session == next.session
            && old.consumption.epoch == next.consumption.epoch
            && !next.consumption.gap
            && next.consumption.sequence >= old.consumption.sequence
            && next.consumption.observed_at_ms >= old.consumption.observed_at_ms
            && next.consumption.input_tokens >= old.consumption.input_tokens
            && next.consumption.output_tokens >= old.consumption.output_tokens
            && next.consumption.cached_input_tokens >= old.consumption.cached_input_tokens
            && (next.consumption.sequence != old.consumption.sequence
                || next.consumption == old.consumption)
    });
    let from = now / BUCKET_MS * BUCKET_MS;
    let mut bucket = Bucket::read(connection, id, from)?;
    let encoded = next
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| invalid())?;
    bucket.latest = encoded.clone();
    bucket.sampled = Some(now);
    bucket.incomplete |= next
        .as_ref()
        .is_none_or(|value| !value.consumption.complete);
    bucket.gap |= continuous.is_none();
    bucket.discontinuous |= old.as_ref().zip(next.as_ref()).is_some_and(|(old, next)| {
        old.driver != next.driver
            || old.session != next.session
            || old.consumption.epoch != next.consumption.epoch
    });
    if let Some((old, next)) = continuous {
        bucket.input = bucket
            .input
            .checked_add(next.consumption.input_tokens - old.consumption.input_tokens)
            .ok_or_else(invalid)?;
        bucket.output = bucket
            .output
            .checked_add(next.consumption.output_tokens - old.consumption.output_tokens)
            .ok_or_else(invalid)?;
        bucket.cached = bucket
            .cached
            .checked_add(next.consumption.cached_input_tokens - old.consumption.cached_input_tokens)
            .ok_or_else(invalid)?;
    }
    bucket.write(connection, id)?;
    if let Some((old, next)) = continuous
        && old.consumption.complete
        && next.consumption.complete
        && let Some(start) = old_time
        && now.saturating_sub(start) <= 2 * BUCKET_MS
    {
        // Accepted reads are serialized through this transaction, so these
        // adjacent intervals do not overlap, even when Stop races the sampler.
        let mut cursor = start;
        while cursor < now {
            let slot = cursor / BUCKET_MS * BUCKET_MS;
            let end = (slot + BUCKET_MS).min(now);
            let mut covered = Bucket::read(connection, id, slot)?;
            covered.covered = covered
                .covered
                .checked_add(end - cursor)
                .ok_or_else(invalid)?;
            covered.write(connection, id)?;
            cursor = end;
        }
    }
    connection
        .execute(
            "UPDATE consumption_sources SET sampled_at_ms=?,latest=? WHERE identity_id=?",
            params![now as i64, encoded, id],
        )
        .map_err(|e| classify(e, "Advance consumption observation"))?;
    let cutoff = from.saturating_sub(HISTORY_MS);
    connection
        .execute(
            "DELETE FROM consumption_buckets WHERE identity_id=? AND from_ms<?",
            params![id, cutoff as i64],
        )
        .map_err(|e| classify(e, "Prune identity consumption history"))?;
    connection.execute("DELETE FROM consumption_buckets WHERE rowid IN (SELECT rowid FROM consumption_buckets WHERE from_ms<? ORDER BY from_ms LIMIT 256)",[cutoff as i64]).map_err(|e|classify(e,"Prune inactive consumption history"))?;
    Ok(())
}

#[cfg(test)]
mod tests;
