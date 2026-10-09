//! Free-plan (Spark) Firestore budget model and the client guard that refuses before the
//! modeled limit (#2180). Pure arithmetic: no I/O and no clock (callers pass `now_ms`).
//!
//! The official quotas are project-wide, every client sees only its own traffic, and Rules
//! cannot read quotas, so the guard works from what a client really has: the member count
//! from the signed membership, the plan's parameters, its own persisted counter for today
//! and this table. It does not read Google's counter and the emulator cannot enforce one:
//! tests assert this arithmetic against independent vectors. The dated table and the
//! derivation are in `.agents/skills/tmt-remote/references/firestore-free-plan.md`.
//!
//! Quota numbers below were read on 2026-10-09 (pages last updated 2026-10-07 UTC) from
//! <https://firebase.google.com/docs/firestore/quotas> and
//! <https://firebase.google.com/docs/firestore/pricing>; Hosting from
//! <https://firebase.google.com/pricing>; Auth from
//! <https://firebase.google.com/docs/auth/limits>. Limits change: recheck before relying on them.

/// Spark (no-cost) Firestore document reads per day. The other limits and their sources are in
/// the reference guide's dated table, which is their only owner.
pub const READS_PER_DAY: u64 = 50_000;
/// Spark (no-cost) Firestore document writes per day.
pub const WRITES_PER_DAY: u64 = 20_000;

/// Rules dependent documents read when a client reads: the member projection or link-device
/// enrollment, plus the page head (Colab's design).
pub const READ_LOOKUPS: u64 = 2;
/// Dependent documents when a client appends: the same two plus the head read by
/// `ext.getAfter` for the create-only sequence.
pub const WRITE_LOOKUPS: u64 = 3;
/// Thresholds of the guard, one table: warn at this share of a client's modeled allowance,
/// refuse at this one. A refusal is always recoverable (see `Refusal`) and never silent.
pub const WARN_PERCENT: u64 = 70;
pub const REFUSE_PERCENT: u64 = 90;
/// Planning assumption for the minimum flush interval: this many active seconds per day.
pub const ACTIVE_SECONDS_PER_DAY: u64 = 2 * 60 * 60;
/// Largest member count the model accepts.
pub const MAX_MEMBERS: u64 = 10_000;
/// The error code of a refusal made before any effect.
pub const BUDGET_EXHAUSTED: &str = "REMOTE_BUDGET_EXHAUSTED";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// No members, no writers, more writers than members, or too many members.
    OutOfRange,
    /// The free plan cannot host the page: the modeled share is too small for the guard to
    /// allow even one append. A page like that is refused here, when the model is built,
    /// never as a pause that would end at the next reset and begin again.
    FreePlanCannotHost,
}
/// One shared page: `members` have it open (every one is a listener), `writers` of them append.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    members: u64,
    writers: u64,
}
impl Model {
    pub fn new(members: u64, writers: u64) -> Result<Self, ModelError> {
        if !((1..=MAX_MEMBERS).contains(&members) && (1..=members).contains(&writers)) {
            return Err(ModelError::OutOfRange);
        }
        let model = Self { members, writers };
        if model.refuse_at() == 0 {
            return Err(ModelError::FreePlanCannotHost);
        }
        Ok(model)
    }
    /// Reads one append costs the project, pessimistically: the writer's own Rules lookups,
    /// and for each of the other listeners the delivered document plus the Rules lookups
    /// Firestore bills again when their query results update.
    pub fn reads_per_append(&self) -> u64 {
        WRITE_LOOKUPS + (self.members - 1) * (1 + READ_LOOKUPS)
    }
    /// Appends per day the free plan allows this page if it were the project's only traffic.
    pub fn daily_append_limit(&self) -> u64 {
        (READS_PER_DAY / self.reads_per_append()).min(WRITES_PER_DAY)
    }
    /// One client's modeled share of the page's daily allowance.
    pub fn share(&self) -> u64 {
        self.daily_append_limit() / self.writers
    }
    /// Appends after which the guard warns, and the count at which it refuses.
    pub fn warn_at(&self) -> u64 {
        self.share() * WARN_PERCENT / 100
    }
    pub fn refuse_at(&self) -> u64 {
        self.share() * REFUSE_PERCENT / 100
    }
    /// Shortest average interval between flushed updates, in milliseconds, that keeps
    /// `ACTIVE_SECONDS_PER_DAY` of editing at the warning threshold of the whole page.
    pub fn min_flush_interval_ms(&self) -> u64 {
        let appends = (self.daily_append_limit() * WARN_PERCENT / 100).max(1);
        (ACTIVE_SECONDS_PER_DAY * 1000).div_ceil(appends)
    }
    /// What the guard says about the next append, given how many this client sent today.
    pub fn assess(&self, appends_today: u64) -> Verdict {
        if appends_today >= self.refuse_at() {
            Verdict::Refuse
        } else if appends_today >= self.warn_at() {
            Verdict::Warn
        } else {
            Verdict::Ok
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Warn,
    Refuse,
}

/// A Pacific calendar day, as days since 1970-01-01 of the Pacific local date. Firestore
/// quotas "reset around midnight Pacific time".
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PacificDay(pub i64);
const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn civil_year(days: i64) -> i64 {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(month <= 2)
}
/// The n-th Sunday (1-based) of `month` in `year`, as a day number.
fn nth_sunday(year: i64, month: i64, n: i64) -> i64 {
    let first = days_from_civil(year, month, 1);
    // 1970-01-01 was a Thursday: day 3 of the week counting Sunday as 0.
    let weekday = (first + 4).rem_euclid(7);
    first + (7 - weekday) % 7 + (n - 1) * 7
}
/// United States rule since 2007: daylight time from 02:00 local on the second Sunday of
/// March to 02:00 local on the first Sunday of November. `local_ms` is wall-clock time.
fn is_dst_local(local_ms: i64) -> bool {
    let year = civil_year(local_ms.div_euclid(DAY_MS));
    let start = nth_sunday(year, 3, 2) * DAY_MS + 2 * HOUR_MS;
    let end = nth_sunday(year, 11, 1) * DAY_MS + 2 * HOUR_MS;
    (start..end).contains(&local_ms)
}
/// The Pacific UTC offset at a UTC instant, in milliseconds (-8 h, or -7 h in daylight time).
/// Daylight time runs from 10:00 UTC on the second Sunday of March (02:00 PST) to 09:00 UTC
/// on the first Sunday of November (02:00 PDT).
fn offset_ms(utc_ms: i64) -> i64 {
    let year = civil_year((utc_ms - 8 * HOUR_MS).div_euclid(DAY_MS));
    let start = nth_sunday(year, 3, 2) * DAY_MS + 10 * HOUR_MS;
    let end = nth_sunday(year, 11, 1) * DAY_MS + 9 * HOUR_MS;
    if (start..end).contains(&utc_ms) {
        -7 * HOUR_MS
    } else {
        -8 * HOUR_MS
    }
}
/// The Pacific day containing `now_ms` and the instant its quotas next reset (the next local
/// midnight), both from the US daylight-saving rule.
pub fn pacific_day(now_ms: u64) -> (PacificDay, u64) {
    let now = i64::try_from(now_ms).unwrap_or(i64::MAX / 2);
    let day = (now + offset_ms(now)).div_euclid(DAY_MS);
    let next_midnight_local = (day + 1) * DAY_MS;
    let next_offset = if is_dst_local(next_midnight_local) {
        -7 * HOUR_MS
    } else {
        -8 * HOUR_MS
    };
    (PacificDay(day), (next_midnight_local - next_offset) as u64)
}

/// A client's appends today, rolled over when the Pacific day changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub day: PacificDay,
    pub appends: u64,
}
impl Usage {
    pub fn new(now_ms: u64) -> Self {
        Self {
            day: pacific_day(now_ms).0,
            appends: 0,
        }
    }
    /// The same usage if still the same Pacific day, else a fresh day.
    pub fn at(self, now_ms: u64) -> Self {
        if pacific_day(now_ms).0 == self.day {
            self
        } else {
            Self::new(now_ms)
        }
    }
    /// Count one append sent.
    pub fn recorded(self, now_ms: u64) -> Self {
        let now = self.at(now_ms);
        Self {
            appends: now.appends.saturating_add(1),
            ..now
        }
    }
}

/// An append refused by the guard BEFORE any effect: nothing was sent, so the same sealed
/// update can be sent unchanged after the reset. Carries when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub reset_at_ms: u64,
    pub retry_after_ms: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Allowed, and the client should tell its user the share is nearly spent.
    Warn,
    Refuse(Refusal),
}
/// Judge the next append. Rolls the usage over to today first.
pub fn decide(model: &Model, usage: Usage, now_ms: u64) -> Decision {
    let usage = usage.at(now_ms);
    match model.assess(usage.appends) {
        Verdict::Ok => Decision::Allow,
        Verdict::Warn => Decision::Warn,
        Verdict::Refuse => {
            let (_, reset_at_ms) = pacific_day(now_ms);
            Decision::Refuse(Refusal {
                reset_at_ms,
                retry_after_ms: reset_at_ms.saturating_sub(now_ms),
            })
        }
    }
}

