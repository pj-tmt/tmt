//! Advisory originator hints never own a reply body or acknowledge attention.

use super::WakeState;
use crate::endpoint::ProcessIncarnation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintKind {
    Reply,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationPolicy {
    pub deadline_ms: u64,
    pub timeout_ms: u64,
    /// Only blocking talk owns response delivery. The detached observer does not.
    pub waiter: Option<ProcessIncarnation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRecord {
    pub policy: NotificationPolicy,
    pub reply: WakeState,
    pub timeout: WakeState,
    pub observed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginatorHint {
    pub request_id: String,
    pub originator_id: String,
    pub recipient_id: Option<String>,
    pub kind: HintKind,
    pub timeout_ms: u64,
}
