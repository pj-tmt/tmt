//! Driver actions share the existing binding evidence and pane IO owners.

use super::*;
use crate::process::{CommandError, runtime::observe_runtime_process};
use crate::tmux::{DeliveryError, FocusError};
use tmt_core::{
    binding::{
        BindingEntry, BindingEvidence, evaluate_binding,
        session::{RuntimeLiveness, RuntimeState},
    },
    driver::{
        ActionResult, DeliveryAcceptance, Driver, Focused, InterfacePresence, InterfaceStatus,
        SendFailure,
    },
};

#[derive(Debug)]
pub enum ActionError {
    Evidence(TmuxError),
    Unverified,
    Offline,
    /// No client of the invoking user can be focused; nothing changed.
    HostUnsupported,
    Delivery(DeliveryError),
    Process(CommandError),
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Evidence(error) => error.fmt(f),
            Self::Unverified => f.write_str("Could not verify the identity binding."),
            Self::Offline => f.write_str("The agent runtime has ended; no pane input was sent."),
            Self::HostUnsupported => {
                f.write_str("No tmux client for this invocation can be focused.")
            }
            Self::Delivery(error) => error.fmt(f),
            Self::Process(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ActionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Evidence(error) => Some(error),
            Self::Delivery(error) => Some(error),
            Self::Process(error) => Some(error),
            Self::Unverified | Self::Offline | Self::HostUnsupported => None,
        }
    }
}

impl<R: CommandRunner> BindingSession<'_, R> {
    /// Observe runtime liveness after the caller has verified this binding's
    /// endpoint. This does not establish presence or grant routing authority.
    /// It uses the current coordination deadline, without another pane query.
    pub fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, ActionError> {
        let mut runtime = binding.session.state;
        if runtime == RuntimeState::Ended {
            return Ok(runtime);
        }
        if let Some(key) = &binding.session.key {
            let observation =
                observe_runtime_process(&self.tmux.runner, key.incarnation.pid(), self.deadline)
                    .map_err(ActionError::Process)?;
            runtime = match observation.matches(&key.incarnation) {
                RuntimeLiveness::Alive => runtime,
                RuntimeLiveness::Gone => RuntimeState::Ended,
                RuntimeLiveness::Unknown => RuntimeState::Unknown,
            };
            if runtime == RuntimeState::Running
                && let Some(owner) = &binding.session.launch_owner
            {
                let observation =
                    observe_runtime_process(&self.tmux.runner, owner.pid(), self.deadline)
                        .map_err(ActionError::Process)?;
                if observation.matches(owner) != RuntimeLiveness::Alive {
                    runtime = RuntimeState::Unknown;
                }
            }
        } else if runtime == RuntimeState::Running {
            runtime = RuntimeState::Unknown;
        }
        Ok(runtime)
    }
}

