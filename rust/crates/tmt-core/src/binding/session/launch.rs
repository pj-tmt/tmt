//! Bounded observations of a managed launch, never admission or presence authority.
use super::{DriverState, HarnessId, ProviderSessionId, RuntimeMode, SessionTransition};
use crate::endpoint::ProcessIncarnation;

pub const RETENTION_MS: u64 = 24 * 60 * 60 * 1_000;
pub const MAXIMUM_PER_BINDING: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStart {
    pub process: ProcessIncarnation,
    pub transition: SessionTransition,
    pub session: ProviderSessionId,
    pub mode: RuntimeMode,
    pub state: Option<DriverState>,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningStep {
    Input,
    Decode,
    DriverAdmission,
    HostDiscovery,
    PaneSnapshot,
    ProviderVerification,
    Storage,
    Context,
}
impl SessionLearningStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Decode => "decode",
            Self::DriverAdmission => "driver_admission",
            Self::HostDiscovery => "host_discovery",
            Self::PaneSnapshot => "pane_snapshot",
            Self::ProviderVerification => "provider_verification",
            Self::Storage => "storage",
            Self::Context => "context",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "input" => Self::Input,
            "decode" => Self::Decode,
            "driver_admission" => Self::DriverAdmission,
            "host_discovery" => Self::HostDiscovery,
            "pane_snapshot" => Self::PaneSnapshot,
            "provider_verification" => Self::ProviderVerification,
            "storage" => Self::Storage,
            "context" => Self::Context,
            _ => return None,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningReason {
    InProgress,
    NotAdmitted,
    WorkerError,
    BudgetTimeout,
    SpawnFailed,
    LaunchEnded,
    Conflict,
}
impl SessionLearningReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::NotAdmitted => "not_admitted",
            Self::WorkerError => "worker_error",
            Self::BudgetTimeout => "budget_timeout",
            Self::SpawnFailed => "spawn_failed",
            Self::LaunchEnded => "launch_ended",
            Self::Conflict => "conflict",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "in_progress" => Self::InProgress,
            "not_admitted" => Self::NotAdmitted,
            "worker_error" => Self::WorkerError,
            "budget_timeout" => Self::BudgetTimeout,
            "spawn_failed" => Self::SpawnFailed,
            "launch_ended" => Self::LaunchEnded,
            "conflict" => Self::Conflict,
            _ => return None,
        })
    }
}
/// The decoder family that owns a hook event name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningDecoder {
    Turn,
    Prompt,
    ActivitySession,
    Lifecycle,
}
impl SessionLearningDecoder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Turn => "turn",
            Self::Prompt => "prompt",
            Self::ActivitySession => "activity_session",
            Self::Lifecycle => "lifecycle",
        }
    }
}

/// A hook event name from the first-party list. A payload with any other name
/// (or none) is `Unknown`, so no payload text is ever stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningEvent {
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    Stop,
    Interrupt,
    Notification,
    PostToolUse,
    PostToolUseFailure,
    Unknown,
}
impl SessionLearningEvent {
    pub fn from_hook_name(name: Option<&str>) -> Self {
        match name {
            Some("SessionStart") => Self::SessionStart,
            Some("SessionEnd") => Self::SessionEnd,
            Some("UserPromptSubmit") => Self::UserPromptSubmit,
            Some("Stop") => Self::Stop,
            Some("Interrupt") => Self::Interrupt,
            Some("Notification") => Self::Notification,
            Some("PostToolUse") => Self::PostToolUse,
            Some("PostToolUseFailure") => Self::PostToolUseFailure,
            _ => Self::Unknown,
        }
    }
    /// The decoder that should have accepted this event; none for an unknown name.
    pub fn decoder(self) -> Option<SessionLearningDecoder> {
        match self {
            Self::SessionStart | Self::SessionEnd => Some(SessionLearningDecoder::Lifecycle),
            Self::UserPromptSubmit => Some(SessionLearningDecoder::Prompt),
            Self::Stop | Self::Interrupt => Some(SessionLearningDecoder::Turn),
            Self::Notification | Self::PostToolUse | Self::PostToolUseFailure => {
                Some(SessionLearningDecoder::ActivitySession)
            }
            Self::Unknown => None,
        }
    }
}

