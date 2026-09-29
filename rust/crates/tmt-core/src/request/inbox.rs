//! Requests waiting on one recipient for a final. This is an open-request
//! question, independent of attention: acknowledging never removes an item;
//! only a final or the acceptance deadline does.

use super::{AttemptStatus, Originator};

pub const INBOX_LIMIT: u64 = 50;
pub const INBOX_MAX_LIMIT: u64 = 200;

/// Delivery states whose request still accepts a first final. The storage
/// read narrows by this list; the acceptance check owns the decision.
pub const ANSWERABLE: [AttemptStatus; 4] = [
    AttemptStatus::Sending,
    AttemptStatus::Sent,
    AttemptStatus::Queued,
    AttemptStatus::Uncertain,
];

/// Storage narrowing for open requests to one recipient, oldest first.
pub struct OpenQuery {
    pub recipient_identity_id: String,
    pub originator_identity_id: Option<String>,
    /// A request prepared after this instant is still inside its acceptance
    /// window even when its expiry has passed.
    pub window_start_ms: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    pub request_id: String,
    pub room_id: Option<String>,
    pub originator: Originator,
    pub prepared_at_ms: u64,
    pub delivery: AttemptStatus,
    pub preview: Option<String>,
}

pub struct OpenPage {
    pub items: Vec<OpenRequest>,
    /// Newer open requests exist beyond the limit.
    pub more: bool,
}

/// Why an answer selected no request. Selection never guesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerRejection {
    NotWaiting,
    Ambiguous(Vec<OpenRequest>),
}

impl AnswerRejection {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotWaiting => "ANSWER_NOT_WAITING",
            Self::Ambiguous(_) => "ANSWER_AMBIGUOUS",
        }
    }
}
