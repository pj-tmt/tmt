//! Normalized consumption persistence. Provider parsing stays with the driver;
//! history, source coordinates and the opaque resume state commit in one CAS.
use super::{
    IdentityContextSnapshot, Storage, StorageError, StorageErrorCode, bindings::BindingRows,
    errors::classify, identities::with_immediate_transaction,
};
use crate::runtime::consumption::{
    Consumption, Credits, ModelUsage, RateLimits, valid_service_tier,
};
mod details;
use details::Details;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::time::Instant;
use tmt_core::{
    binding::{BindingRecords, session::SessionPreferences},
    limits::MAX_JS_SAFE_INTEGER,
};

pub const BUCKET_MS: u64 = 5_000;
pub const HISTORY_MS: u64 = 2 * 60 * 60 * 1_000;
/// The rollup keeps 5-minute sums of the 5-second buckets for 30 days (#3248).
pub const ROLLUP_BUCKET_MS: u64 = 300_000;
pub const ROLLUP_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
/// The longest window read from the 5-second buckets; a longer one reads the rollup.
pub const FINE_WINDOW_MS: u64 = 3_600_000;
pub const MAX_BUCKETS: u64 = 720;
/// The most buckets one response carries across its identities and windows.
pub const MAX_RESPONSE_BUCKETS: u64 = 23_040;

/// The resolution that serves `window_ms`, or none when it is not a valid window:
/// 5-second multiples up to an hour read the 5-second buckets, 5-minute multiples
/// above an hour up to 30 days read the rollup.
pub fn window_resolution_ms(window_ms: u64) -> Option<u64> {
    if (BUCKET_MS..=FINE_WINDOW_MS).contains(&window_ms) && window_ms.is_multiple_of(BUCKET_MS) {
        Some(BUCKET_MS)
    } else if (FINE_WINDOW_MS + 1..=ROLLUP_MS).contains(&window_ms)
        && window_ms.is_multiple_of(ROLLUP_BUCKET_MS)
    {
        Some(ROLLUP_BUCKET_MS)
    } else {
        None
    }
}

/// One stored resolution of the same evidence. `snapshot` names the text column:
/// the fine tier keeps the accepted observation there, the rollup only its newest
/// rate limits. `account` marks the rollup's two further columns, the newest credits
/// and the service tier of the newest token-moving read (#3546).
#[derive(Clone, Copy)]
struct Tier {
    table: &'static str,
    snapshot: &'static str,
    width: u64,
    account: bool,
}
const FINE: Tier = Tier {
    table: "consumption_buckets",
    snapshot: "latest",
    width: BUCKET_MS,
    account: false,
};
const ROLLUP: Tier = Tier {
    table: "consumption_tier5m",
    snapshot: "limits",
    width: ROLLUP_BUCKET_MS,
    account: true,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumptionLatest {
    pub driver: String,
    pub session: String,
    pub consumption: Consumption,
}

/// A main-session turn boundary the hook admitted. A passthrough's text is matched to the turn
/// it started by the prompt the provider reported; the turn's end settles what it earned (#4336).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnBoundary<'a> {
    Started {
        turn: Option<&'a str>,
        prompt: &'a str,
    },
    Ended {
        turn: Option<&'a str>,
        /// The end of a continuation after a Stop hook blocked the stop (#4858).
        continuation: bool,
    },
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
    pub response: Option<super::ResponseCapture>,
    /// The main-session turn boundary this event admitted, for the passthrough it settles.
    pub boundary: Option<TurnBoundary<'a>>,
    /// The started sessions' rows, written with the sample that counted them.
    pub family: Option<crate::runtime::consumption::family::Write>,
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
    snapshot: Option<String>,
    /// Rollup only: the newest credits observed inside the bucket, as JSON.
    credits: Option<String>,
    /// Rollup only: the service tier of the newest token-moving read.
    service_tier: Option<String>,
    details: Details,
}