/// Why a hook that decoded and could not bind admitted nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningStop {
    /// An activity-only event carried no activity to record.
    ActivityMissing,
    /// A finished tool call with no waiting activity to finish.
    NotWaiting,
    /// An installed answer observer owns this turn end.
    ObserverOwnsStop,
    /// No bound launch is waiting for this provider session.
    UnboundSession,
    /// The bound session or process is not the one the payload names.
    NotCurrent,
}
/// Why a Digest Stop hook ended with no output on an admitted launch (#4871). Only a
/// refusal is named: a Stop with no Digest due is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningDigestStop {
    /// The supervisor could not start its worker.
    WorkerSpawn,
    /// The worker did not finish before its share of the hook budget.
    WorkerTimeout,
    /// The worker failed without naming a check.
    WorkerError,
    /// The worker finished but wrote to stderr.
    WorkerStderr,
    /// The answer capture that runs before a Digest failed.
    CaptureFailed,
    /// The supplied launch coordinates are not valid.
    LaunchArgument,
    /// The driver reported no host evidence.
    LaunchHost,
    /// The hook's caller is not a verified bound pane.
    LaunchCaller,
    /// The caller's verification failed before it gave an answer.
    LaunchCallerProbe,
    /// The binding is not this launch's running binding.
    LaunchBinding,
    /// The driver found no provider process for the session.
    LaunchProcess,
    /// The bound session key is not this process and provider session.
    LaunchSession,
    /// The remembered session is not this provider session.
    LaunchRemembered,
    /// The binding is not active on its live endpoint.
    LaunchEndpoint,
    /// The launch owner is live with a start identity other than the recorded one.
    LaunchOwner,
    /// The launch owner's process probe gave no answer.
    LaunchOwnerProbe,
    /// The launch owner is not live.
    LaunchOwnerNotLive,
    /// The hook deadline passed during verification.
    LaunchDeadline,
    /// The restart notice could not be claimed: its private directory is unusable.
    NoticeUnavailable,
    /// The worker's handoff did not match this launch or protocol.
    HandoffInvalid,
    /// The claimed checklist changed before it could be published.
    ChecklistChanged,
}
impl SessionLearningDigestStop {
    pub const ALL: [Self; 21] = [
        Self::WorkerSpawn,
        Self::WorkerTimeout,
        Self::WorkerError,
        Self::WorkerStderr,
        Self::CaptureFailed,
        Self::LaunchArgument,
        Self::LaunchHost,
        Self::LaunchCaller,
        Self::LaunchCallerProbe,
        Self::LaunchBinding,
        Self::LaunchProcess,
        Self::LaunchSession,
        Self::LaunchRemembered,
        Self::LaunchEndpoint,
        Self::LaunchOwner,
        Self::LaunchOwnerProbe,
        Self::LaunchOwnerNotLive,
        Self::LaunchDeadline,
        Self::NoticeUnavailable,
        Self::HandoffInvalid,
        Self::ChecklistChanged,
    ];
    /// The name without the `digest_stop:` family prefix. A worker reports its own
    /// refusal to its supervisor with this name.
    pub fn name(self) -> &'static str {
        match self {
            Self::WorkerSpawn => "worker_spawn",
            Self::WorkerTimeout => "worker_timeout",
            Self::WorkerError => "worker_error",
            Self::WorkerStderr => "worker_stderr",
            Self::CaptureFailed => "capture_failed",
            Self::LaunchArgument => "launch:argument",
            Self::LaunchHost => "launch:host",
            Self::LaunchCaller => "launch:caller",
            Self::LaunchCallerProbe => "launch:caller_probe",
            Self::LaunchBinding => "launch:binding",
            Self::LaunchProcess => "launch:process",
            Self::LaunchSession => "launch:session",
            Self::LaunchRemembered => "launch:remembered",
            Self::LaunchEndpoint => "launch:endpoint",
            Self::LaunchOwner => "launch:owner",
            Self::LaunchOwnerProbe => "launch:owner_probe",
            Self::LaunchOwnerNotLive => "launch:owner_not_live",
            Self::LaunchDeadline => "launch:deadline",
            Self::NoticeUnavailable => "notice_unavailable",
            Self::HandoffInvalid => "handoff_invalid",
            Self::ChecklistChanged => "checklist_changed",
        }
    }
    /// A refusal the worker itself can report: the capture or one launch check.
    pub fn from_worker(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stop| {
            stop.name() == name
                && (*stop == Self::CaptureFailed || stop.name().starts_with("launch:"))
        })
    }
    /// The stored diagnostic: the step that refused, and the closest reason.
    pub fn diagnostic(self) -> SessionLearningDiagnostic {
        let step = match self {
            Self::CaptureFailed => SessionLearningStep::Storage,
            stop if stop.name().starts_with("launch:") => SessionLearningStep::ProviderVerification,
            _ => SessionLearningStep::Context,
        };
        let reason = match self {
            Self::WorkerSpawn => SessionLearningReason::SpawnFailed,
            Self::WorkerTimeout | Self::LaunchDeadline => SessionLearningReason::BudgetTimeout,
            Self::NoticeUnavailable | Self::ChecklistChanged => SessionLearningReason::Conflict,
            _ => SessionLearningReason::WorkerError,
        };
        SessionLearningDiagnostic {
            step,
            reason,
            detail: Some(SessionLearningDetail::DigestStop(self)),
        }
    }
}

