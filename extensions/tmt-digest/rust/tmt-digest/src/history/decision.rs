//! Content-free records of why a tick did or did not offer a member delivery.

use super::{MAX, invalid};
use crate::core::Error;

/// The rule that decided a member at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// An interval member's oldest held message waited its interval.
    IntervalDeadline,
    /// An interval member reached its flush count.
    FlushCount,
    /// An `auto` member with held messages was offered a flush.
    AutoIdlePoint,
    /// An `off` member with held messages was offered a flush.
    Off,
    /// An interval member's deadline had not been reached.
    NotDue,
}
impl Rule {
    pub fn text(self) -> &'static str {
        match self {
            Self::IntervalDeadline => "interval_deadline",
            Self::FlushCount => "flush_count",
            Self::AutoIdlePoint => "auto_idle_point",
            Self::Off => "off",
            Self::NotDue => "not_due",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::IntervalDeadline,
            Self::FlushCount,
            Self::AutoIdlePoint,
            Self::Off,
            Self::NotDue,
        ]
        .into_iter()
        .find(|rule| rule.text() == text)
    }
    fn mode(self) -> &'static str {
        match self {
            Self::AutoIdlePoint => "auto",
            Self::Off => "off",
            _ => "interval",
        }
    }
}

/// What Core's flush opportunity answered; `error` is a failed or malformed call.
pub const OUTCOMES: [&str; 6] = [
    "delivered",
    "not_idle",
    "nothing_due",
    "unavailable",
    "uncertain",
    "error",
];

/// Counts and times only: no message text, subject or sender.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub at_ms: u64,
    pub identity_id: String,
    pub interval_ms: Option<u64>,
    pub flush_count: u64,
    pub rule: Rule,
    pub held_count: u64,
    pub oldest_held_age_ms: Option<u64>,
    /// Absent exactly for `not_due`: no opportunity was offered.
    pub outcome: Option<&'static str>,
}
impl Decision {
    /// The mode is implied by the rule; only an interval mode carries an interval.
    pub fn mode(&self) -> &'static str {
        self.rule.mode()
    }
    /// When the oldest held message arrived: stable while that message is held, so
    /// a new holding episode differs from the one before it.
    pub fn oldest_held_at_ms(&self) -> Option<u64> {
        self.oldest_held_age_ms
            .map(|age| self.at_ms.saturating_sub(age))
    }
    pub fn validate(&self) -> Result<(), Error> {
        if !crate::settings::canonical_uuid(&self.identity_id)
            || [self.at_ms, self.flush_count, self.held_count]
                .into_iter()
                .chain(self.interval_ms)
                .chain(self.oldest_held_age_ms)
                .any(|n| n > MAX)
            || self.interval_ms.is_some() != (self.mode() == "interval")
            || self.outcome.is_some() == (self.rule == Rule::NotDue)
            || self
                .outcome
                .is_some_and(|outcome| !OUTCOMES.contains(&outcome))
            || (self.held_count == 0) != self.oldest_held_age_ms.is_none()
        {
            return Err(invalid("Invalid decision record"));
        }
        Ok(())
    }
}
