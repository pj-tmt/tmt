//! Owner-visible retained conversation projections, independent of unread attention.

use super::{
    AttemptStatus, Originator, RequestKind, RequestPrompt,
    attention::{AttentionRecord, FinalState},
};

pub const HISTORY_LIMIT: u64 = 20;
pub const RESULTS_LIMIT: u64 = 8;
pub const HISTORY_MAX_LIMIT: u64 = 50;
pub const HISTORY_PREVIEW_CHARS: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryScope {
    Recipient {
        identity_id: String,
        room_id: Option<String>,
    },
    Room(String),
    /// Submitted finals authored by one identity across recipients and rooms,
    /// or only those answered by one recipient.
    OriginatorResults {
        originator_id: String,
        recipient_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryCursor {
    Prepared {
        prepared_at_ms: u64,
        request_id: String,
    },
    Submitted {
        submitted_at_ms: u64,
        request_id: String,
    },
}

impl HistoryCursor {
    pub fn timestamp(&self) -> u64 {
        match self {
            Self::Prepared { prepared_at_ms, .. } => *prepared_at_ms,
            Self::Submitted {
                submitted_at_ms, ..
            } => *submitted_at_ms,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::Prepared { request_id, .. } | Self::Submitted { request_id, .. } => request_id,
        }
    }
}

pub struct HistoryQuery {
    pub scope: HistoryScope,
    pub before: Option<HistoryCursor>,
    pub limit: u64,
}

/// Only the results view loads a bounded response prefix; full text stays in detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponsePreview {
    pub text: String,
    pub truncated: bool,
}

pub struct HistoryRecord {
    pub attention: AttentionRecord,
    pub preview: Option<String>,
    pub response_preview: Option<ResponsePreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryItem<T = ()> {
    pub delivery_policy: super::digest::DeliveryPolicy,
    /// What a passthrough's turn earned, with its state as of the read.
    pub capture: Option<super::passthrough::PassthroughCapture>,
    pub request_id: String,
    pub room_id: Option<String>,
    pub recipient_identity_id: Option<String>,
    pub originator: Originator,
    pub kind: RequestKind,
    pub origin: super::RequestOrigin,
    pub prepared_at_ms: u64,
    pub delivery: AttemptStatus,
    /// The one state a person sees for this message; clients paint it as given.
    pub delivery_view: super::delivery::DeliveryView,
    /// Inbox acknowledgment only. Pane transport cannot prove agent read state.
    pub recipient_acknowledged: Option<bool>,
    pub final_state: FinalState<T>,
}

pub struct HistorySummary {
    pub item: HistoryItem,
    pub preview: Option<String>,
    pub response_preview: Option<ResponsePreview>,
}

pub struct HistoryPage {
    pub items: Vec<HistorySummary>,
    pub next_before: Option<HistoryCursor>,
}

pub struct HistoryDetail {
    pub item: HistoryItem<String>,
    pub prompt: RequestPrompt,
}

/// What answered one of an originator's requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerKind {
    /// The reply submitted with the request's receipt.
    Final,
    /// The turn output a passthrough message earned (#4336).
    Turn,
}

impl AnswerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::Turn => "turn",
        }
    }
}

/// Where an answers page stops: answers are ordered by when they arrived, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerCursor {
    pub answered_at_ms: u64,
    pub request_id: String,
}

/// An originator's answered requests, finals and passthrough turns in one order.
pub struct AnswersQuery {
    pub originator_id: String,
    pub before: Option<AnswerCursor>,
    pub limit: u64,
}

/// An answered request without its text: enough to count what is new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsweredRequest {
    pub request_id: String,
    pub kind: RequestKind,
    pub originator: Originator,
    pub recipient_identity_id: Option<String>,
    pub answered_at_ms: u64,
    pub answer: AnswerKind,
    /// The originator's attention revision for this request and whether they have acknowledged
    /// it; a final and an answered turn raise it alike (#4915).
    pub revision: u64,
    pub acknowledged: bool,
}

pub struct AnswersPage {
    pub items: Vec<AnsweredRequest>,
    pub next_before: Option<AnswerCursor>,
}