/// What a stage stopped on, beyond its step and reason. Fixed tokens only:
/// never a payload body, path or identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLearningDetail {
    /// No decoder accepted the payload of this event.
    Refused(SessionLearningEvent),
    /// The driver found no provider process among the hook's ancestors.
    HostUnsupported,
    /// A decoded event that cannot bind admitted nothing, for this reason.
    Stopped(SessionLearningEvent, SessionLearningStop),
    /// A Digest Stop hook on an admitted launch refused, for this reason.
    DigestStop(SessionLearningDigestStop),
}
impl SessionLearningDetail {
    /// Every `Stopped` pair a hook path can produce, with its stored token. Nothing else is
    /// writable, so the stored list stays closed. An event no decoder accepts is `Unknown`.
    const STOPPED: [(SessionLearningEvent, SessionLearningStop, &'static str); 24] = {
        use SessionLearningEvent as E;
        use SessionLearningStop as S;
        [
            (
                E::Stop,
                S::ObserverOwnsStop,
                "stopped:turn:Stop:observer_owns_stop",
            ),
            (
                E::Stop,
                S::UnboundSession,
                "stopped:turn:Stop:unbound_session",
            ),
            (E::Stop, S::NotCurrent, "stopped:turn:Stop:not_current"),
            (
                E::Interrupt,
                S::ObserverOwnsStop,
                "stopped:turn:Interrupt:observer_owns_stop",
            ),
            (
                E::Interrupt,
                S::UnboundSession,
                "stopped:turn:Interrupt:unbound_session",
            ),
            (
                E::Interrupt,
                S::NotCurrent,
                "stopped:turn:Interrupt:not_current",
            ),
            (
                E::UserPromptSubmit,
                S::UnboundSession,
                "stopped:prompt:UserPromptSubmit:unbound_session",
            ),
            (
                E::UserPromptSubmit,
                S::NotCurrent,
                "stopped:prompt:UserPromptSubmit:not_current",
            ),
            (
                E::Notification,
                S::ActivityMissing,
                "stopped:activity_session:Notification:activity_missing",
            ),
            (
                E::Notification,
                S::UnboundSession,
                "stopped:activity_session:Notification:unbound_session",
            ),
            (
                E::Notification,
                S::NotCurrent,
                "stopped:activity_session:Notification:not_current",
            ),
            (
                E::PostToolUse,
                S::ActivityMissing,
                "stopped:activity_session:PostToolUse:activity_missing",
            ),
            (
                E::PostToolUse,
                S::NotWaiting,
                "stopped:activity_session:PostToolUse:not_waiting",
            ),
            (
                E::PostToolUse,
                S::UnboundSession,
                "stopped:activity_session:PostToolUse:unbound_session",
            ),
            (
                E::PostToolUse,
                S::NotCurrent,
                "stopped:activity_session:PostToolUse:not_current",
            ),
            (
                E::PostToolUseFailure,
                S::ActivityMissing,
                "stopped:activity_session:PostToolUseFailure:activity_missing",
            ),
            (
                E::PostToolUseFailure,
                S::NotWaiting,
                "stopped:activity_session:PostToolUseFailure:not_waiting",
            ),
            (
                E::PostToolUseFailure,
                S::UnboundSession,
                "stopped:activity_session:PostToolUseFailure:unbound_session",
            ),
            (
                E::PostToolUseFailure,
                S::NotCurrent,
                "stopped:activity_session:PostToolUseFailure:not_current",
            ),
            (
                E::Unknown,
                S::ActivityMissing,
                "stopped:unknown:activity_missing",
            ),
            (E::Unknown, S::NotWaiting, "stopped:unknown:not_waiting"),
            (
                E::Unknown,
                S::ObserverOwnsStop,
                "stopped:unknown:observer_owns_stop",
            ),
            (
                E::Unknown,
                S::UnboundSession,
                "stopped:unknown:unbound_session",
            ),
            (E::Unknown, S::NotCurrent, "stopped:unknown:not_current"),
        ]
    };

    /// Every detail, in the order the latest `detail` migration (`schema/080.sql`) lists them.
    pub const ALL: [Self; 55] = {
        use SessionLearningEvent as E;
        let mut all = [Self::HostUnsupported; 55];
        let head = [
            Self::Refused(E::SessionStart),
            Self::Refused(E::SessionEnd),
            Self::Refused(E::UserPromptSubmit),
            Self::Refused(E::Stop),
            Self::Refused(E::Interrupt),
            Self::Refused(E::Notification),
            Self::Refused(E::PostToolUse),
            Self::Refused(E::PostToolUseFailure),
            Self::Refused(E::Unknown),
            Self::HostUnsupported,
        ];
        let mut index = 0;
        while index < head.len() {
            all[index] = head[index];
            index += 1;
        }
        let mut stopped = 0;
        while stopped < Self::STOPPED.len() {
            all[index + stopped] =
                Self::Stopped(Self::STOPPED[stopped].0, Self::STOPPED[stopped].1);
            stopped += 1;
        }
        index += stopped;
        let mut digest = 0;
        while digest < SessionLearningDigestStop::ALL.len() {
            all[index + digest] = Self::DigestStop(SessionLearningDigestStop::ALL[digest]);
            digest += 1;
        }
        all
    };

    /// The detail for a decoded event that stopped for this reason, or `None` for a pair no
    /// hook path can produce.
    pub fn stopped(event: SessionLearningEvent, stop: SessionLearningStop) -> Option<Self> {
        Self::STOPPED
            .iter()
            .any(|(known_event, known_stop, _)| *known_event == event && *known_stop == stop)
            .then_some(Self::Stopped(event, stop))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Refused(SessionLearningEvent::SessionStart) => "refused:lifecycle:SessionStart",
            Self::Refused(SessionLearningEvent::SessionEnd) => "refused:lifecycle:SessionEnd",
            Self::Refused(SessionLearningEvent::UserPromptSubmit) => {
                "refused:prompt:UserPromptSubmit"
            }
            Self::Refused(SessionLearningEvent::Stop) => "refused:turn:Stop",
            Self::Refused(SessionLearningEvent::Interrupt) => "refused:turn:Interrupt",
            Self::Refused(SessionLearningEvent::Notification) => {
                "refused:activity_session:Notification"
            }
            Self::Refused(SessionLearningEvent::PostToolUse) => {
                "refused:activity_session:PostToolUse"
            }
            Self::Refused(SessionLearningEvent::PostToolUseFailure) => {
                "refused:activity_session:PostToolUseFailure"
            }
            Self::Refused(SessionLearningEvent::Unknown) => "refused:unknown",
            Self::HostUnsupported => "host_unsupported",
            Self::Stopped(event, stop) => Self::STOPPED
                .iter()
                .find(|(known_event, known_stop, _)| *known_event == event && *known_stop == stop)
                .map_or("stopped:invalid", |(_, _, token)| token),
            Self::DigestStop(stop) => DIGEST_STOP_TOKENS[stop as usize],
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|detail| detail.as_str() == value)
    }
}