impl Bucket {
    fn read(
        connection: &Connection,
        tier: Tier,
        id: &str,
        from: u64,
    ) -> Result<Self, StorageError> {
        let account = if tier.account {
            ",credits,service_tier"
        } else {
            ""
        };
        connection.query_row(&format!("SELECT input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,sampled_at_ms,{},details,from_ms{account} FROM {} WHERE identity_id=? AND from_ms=?", tier.snapshot, tier.table), params![id,from as i64], |row| Self::from_row(row, from))
            .optional().map(|value| value.unwrap_or(Self { from, ..Self::default() })).map_err(|e| classify(e,"Read consumption bucket"))
    }

    fn from_row(row: &rusqlite::Row<'_>, from: u64) -> rusqlite::Result<Self> {
        let bucket = Self {
            from,
            input: integer(row, 0)?,
            output: integer(row, 1)?,
            cached: integer(row, 2)?,
            covered: integer(row, 3)?,
            incomplete: !row.get::<_, bool>(4)?,
            gap: row.get(5)?,
            discontinuous: row.get(6)?,
            sampled: optional_integer(row, 7)?,
            snapshot: row.get(8)?,
            // The rollup's reads select the two account columns after `from_ms`.
            credits: if row.as_ref().column_count() > 11 {
                row.get(11)?
            } else {
                None
            },
            service_tier: if row.as_ref().column_count() > 11 {
                row.get(12)?
            } else {
                None
            },
            details: row
                .get::<_, Option<String>>(9)?
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        9,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .unwrap_or_default(),
        };
        if !bucket
            .details
            .valid(bucket.input, bucket.output, bucket.cached)
        {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(invalid()),
            ));
        }
        Ok(bucket)
    }

    fn write(&self, connection: &Connection, tier: Tier, id: &str) -> Result<(), StorageError> {
        if self
            .input
            .checked_add(self.output)
            .is_none_or(|n| n > MAX_JS_SAFE_INTEGER)
            || self.cached > self.input
            || self.covered > tier.width
            || !self.details.valid(self.input, self.output, self.cached)
        {
            return Err(invalid());
        }
        let (table, snapshot) = (tier.table, tier.snapshot);
        let (columns, marks, updates) = if tier.account {
            (
                ",credits,service_tier",
                ",?,?",
                ",credits=excluded.credits,service_tier=excluded.service_tier",
            )
        } else {
            ("", "", "")
        };
        let mut values: Vec<rusqlite::types::Value> = vec![
            id.to_owned().into(),
            (self.from as i64).into(),
            (self.input as i64).into(),
            (self.output as i64).into(),
            (self.cached as i64).into(),
            (self.covered as i64).into(),
            (!self.incomplete).into(),
            self.gap.into(),
            self.discontinuous.into(),
            self.sampled.map(|n| n as i64).into(),
            self.snapshot.clone().into(),
            serde_json::to_string(&self.details)
                .map_err(|_| invalid())?
                .into(),
        ];
        if tier.account {
            values.push(self.credits.clone().into());
            values.push(self.service_tier.clone().into());
        }
        connection.execute(&format!("INSERT INTO {table}(identity_id,from_ms,input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,sampled_at_ms,{snapshot},details{columns}) VALUES(?,?,?,?,?,?,?,?,?,?,?,?{marks}) ON CONFLICT(identity_id,from_ms) DO UPDATE SET input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cached_input_tokens=excluded.cached_input_tokens,covered_ms=excluded.covered_ms,complete=excluded.complete,gap=excluded.gap,discontinuous=excluded.discontinuous,sampled_at_ms=excluded.sampled_at_ms,{snapshot}=excluded.{snapshot},details=excluded.details{updates}"), rusqlite::params_from_iter(values)).map_err(|e|classify(e,"Write consumption bucket"))?;
        Ok(())
    }

    /// Folds one accepted read into the bucket. Every tier uses this one rule, so a
    /// rollup bucket is the sum of the fine buckets it covers.
    fn absorb(&mut self, read: &Accepted) -> Result<(), StorageError> {
        self.incomplete |= read.incomplete;
        self.gap |= read.delta.is_none();
        self.discontinuous |= read.discontinuous;
        if let Some(delta) = &read.delta {
            if self.input == 0 && self.output == 0 {
                self.details = delta.details.clone();
            } else if delta.changed {
                self.details.merge(&delta.details)?;
            }
            self.input = self.input.checked_add(delta.input).ok_or_else(invalid)?;
            self.output = self.output.checked_add(delta.output).ok_or_else(invalid)?;
            self.cached = self.cached.checked_add(delta.cached).ok_or_else(invalid)?;
        }
        Ok(())
    }
}

