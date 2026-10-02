//! Reply notices share a fixed pane window, never response acceptance or attention.
use crate::{
    driver::{InputActivity, InputState},
    endpoint::ProcessIncarnation,
};

pub const MAX_TYPING_WAIT_MS: u64 = 30_000;
pub const MAX_NOTICES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub id: String,
    pub originator_id: String,
    /// A binding UUID fences server, pane and process identity without duplicating them.
    pub binding_id: String,
    pub due_ms: u64,
    pub window_ms: u64,
    pub quiet_ms: u64,
    pub worker: Option<ProcessIncarnation>,
    pub sending: bool,
    /// At least one notice has never attempted external input.
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub request_id: String,
    pub text: String,
}

/// Only the owner may seal a batch. Another sealed batch for the same pane
/// retains transport ownership until settlement or proven process death.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendClaim {
    Ready(Vec<Notice>),
    Waiting(Batch),
    Lost,
}

/// A quiet client is evidence of no recent typing, not an empty input buffer.
pub fn typing_state(activity: InputActivity, quiet_ms: u64) -> InputState {
    match activity {
        InputActivity::ElapsedMs(elapsed) if elapsed < quiet_ms => InputState::Pending,
        InputActivity::ElapsedMs(_) => InputState::Empty,
        InputActivity::Unknown => InputState::Unknown,
    }
}

/// The monotonic lifetime is independent of wall-clock movement. Unknown host
/// evidence is not a typing claim; usability wins at the hard deadline.
pub fn notice_is_ready(input: InputState, window_elapsed: bool, wait_elapsed: bool) -> bool {
    window_elapsed && (wait_elapsed || input != InputState::Pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_period_uses_host_evidence_and_can_be_disabled() {
        assert_eq!(typing_state(InputActivity::Unknown, 0), InputState::Unknown);
        assert_eq!(
            typing_state(InputActivity::ElapsedMs(0), 0),
            InputState::Empty
        );
        assert_eq!(
            typing_state(InputActivity::ElapsedMs(1999), 2000),
            InputState::Pending
        );
        assert_eq!(
            typing_state(InputActivity::ElapsedMs(2000), 2000),
            InputState::Empty
        );
    }

    #[test]
    fn the_window_and_typing_bound_are_independent() {
        for state in [InputState::Empty, InputState::Pending, InputState::Unknown] {
            assert!(!notice_is_ready(state, false, false));
            assert!(notice_is_ready(state, true, true));
        }
        assert!(!notice_is_ready(InputState::Pending, true, false));
        assert!(notice_is_ready(InputState::Unknown, true, false));
        assert!(notice_is_ready(InputState::Empty, true, false));
    }
}
