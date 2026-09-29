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

pub(crate) mod evidence;
pub mod hook_protocol;
pub mod lifecycle;
pub mod model_state;

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
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSession => "The remembered runtime session is not an exact UUID.",
            Self::InvalidRegistration => {
                "Runtime registration requires a unique harness ID and a bare command name."
            }
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
        if self
            .lifecycle(&harness)
            .and_then(|lifecycle| lifecycle.state_version())
            != Some(version)
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
    ActionResult::Completed(resume.state.and_then(model_state::state_model))
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
            BindingSessionState, ObservedSessionKey, RuntimeIncarnation, RuntimeState,
            SessionPreferences,
        };
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
                    incarnation: RuntimeIncarnation::new(2, "child").unwrap(),
                    provider_session: None,
                },
                RuntimeIncarnation::new(1, "owner").unwrap(),
                &SessionPreferences::default(),
            )
            .unwrap();
        assert_eq!(ended.state, RuntimeState::Ended);
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
            assert_eq!(
                lifecycle.state_version(),
                Some(model_state::MODEL_STATE_VERSION)
            );
            let event = lifecycle.decode(fixture.as_bytes()).expect(harness);
            assert!(event.starting());
            let state = event.driver_state(None).expect("reported model is kept");
            assert_eq!(model_state::state_model(&state).as_deref(), Some(model));

            // A start without a model (Claude omits it after /clear) keeps it.
            let unreported = fixture.replace(&format!(r#""model": "{model}","#), "");
            assert_ne!(unreported, fixture);
            let event = lifecycle.decode(unreported.as_bytes()).expect(harness);
            assert_eq!(event.driver_state(Some(&state)), Some(state.clone()));
            assert_eq!(event.driver_state(None), None, "nothing is guessed");

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