/// `digest_stop:` and the name, in `SessionLearningDigestStop` order.
const DIGEST_STOP_TOKENS: [&str; 21] = [
    "digest_stop:worker_spawn",
    "digest_stop:worker_timeout",
    "digest_stop:worker_error",
    "digest_stop:worker_stderr",
    "digest_stop:capture_failed",
    "digest_stop:launch:argument",
    "digest_stop:launch:host",
    "digest_stop:launch:caller",
    "digest_stop:launch:caller_probe",
    "digest_stop:launch:binding",
    "digest_stop:launch:process",
    "digest_stop:launch:session",
    "digest_stop:launch:remembered",
    "digest_stop:launch:endpoint",
    "digest_stop:launch:owner",
    "digest_stop:launch:owner_probe",
    "digest_stop:launch:owner_not_live",
    "digest_stop:launch:deadline",
    "digest_stop:notice_unavailable",
    "digest_stop:handoff_invalid",
    "digest_stop:checklist_changed",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLearningDiagnostic {
    pub step: SessionLearningStep,
    pub reason: SessionLearningReason,
    pub detail: Option<SessionLearningDetail>,
}
impl SessionLearningDiagnostic {
    pub fn new(step: SessionLearningStep, reason: SessionLearningReason) -> Self {
        Self {
            step,
            reason,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStatus {
    Intent,
    Pending,
    Admitted,
    Settled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchObservation {
    pub binding_id: String,
    pub preferences_fingerprint: String,
    pub owner: ProcessIncarnation,
    pub driver: HarnessId,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub pending: Option<PendingStart>,
    pub diagnostic: Option<SessionLearningDiagnostic>,
    pub status: LaunchStatus,
}
impl LaunchObservation {
    /// A match only selects evidence; the caller must still use the ordinary
    /// live child/owner verification and binding/session admission CAS.
    pub fn pending_for(
        &self,
        owner: &ProcessIncarnation,
        child: &ProcessIncarnation,
        driver: &HarnessId,
        now_ms: u64,
    ) -> Option<&PendingStart> {
        if matches!(self.status, LaunchStatus::Settled | LaunchStatus::Admitted)
            || &self.owner != owner
            || &self.driver != driver
            || now_ms < self.created_at_ms
            || now_ms >= self.expires_at_ms
        {
            return None;
        }
        self.pending
            .as_ref()
            .filter(|pending| &pending.process == child)
    }
}

/// Recovery is presentation only; this command is never executed automatically.
pub fn session_learning_recovery(
    identity: &crate::identity::Identity,
    driver: &HarnessId,
    preferences: &super::SessionPreferences,
) -> String {
    if preferences
        .remembered
        .as_ref()
        .is_some_and(|remembered| &remembered.harness == driver)
    {
        return format!(
            "Exit the agent in its pane, then run: tmt resume -- {}",
            identity.id
        );
    }
    let command = crate::driver::descriptor::named(driver.as_str())
        .and_then(|driver| driver.executables.first())
        .map(|executable| {
            format!(
                "Exit the agent in its pane, then run: tmt run -- {} {}",
                crate::names::command_word(&identity.name),
                crate::names::command_word(executable)
            )
        });
    command
        .filter(|command| command.len() <= 256)
        .unwrap_or_else(|| {
            "Exit the agent in its pane, then start it again with tmt run in that pane.".into()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_detail_token_is_distinct_parses_back_and_names_its_decoder() {
        let mut seen = std::collections::HashSet::new();
        for detail in SessionLearningDetail::ALL {
            assert!(seen.insert(detail.as_str()), "{detail:?} repeats a token");
            assert_eq!(SessionLearningDetail::parse(detail.as_str()), Some(detail));
            assert!(
                detail
                    .as_str()
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b':')
            );
            if let SessionLearningDetail::Stopped(event, stop) = detail {
                let expected = match event.decoder() {
                    Some(decoder) => format!("stopped:{}:{event:?}:", decoder.as_str()),
                    None => "stopped:unknown:".to_owned(),
                };
                assert!(detail.as_str().starts_with(&expected), "{detail:?}");
                assert_eq!(SessionLearningDetail::stopped(event, stop), Some(detail));
                continue;
            }
            if let SessionLearningDetail::DigestStop(stop) = detail {
                assert_eq!(detail.as_str(), format!("digest_stop:{}", stop.name()));
                assert_eq!(stop.diagnostic().detail, Some(detail));
                continue;
            }
            let SessionLearningDetail::Refused(event) = detail else {
                continue;
            };
            let expected = match event.decoder() {
                Some(decoder) => format!("refused:{}:{event:?}", decoder.as_str()),
                None => "refused:unknown".to_owned(),
            };
            assert_eq!(detail.as_str(), expected);
            if event != SessionLearningEvent::Unknown {
                assert_eq!(
                    SessionLearningEvent::from_hook_name(Some(&format!("{event:?}"))),
                    event
                );
            }
        }
        assert_eq!(SessionLearningDetail::parse("refused:turn:Anything"), None);
    }

    #[test]
    fn a_worker_reports_only_its_capture_or_a_launch_check() {
        use SessionLearningDigestStop as D;
        for stop in D::ALL {
            let expected = stop == D::CaptureFailed || stop.name().starts_with("launch:");
            assert_eq!(
                D::from_worker(stop.name()) == Some(stop),
                expected,
                "{stop:?}"
            );
        }
        assert_eq!(D::from_worker("digest_stop:launch:owner"), None);
        assert_eq!(D::from_worker("launch:owner\n"), None);
        assert_eq!(
            D::LaunchOwner.diagnostic().step,
            SessionLearningStep::ProviderVerification
        );
        assert_eq!(
            D::WorkerTimeout.diagnostic().reason,
            SessionLearningReason::BudgetTimeout
        );
    }

    #[test]
    fn only_a_pair_a_hook_path_can_produce_is_a_stopped_detail() {
        use SessionLearningEvent as E;
        use SessionLearningStop as S;
        assert!(SessionLearningDetail::stopped(E::Stop, S::UnboundSession).is_some());
        assert!(SessionLearningDetail::stopped(E::Unknown, S::NotCurrent).is_some());
        // A finished tool call is the only event that can find nothing waiting, and a
        // session event is never decoded by an observer, so neither pairs with these.
        assert_eq!(SessionLearningDetail::stopped(E::Stop, S::NotWaiting), None);
        assert_eq!(
            SessionLearningDetail::stopped(E::UserPromptSubmit, S::ActivityMissing),
            None
        );
        assert_eq!(
            SessionLearningDetail::stopped(E::SessionStart, S::UnboundSession),
            None
        );
        // An invalid pair never reads like a stored token.
        assert_eq!(
            SessionLearningDetail::Stopped(E::SessionEnd, S::NotCurrent).as_str(),
            "stopped:invalid"
        );
        assert_eq!(SessionLearningDetail::parse("stopped:invalid"), None);
    }

    #[test]
    fn an_event_name_outside_the_list_or_missing_is_unknown() {
        for name in [
            None,
            Some(""),
            Some("Stop "),
            Some("stop"),
            Some("PreInvocation"),
        ] {
            assert_eq!(
                SessionLearningEvent::from_hook_name(name),
                SessionLearningEvent::Unknown
            );
        }
        assert_eq!(SessionLearningEvent::Unknown.decoder(), None);
    }

    #[test]
    fn recovery_never_suggests_resume_without_coordinates_or_truncates_a_command() {
        let mut identity = crate::identity::Identity {
            id: "123e4567-e89b-42d3-a456-426614174000".into(),
            name: "Alpha".into(),
            canonical_name: "alpha".into(),
            lifetime: crate::identity::Lifetime::Saved,
            created_at: "created".into(),
            updated_at: "updated".into(),
        };
        let driver = HarnessId::new("codex").unwrap();
        let mut preferences = super::super::SessionPreferences::default();
        assert!(
            session_learning_recovery(&identity, &driver, &preferences)
                .ends_with("tmt run -- Alpha codex")
        );
        identity.name = "🧑".repeat(64);
        let fresh = session_learning_recovery(&identity, &driver, &preferences);
        assert!(fresh.len() <= 256 && !fresh.contains('🧑'));
        preferences.remember(
            driver.clone(),
            RuntimeMode::new("default").unwrap(),
            ProviderSessionId::new("known").unwrap(),
        );
        assert!(
            session_learning_recovery(&identity, &driver, &preferences)
                .ends_with(&format!("tmt resume -- {}", identity.id))
        );
        assert!(
            !session_learning_recovery(&identity, &HarnessId::new("claude").unwrap(), &preferences)
                .contains("tmt resume")
        );
    }

    #[test]
    fn a_pending_start_selects_only_its_exact_live_launch_coordinates() {
        let owner = ProcessIncarnation::new(10, "owner").unwrap();
        let child = ProcessIncarnation::new(20, "child").unwrap();
        let driver = HarnessId::new("claude").unwrap();
        let mut observation = LaunchObservation {
            binding_id: "binding".into(),
            preferences_fingerprint: "test-fingerprint".into(),
            owner: owner.clone(),
            driver: driver.clone(),
            created_at_ms: 100,
            expires_at_ms: 200,
            status: LaunchStatus::Pending,
            diagnostic: None,
            pending: Some(PendingStart {
                process: child.clone(),
                transition: SessionTransition::Started,
                session: ProviderSessionId::new("session").unwrap(),
                mode: RuntimeMode::new("default").unwrap(),
                state: None,
                observed_at_ms: 110,
            }),
        };
        assert!(
            observation
                .pending_for(&owner, &child, &driver, 150)
                .is_some()
        );
        for time in [99, 200, u64::MAX] {
            assert!(
                observation
                    .pending_for(&owner, &child, &driver, time)
                    .is_none()
            );
        }
        assert!(
            observation
                .pending_for(
                    &ProcessIncarnation::new(10, "reused").unwrap(),
                    &child,
                    &driver,
                    150
                )
                .is_none()
        );
        assert!(
            observation
                .pending_for(
                    &owner,
                    &ProcessIncarnation::new(20, "reused").unwrap(),
                    &driver,
                    150
                )
                .is_none()
        );
        assert!(
            observation
                .pending_for(&owner, &child, &HarnessId::new("codex").unwrap(), 150)
                .is_none()
        );
        observation.status = LaunchStatus::Settled;
        assert!(
            observation
                .pending_for(&owner, &child, &driver, 150)
                .is_none()
        );
        observation.status = LaunchStatus::Admitted;
        assert!(
            observation
                .pending_for(&owner, &child, &driver, 150)
                .is_none()
        );
    }
}
