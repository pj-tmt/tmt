use super::*;
use crate::process::{CommandError, CommandFailure, CommandOutput};
use std::{cell::RefCell, collections::VecDeque};

const SESSION: &str = "11111111-1111-4111-8111-111111111111";

struct Probe {
    outputs: RefCell<VecDeque<Result<String, CommandFailure>>>,
    calls: RefCell<Vec<(Vec<OsString>, Instant)>>,
}

impl Probe {
    fn new(outputs: impl IntoIterator<Item = Result<&'static str, CommandFailure>>) -> Self {
        Self {
            outputs: RefCell::new(outputs.into_iter().map(|v| v.map(str::to_owned)).collect()),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn identify(
        &self,
        session: Option<&str>,
    ) -> ActionResult<RuntimeCaller, CallerObservationUnavailable> {
        CodexCaller::new(
            self,
            CallerEnvironment {
                thread_id: session.map(OsString::from),
                process_id: 42,
            },
        )
        .identify_caller()
    }
}

impl CommandRunner for Probe {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert_eq!(request.program, "/bin/ps");
        assert!(request.input.is_empty());
        assert!(matches!(request.max_output_bytes, 4096 | 16384));
        self.calls
            .borrow_mut()
            .push((request.args.to_vec(), request.deadline));
        self.outputs
            .borrow_mut()
            .pop_front()
            .expect("unexpected probe")
            .map(|text| CommandOutput {
                stdout: text.into_bytes(),
                stderr: vec![],
            })
            .map_err(CommandError::new)
    }
}

#[test]
fn shared_host_fences_the_inherited_pane_with_or_without_session_marker() {
    for session in [Some(SESSION), None, Some("invalid")] {
        let probe = Probe::new([
            Ok("41 /usr/bin/zsh\n"),
            Ok("1 /opt/Codex App/bin/codex\n"),
            Ok("/opt/Codex App/bin/codex app-server --remote-control\n"),
        ]);
        let ActionResult::Completed(caller) = probe.identify(session) else {
            panic!("recognized Codex host");
        };
        assert_eq!(caller.harness.as_str(), "codex");
        assert_eq!(
            caller.session.as_ref().map(ProviderSessionId::as_str),
            session.filter(|v| *v == SESSION)
        );
        assert!(!caller.permits_host_fallback());
        let calls = probe.calls.borrow();
        assert_eq!(calls.len(), 3);
        assert!(calls.iter().all(|(_, deadline)| *deadline == calls[0].1));
        assert!(probe.outputs.borrow().is_empty());
    }
}

#[test]
fn independent_invocation_still_needs_host_verification() {
    let probe = Probe::new([
        Ok("41 /usr/bin/zsh\n"),
        Ok("1 codex\n"),
        Ok("/opt/bin/codex --no-daemon\n"),
    ]);
    let ActionResult::Completed(caller) = probe.identify(Some(SESSION)) else {
        panic!("session hint");
    };
    assert!(caller.permits_host_fallback());
    assert_eq!(caller.session.unwrap().as_str(), SESSION);
    assert_eq!(
        Probe::new([Ok("1 /bin/zsh\n")]).identify(None),
        ActionResult::Unsupported
    );
}

#[test]
fn failed_or_malformed_observations_do_not_invent_a_provider_or_fallback() {
    for output in [
        Err(CommandFailure::Timeout),
        Err(CommandFailure::OutputLimit),
        Ok(""),
        Ok("1 /bin/zsh\n2 /bin/zsh\n"),
        Ok("invalid /bin/zsh"),
        Ok("42 /bin/zsh"),
    ] {
        for session in [None, Some(SESSION)] {
            assert_eq!(
                Probe::new([output]).identify(session),
                ActionResult::Failed(CallerObservationUnavailable)
            );
        }
    }
    let ActionResult::Completed(caller) = Probe::new([Ok("1 /bin/zsh")]).identify(Some("invalid"))
    else {
        panic!("invalid marker is fenced");
    };
    assert!(caller.session.is_none());
    assert!(!caller.permits_host_fallback());
}

#[test]
fn an_orphaned_session_marker_does_not_authorize_an_inherited_host_pane() {
    let ActionResult::Completed(caller) = Probe::new([Ok("1 /bin/zsh")]).identify(Some(SESSION))
    else {
        panic!("retained session hint");
    };
    assert_eq!(caller.session.unwrap().as_str(), SESSION);
    assert_eq!(caller.host, HostAttribution::Ambiguous);

    // A live independent runtime can still use verified host attribution even
    // when the provider does not expose a conversation ID to its tool command.
    let probe = Probe::new([Ok("1 codex"), Ok("/opt/bin/codex --no-daemon")]);
    let ActionResult::Completed(caller) = probe.identify(None) else {
        panic!("observed independent runtime");
    };
    assert!(caller.permits_host_fallback());
    assert!(caller.session.is_none());
}

#[test]
fn ancestry_is_bounded_and_global_options_do_not_hide_shared_host() {
    let probe = Probe::new([]);
    probe
        .outputs
        .borrow_mut()
        .extend((43..107).map(|pid| Ok(format!("{pid} /bin/zsh"))));
    assert_eq!(
        probe.identify(None),
        ActionResult::Failed(CallerObservationUnavailable)
    );
    assert_eq!(probe.calls.borrow().len(), 64);
    let probe = Probe::new([
        Ok("1 /bin/codex"),
        Ok("/bin/codex -c setting=value app-server --remote-control"),
    ]);
    let ActionResult::Completed(caller) = probe.identify(None) else {
        panic!("shared host");
    };
    assert!(!caller.permits_host_fallback());
}