impl<R: CommandRunner> Driver for BindingSession<'_, R> {
    type Target = BindingEntry;
    type Error = ActionError;
    type Launch = ();

    fn status(&mut self, entry: &BindingEntry) -> ActionResult<InterfaceStatus, ActionError> {
        let Some(binding) = &entry.binding else {
            return ActionResult::Completed(InterfaceStatus {
                presence: InterfacePresence::Gone,
                runtime: RuntimeState::Unknown,
            });
        };
        self.begin_coordination();
        let probe =
            match self.probe_binding(&binding.server, std::slice::from_ref(&binding.pane_id)) {
                Ok(probe) => probe,
                Err(error) => return ActionResult::Failed(ActionError::Evidence(error)),
            };
        let presence = match evaluate_binding(entry, &probe) {
            BindingEvidence::Active(_) => InterfacePresence::Present,
            BindingEvidence::EndpointLost => InterfacePresence::Gone,
            BindingEvidence::MarkerMismatch | BindingEvidence::Unknown => {
                InterfacePresence::Unknown
            }
        };
        let runtime = if presence == InterfacePresence::Present {
            match self.observed_runtime(binding) {
                Ok(runtime) => runtime,
                Err(error) => return ActionResult::Failed(error),
            }
        } else {
            RuntimeState::Unknown
        };
        ActionResult::Completed(InterfaceStatus { presence, runtime })
    }

    fn send(
        &mut self,
        entry: &BindingEntry,
        message: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
        match self.status(entry) {
            ActionResult::Completed(InterfaceStatus {
                presence: InterfacePresence::Present,
                runtime: RuntimeState::Ended,
            }) => return ActionResult::Failed(SendFailure::NotSent(ActionError::Offline)),
            ActionResult::Completed(InterfaceStatus {
                presence: InterfacePresence::Present,
                runtime: RuntimeState::Unknown,
            }) if entry.binding.as_ref().is_some_and(|binding| {
                binding.session.key.is_some() || binding.session.state != RuntimeState::Unknown
            }) =>
            {
                // A failed verification of a recorded runtime is not permission
                // to downgrade to the legacy no-observation delivery path.
                return ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified));
            }
            ActionResult::Completed(InterfaceStatus {
                presence: InterfacePresence::Present,
                ..
            }) => {}
            ActionResult::Failed(error) => {
                return ActionResult::Failed(SendFailure::NotSent(error));
            }
            _ => return ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified)),
        }
        let Some(binding) = &entry.binding else {
            return ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified));
        };
        match self.tmux.send_on(
            &binding.server.socket_path,
            &binding.pane_id,
            message,
            self.enter_delay,
        ) {
            Ok(()) => ActionResult::Completed(DeliveryAcceptance::Submitted),
            Err(error) if error.uncertain() => {
                ActionResult::Failed(SendFailure::Uncertain(ActionError::Delivery(error)))
            }
            Err(error) => ActionResult::Failed(SendFailure::NotSent(ActionError::Delivery(error))),
        }
    }

    /// Requires present endpoint evidence (as before input) but no running
    /// agent: a pane whose agent ended is still where the member worked.
    fn focus(&mut self, entry: &BindingEntry) -> ActionResult<Focused, ActionError> {
        let (Some(binding), Some(invoker)) = (&entry.binding, self.invoker.clone()) else {
            return ActionResult::Failed(if entry.binding.is_none() {
                ActionError::Unverified
            } else {
                ActionError::HostUnsupported
            });
        };
        // The user's client must be on the binding's own server.
        if invoker.socket != binding.server.socket_path {
            return ActionResult::Failed(ActionError::HostUnsupported);
        }
        match self.status(entry) {
            ActionResult::Completed(InterfaceStatus {
                presence: InterfacePresence::Present,
                ..
            }) => {}
            ActionResult::Failed(error) => return ActionResult::Failed(error),
            _ => return ActionResult::Failed(ActionError::Unverified),
        }
        match self
            .tmux
            .focus_pane(&invoker, &binding.pane_id, self.options(None))
        {
            Ok(before) => ActionResult::Completed(Focused {
                interface: binding.pane_id.clone(),
                previous: before.pane,
                viewer: before.client,
            }),
            Err(FocusError::HostUnsupported) => ActionResult::Failed(ActionError::HostUnsupported),
            Err(FocusError::PaneNotFound) => ActionResult::Failed(ActionError::Unverified),
            Err(FocusError::Evidence(error)) => ActionResult::Failed(ActionError::Evidence(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::{
        evidence, metadata,
        test_support::{ScriptedRunner, failure},
    };
    use tmt_core::identity::Lifetime;

    fn entry() -> BindingEntry {
        BindingEntry {
            identity: Identity {
                id: "identity".into(),
                name: "Alice".into(),
                canonical_name: "alice".into(),
                lifetime: Lifetime::Saved,
                created_at: "created".into(),
                updated_at: "updated".into(),
            },
            binding: Some(Binding {
                id: "binding".into(),
                identity_id: "identity".into(),
                server: ServerEvidence {
                    host: tmt_core::host::HostKind::Tmux,
                    server_id: "123e4567-e89b-42d3-a456-426614174000".into(),
                    socket_path: "/tmp/tmt-driver.sock".into(),
                    server_pid: 321,
                    server_start_time: "1700000000".into(),
                },
                pane_id: "%9".into(),
                pane_pid: 654,
                session: Default::default(),
            }),
        }
    }

    fn observation(entry: &BindingEntry, pane_pid: u64, marked: bool) -> Vec<u8> {
        let binding = entry.binding.as_ref().unwrap();
        let server = &binding.server;
        let mut document = serde_json::json!({"version": 1});
        if marked {
            metadata::replace(&mut document, &binding.marker(&entry.identity));
        }
        [
            server.server_id.clone(),
            server.socket_path.clone(),
            server.server_pid.to_string(),
            server.server_start_time.clone(),
            binding.pane_id.clone(),
            "main:1.0".into(),
            "/repo".into(),
            "codex".into(),
            pane_pid.to_string(),
            "0".into(),
            document.to_string(),
        ]
        .join(evidence::SEPARATOR)
        .into_bytes()
    }

    #[test]
    fn verified_send_uses_existing_transport_once_and_reports_submission_only() {
        let entry = entry();
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 654, true), Vec::new());
        for _ in 0..3 {
            runner.push_output(Vec::new(), Vec::new());
        }
        let tmux = Tmux::new(runner);
        let mut driver = BindingSession::new(&tmux);
        assert!(matches!(
            driver.send(&entry, "hello!"),
            ActionResult::Completed(DeliveryAcceptance::Submitted)
        ));
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[1].args[2], "set-buffer");
        assert_eq!(calls[1].args.last().unwrap(), "hello！\n");
        assert_eq!(calls[2].args[2], "paste-buffer");
        assert_eq!(calls[3].args.last().unwrap(), "Enter");
    }

    #[test]
    fn failing_observations_do_not_veto_or_repeat_a_real_driver_submission() {
        use tmt_core::driver::{HookEvent, HookObserver, observe_driver_hook};
        struct Observer(Vec<String>);
        impl HookObserver for Observer {
            type Error = &'static str;
            fn observe(&mut self, event: &HookEvent<'_>) -> Result<(), Self::Error> {
                self.0.push(format!("{event:?}"));
                Err("extension unavailable")
            }
        }
        let entry = entry();
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 654, true), Vec::new());
        for _ in 0..3 {
            runner.push_output(Vec::new(), Vec::new());
        }
        let tmux = Tmux::new(runner);
        let mut observer = Observer(Vec::new());
        let before = observe_driver_hook(
            &mut observer,
            &HookEvent::BeforeMessage {
                request_id: "request",
            },
        );
        let result = BindingSession::new(&tmux).send(&entry, "exact message");
        let ActionResult::Completed(acceptance) = result else {
            panic!("submission must survive observation failure")
        };
        let after = observe_driver_hook(
            &mut observer,
            &HookEvent::MessageDelivered {
                request_id: "request",
                acceptance,
            },
        );
        assert_eq!(before, Some("extension unavailable"));
        assert_eq!(after, Some("extension unavailable"));
        assert_eq!(acceptance, DeliveryAcceptance::Submitted);
        assert_eq!(observer.0.len(), 2);
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[1].args.last().unwrap(), "exact message\n");
        assert_eq!(calls[2].args[2], "paste-buffer");
        assert_eq!(calls[3].args.last().unwrap(), "Enter");
    }

    #[test]
    fn missing_marker_and_reused_pane_never_receive_input() {
        for (pid, marked, expected) in [
            (654, false, InterfacePresence::Unknown),
            (655, true, InterfacePresence::Gone),
        ] {
            let mut entry = entry();
            entry.binding.as_mut().unwrap().session.state = RuntimeState::Running;
            let runner = ScriptedRunner::default();
            for _ in 0..2 {
                runner.push_output(observation(&entry, pid, marked), Vec::new());
            }
            let tmux = Tmux::new(runner);
            let mut driver = BindingSession::new(&tmux);
            assert!(
                matches!(driver.status(&entry), ActionResult::Completed(InterfaceStatus { presence, runtime: RuntimeState::Unknown }) if presence == expected)
            );
            assert!(matches!(
                driver.send(&entry, "must not paste"),
                ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified))
            ));
            assert_eq!(tmux.runner.calls.borrow().len(), 2);
        }
    }

    #[test]
    fn ended_runtime_never_receives_pane_input_or_falls_through() {
        let mut entry = entry();
        entry.binding.as_mut().unwrap().session.state = RuntimeState::Ended;
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 654, true), Vec::new());
        let tmux = Tmux::new(runner);
        let result = BindingSession::new(&tmux)
            .send(&entry, "must not reach the shell")
            .or_unsupported(|| panic!("offline is not an unsupported action"));
        assert!(matches!(
            result,
            ActionResult::Failed(SendFailure::NotSent(ActionError::Offline))
        ));
        assert_eq!(tmux.runner.calls.borrow().len(), 1);
    }

    #[test]
    fn recorded_runtime_is_checked_before_input_even_without_an_end_hook() {
        use tmt_core::binding::session::{ObservedSessionKey, RuntimeIncarnation};
        for (process, expected_offline) in [
            ("Sun Sep 27 10:00:01 2026 S\n", true),
            ("Sun Sep 27 10:00:00 2026 Z\n", true),
            ("Sun Sep 27 10:00:00 2026 T\n", false),
            ("Sun Sep 27 10:00:00 2026 T+\n", false),
            ("unavailable\n", false),
        ] {
            let mut entry = entry();
            let session = &mut entry.binding.as_mut().unwrap().session;
            session.state = RuntimeState::Running;
            session.key = Some(ObservedSessionKey {
                incarnation: RuntimeIncarnation::new(42, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap(),
                provider_session: None,
            });
            let runner = ScriptedRunner::default();
            runner.push_output(observation(&entry, 654, true), Vec::new());
            runner.push_output(process.as_bytes().to_vec(), Vec::new());
            let tmux = Tmux::new(runner);
            let result = BindingSession::new(&tmux).send(&entry, "must not reach shell");
            if expected_offline {
                assert!(matches!(
                    result,
                    ActionResult::Failed(SendFailure::NotSent(ActionError::Offline))
                ));
            } else {
                assert!(matches!(
                    result,
                    ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified))
                ));
            }
            let calls = tmux.runner.calls.borrow();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[1].program, "/usr/bin/env");
        }
    }

    #[test]
    fn launched_runtime_requires_its_owner_but_owner_loss_does_not_mean_child_exit() {
        use tmt_core::binding::session::{ObservedSessionKey, RuntimeIncarnation};
        for (child, owner, expected) in [
            ("S+", Some("S+"), RuntimeState::Running),
            ("S+", Some("Z"), RuntimeState::Unknown),
            ("S+", Some("T"), RuntimeState::Unknown),
            ("Z", None, RuntimeState::Ended),
        ] {
            let mut entry = entry();
            let session = &mut entry.binding.as_mut().unwrap().session;
            session.state = RuntimeState::Running;
            session.key = Some(ObservedSessionKey {
                incarnation: RuntimeIncarnation::new(42, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap(),
                provider_session: None,
            });
            session.launch_owner =
                Some(RuntimeIncarnation::new(43, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap());
            // A same-process hook must not remove the wrapper's delivery fence.
            *session = session
                .admit(
                    session.key.clone().unwrap(),
                    tmt_core::binding::session::SessionTransition::Started,
                    RuntimeLiveness::Alive,
                )
                .unwrap();
            *session = session
                .transition(
                    session.key.as_ref().unwrap(),
                    tmt_core::binding::session::SessionTransition::Compacted,
                    None,
                )
                .unwrap();
            let runner = ScriptedRunner::default();
            runner.push_output(observation(&entry, 654, true), Vec::new());
            runner.push_output(
                format!("Sun Sep 27 10:00:00 2026 {child}\n").into_bytes(),
                Vec::new(),
            );
            if let Some(owner) = owner {
                runner.push_output(
                    format!("Sun Sep 27 10:00:00 2026 {owner}\n").into_bytes(),
                    Vec::new(),
                );
            }
            if expected == RuntimeState::Running {
                for _ in 0..3 {
                    runner.push_output(Vec::new(), Vec::new());
                }
            }
            let tmux = Tmux::new(runner);
            let result = BindingSession::new(&tmux).send(&entry, "hello");
            match expected {
                RuntimeState::Running => assert!(matches!(
                    result,
                    ActionResult::Completed(DeliveryAcceptance::Submitted)
                )),
                RuntimeState::Unknown => assert!(matches!(
                    result,
                    ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified))
                )),
                RuntimeState::Ended => assert!(matches!(
                    result,
                    ActionResult::Failed(SendFailure::NotSent(ActionError::Offline))
                )),
            }
            let calls = tmux.runner.calls.borrow();
            assert_eq!(
                calls.len(),
                match expected {
                    RuntimeState::Running => 6,
                    RuntimeState::Unknown => 3,
                    RuntimeState::Ended => 2,
                }
            );
            if owner.is_some() {
                assert_eq!(calls[2].args[4], "43");
            }
        }
    }

    #[test]
    fn matching_live_runtime_allows_exactly_one_submission() {
        use tmt_core::binding::session::{ObservedSessionKey, RuntimeIncarnation};
        let mut entry = entry();
        let session = &mut entry.binding.as_mut().unwrap().session;
        session.state = RuntimeState::Running;
        session.key = Some(ObservedSessionKey {
            incarnation: RuntimeIncarnation::new(42, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap(),
            provider_session: None,
        });
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 654, true), Vec::new());
        runner.push_output(b"Sun Sep 27 10:00:00 2026 S+\n".to_vec(), Vec::new());
        for _ in 0..3 {
            runner.push_output(Vec::new(), Vec::new());
        }
        let tmux = Tmux::new(runner);
        assert!(matches!(
            BindingSession::new(&tmux).send(&entry, "hello"),
            ActionResult::Completed(DeliveryAcceptance::Submitted)
        ));
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 5);
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.args.last().is_some_and(|arg| arg == "Enter"))
                .count(),
            1
        );
    }

    #[test]
    fn paste_failure_is_uncertain_and_does_not_fall_through() {
        let entry = entry();
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 654, true), Vec::new());
        runner.push_output(Vec::new(), Vec::new());
        runner.results.borrow_mut().push_back(Err(failure(false)));
        runner.push_output(Vec::new(), Vec::new());
        let tmux = Tmux::new(runner);
        let result = BindingSession::new(&tmux)
            .send(&entry, "once")
            .or_unsupported(|| panic!("uncertain input must not be resent"));
        assert!(matches!(
            result,
            ActionResult::Failed(SendFailure::Uncertain(_))
        ));
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[3].args[2], "delete-buffer");
        assert!(
            calls
                .iter()
                .all(|call| !call.args.iter().any(|arg| arg == "Enter"))
        );
    }

    fn invoker(socket: &str) -> crate::tmux::Invoker {
        crate::tmux::Invoker {
            socket: socket.into(),
            pane: Some("%2".into()),
            session: None,
        }
    }

    #[test]
    fn focus_verifies_evidence_then_switches_only_the_invokers_client() {
        let entry = entry();
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 654, true), Vec::new());
        runner.push_output(b"%9\n".to_vec(), Vec::new());
        runner.push_output(b"$1\n".to_vec(), Vec::new());
        runner.push_output(
            format!(
                "client-1{s}$1{s}10{s}%2",
                s = crate::tmux::evidence::SEPARATOR
            )
            .into_bytes(),
            Vec::new(),
        );
        runner.push_output(Vec::new(), Vec::new());
        let tmux = Tmux::new(runner);
        let mut driver = BindingSession::new(&tmux).with_invoker(invoker("/tmp/tmt-driver.sock"));
        let ActionResult::Completed(focused) = driver.focus(&entry) else {
            panic!("focus");
        };
        assert_eq!(
            focused,
            Focused {
                interface: "%9".into(),
                previous: Some("%2".into()),
                viewer: "client-1".into()
            }
        );
        let calls = tmux.runner.calls.borrow();
        assert_eq!(
            calls.len(),
            5,
            "evidence, target, invoker session, clients, switch"
        );
        assert_eq!(
            calls[4].args[2..6],
            ["switch-client", "-c", "client-1", "-t"]
        );
        assert!(calls.iter().all(|call| {
            !call
                .args
                .iter()
                .any(|arg| ["send-keys", "paste-buffer", "set-buffer"].contains(&arg.as_str()))
        }));
    }

    #[test]
    fn focus_refuses_without_changing_anything_when_it_cannot_prove_the_target() {
        let entry = entry();
        // No invoker, or an invoker on another server: no tmux call at all.
        let tmux = Tmux::new(ScriptedRunner::default());
        assert!(matches!(
            BindingSession::new(&tmux).focus(&entry),
            ActionResult::Failed(ActionError::HostUnsupported)
        ));
        let tmux = Tmux::new(ScriptedRunner::default());
        let mut driver = BindingSession::new(&tmux).with_invoker(invoker("/tmp/other.sock"));
        assert!(matches!(
            driver.focus(&entry),
            ActionResult::Failed(ActionError::HostUnsupported)
        ));
        assert!(tmux.runner.calls.borrow().is_empty());

        // A replaced pane (different pid) fails evidence before any focus call.
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 999, true), Vec::new());
        let tmux = Tmux::new(runner);
        let mut driver = BindingSession::new(&tmux).with_invoker(invoker("/tmp/tmt-driver.sock"));
        assert!(matches!(
            driver.focus(&entry),
            ActionResult::Failed(ActionError::Unverified)
        ));
        assert_eq!(
            tmux.runner.calls.borrow().len(),
            1,
            "only the evidence probe ran"
        );
    }
}