/// The provider answered `resource-exhausted`. Whether that is a refusal or an unknown
/// outcome depends only on whether the request could have changed anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderExhausted {
    /// The request was a write that may have been applied before the answer.
    pub mutation: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExhaustedOutcome {
    /// A read cannot have changed anything: reported as `REMOTE_BUDGET_EXHAUSTED` with the reset.
    Refused(Refusal),
    /// A write may have taken effect: an unknown outcome, never `REMOTE_BUDGET_EXHAUSTED`. The
    /// caller reads back the original ID and does not resend.
    Unknown,
}
/// Classify a provider refusal, and record it as `exhausted` evidence for the readiness view.
pub fn classify_provider_exhausted(
    attempt: ProviderExhausted,
    now_ms: u64,
) -> (ExhaustedOutcome, ExhaustedEvidence) {
    let evidence = ExhaustedEvidence::recorded(now_ms);
    if attempt.mutation {
        return (ExhaustedOutcome::Unknown, evidence);
    }
    let reset_at_ms = evidence.resets_at_ms;
    let refusal = Refusal {
        reset_at_ms,
        retry_after_ms: reset_at_ms.saturating_sub(now_ms),
    };
    (ExhaustedOutcome::Refused(refusal), evidence)
}
/// A recorded provider refusal. It says nothing after the quota resets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExhaustedEvidence {
    pub day: PacificDay,
    pub resets_at_ms: u64,
}
impl ExhaustedEvidence {
    pub fn recorded(now_ms: u64) -> Self {
        let (day, resets_at_ms) = pacific_day(now_ms);
        Self { day, resets_at_ms }
    }
    /// Still a statement about today's quota.
    pub fn is_current(&self, now_ms: u64) -> bool {
        now_ms < self.resets_at_ms
    }
}
