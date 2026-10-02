//! tmux behind the host-driver trait: the operations only tmux can do.
//! Which evidence makes a binding present, and what a send or focus may do,
//! is `host::driver`'s policy for every host.

use super::*;
use crate::{
    host::{ActionError, DeliveryError, HostError, driver::HostDriver},
    process::CommandError,
    tmux::FocusError,
};
use tmt_core::{
    binding::{BindingEndpoint, session::RuntimeState},
    driver::{ActionResult, DeliveryAcceptance, Focused, SendFailure},
};

impl<R: CommandRunner> HostDriver for BindingSession<'_, R> {
    fn begin_coordination(&mut self) {
        BindingEndpoint::begin_coordination(self);
    }

    fn budget_available(&self) -> bool {
        BindingEndpoint::budget_available(self)
    }

    fn snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, HostError> {
        Ok(self.current_snapshot(panes)?)
    }

    fn probe(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, HostError> {
        Ok(self.probe_binding(server, panes)?)
    }

    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), HostError> {
        Ok(BindingEndpoint::publish(self, binding, identity)?)
    }

    fn clear(&mut self, binding: &Binding) -> Result<bool, HostError> {
        Ok(BindingEndpoint::clear(self, binding)?)
    }

    /// Uses the current coordination deadline, without another pane query.
    fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, CommandError> {
        crate::process::runtime::binding_runtime(&self.tmux.runner, binding, self.deadline)
    }

    fn pane_incarnation(&mut self, pane_pid: u64) -> Result<Option<String>, HostError> {
        Ok(self.observe_pane_start(pane_pid)?)
    }

    fn has_input(&self) -> bool {
        true
    }

    /// tmux knows no agents: every message is raw pane input.
    fn prompt(
        &mut self,
        _: &Binding,
        _: &str,
    ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
        ActionResult::Unsupported
    }

    fn input(&mut self, binding: &Binding, message: &str) -> Result<(), DeliveryError> {
        self.tmux.send_on(
            &binding.server.socket_path,
            &binding.pane_id,
            message,
            self.enter_delay,
        )
    }

    /// Focus moves the user's own client, which must be on the binding's
    /// server.
    fn focus_preflight(&self, binding: Option<&Binding>) -> Result<(), ActionError> {
        let Some(binding) = binding else {
            return Err(ActionError::Unverified);
        };
        match &self.invoker {
            Some(invoker) if invoker.socket == binding.server.socket_path => Ok(()),
            _ => Err(ActionError::HostUnsupported),
        }
    }

    fn focus(&mut self, binding: &Binding) -> Result<Focused, ActionError> {
        let Some(invoker) = &self.invoker else {
            return Err(ActionError::HostUnsupported);
        };
        match self
            .tmux
            .focus_pane(invoker, &binding.pane_id, self.options(None))
        {
            Ok(before) => Ok(Focused {
                interface: binding.pane_id.clone(),
                previous: before.pane,
                viewer: before.client,
            }),
            Err(FocusError::HostUnsupported) => Err(ActionError::HostUnsupported),
            Err(FocusError::PaneNotFound) => Err(ActionError::Unverified),
            Err(FocusError::Evidence(error)) => Err(ActionError::Evidence(error.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        host::driver,
        scripted_runner::{ScriptedRunner, failure},
        tmux::{evidence, metadata},
    };
    use tmt_core::{
        binding::BindingEntry,
        driver::{
            ActionResult, DeliveryAcceptance, InterfacePresence, InterfaceStatus, SendFailure,
        },
        identity::Lifetime,
    };

    /// The shared binding policy over tmux, as the host session runs it.
    trait Actions {
        fn status(&mut self, entry: &BindingEntry) -> ActionResult<InterfaceStatus, ActionError>;
        fn send(
            &mut self,
            entry: &BindingEntry,
            message: &str,
        ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>>;
        fn focus(&mut self, entry: &BindingEntry) -> ActionResult<Focused, ActionError>;
    }

    impl<R: CommandRunner> Actions for BindingSession<'_, R> {
        fn status(&mut self, entry: &BindingEntry) -> ActionResult<InterfaceStatus, ActionError> {
            driver::status(self, entry)
        }
        fn send(
            &mut self,
            entry: &BindingEntry,
            message: &str,
        ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
            driver::send(self, entry, message)
        }
        fn focus(&mut self, entry: &BindingEntry) -> ActionResult<Focused, ActionError> {
            driver::focus(self, entry)
        }
    }

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
                pane_incarnation: None,
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
    fn a_pane_incarnation_is_one_ps_of_the_pane_shell_or_unknown() {
        let runner = ScriptedRunner::default();
        runner.push_output(b"Thu Oct  1 09:00:00 2026 S+\n".to_vec(), Vec::new());
        runner.push_output(b"garbage\n".to_vec(), Vec::new());
        let tmux = Tmux::new(runner);
        let mut driver = BindingSession::new(&tmux);
        BindingEndpoint::begin_coordination(&mut driver);
        assert_eq!(
            HostDriver::pane_incarnation(&mut driver, 654)
                .unwrap()
                .as_deref(),
            Some("ps-v1:Thu Oct 1 09:00:00 2026")
        );
        assert_eq!(
            HostDriver::pane_incarnation(&mut driver, 654).unwrap(),
            None
        );
        let calls = tmux.runner.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0].args[3..],
            ["-p", "654", "-o", "lstart=", "-o", "stat="].map(String::from)
        );
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
        use tmt_core::{binding::session::ObservedSessionKey, endpoint::ProcessIncarnation};
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
                incarnation: ProcessIncarnation::new(42, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap(),
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
        use tmt_core::{binding::session::ObservedSessionKey, endpoint::ProcessIncarnation};
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
                incarnation: ProcessIncarnation::new(42, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap(),
                provider_session: None,
            });
            session.launch_owner =
                Some(ProcessIncarnation::new(43, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap());
            // A same-process hook must not remove the wrapper's delivery fence.
            *session = session
                .admit(
                    session.key.clone().unwrap(),
                    tmt_core::binding::session::SessionTransition::Started,
                    tmt_core::binding::session::RuntimeLiveness::Alive,
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
        use tmt_core::{binding::session::ObservedSessionKey, endpoint::ProcessIncarnation};
        let mut entry = entry();
        let session = &mut entry.binding.as_mut().unwrap().session;
        session.state = RuntimeState::Running;
        session.key = Some(ObservedSessionKey {
            incarnation: ProcessIncarnation::new(42, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap(),
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
        let ActionResult::Completed(focused) = Actions::focus(&mut driver, &entry) else {
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
            Actions::focus(&mut BindingSession::new(&tmux), &entry),
            ActionResult::Failed(ActionError::HostUnsupported)
        ));
        let tmux = Tmux::new(ScriptedRunner::default());
        let mut driver = BindingSession::new(&tmux).with_invoker(invoker("/tmp/other.sock"));
        assert!(matches!(
            Actions::focus(&mut driver, &entry),
            ActionResult::Failed(ActionError::HostUnsupported)
        ));
        assert!(tmux.runner.calls.borrow().is_empty());

        // A replaced pane (different pid) fails evidence before any focus call.
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 999, true), Vec::new());
        let tmux = Tmux::new(runner);
        let mut driver = BindingSession::new(&tmux).with_invoker(invoker("/tmp/tmt-driver.sock"));
        assert!(matches!(
            Actions::focus(&mut driver, &entry),
            ActionResult::Failed(ActionError::Unverified)
        ));
        assert_eq!(
            tmux.runner.calls.borrow().len(),
            1,
            "only the evidence probe ran"
        );
    }

    /// tmux evidence and transport, behind a host that recognizes agents and
    /// answers every prompt with `answer`.
    struct AgentHost<'a, R> {
        tmux: BindingSession<'a, R>,
        answer: fn() -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>>,
        prompts: Vec<String>,
    }

    impl<R: CommandRunner> HostDriver for AgentHost<'_, R> {
        fn begin_coordination(&mut self) {
            HostDriver::begin_coordination(&mut self.tmux)
        }
        fn budget_available(&self) -> bool {
            HostDriver::budget_available(&self.tmux)
        }
        fn snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, HostError> {
            self.tmux.snapshot(panes)
        }
        fn probe(
            &mut self,
            server: &ServerEvidence,
            panes: &[String],
        ) -> Result<EndpointProbe, HostError> {
            self.tmux.probe(server, panes)
        }
        fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), HostError> {
            HostDriver::publish(&mut self.tmux, binding, identity)
        }
        fn clear(&mut self, binding: &Binding) -> Result<bool, HostError> {
            HostDriver::clear(&mut self.tmux, binding)
        }
        fn observed_runtime(&self, binding: &Binding) -> Result<RuntimeState, CommandError> {
            self.tmux.observed_runtime(binding)
        }
        fn pane_incarnation(&mut self, pane_pid: u64) -> Result<Option<String>, HostError> {
            HostDriver::pane_incarnation(&mut self.tmux, pane_pid)
        }
        fn has_input(&self) -> bool {
            true
        }
        fn prompt(
            &mut self,
            _: &Binding,
            message: &str,
        ) -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>> {
            self.prompts.push(message.into());
            (self.answer)()
        }
        fn input(&mut self, binding: &Binding, message: &str) -> Result<(), DeliveryError> {
            self.tmux.input(binding, message)
        }
        fn focus_preflight(&self, binding: Option<&Binding>) -> Result<(), ActionError> {
            self.tmux.focus_preflight(binding)
        }
        fn focus(&mut self, binding: &Binding) -> Result<Focused, ActionError> {
            HostDriver::focus(&mut self.tmux, binding)
        }
    }

    #[test]
    fn a_prompt_falls_back_to_raw_input_only_when_unsupported() {
        type Answer = fn() -> ActionResult<DeliveryAcceptance, SendFailure<ActionError>>;
        let cases: [(Answer, usize); 6] = [
            (|| ActionResult::Completed(DeliveryAcceptance::Submitted), 1),
            (
                || ActionResult::Failed(SendFailure::AwaitingApproval(ActionError::Unverified)),
                1,
            ),
            (
                || ActionResult::Failed(SendFailure::NotSent(ActionError::Unverified)),
                1,
            ),
            (
                || ActionResult::Failed(SendFailure::Uncertain(ActionError::Unverified)),
                1,
            ),
            (
                || ActionResult::Failed(SendFailure::Denied(ActionError::Unverified)),
                1,
            ),
            // No agent recognized: tmux's own paste, buffer and Enter.
            (|| ActionResult::Unsupported, 4),
        ];
        for (answer, calls) in cases {
            let entry = entry();
            let runner = ScriptedRunner::default();
            runner.push_output(observation(&entry, 654, true), Vec::new());
            for _ in 0..3 {
                runner.push_output(Vec::new(), Vec::new());
            }
            let tmux = Tmux::new(runner);
            let mut host = AgentHost {
                tmux: BindingSession::new(&tmux),
                answer,
                prompts: Vec::new(),
            };
            let sent = driver::send(&mut host, &entry, "hello!");
            assert_eq!(host.prompts, ["hello!"]);
            let expected = match answer() {
                ActionResult::Unsupported => ActionResult::Completed(DeliveryAcceptance::Submitted),
                answer => answer,
            };
            assert_eq!(format!("{sent:?}"), format!("{expected:?}"));
            assert_eq!(tmux.runner.calls.borrow().len(), calls, "{expected:?}");
        }
    }

    #[test]
    fn a_runtime_not_verified_running_is_never_prompted() {
        let mut entry = entry();
        entry.binding.as_mut().unwrap().session.state = RuntimeState::Ended;
        let runner = ScriptedRunner::default();
        runner.push_output(observation(&entry, 654, true), Vec::new());
        let tmux = Tmux::new(runner);
        let mut host = AgentHost {
            tmux: BindingSession::new(&tmux),
            answer: || ActionResult::Completed(DeliveryAcceptance::Submitted),
            prompts: Vec::new(),
        };
        let sent = driver::send(&mut host, &entry, "hello");
        assert!(host.prompts.is_empty(), "{sent:?}");
        assert!(matches!(
            sent,
            ActionResult::Failed(SendFailure::NotSent(_))
        ));
    }
}
