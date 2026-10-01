//! Registered runtime recognition and exact resume commands.
//! Explicit launches bypass argument planning entirely: they preserve argv.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::Path,
};
use tmt_core::{
    binding::BindingEntry,
    binding::session::{HarnessId, RememberedSession, SessionPreferences},
    driver::{ActionResult, DeliveryAcceptance, Driver, HarnessResume, HarnessStart, SendFailure},
};

pub mod channel;
pub mod driver_state;
pub(crate) mod evidence;
pub mod hook_protocol;
pub mod lifecycle;
#[cfg(test)]
mod prompt_tests;
pub mod transcript;
#[cfg(test)]
mod usage_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCommand {
    pub executable: OsString,
    pub args: Vec<OsString>,
}

impl RuntimeCommand {
    pub fn verbatim(command: &[OsString]) -> Option<Self> {
        let (executable, args) = command.split_first()?;
        Some(Self {
            executable: executable.clone(),
            args: args.to_vec(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    InvalidSession,
    InvalidRegistration,
    Channel(channel::ChannelFault),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSession => "The remembered runtime session is not an exact UUID.",
            Self::InvalidRegistration => {
                "Runtime registration requires a unique harness ID and a bare command name."
            }
            Self::Channel(fault) => return fault.fmt(formatter),
        })
    }
}
impl std::error::Error for RuntimeError {}

type RegisteredDriver =
    dyn Driver<Target = BindingEntry, Error = RuntimeError, Launch = RuntimeCommand>;

struct Registration {
    harness: HarnessId,
    executable: OsString,
    priority: i32,
    driver: Box<RegisteredDriver>,
    lifecycle: Option<Box<dyn lifecycle::RuntimeLifecycle>>,
    channel: Option<Box<dyn channel::RuntimeChannel>>,
}

/// First-party and community drivers use the same registration boundary.
/// Higher priority wins; equal priority is ordered by stable harness ID.
/// This registry does not discover, install or execute extension code.
/// Future external registration must require explicit trust before overriding
/// a first-party claim; priority alone is not permission to load driver code.
#[derive(Default)]
pub struct RuntimeRegistry {
    registrations: Vec<Registration>,
}

impl RuntimeRegistry {
    /// The runtimes of the built-in drivers.
    pub fn first_party() -> Self {
        Self::from_drivers(&crate::drivers::Registry::builtin())
    }

    /// One registration per driver with a runtime, run by its first executable.
    pub fn from_drivers(drivers: &crate::drivers::Registry) -> Self {
        let mut registry = Self::default();
        for driver in drivers.iter() {
            let Some(runtime) = &driver.runtime else {
                continue;
            };
            let harness = HarnessId::new(driver.name()).expect("valid driver name");
            let executable = driver.descriptor.executables[0];
            registry
                .register_boxed(harness.clone(), executable, 0, (runtime.driver)())
                .expect("unique built-in runtime registration");
            registry
                .register_lifecycle(&harness, (runtime.lifecycle)())
                .expect("registered driver lifecycle");
            if let Some(channel) = runtime.channel {
                registry
                    .register_channel(&harness, channel())
                    .expect("registered driver channel");
            }
        }
        registry
    }

    pub fn register(
        &mut self,
        harness: HarnessId,
        executable: &str,
        priority: i32,
        driver: impl Driver<Target = BindingEntry, Error = RuntimeError, Launch = RuntimeCommand>
        + 'static,
    ) -> Result<(), RuntimeError> {
        self.register_boxed(harness, executable, priority, Box::new(driver))
    }

