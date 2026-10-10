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
    /// Submitted finals authored by one identity across recipients and rooms.
    OriginatorResults(String),
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
    pub request_id: String,
    pub room_id: Option<String>,
    pub recipient_identity_id: Option<String>,
    pub originator: Originator,
    pub kind: RequestKind,
    pub prepared_at_ms: u64,
    pub delivery: AttemptStatus,
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
