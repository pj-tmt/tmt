//! Focus is a delivery window over canonical requests, never a second inbox.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusKind {
    Decision,
    Review,
    #[default]
    Fyi,
    Result,
}
impl FocusKind {
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
    pub kind: FocusKind,
    /// Explicit inbox publication remains pull-only, even at a later opportunity.
    pub automatic: bool,
}
impl Default for DeliveryPolicy {
    fn default() -> Self {
        Self {
            urgent: false,
            kind: FocusKind::Fyi,
            automatic: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusPolicy {
    pub identity_id: String,
    pub revision: u64,
    pub until_ms: u64,
    pub owner_identity_id: String,
    pub setter_identity_id: String,
}
impl FocusPolicy {
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

pub struct FocusPolicyWrite {
    pub identity_id: String,
    pub expected_revision: u64,
    /// Zero is an explicit clear; a set must be in the future.
    pub until_ms: u64,
    pub owner_identity_id: String,
    pub setter_identity_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusPolicyView {
    pub policy: Option<FocusPolicy>,
    pub active_checklist: Option<FocusChecklist>,
    pub held_count: u64,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusItem {
    pub sequence: u64,
    pub identity_id: String,
    pub request_id: String,
    pub kind: FocusKind,
    pub source: FocusSource,
    pub created_at_ms: u64,
    pub checklist_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusSource {
    Incoming,
    Result,
    Timeout,
}
impl FocusSource {
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
pub enum FocusState {
    Claimed,
    Delivered,
    Unsent,
    Uncertain,
}
impl FocusState {
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
pub struct FocusChecklist {
    pub id: String,
    pub identity_id: String,
    pub attempt_token: String,
    pub through_sequence: u64,
    pub state: FocusState,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusOpportunity {
    /// The provider adapter has admitted this exact launch's turn boundary.
    TurnBoundary,
    /// Normal transport independently verified the current seat is idle.
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusRejection {
    Invalid,
    IdentityUnavailable,
    Conflict,
    AttemptMismatch,
    StateInvalid,
}
impl FocusRejection {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "FOCUS_INPUT_INVALID",
            Self::IdentityUnavailable => "FOCUS_IDENTITY_UNAVAILABLE",
            Self::Conflict => "FOCUS_REVISION_CONFLICT",
            Self::AttemptMismatch => "FOCUS_ATTEMPT_MISMATCH",
            Self::StateInvalid => "FOCUS_STATE_INVALID",
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
        super::notification::HintKind::Reply => (FocusSource::Result, FocusKind::Result),
        super::notification::HintKind::Timeout => (FocusSource::Timeout, FocusKind::Fyi),
    };
    let held = records.has_focus_item(identity, &attempt.request_id, source)?;
    if !held
        && (!records.identity_is_active(identity)?
            || !records
                .focus_policy(identity)?
                .is_some_and(|p| p.holds(attempt.recipient_identity_id.as_deref(), false, now)))
    {
        return Ok(false);
    }
    records.hold_focus_item(identity, &attempt.request_id, label, source, now)?;
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