    fn register_boxed(
        &mut self,
        harness: HarnessId,
        executable: &str,
        priority: i32,
        driver: Box<RegisteredDriver>,
    ) -> Result<(), RuntimeError> {
        if executable.is_empty()
            || executable.contains('/')
            || executable.chars().any(char::is_control)
            || self
                .registrations
                .iter()
                .any(|entry| entry.harness == harness)
        {
            return Err(RuntimeError::InvalidRegistration);
        }
        self.registrations.push(Registration {
            harness,
            executable: executable.into(),
            priority,
            driver,
            lifecycle: None,
            channel: None,
        });
        self.registrations.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.harness.as_str().cmp(b.harness.as_str()))
        });
        Ok(())
    }

    pub fn register_lifecycle(
        &mut self,
        harness: &HarnessId,
        lifecycle: Box<dyn lifecycle::RuntimeLifecycle>,
    ) -> Result<(), RuntimeError> {
        let entry = self
            .registrations
            .iter_mut()
            .find(|entry| &entry.harness == harness)
            .filter(|entry| entry.lifecycle.is_none())
            .ok_or(RuntimeError::InvalidRegistration)?;
        entry.lifecycle = Some(lifecycle);
        Ok(())
    }

    pub fn register_channel(
        &mut self,
        harness: &HarnessId,
        channel: Box<dyn channel::RuntimeChannel>,
    ) -> Result<(), RuntimeError> {
        let entry = self
            .registrations
            .iter_mut()
            .find(|entry| &entry.harness == harness)
            .filter(|entry| entry.channel.is_none())
            .ok_or(RuntimeError::InvalidRegistration)?;
        entry.channel = Some(channel);
        Ok(())
    }

    /// The driver that has an enrollment on record for this binding, whichever
    /// harness the identity currently prefers: the enrollment is written before the
    /// preference is, so it, not the preference, decides who owns the route.
    /// `Err` when any driver cannot tell, or when two claim the same binding.
    pub fn enrolled_harness(
        &self,
        directory: &Path,
        binding_id: &str,
    ) -> Result<Option<HarnessId>, channel::ChannelFault> {
        let mut found = None;
        for entry in &self.registrations {
            let Some(channel) = &entry.channel else {
                continue;
            };
            if channel.enrolled(directory, binding_id)?
                && found.replace(entry.harness.clone()).is_some()
            {
                return Err(channel::ChannelFault::Mismatch);
            }
        }
        Ok(found)
    }

    /// What the drivers' enrollments say about one pane, whatever binding or
    /// identity the pane has now. The first driver that finds a live or unconfirmed
    /// enrollment attributed to it decides; records no driver could attribute are
    /// merged so the caller can name them. A driver that cannot tell makes it
    /// unknown (`Err`).
    pub fn enrolled_in_pane(
        &self,
        directory: &Path,
        pane: &channel::PaneAddress<'_>,
        binding_id: Option<&str>,
        deadline: std::time::Instant,
    ) -> Result<channel::PaneEvidence, channel::EvidenceError> {
        let mut merged = channel::PaneEvidence::default();
        for entry in &self.registrations {
            let Some(channel) = &entry.channel else {
                continue;
            };
            let found = channel.enrolled_in_pane(directory, pane, binding_id, deadline)?;
            merged.skipped.extend(found.skipped);
            if found.enrolled {
                merged.enrolled = true;
                return Ok(merged);
            }
        }
        Ok(merged)
    }

    /// The harness's channel, if its driver offers one.
    pub fn channel(&self, harness: &HarnessId) -> Option<&dyn channel::RuntimeChannel> {
        self.registrations
            .iter()
            .find(|entry| &entry.harness == harness)?
            .channel
            .as_deref()
    }

    pub fn lifecycle(&self, harness: &HarnessId) -> Option<&dyn lifecycle::RuntimeLifecycle> {
        self.registrations
            .iter()
            .find(|entry| &entry.harness == harness)?
            .lifecycle
            .as_deref()
    }

    pub fn claim(&self, executable: &OsStr) -> Option<HarnessId> {
        let executable = executable.to_str()?;
        self.registrations.iter().find_map(|entry| {
            entry
                .driver
                .claims(executable)
                .filter(|claim| claim == &entry.harness)
        })
    }

    pub fn send(
        &mut self,
        harness: &HarnessId,
        target: &BindingEntry,
        message: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<RuntimeError>> {
        match self
            .registrations
            .iter_mut()
            .find(|entry| &entry.harness == harness)
        {
            Some(entry) => entry.driver.send(target, message),
            None => ActionResult::Unsupported,
        }
    }

    /// Relaunch resolves the registered bare executable through PATH, without
    /// replaying any previous arguments or executable path.
    pub fn relaunch(&self, harness: &HarnessId) -> Option<RuntimeCommand> {
        self.registrations
            .iter()
            .find(|entry| &entry.harness == harness)
            .map(|entry| RuntimeCommand {
                executable: entry.executable.clone(),
                args: Vec::new(),
            })
    }

    pub fn resume(
        &mut self,
        session: &RememberedSession,
    ) -> ActionResult<RuntimeCommand, RuntimeError> {
        let Some(entry) = self
            .registrations
            .iter_mut()
            .find(|entry| entry.harness == session.harness)
        else {
            return ActionResult::Unsupported;
        };
        entry.driver.resume(HarnessResume {
            start: HarnessStart {
                harness: &session.harness,
                context: None,
            },
            session: &session.provider_session,
            mode: &session.mode,
            state: session.state.as_ref(),
        })
    }
}

