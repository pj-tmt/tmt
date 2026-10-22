//! What a person's passthrough message earns after it is typed: the turn output of the turn
//! the typed text started (#4336). The text carries no request id, so a turn is matched to
//! its passthrough by the prompt the provider reported at turn start, never by time alone and
//! never by guessing from the recent finals.
//!
//! A capture is `pending` from the moment the text is typed, `bound` once a turn started with
//! that prompt, then `answered` with that turn's final message or `no_answer`. Neither wait is
//! open-ended: a capture no turn started for, or whose bound turn never ended, reads and settles
//! `no_answer` ([`BIND_WINDOW_MS`], [`BOUND_WINDOW_MS`]), and so does one whose turn a later turn
//! of the same session overtook.

use crate::driver::pane_input_text;

/// A passthrough no turn started for within this long can no longer bind. Once bound, the
/// turn is awaited however long it runs.
pub const BIND_WINDOW_MS: u64 = 600_000;

/// A bound turn that has not ended within this long is not awaited any more: a turn whose
/// end the hooks never admitted (a killed agent, a closed pane) would otherwise stay `bound`
/// for ever. Long enough for any single turn a person would still be waiting on.
pub const BOUND_WINDOW_MS: u64 = 21_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    Pending,
    Bound,
    Answered,
    NoAnswer,
}

impl CaptureState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Bound => "bound",
            Self::Answered => "answered",
            Self::NoAnswer => "no_answer",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "bound" => Some(Self::Bound),
            "answered" => Some(Self::Answered),
            "no_answer" => Some(Self::NoAnswer),
            _ => None,
        }
    }
}

/// The final assistant message of the bound turn, as the provider reported it. Plain
/// untrusted text, capped like any retained final.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassthroughAnswer {
    pub text: String,
    pub truncated: bool,
    pub provider: String,
    pub ended_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassthroughCapture {
    /// When the text was typed into the pane: the start of the binding window.
    pub delivered_at_ms: u64,
    pub state: CaptureState,
    pub bound_session: Option<String>,
    /// The provider's turn id when it gives one; Claude orders turns by session.
    pub bound_turn: Option<String>,
    pub bound_at_ms: Option<u64>,
    pub answer: Option<PassthroughAnswer>,
    pub settled_at_ms: Option<u64>,
}

impl PassthroughCapture {
    pub fn pending(delivered_at_ms: u64) -> Self {
        Self {
            delivered_at_ms,
            state: CaptureState::Pending,
            bound_session: None,
            bound_turn: None,
            bound_at_ms: None,
            answer: None,
            settled_at_ms: None,
        }
    }

    /// Whether a turn that starts at `now` may still bind to this capture.
    pub fn can_bind(&self, now: u64) -> bool {
        self.state == CaptureState::Pending
            && now.saturating_sub(self.delivered_at_ms) <= BIND_WINDOW_MS
    }

    /// Whether the bound turn is still being awaited at `now`.
    pub fn awaits_end(&self, now: u64) -> bool {
        self.state == CaptureState::Bound
            && self
                .bound_at_ms
                .is_some_and(|bound| now.saturating_sub(bound) <= BOUND_WINDOW_MS)
    }

    /// The state to show at `now`: a capture past its binding or bound window reads
    /// `no_answer` before anything has written that, so a read never needs a sweeper.
    pub fn state_at(&self, now: u64) -> CaptureState {
        match self.state {
            CaptureState::Pending if !self.can_bind(now) => CaptureState::NoAnswer,
            CaptureState::Bound if !self.awaits_end(now) => CaptureState::NoAnswer,
            state => state,
        }
    }
}

/// Whether the prompt a provider reported at turn start is the text a passthrough typed. Both
/// sides go through the pane's own `!` conversion and newline normalisation, and a provider
/// may trim the ends, so only the text between them has to be equal.
pub fn same_prompt(sent: &str, prompt: &str) -> bool {
    let normal = |text: &str| {
        pane_input_text(&text.replace("\r\n", "\n").replace('\r', "\n"))
            .trim()
            .to_owned()
    };
    let sent = normal(sent);
    !sent.is_empty() && sent == normal(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_matches_the_typed_text_after_the_panes_own_normalisation() {
        // The pane typed `！` for `!`; the provider reports what it received.
        assert!(same_prompt("Ship it!", "Ship it！"));
        assert!(same_prompt("Ship it!", "Ship it!"));
        assert!(same_prompt("line one\r\nline two", "line one\nline two\n"));
        assert!(same_prompt("  padded  ", "padded"));
        // A leading slash is an ordinary character here.
        assert!(same_prompt("/compact now", "/compact now"));
        assert!(!same_prompt("Ship it", "Ship it now"));
        assert!(!same_prompt("   ", "   "), "empty text matches nothing");
        assert!(
            !same_prompt("a b", "a  b"),
            "inner whitespace is part of the text"
        );
    }

    #[test]
    fn an_unbound_capture_stops_binding_after_the_window_and_then_reads_no_answer() {
        let capture = PassthroughCapture::pending(1_000);
        assert!(capture.can_bind(1_000 + BIND_WINDOW_MS));
        assert_eq!(
            capture.state_at(1_000 + BIND_WINDOW_MS),
            CaptureState::Pending
        );
        assert!(!capture.can_bind(1_001 + BIND_WINDOW_MS));
        assert_eq!(
            capture.state_at(1_001 + BIND_WINDOW_MS),
            CaptureState::NoAnswer
        );
        assert!(
            !capture.awaits_end(1_000),
            "only a bound capture awaits an end"
        );
    }

    #[test]
    fn a_bound_capture_awaits_its_turn_end_for_the_bound_window_and_then_reads_no_answer() {
        let bound = PassthroughCapture {
            state: CaptureState::Bound,
            bound_at_ms: Some(5_000),
            ..PassthroughCapture::pending(1_000)
        };
        assert!(!bound.can_bind(5_000), "a bound capture never binds again");
        let last = 5_000 + BOUND_WINDOW_MS;
        assert_eq!(bound.state_at(last), CaptureState::Bound);
        assert!(bound.awaits_end(last));
        assert_eq!(bound.state_at(last + 1), CaptureState::NoAnswer);
        assert!(!bound.awaits_end(last + 1));
        // A settled capture keeps what it settled as, however old.
        for state in [CaptureState::Answered, CaptureState::NoAnswer] {
            let settled = PassthroughCapture {
                state,
                ..bound.clone()
            };
            assert_eq!(settled.state_at(u64::MAX / 2), state);
        }
    }

    #[test]
    fn states_round_trip_through_their_stored_names() {
        for state in [
            CaptureState::Pending,
            CaptureState::Bound,
            CaptureState::Answered,
            CaptureState::NoAnswer,
        ] {
            assert_eq!(CaptureState::parse(state.as_str()), Some(state));
        }
        assert_eq!(CaptureState::parse("settled"), None);
    }
}
