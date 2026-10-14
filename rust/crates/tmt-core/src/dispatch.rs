//! Shared request composition. Delivery and response state remain request-owned.

use crate::exact_text::validate_exact_text;
use uuid::Uuid;

/// Bound one explicit composition transaction, not an identity directory.
pub const MAX_RECIPIENTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchInput {
    /// Selected by a trusted adapter, never by an untrusted JSON envelope.
    pub originator: crate::request::Originator,
    pub kind: crate::request::RequestKind,
    pub operation_id: String,
    pub recipient_ids: Vec<String>,
    pub message: String,
    pub room: Option<DispatchRoom>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchRoom {
    /// One identified recipient; membership is checked by RequestService at enqueue.
    Direct { room_id: String },
    /// Explicit full-room fan-out, fenced to the roster the sender reviewed.
    Roster { room_id: String, revision: u64 },
}

impl DispatchRoom {
    pub fn room_id(&self) -> &str {
        match self {
            Self::Direct { room_id } | Self::Roster { room_id, .. } => room_id,
        }
    }
}

impl DispatchInput {
    /// Ordering and duplicate selections do not change the intended audience.
    pub fn normalize(mut self) -> Option<Self> {
        if !canonical_id(&self.operation_id)
            || self
                .originator
                .identity_id()
                .is_some_and(|id| !canonical_id(id))
            || self.recipient_ids.is_empty()
            || self.recipient_ids.len() > MAX_RECIPIENTS
            || self.recipient_ids.iter().any(|id| !canonical_id(id))
            || self.message.trim().is_empty()
            || validate_exact_text(self.message.as_bytes()).is_err()
            || self.room.as_ref().is_some_and(|room| {
                !canonical_id(room.room_id())
                    || matches!(room, DispatchRoom::Roster { revision, .. }
                        if *revision == 0 || *revision > crate::limits::MAX_JS_SAFE_INTEGER)
            })
        {
            return None;
        }
        self.recipient_ids.sort();
        self.recipient_ids.dedup();
        if matches!(self.room, Some(DispatchRoom::Direct { .. })) && self.recipient_ids.len() != 1 {
            return None;
        }
        Some(self)
    }
}

pub fn canonical_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value && !id.is_nil())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acceptance {
    Queued,
    RecipientUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchItem {
    pub recipient_id: String,
    pub request_id: String,
    pub acceptance: Acceptance,
}

/// Immutable acceptance receipt, not a cache of current delivery or final state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchReceipt {
    pub operation_id: String,
    pub created_at_ms: u64,
    pub items: Vec<DispatchItem>,
}