/// What reconciliation removed from a remembered session, for reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateReconciliation {
    /// The session's driver is not registered: session and state are purged.
    UnregisteredDriver(HarnessId),
    /// The driver cannot read this state version (or has no persistence):
    /// the state is discarded and the session kept.
    DiscardedState { harness: HarnessId, version: u16 },
}

impl RuntimeRegistry {
    /// Apply the persistence rules before a remembered session is used or
    /// updated: never force-read state a driver cannot read, and leave no
    /// session behind for a driver that is gone. Callers persist the result.
    pub fn reconcile(&self, preferences: &mut SessionPreferences) -> Option<StateReconciliation> {
        let remembered = preferences.remembered.as_mut()?;
        let harness = remembered.harness.clone();
        if !self
            .registrations
            .iter()
            .any(|entry| entry.harness == harness)
        {
            preferences.remembered = None;
            return Some(StateReconciliation::UnregisteredDriver(harness));
        }
        let version = remembered.state.as_ref()?.version();
        if !self
            .lifecycle(&harness)
            .is_some_and(|lifecycle| lifecycle.reads_state(version))
        {
            remembered.state = None;
            return Some(StateReconciliation::DiscardedState { harness, version });
        }
        None
    }

    /// The model a remembered session's own driver recorded, if it can read
    /// its state. Core never parses driver state itself.
    pub fn remembered_model(&self, session: &RememberedSession) -> Option<String> {
        self.lifecycle(&session.harness)?
            .state_model(session.state.as_ref()?)
    }

    /// The context usage a remembered session's own driver recorded, if it
    /// can read its state.
    pub fn remembered_usage(&self, session: &RememberedSession) -> Option<driver_state::Usage> {
        self.lifecycle(&session.harness)?
            .state_usage(session.state.as_ref()?)
    }

    /// Harness IDs with a registration, for purging sessions of removed drivers.
    pub fn harnesses(&self) -> impl Iterator<Item = &str> {
        self.registrations
            .iter()
            .map(|entry| entry.harness.as_str())
    }
}

/// A first-party driver claims a command whose file name is its own name.
pub(crate) fn claim_named(command: &str, name: &str) -> Option<HarnessId> {
    (Path::new(command).file_name() == Some(OsStr::new(name)))
        .then(|| HarnessId::new(name).expect("valid driver name"))
}

