//! Attention is a view over retained requests, not a second request lifecycle.

use super::{AttemptStatus, RequestAttempt, RequestPrompt, Withdrawal};
use std::fmt;

pub const DEFAULT_LIST_LIMIT: u64 = 50;
pub const MAX_LIST_LIMIT: u64 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseMetadata {
    pub body_bytes: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionRecord {
    pub attempt: RequestAttempt,
    pub revision: u64,
    pub acknowledged_revision: u64,
    pub acknowledged_through: u64,
    pub response_metadata: Option<ResponseMetadata>,
}

/// Summary carries unit content; detail carries exact text. Metadata and
/// state variants therefore have one definition without optional body flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalState<T> {
    /// Terminal originator intent, never a recipient final or approval.
    Withdrawn(Withdrawal),
    NotRequired,
    NotSubmitted,
    Retained {
        content: T,
        submitted_at_ms: u64,
        body_bytes: u64,
        expires_at_ms: u64,
    },
    Expired {
        submitted_at_ms: u64,
        expires_at_ms: u64,
    },
    Unavailable {
        submitted_at_ms: u64,
        expires_at_ms: u64,
    },
}

impl<T> FinalState<T> {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Withdrawn(_) => "withdrawn",
            Self::NotRequired => "not_required",
            Self::NotSubmitted => "not_submitted",
            Self::Retained { .. } => "retained",
            Self::Expired { .. } => "expired",
            Self::Unavailable { .. } => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange<T = ()> {
    pub delivery_policy: super::digest::DeliveryPolicy,
    pub request_id: String,
    pub room_id: Option<String>,
    pub recipient_identity_id: Option<String>,
    pub prepared_at_ms: u64,
    pub delivery: AttemptStatus,
    pub final_state: FinalState<T>,
    pub revision: u64,
    pub acknowledged: bool,
    pub settled: bool,
    pub retention_expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeDetail {
    pub exchange: Exchange<String>,
    pub prompt: RequestPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangePage {
    pub items: Vec<Exchange>,
    pub next_after: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingKind {
    Announcement,
    Request,
    Response,
}

impl IncomingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Announcement => "announcement",
            Self::Request => "request",
            Self::Response => "response",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingItem {
    pub exchange: Exchange,
    pub kind: IncomingKind,
    pub sender_identity_id: Option<String>,
    pub recipient_identity_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingPage {
    pub items: Vec<IncomingItem>,
    pub next_after: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acknowledged {
    pub request_id: String,
    pub revision: u64,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionRejection {
    Invalid(&'static str),
    NotFound,
    RevisionConflict { current: u64, expected: u64 },
}

impl AttentionRejection {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "X_INPUT_INVALID",
            Self::NotFound => "X_NOT_FOUND",
            Self::RevisionConflict { .. } => "X_REVISION_CONFLICT",
        }
    }
}

impl fmt::Display for AttentionRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::NotFound => {
                f.write_str("Exchange was not found for this identity or is no longer retained.")
            }
            Self::RevisionConflict { current, expected } => {
                write!(f, "Exchange is at revision {current}, not {expected}.")
            }
        }
    }
}
