//! Diagnostic observations of refused starts, never session admission authority.
use super::{BindingSessionState, HarnessId, ProviderSessionId, RuntimeState, SessionPreferences};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefusalReason {
    DriverMismatch,
    PreviousSessionLive,
    PreviousSessionUnknown,
    PrivateProcessUnverified,
    SharedRuntime,
    PaneProcess,
    AdmissionRefused,
}

impl StartRefusalReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DriverMismatch => "driver_mismatch",
            Self::PreviousSessionLive => "previous_session_live",
            Self::PreviousSessionUnknown => "previous_session_unknown",
            Self::PrivateProcessUnverified => "private_process_unverified",
            Self::SharedRuntime => "shared_runtime",
            Self::PaneProcess => "pane_process",
            Self::AdmissionRefused => "admission_refused",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Self::DriverMismatch => "the provider differs from the recorded driver",
            Self::PreviousSessionLive => "the recorded session is still running",
            Self::PreviousSessionUnknown => "the old process could not be conclusively verified",
            Self::PrivateProcessUnverified => {
                "a private main provider process could not be verified"
            }
            Self::SharedRuntime => "a shared runtime cannot be adopted from pane evidence",
            Self::PaneProcess => "the provider is the pane process itself",
            Self::AdmissionRefused => "the session admission was refused",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "driver_mismatch" => Self::DriverMismatch,
            "previous_session_live" => Self::PreviousSessionLive,
            "previous_session_unknown" => Self::PreviousSessionUnknown,
            "private_process_unverified" => Self::PrivateProcessUnverified,
            "shared_runtime" => Self::SharedRuntime,
            "pane_process" => Self::PaneProcess,
            "admission_refused" => Self::AdmissionRefused,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRefusal {
    pub harness: HarnessId,
    pub session: ProviderSessionId,
    pub reason: StartRefusalReason,
    pub observed_at_ms: u64,
}

impl BindingSessionState {
    /// Only current stored observations count; missing telemetry proves nothing.
    pub fn unbound(&self, preferences: &SessionPreferences) -> Option<bool> {
        if self.start_refusal.is_some() {
            return Some(true);
        }
        (self.state == RuntimeState::Running
            && self.key.as_ref().is_some_and(|key| {
                preferences.remembered.as_ref().is_some_and(|remembered| {
                    key.provider_session.as_ref() == Some(&remembered.provider_session)
                })
            }))
        .then_some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_codes_are_bounded_and_unknown_values_are_not_guessed() {
        for reason in [
            StartRefusalReason::DriverMismatch,
            StartRefusalReason::PreviousSessionLive,
            StartRefusalReason::PreviousSessionUnknown,
            StartRefusalReason::PrivateProcessUnverified,
            StartRefusalReason::SharedRuntime,
            StartRefusalReason::PaneProcess,
            StartRefusalReason::AdmissionRefused,
        ] {
            assert_eq!(StartRefusalReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(StartRefusalReason::parse("future"), None);
        assert_eq!(
            BindingSessionState::default().unbound(&SessionPreferences::default()),
            None
        );
    }
}