/// The checks every first-party resume shares: its own harness, no injected
/// context, one of its modes and a UUID session. Completes with the model to
/// replay, if the driver recorded one it can read.
pub(crate) fn first_party_resume(
    resume: &HarnessResume<'_>,
    name: &str,
    modes: &[&str],
) -> ActionResult<Option<String>, RuntimeError> {
    if resume.start.harness.as_str() != name
        || resume.start.context.is_some()
        || !modes.contains(&resume.mode.as_str())
    {
        return ActionResult::Unsupported;
    }
    // The first-party versions verified in #321 expose UUID session IDs.
    // Community drivers retain their own opaque-ID contract.
    if uuid::Uuid::parse_str(resume.session.as_str()).is_err() {
        return ActionResult::Failed(RuntimeError::InvalidSession);
    }
    // Only a model the provider reported is replayed; unreadable state
    // resumes with the provider's default rather than a guess. Placement
    // follows each CLI's recorded usage (runtime/fixtures/README.md).
    ActionResult::Completed(resume.state.and_then(driver_state::state_model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drivers::{claude, codex};
    use std::os::unix::ffi::OsStringExt;
    use tmt_core::binding::session::{ProviderSessionId, RuntimeMode};

    fn id(name: &str) -> HarnessId {
        HarnessId::new(name).unwrap()
    }

    #[test]
    fn launch_preserves_every_argument_byte_and_claim_never_changes_it() {
        let command = vec![
            OsString::from("/custom/bin/claude"),
            "--session-id".into(),
            "user-session".into(),
            "--secret=do-not-store".into(),
            "".into(),
            "two words\nnext line".into(),
            OsString::from_vec(vec![0xff, b'a']),
        ];
        let before = command.clone();
        let registry = RuntimeRegistry::first_party();
        assert_eq!(registry.claim(&command[0]), Some(id("claude")));
        let plan = RuntimeCommand::verbatim(&command).unwrap();
        assert_eq!(plan.executable, before[0]);
        assert_eq!(plan.args, before[1..]);
        assert_eq!(command, before);
        assert_eq!(
            registry.relaunch(&id("claude")).unwrap(),
            RuntimeCommand {
                executable: "claude".into(),
                args: vec![]
            }
        );
        assert_eq!(registry.claim(OsStr::new("/bin/sh")), None);
    }

    struct Community {
        id: HarnessId,
    }

    struct CommunityLifecycle;
    impl lifecycle::RuntimeLifecycle for CommunityLifecycle {
        fn encode_context(&self, text: &str) -> Option<String> {
            Some(format!("community:{text}"))
        }
    }

    #[test]
    fn lifecycle_registration_is_harness_owned_and_generic_exit_keeps_its_default() {
        use lifecycle::RuntimeLifecycle;
        use tmt_core::binding::session::{
            BindingSessionState, ObservedSessionKey, RuntimeState, SessionPreferences,
        };
        use tmt_core::endpoint::ProcessIncarnation;
        let mut registry = RuntimeRegistry::first_party();
        assert!(
            registry
                .register_lifecycle(&id("missing"), Box::new(CommunityLifecycle))
                .is_err()
        );
        registry
            .register(
                id("community"),
                "community",
                0,
                Community {
                    id: id("community"),
                },
            )
            .unwrap();
        assert!(registry.lifecycle(&id("community")).is_none());
        registry
            .register_lifecycle(&id("community"), Box::new(CommunityLifecycle))
            .unwrap();
        assert_eq!(
            registry
                .lifecycle(&id("community"))
                .unwrap()
                .encode_context("identity"),
            Some("community:identity".into())
        );
        assert!(
            registry
                .register_lifecycle(&id("community"), Box::new(CommunityLifecycle))
                .is_err()
        );
        let decoded = registry
            .lifecycle(&id("codex"))
            .unwrap()
            .decode(
                br#"{"hook_event_name":"SessionStart","source":"startup","session_id":"opaque"}"#,
            )
            .unwrap();
        assert_eq!(decoded.session().as_str(), "opaque");
        assert!(decoded.starting());
        let ended = lifecycle::NoLifecycle
            .client_exit(
                &BindingSessionState::default(),
                ObservedSessionKey {
                    incarnation: ProcessIncarnation::new(2, "child").unwrap(),
                    provider_session: None,
                },
                ProcessIncarnation::new(1, "owner").unwrap(),
                &SessionPreferences::default(),
            )
            .unwrap();
        assert_eq!(ended.state, RuntimeState::Ended);
    }

    struct CommunityChannel;
    impl channel::RuntimeChannel for CommunityChannel {
        fn preflight(
            &self,
            _: &OsStr,
            _: &std::path::Path,
            _: std::time::Instant,
        ) -> Result<Option<String>, channel::ChannelError> {
            Ok(None)
        }

        fn enrolled(&self, _: &std::path::Path, _: &str) -> Result<bool, channel::ChannelFault> {
            Ok(false)
        }

        fn enrolled_in_pane(
            &self,
            _: &std::path::Path,
            _: &channel::PaneAddress<'_>,
            _: Option<&str>,
            _: std::time::Instant,
        ) -> Result<channel::PaneEvidence, channel::EvidenceError> {
            Ok(channel::PaneEvidence::default())
        }

        fn enroll(
            &self,
            _: &channel::ChannelPlan<'_>,
        ) -> Result<Box<dyn channel::ChannelEnrollment>, channel::ChannelError> {
            Err(channel::ChannelError::Unsupported)
        }
    }

    struct Answering(Result<bool, channel::ChannelFault>);
    impl channel::RuntimeChannel for Answering {
        fn preflight(
            &self,
            _: &OsStr,
            _: &std::path::Path,
            _: std::time::Instant,
        ) -> Result<Option<String>, channel::ChannelError> {
            Ok(None)
        }

        fn enrolled(&self, _: &std::path::Path, _: &str) -> Result<bool, channel::ChannelFault> {
            self.0
        }

        fn enrolled_in_pane(
            &self,
            _: &std::path::Path,
            _: &channel::PaneAddress<'_>,
            _: Option<&str>,
            _: std::time::Instant,
        ) -> Result<channel::PaneEvidence, channel::EvidenceError> {
            Ok(channel::PaneEvidence::default())
        }

        fn enroll(
            &self,
            _: &channel::ChannelPlan<'_>,
        ) -> Result<Box<dyn channel::ChannelEnrollment>, channel::ChannelError> {
            Err(channel::ChannelError::Unsupported)
        }
    }

    struct InPane(Result<channel::PaneEvidence, channel::EvidenceError>);
    impl channel::RuntimeChannel for InPane {
        fn preflight(
            &self,
            _: &OsStr,
            _: &std::path::Path,
            _: std::time::Instant,
        ) -> Result<Option<String>, channel::ChannelError> {
            Ok(None)
        }

        fn enrolled(&self, _: &std::path::Path, _: &str) -> Result<bool, channel::ChannelFault> {
            Ok(false)
        }

        fn enrolled_in_pane(
            &self,
            _: &std::path::Path,
            _: &channel::PaneAddress<'_>,
            _: Option<&str>,
            _: std::time::Instant,
        ) -> Result<channel::PaneEvidence, channel::EvidenceError> {
            self.0.clone()
        }

        fn enroll(
            &self,
            _: &channel::ChannelPlan<'_>,
        ) -> Result<Box<dyn channel::ChannelEnrollment>, channel::ChannelError> {
            Err(channel::ChannelError::Unsupported)
        }
    }

    #[test]
    fn a_pane_is_enrolled_when_any_driver_says_so_and_unknown_when_one_cannot_tell() {
        use channel::PaneEvidence;
        let directory = std::path::Path::new("/channels");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let server = tmt_core::endpoint::ServerEvidence {
            host: tmt_core::host::HostKind::Tmux,
            server_id: "server".into(),
            socket_path: "/tmp/tmux-test".into(),
            server_pid: 1,
            server_start_time: "start".into(),
        };
        let pane = channel::PaneAddress {
            server: &server,
            pane_id: "%1",
            pane_pid: 2,
        };
        let ask = |answers: &[(&str, Result<PaneEvidence, channel::EvidenceError>)]| {
            let mut registry = RuntimeRegistry::default();
            for (name, answer) in answers {
                registry
                    .register(id(name), name, 0, Community { id: id(name) })
                    .unwrap();
                registry
                    .register_channel(&id(name), Box::new(InPane(answer.clone())))
                    .unwrap();
            }
            registry.enrolled_in_pane(directory, &pane, Some("binding"), deadline)
        };
        let none = || Ok(PaneEvidence::default());
        let live = || {
            Ok(PaneEvidence {
                enrolled: true,
                skipped: vec![],
            })
        };
        let skipping = |name: &str| {
            Ok(PaneEvidence {
                enrolled: false,
                skipped: vec![std::path::PathBuf::from(name)],
            })
        };
        let unreadable = channel::EvidenceError::at(
            channel::ChannelFault::InvalidRecord,
            std::path::Path::new("/channels/a.json"),
        );
        // No driver, or none with evidence: the baseline applies.
        assert_eq!(ask(&[]), Ok(PaneEvidence::default()));
        assert_eq!(
            ask(&[("a", none()), ("b", none())]),
            Ok(PaneEvidence::default())
        );
        // Any one driver that finds an enrollment decides, in any order.
        assert!(ask(&[("a", none()), ("b", live())]).unwrap().enrolled);
        assert!(ask(&[("b", none()), ("a", live())]).unwrap().enrolled);
        // Records no driver could attribute are merged so the caller can name them,
        // and never make the pane enrolled.
        assert_eq!(
            ask(&[
                ("a", skipping("/channels/x.json")),
                ("b", skipping("/channels/y.json"))
            ]),
            Ok(PaneEvidence {
                enrolled: false,
                skipped: vec!["/channels/x.json".into(), "/channels/y.json".into()],
            })
        );
        // A driver that cannot tell is terminal and keeps naming its file, unless an
        // earlier driver (priority, then name) already found the enrollment.
        assert_eq!(
            ask(&[("a", Err(unreadable.clone())), ("b", none())]),
            Err(unreadable.clone())
        );
        assert_eq!(
            ask(&[("a", none()), ("b", Err(unreadable.clone()))]),
            Err(unreadable.clone())
        );
        assert!(
            ask(&[("a", live()), ("b", Err(unreadable))])
                .unwrap()
                .enrolled
        );
    }

    fn registry_answering(
        answers: &[(&str, Result<bool, channel::ChannelFault>)],
    ) -> RuntimeRegistry {
        let mut registry = RuntimeRegistry::default();
        for (name, answer) in answers {
            registry
                .register(id(name), name, 0, Community { id: id(name) })
                .unwrap();
            registry
                .register_channel(&id(name), Box::new(Answering(*answer)))
                .unwrap();
        }
        registry
    }

    #[test]
    fn the_enrolled_driver_is_found_by_evidence_alone_and_never_guessed() {
        let directory = std::path::Path::new("/channels");
        let owner = |answers: &[(&str, Result<bool, channel::ChannelFault>)]| {
            registry_answering(answers).enrolled_harness(directory, "binding")
        };
        // Nothing enrolled, whatever drivers exist: the baseline applies.
        assert_eq!(
            RuntimeRegistry::default().enrolled_harness(directory, "b"),
            Ok(None)
        );
        assert_eq!(owner(&[("a", Ok(false)), ("b", Ok(false))]), Ok(None));
        // Exactly one driver holds the record: it owns the route, in any order.
        assert_eq!(
            owner(&[("a", Ok(false)), ("b", Ok(true))]),
            Ok(Some(id("b")))
        );
        assert_eq!(
            owner(&[("b", Ok(true)), ("a", Ok(false))]),
            Ok(Some(id("b")))
        );
        // Two drivers claiming one binding is ambiguous, never resolved by order.
        assert_eq!(
            owner(&[("a", Ok(true)), ("b", Ok(true))]),
            Err(channel::ChannelFault::Mismatch)
        );
        // A driver that cannot tell is terminal even when another says "no" or "yes".
        for others in [Ok(false), Ok(true)] {
            assert_eq!(
                owner(&[
                    ("a", Err(channel::ChannelFault::InvalidRecord)),
                    ("b", others)
                ]),
                Err(channel::ChannelFault::InvalidRecord),
                "{others:?}"
            );
            assert_eq!(
                owner(&[
                    ("b", others),
                    ("a", Err(channel::ChannelFault::Unverifiable))
                ]),
                Err(channel::ChannelFault::Unverifiable),
                "{others:?}"
            );
        }
    }

    #[test]
    fn channel_registration_is_harness_owned_and_never_given_twice() {
        let mut registry = RuntimeRegistry::first_party();
        // Both first-party drivers register their channel.
        assert!(registry.channel(&id("claude")).is_some());
        assert!(registry.channel(&id("codex")).is_some());
        assert!(
            registry
                .register_channel(&id("missing"), Box::new(CommunityChannel))
                .is_err()
        );
        registry
            .register(
                id("community"),
                "community",
                0,
                Community {
                    id: id("community"),
                },
            )
            .unwrap();
        assert!(registry.channel(&id("community")).is_none());
        registry
            .register_channel(&id("community"), Box::new(CommunityChannel))
            .unwrap();
        assert!(registry.channel(&id("community")).is_some());
        // A harness that already has a channel cannot be given a second one.
        for first_party in ["claude", "codex"] {
            assert!(
                registry
                    .register_channel(&id(first_party), Box::new(CommunityChannel))
                    .is_err(),
                "{first_party}"
            );
        }
        assert!(
            registry
                .register_channel(&id("community"), Box::new(CommunityChannel))
                .is_err()
        );
    }

    impl Driver for Community {
        type Target = BindingEntry;
        type Error = RuntimeError;
        type Launch = RuntimeCommand;
        fn claims(&self, command: &str) -> Option<HarnessId> {
            (command == "shared-command").then(|| self.id.clone())
        }
    }

    #[test]
    fn community_registration_uses_deterministic_priority_and_stable_ties() {
        for names in [["beta", "alpha"], ["alpha", "beta"]] {
            let mut registry = RuntimeRegistry::first_party();
            for name in names {
                registry
                    .register(id(name), "shared-command", 5, Community { id: id(name) })
                    .unwrap();
            }
            assert_eq!(
                registry.claim(OsStr::new("shared-command")),
                Some(id("alpha"))
            );
            registry
                .register(
                    id("preferred"),
                    "shared-command",
                    10,
                    Community {
                        id: id("preferred"),
                    },
                )
                .unwrap();
            assert_eq!(
                registry.claim(OsStr::new("shared-command")),
                Some(id("preferred"))
            );
            assert!(
                registry
                    .register(
                        id("alpha"),
                        "shared-command",
                        20,
                        Community { id: id("alpha") }
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn exact_resume_is_runtime_owned_and_preserves_mode_without_last_selection() {
        let mut registry = RuntimeRegistry::first_party();
        for (harness, mode, expected) in [
            (
                "claude",
                claude::MODE_DEFAULT,
                vec!["--resume", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
            ),
            (
                "codex",
                codex::MODE_SHARED,
                vec!["resume", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
            ),
            (
                "codex",
                codex::MODE_EMBEDDED,
                vec![
                    "resume",
                    "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                    "--no-daemon",
                ],
            ),
        ] {
            let session = RememberedSession {
                harness: id(harness),
                mode: RuntimeMode::new(mode).unwrap(),
                provider_session: ProviderSessionId::new("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
                    .unwrap(),
                state: None,
                stale_at_ms: None,
                resume_pending_at_ms: None,
            };
            assert_eq!(
                registry.resume(&session),
                ActionResult::Completed(RuntimeCommand {
                    executable: harness.into(),
                    args: expected.into_iter().map(OsString::from).collect()
                })
            );
        }
        let mut session = RememberedSession {
            harness: id("codex"),
            mode: RuntimeMode::new("remote").unwrap(),
            provider_session: ProviderSessionId::new("--last").unwrap(),
            state: None,
            stale_at_ms: None,
            resume_pending_at_ms: None,
        };
        assert_eq!(registry.resume(&session), ActionResult::Unsupported);
        session.mode = RuntimeMode::new("shared").unwrap();
        assert_eq!(
            registry.resume(&session),
            ActionResult::Failed(RuntimeError::InvalidSession)
        );
    }

    #[test]
    fn reconciliation_purges_unregistered_drivers_and_discards_unreadable_state() {
        use tmt_core::binding::session::DriverState;
        let registry = RuntimeRegistry::first_party();
        let remembered = |harness: &str, state: Option<DriverState>| SessionPreferences {
            preferred_harness: None,
            remembered: Some(RememberedSession {
                harness: id(harness),
                mode: RuntimeMode::new("default").unwrap(),
                provider_session: ProviderSessionId::new("kept").unwrap(),
                state,
                stale_at_ms: None,
                resume_pending_at_ms: None,
            }),
        };
        let mut clean = remembered("claude", None);
        assert_eq!(registry.reconcile(&mut clean), None);
        assert!(clean.remembered.is_some());

        // A version the driver does not read is dropped, never force-read.
        let mut versioned = remembered("claude", Some(DriverState::new(9, "{}").unwrap()));
        assert_eq!(
            registry.reconcile(&mut versioned),
            Some(StateReconciliation::DiscardedState {
                harness: id("claude"),
                version: 9
            })
        );
        let session = versioned.remembered.unwrap();
        assert_eq!(
            (session.state, session.provider_session.as_str()),
            (None, "kept")
        );

        let mut orphaned = remembered("removed-driver", None);
        assert_eq!(
            registry.reconcile(&mut orphaned),
            Some(StateReconciliation::UnregisteredDriver(id(
                "removed-driver"
            )))
        );
        assert_eq!(orphaned.remembered, None);
        assert_eq!(
            registry.harnesses().collect::<Vec<_>>(),
            ["claude", "codex"]
        );
    }

    #[test]
    fn documented_session_start_payloads_capture_the_model_and_resume_replays_it() {
        use lifecycle::RuntimeLifecycle;
        use tmt_core::binding::session::DriverState;
        let registry = RuntimeRegistry::first_party();
        for (harness, lifecycle, fixture, model) in [
            (
                "claude",
                &claude::ClaudeLifecycle as &dyn RuntimeLifecycle,
                include_str!("runtime/fixtures/claude-session-start.json"),
                "claude-opus-5",
            ),
            (
                "codex",
                &codex::CodexLifecycle,
                include_str!("runtime/fixtures/codex-session-start.json"),
                "gpt-5.2-codex",
            ),
        ] {
            assert!(lifecycle.reads_state(driver_state::MODEL_STATE_VERSION));
            assert!(lifecycle.reads_state(driver_state::USAGE_STATE_VERSION));
            const NOW: u64 = 1_780_000_000_000;
            let event = lifecycle.decode(fixture.as_bytes()).expect(harness);
            assert!(event.starting());
            let state = event
                .driver_state(None, NOW)
                .expect("reported model is kept");
            assert_eq!(driver_state::state_model(&state).as_deref(), Some(model));
            // Claude's documented resume start reports the context it re-sends;
            // Codex's start (source startup) reports none.
            assert_eq!(
                lifecycle.state_usage(&state),
                (harness == "claude")
                    .then(|| driver_state::Usage::new(182_340, None, NOW).unwrap())
            );

            // A start without a model (Claude omits it after /clear) keeps it.
            let unreported = fixture.replace(&format!(r#""model": "{model}","#), "");
            assert_ne!(unreported, fixture);
            let event = lifecycle.decode(unreported.as_bytes()).expect(harness);
            assert_eq!(event.driver_state(Some(&state), NOW), Some(state.clone()));
            if harness == "codex" {
                assert_eq!(event.driver_state(None, NOW), None, "nothing is guessed");
            }

            let mut preferences = SessionPreferences {
                preferred_harness: None,
                remembered: Some(RememberedSession {
                    harness: id(harness),
                    mode: RuntimeMode::new(if harness == "claude" {
                        "default"
                    } else {
                        "shared"
                    })
                    .unwrap(),
                    provider_session: ProviderSessionId::new(
                        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                    )
                    .unwrap(),
                    state: Some(state),
                    stale_at_ms: None,
                    resume_pending_at_ms: None,
                }),
            };
            assert_eq!(
                registry.reconcile(&mut preferences),
                None,
                "readable state is kept"
            );
            let id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
            // `claude [options]`; `codex resume [OPTIONS] [SESSION_ID]`.
            let (expected, bare): (Vec<&str>, Vec<&str>) = if harness == "claude" {
                (vec!["--resume", id, "--model", model], vec!["--resume", id])
            } else {
                (vec!["resume", "-m", model, id], vec!["resume", id])
            };
            let expected = expected.into_iter().map(OsString::from).collect::<Vec<_>>();
            let bare = bare.into_iter().map(OsString::from).collect::<Vec<_>>();
            let session = preferences.remembered.as_mut().unwrap();
            let mut registry = RuntimeRegistry::first_party();
            assert_eq!(
                registry.resume(session),
                ActionResult::Completed(RuntimeCommand {
                    executable: harness.into(),
                    args: expected.clone(),
                })
            );
            // Unreadable state resumes with the provider default, never a guess.
            session.state = Some(DriverState::new(1, r#"{"model":"--yolo"}"#).unwrap());
            assert_eq!(
                registry.resume(session),
                ActionResult::Completed(RuntimeCommand {
                    executable: harness.into(),
                    args: bare,
                })
            );
        }
    }
}
