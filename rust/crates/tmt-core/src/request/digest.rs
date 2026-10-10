//! Digest is a delivery window over canonical requests, never a second inbox.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DigestKind {
    Decision,
    Review,
    #[default]
    Fyi,
    Result,
}
impl DigestKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "decision" => Some(Self::Decision),
            "review" => Some(Self::Review),
            "fyi" => Some(Self::Fyi),
            "result" => Some(Self::Result),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Review => "review",
            Self::Fyi => "fyi",
            Self::Result => "result",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryPolicy {
    pub urgent: bool,
    pub kind: DigestKind,
    /// Explicit inbox publication remains pull-only, even at a later opportunity.
    pub automatic: bool,
}
impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            urgent: false,
            kind: DigestKind::Fyi,
            automatic: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestPolicy {
    pub identity_id: String,
    pub revision: u64,
    pub until_ms: u64,
    pub owner_identity_id: String,
    pub setter_identity_id: String,
}
impl DigestPolicy {
    pub fn active(&self, now: u64) -> bool {
        now < self.until_ms
    }
    pub fn remaining_ms(&self, now: u64) -> u64 {
        self.until_ms.saturating_sub(now)
    }
    pub fn holds(&self, sender: Option<&str>, urgent: bool, now: u64) -> bool {
        self.active(now) && !urgent && sender != Some(self.owner_identity_id.as_str())
    }
}

pub struct DigestPolicyWrite {
    pub identity_id: String,
    pub expected_revision: u64,
    /// Zero is an explicit clear; a set must be in the future.
    pub until_ms: u64,
    pub owner_identity_id: String,
    pub setter_identity_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestPolicyView {
    pub policy: Option<DigestPolicy>,
    pub active_checklist: Option<DigestChecklist>,
    pub held_count: u64,
    pub observed_at_ms: u64,
}

/// Durable eligibility watermark and successful-checklist counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DigestCounters {
    pub due_through_sequence: u64,
    pub delivered_digests: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestStats {
    pub identity_id: String,
    pub held_count: u64,
    pub oldest_held_age_ms: Option<u64>,
    pub delivered_digests: u64,
    pub due_count: u64,
    /// Core eligibility only; an extension owns the configured delivery deadline.
    pub next_eligible_at_ms: Option<u64>,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigestDue {
    pub held_count: u64,
    pub through_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestItem {
    pub sequence: u64,
    pub identity_id: String,
    pub request_id: String,
    pub kind: DigestKind,
    pub source: DigestSource,
    pub created_at_ms: u64,
    pub checklist_id: Option<String>,
    /// Latest verified driver observation available when this reference first held.
    pub context_tokens_at_arrival: Option<u64>,
    pub context_observed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestSource {
    Incoming,
    Result,
    Timeout,
}
impl DigestSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Result => "result",
            Self::Timeout => "timeout",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "incoming" => Some(Self::Incoming),
            "result" => Some(Self::Result),
            "timeout" => Some(Self::Timeout),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestState {
    Claimed,
    Delivered,
    Unsent,
    Uncertain,
}
impl DigestState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::Delivered => "delivered",
            Self::Unsent => "definitely_unsent",
            Self::Uncertain => "uncertain",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claimed" => Some(Self::Claimed),
            "delivered" => Some(Self::Delivered),
            "definitely_unsent" => Some(Self::Unsent),
            "uncertain" => Some(Self::Uncertain),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestChecklist {
    pub id: String,
    pub identity_id: String,
    pub attempt_token: String,
    pub through_sequence: u64,
    pub state: DigestState,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestOpportunity {
    /// The provider adapter has admitted this exact launch's turn boundary.
    TurnBoundary,
    /// Normal transport independently verified the current seat is idle.
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestRejection {
    Invalid,
    IdentityUnavailable,
    Conflict,
    AttemptMismatch,
    StateInvalid,
}
impl DigestRejection {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "DIGEST_INPUT_INVALID",
            Self::IdentityUnavailable => "DIGEST_IDENTITY_UNAVAILABLE",
            Self::Conflict => "DIGEST_REVISION_CONFLICT",
            Self::AttemptMismatch => "DIGEST_ATTEMPT_MISMATCH",
            Self::StateInvalid => "DIGEST_STATE_INVALID",
        }
    }
}

/// Admission shared with the adapter's existing per-frame and joined-send writer
/// claims. No caller may mark external notice input before this decision.
pub fn hold_originator_notice<E>(
    records: &mut dyn super::RequestRecords<Error = E>,
    attempt: &super::RequestAttempt,
    kind: super::notification::HintKind,
    now: u64,
) -> Result<bool, super::RequestError<E>> {
    let Some(identity) = attempt.originator.identity_id() else {
        return Ok(false);
    };
    let (source, label) = match kind {
        super::notification::HintKind::Reply => (DigestSource::Result, DigestKind::Result),
        super::notification::HintKind::Timeout => (DigestSource::Timeout, DigestKind::Fyi),
    };
    let held = records.has_digest_item(identity, &attempt.request_id, source)?;
    if !held
        && (!records.identity_is_active(identity)?
            || !records
                .digest_policy(identity)?
                .is_some_and(|p| p.holds(attempt.recipient_identity_id.as_deref(), false, now)))
    {
        return Ok(false);
    }
    records.hold_digest_item(identity, &attempt.request_id, label, source, now)?;
    if let Some(mut value) = records.notification(&attempt.request_id)? {
        let state = match kind {
            super::notification::HintKind::Reply => &mut value.reply,
            super::notification::HintKind::Timeout => &mut value.timeout,
        };
        if *state == super::WakeState::Claimed {
            *state = super::WakeState::Unavailable;
            records.set_notification(&attempt.request_id, &value)?;
        }
    }
    Ok(true)
}