/// What one accepted read adds to a bucket of any tier.
struct Accepted {
    /// The read failed or its source was incomplete.
    incomplete: bool,
    /// An observed driver, session or epoch boundary.
    discontinuous: bool,
    /// Present when the read continues the previous one, so its increments count.
    delta: Option<Delta>,
}

struct Delta {
    input: u64,
    output: u64,
    cached: u64,
    /// The increment moved input or output, so its attribution merges.
    changed: bool,
    details: Details,
}

/// Why an identity that reports nothing is not being sampled, from its binding row:
/// `(runtime_state, an admitted provider session, a foreground launch owner)`.
/// New hosts add values; consumers tolerate one they do not know.
fn not_reporting_cause(binding: Option<(String, bool, bool)>) -> &'static str {
    match binding {
        None => "no_binding",
        Some((state, admitted, _)) if state != "running" || !admitted => "no_agent",
        Some((_, _, false)) => "no_sampler",
        Some(_) => "not_sampled",
    }
}

/// The newest value (rate limits, credits) observed inside the rollup bucket that
/// starts at `from`, after one more read. A read repeats the newest provider line's
/// value for as long as the session idles; one older than the bucket was observed in
/// an earlier bucket, so it is not stored again.
fn newest<T: Serialize + DeserializeOwned>(
    held: Option<&str>,
    seen: Option<&T>,
    from: u64,
    observed_at: impl Fn(&T) -> u64,
    valid: impl Fn(&T) -> bool,
) -> Result<Option<String>, StorageError> {
    let held: Option<T> = held
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| invalid())?;
    if held.as_ref().is_some_and(|value| !valid(value)) {
        return Err(invalid());
    }
    let seen = seen.filter(|value| observed_at(value) >= from);
    let newest = match (held.as_ref(), seen) {
        (Some(held), Some(seen)) if observed_at(seen) < observed_at(held) => Some(held),
        (_, Some(seen)) => Some(seen),
        (held, None) => held,
    };
    newest
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| invalid())
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
            // The coordinator offers text only for a newly admitted Stop. A
            // fresh sampler with unchanged preferences must never append it.
            if let Some(response) = observation.response.as_ref()
                && observation.preferences != &observation.expected.preferences
            {
                super::responses::append(
                    connection,
                    id,
                    remembered.harness.as_str(),
                    remembered.provider_session.as_str(),
                    observation.now_ms,
                    response,
                )?;
            }
            if let Some(boundary) = observation.boundary
                && observation.preferences != &observation.expected.preferences
            {
                let now = crate::request_runtime::wall_time_ms();
                let session = remembered.provider_session.as_str();
                match boundary {
                    TurnBoundary::Started { turn, prompt } => {
                        super::requests::passthrough::bind(
                            connection, id, session, turn, prompt, now,
                        )?;
                        super::requests::turns::open(connection, id, now)?;
                        // A prompt that carries a typed Digest checklist confirms it (#4858).
                        super::requests::confirm_digest_by_prompt(connection, id, prompt, now)?;
                        // A new prompt means the continuation a Stop hook started is over.
                        super::requests::end_digest_continuation(connection, id, now)?;
                    }
                    TurnBoundary::Ended { turn, continuation } => {
                        // Only the continuation's own Stop ends it; the Stop that was blocked
                        // is the event that started it.
                        if continuation {
                            super::requests::end_digest_continuation(connection, id, now)?;
                        }
                        super::requests::turns::close(connection, id, now)?;
                        super::requests::passthrough::settle_turn(
                            connection,
                            id,
                            session,
                            turn,
                            observation
                                .response
                                .as_ref()
                                .map(|response| (&response.text, remembered.harness.as_str())),
                            now,
                        )?;
                    }
                }
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
                if let Some(family) = observation.family.as_ref() {
                    super::consumption_family::apply(connection, id, family)?;
                }
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
    /// Windows up to an hour read the 5-second buckets, longer ones the 5-minute
    /// rollup. A retired identity is answered only when `include_retired` is set.
    ///
    /// `pinned` is the cut a client carries from an earlier batch of the same
    /// refresh (#4176): every window then ends there instead of at the read time.
    /// It must be a multiple of the bucket width and not later than `now`; how old
    /// a pin may be is admission policy, owned by the API.
    pub fn consumption_history(
        &mut self,
        ids: &[String],
        windows: &[u64],
        max_buckets: u64,
        now: u64,
        include_retired: bool,
        pinned: Option<u64>,
    ) -> Result<Value, StorageError> {
        if now > MAX_JS_SAFE_INTEGER
            || !(1..=MAX_BUCKETS).contains(&max_buckets)
            || windows
                .iter()
                .any(|window| window_resolution_ms(*window).is_none())
            || pinned.is_some_and(|cut| !cut.is_multiple_of(BUCKET_MS) || cut > now)
        {
            return Err(invalid());
        }
        let transaction = self
            .connection_mut()?
            .transaction()
            .map_err(|e| classify(e, "Read consumption history snapshot"))?;
        let through = pinned.unwrap_or(now / BUCKET_MS * BUCKET_MS);
        let retained = through.saturating_sub(HISTORY_MS);
        let rollup_through = through / ROLLUP_BUCKET_MS * ROLLUP_BUCKET_MS;
        let rollup_longest = windows
            .iter()
            .copied()
            .filter(|window| window_resolution_ms(*window) == Some(ROLLUP_BUCKET_MS))
            .max();
        let mut identities = Vec::new();
        // Keep the prepared statements for this snapshot. The adapter disables
        // rusqlite's optional statement cache; transaction-local reuse needs no
        // extra dependency or cache lifetime beyond this read.
        let mut identity = transaction
            .prepare("SELECT retired_at_ms IS NOT NULL FROM identities WHERE id=?")
            .map_err(|e| classify(e, "Read consumption identity"))?;
        let mut statement = transaction.prepare("SELECT input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,sampled_at_ms,latest,details,from_ms FROM consumption_buckets WHERE identity_id=? AND from_ms>=? AND from_ms<? ORDER BY from_ms")
            .map_err(|e| classify(e, "Read retained consumption buckets"))?;
        let mut binding = transaction
            .prepare("SELECT runtime_state,observed_provider_session_id IS NOT NULL,launch_owner_pid IS NOT NULL FROM bindings WHERE identity_id=?")
            .map_err(|e| classify(e, "Read consumption binding"))?;
        let mut rollup = transaction.prepare("SELECT input_tokens,output_tokens,cached_input_tokens,covered_ms,complete,gap,discontinuous,sampled_at_ms,limits,details,from_ms,credits,service_tier FROM consumption_tier5m WHERE identity_id=? AND from_ms>=? AND from_ms<? ORDER BY from_ms")
            .map_err(|e| classify(e, "Read retained consumption rollup"))?;
        for id in ids {
            let known: Option<bool> = identity
                .query_row([id], |row| row.get(0))
                .optional()
                .map_err(|e| classify(e, "Read consumption identity"))?;
            let Some(retired) = known.filter(|retired| include_retired || !retired) else {
                identities.push(json!({"id":id,"found":false}));
                continue;
            };
            // Read the whole retained range once: latest/availability and corruption
            // checks still include evidence outside the requested display windows.
            let rows: Vec<Bucket> = statement
                .query_map(params![id, retained as i64, through as i64], |row| {
                    Bucket::from_row(row, integer(row, 10)?)
                })
                .and_then(|rows| rows.collect())
                .map_err(|e| classify(e, "Read consumption bucket"))?;
            let rolled: Vec<Bucket> = match rollup_longest {
                Some(longest) => rollup
                    .query_map(
                        params![
                            id,
                            rollup_through.saturating_sub(longest) as i64,
                            rollup_through as i64
                        ],
                        |row| Bucket::from_row(row, integer(row, 10)?),
                    )
                    .and_then(|rows| rows.collect())
                    .map_err(|e| classify(e, "Read consumption rollup bucket"))?,
                None => Vec::new(),
            };
            let last = rows.iter().rev().find(|row| row.sampled.is_some());
            let latest: Option<ConsumptionLatest> = last
                .and_then(|row| row.snapshot.as_deref())
                .map(serde_json::from_str)
                .transpose()
                .map_err(|_| invalid())?;
            if latest
                .as_ref()
                .is_some_and(|value| !value.consumption.valid())
            {
                return Err(invalid());
            }
            let reporting = rows.iter().any(|row| row.snapshot.is_some());
            let available = rows
                .iter()
                .filter(|row| row.snapshot.is_some())
                .filter_map(|row| row.sampled)
                .min();
            let mut readings = Vec::new();
            for window in windows {
                if window_resolution_ms(*window) == Some(BUCKET_MS) {
                    readings.push(project(&rows, FINE, *window, through, max_buckets)?);
                    continue;
                }
                let mut reading = project(&rolled, ROLLUP, *window, rollup_through, max_buckets)?;
                // The tier start: the first bucket that holds a successful read.
                let from = rolled
                    .iter()
                    .filter(|row| row.sampled.is_some())
                    .map(|row| row.from)
                    .min();
                let object = reading.as_object_mut().ok_or_else(invalid)?;
                object.insert("resolutionMs".into(), json!(ROLLUP_BUCKET_MS));
                object.insert("availableFromMs".into(), json!(from));
                object.insert(
                    "retainedFromMs".into(),
                    json!(
                        rollup_through
                            .saturating_sub(ROLLUP_MS)
                            .max(from.unwrap_or(0))
                    ),
                );
                readings.push(reading);
            }
            let mut found = json!({"id":id,"found":true,"reporting":reporting,"availableFromMs":available,"lastSampleAtMs":last.filter(|row|row.snapshot.is_some()).and_then(|row|row.sampled),"latest":latest,"windows":readings});
            if !reporting {
                // Why nothing is sampled now, from the binding snapshot alone.
                let cause = binding
                    .query_row([id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, bool>(2)?,
                        ))
                    })
                    .optional()
                    .map_err(|e| classify(e, "Read consumption binding"))?;
                found
                    .as_object_mut()
                    .ok_or_else(invalid)?
                    .insert("cause".into(), json!(not_reporting_cause(cause)));
            }
            if retired {
                // The name stays with `references.resolve`, the one owner of labels.
                found
                    .as_object_mut()
                    .ok_or_else(invalid)?
                    .insert("retired".into(), json!(true));
            }
            identities.push(found);
        }
        drop(rollup);
        drop(binding);
        drop(statement);
        drop(identity);
        transaction
            .commit()
            .map_err(|e| classify(e, "Close consumption history snapshot"))?;
        Ok(
            json!({"asOfMs":now,"throughMs":through,"retainedFromMs":retained,"resolutionMs":BUCKET_MS,"identities":identities}),
        )
    }
}

/// One window of `rows` (sorted by `from`) as at most `max_buckets` closed buckets
/// ending at `through`, uncovered intervals included. The rollup tier adds the
/// newest rate limits and credits seen in each bucket and the service tier of its
/// newest row that has one.
fn project(
    rows: &[Bucket],
    tier: Tier,
    window: u64,
    through: u64,
    max_buckets: u64,
) -> Result<Value, StorageError> {
    let from = through.saturating_sub(window);
    let slots = window / tier.width;
    let width = slots.div_ceil(max_buckets) * tier.width;
    let mut buckets = Vec::new();
    let mut start = from;
    let mut cursor = rows.partition_point(|row| row.from < from);
    while start < through {
        let end = (start + width).min(through);
        let mut aggregate = Bucket {
            from: start,
            ..Bucket::default()
        };
        let mut limits: Option<RateLimits> = None;
        let mut credits: Option<Credits> = None;
        let mut service_tier: Option<&str> = None;
        // Sorted, disjoint output intervals consume each retained row
        // at most once per window instead of rescanning for every bin.
        while let Some(row) = rows.get(cursor).filter(|row| row.from < end) {
            cursor += 1;
            if aggregate.input == 0 && aggregate.output == 0 {
                aggregate.details = row.details.clone();
            } else if row.input != 0 || row.output != 0 {
                aggregate.details.merge(&row.details)?;
            }
            aggregate.input = aggregate.input.checked_add(row.input).ok_or_else(invalid)?;
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
            if tier.width == ROLLUP.width
                && let Some(held) = row.snapshot.as_deref()
            {
                let held: RateLimits = serde_json::from_str(held).map_err(|_| invalid())?;
                if !held.valid() {
                    return Err(invalid());
                }
                if limits
                    .as_ref()
                    .is_none_or(|newest| held.observed_at_ms >= newest.observed_at_ms)
                {
                    limits = Some(held);
                }
            }
            if tier.account
                && let Some(held) = row.credits.as_deref()
            {
                let held: Credits = serde_json::from_str(held).map_err(|_| invalid())?;
                if !held.valid() {
                    return Err(invalid());
                }
                if credits
                    .as_ref()
                    .is_none_or(|newest| held.observed_at_ms >= newest.observed_at_ms)
                {
                    credits = Some(held);
                }
            }
            if tier.account
                && let Some(held) = row.service_tier.as_deref()
            {
                if !valid_service_tier(held) {
                    return Err(invalid());
                }
                // Rows are in time order, so the last one with a tier is the newest.
                service_tier = Some(held);
            }
        }
        if aggregate
            .input
            .checked_add(aggregate.output)
            .is_none_or(|n| n > MAX_JS_SAFE_INTEGER)
        {
            return Err(invalid());
        }
        let complete = aggregate.covered == end - start && !aggregate.incomplete && !aggregate.gap;
        let mut projected = json!({"fromMs":start,"toMs":end,"inputTokens":aggregate.input,"outputTokens":aggregate.output,"cachedInputTokens":aggregate.cached,"coveredMs":aggregate.covered,"complete":complete,"gap":aggregate.gap || aggregate.covered<end-start,"discontinuous":aggregate.discontinuous});
        projected.as_object_mut().ok_or_else(invalid)?.extend(
            serde_json::to_value(&aggregate.details)
                .map_err(|_| invalid())?
                .as_object()
                .ok_or_else(invalid)?
                .clone(),
        );
        if let Some(limits) = limits {
            projected
                .as_object_mut()
                .ok_or_else(invalid)?
                .insert("rateLimits".into(), json!(limits));
        }
        if let Some(credits) = credits {
            projected
                .as_object_mut()
                .ok_or_else(invalid)?
                .insert("credits".into(), json!(credits));
        }
        if let Some(tier) = service_tier {
            projected
                .as_object_mut()
                .ok_or_else(invalid)?
                .insert("serviceTier".into(), json!(tier));
        }
        buckets.push(projected);
        start = end;
    }
    Ok(json!({"windowMs":window,"fromMs":from,"toMs":through,"bucketMs":width,"buckets":buckets}))
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
            && next.consumption.cached_input_tokens - old.consumption.cached_input_tokens
                <= next.consumption.input_tokens - old.consumption.input_tokens
            && (next.consumption.sequence != old.consumption.sequence
                || next.consumption == old.consumption)
    });
    let encoded = next
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| invalid())?;
    let accepted = Accepted {
        incomplete: next
            .as_ref()
            .is_none_or(|value| !value.consumption.complete),
        discontinuous: old.as_ref().zip(next.as_ref()).is_some_and(|(old, next)| {
            old.driver != next.driver
                || old.session != next.session
                || old.consumption.epoch != next.consumption.epoch
        }),
        delta: continuous.map(|(old, next)| Delta {
            input: next.consumption.input_tokens - old.consumption.input_tokens,
            output: next.consumption.output_tokens - old.consumption.output_tokens,
            cached: next.consumption.cached_input_tokens - old.consumption.cached_input_tokens,
            changed: next.consumption.input_tokens != old.consumption.input_tokens
                || next.consumption.output_tokens != old.consumption.output_tokens,
            details: Details::delta(&old.consumption, &next.consumption, &next.driver),
        }),
    };
    let from = now / BUCKET_MS * BUCKET_MS;
    let mut bucket = Bucket::read(connection, FINE, id, from)?;
    bucket.snapshot = encoded.clone();
    bucket.sampled = Some(now);
    bucket.absorb(&accepted)?;
    bucket.write(connection, FINE, id)?;
    // The rollup takes the same read in the same transaction. A failed read adds
    // nothing it could mistake for a successful observation.
    let rolled_from = now / ROLLUP_BUCKET_MS * ROLLUP_BUCKET_MS;
    let mut rolled = Bucket::read(connection, ROLLUP, id, rolled_from)?;
    rolled.snapshot = newest(
        rolled.snapshot.as_deref(),
        next.as_ref()
            .and_then(|value| value.consumption.rate_limits.as_ref()),
        rolled_from,
        |limits: &RateLimits| limits.observed_at_ms,
        RateLimits::valid,
    )?;
    rolled.credits = newest(
        rolled.credits.as_deref(),
        next.as_ref()
            .and_then(|value| value.consumption.credits.as_ref()),
        rolled_from,
        |credits: &Credits| credits.observed_at_ms,
        Credits::valid,
    )?;
    // The tier is the setting while tokens moved: an idle read never claims one.
    if accepted.delta.as_ref().is_some_and(|delta| delta.changed)
        && let Some(tier) = next
            .as_ref()
            .and_then(|value| value.consumption.service_tier.clone())
    {
        rolled.service_tier = Some(tier);
    }
    if next.is_some() {
        rolled.sampled = Some(now);
    }
    rolled.absorb(&accepted)?;
    rolled.write(connection, ROLLUP, id)?;
    if continuous.is_some_and(|(old, next)| old.consumption.complete && next.consumption.complete)
        && let Some(start) = old_time
        && now.saturating_sub(start) <= 2 * BUCKET_MS
    {
        // Accepted reads are serialized through this transaction, so these
        // adjacent intervals do not overlap, even when Stop races the sampler.
        let mut cursor = start;
        while cursor < now {
            let slot = cursor / BUCKET_MS * BUCKET_MS;
            let end = (slot + BUCKET_MS).min(now);
            for tier in [FINE, ROLLUP] {
                let mut covered =
                    Bucket::read(connection, tier, id, cursor / tier.width * tier.width)?;
                covered.covered = covered
                    .covered
                    .checked_add(end - cursor)
                    .ok_or_else(invalid)?;
                covered.write(connection, tier, id)?;
            }
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
    // The rollup ages out by time alone: this identity's expired rows on every write,
    // and a few of anyone's, so a retired identity that never writes again expires too.
    let rolled_cutoff = rolled_from.saturating_sub(ROLLUP_MS);
    connection
        .execute(
            "DELETE FROM consumption_tier5m WHERE identity_id=? AND from_ms<?",
            params![id, rolled_cutoff as i64],
        )
        .map_err(|e| classify(e, "Prune identity consumption rollup"))?;
    connection.execute("DELETE FROM consumption_tier5m WHERE (identity_id,from_ms) IN (SELECT identity_id,from_ms FROM consumption_tier5m WHERE from_ms<? ORDER BY from_ms LIMIT 256)",[rolled_cutoff as i64]).map_err(|e|classify(e,"Prune inactive consumption rollup"))?;
    Ok(())
}

#[cfg(test)]
mod passthrough_tests;
#[cfg(test)]
mod rollup_tests;
#[cfg(test)]
mod tests;
