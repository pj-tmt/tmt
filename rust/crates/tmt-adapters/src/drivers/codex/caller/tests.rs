use super::*;
use crate::process::{CommandError, CommandFailure, CommandOutput, CommandRequest};
use std::{cell::RefCell, collections::VecDeque};

const SESSION: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn direct_metadata_lookup_refuses_runtime_configuration_overrides() {
    for argv in [
        "codex -c sqlite_home='/different'",
        "codex --config=sqlite_home='/different'",
        "codex --profile other",
        "codex -p=other",
    ] {
        let probe = Probe::new([Ok("42 41 /bin/sh\n41 1 codex\n"), Ok(argv)]);
        let caller = CodexCaller::new(
            &probe,
            CallerEnvironment {
                thread_id: Some(SESSION.into()),
                process_id: 42,
            },
        );
        assert!(
            caller
                .observe_direct_host(Instant::now() + Duration::from_millis(500))
                .is_err()
        );
    }
}

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
        assert_eq!(request.program, "/usr/bin/env");
        assert_eq!(request.args[0], "LC_ALL=C");
        assert_eq!(request.args[1], "TZ=UTC");
        assert!(request.input.is_empty());
        assert!(request.max_output_bytes == 4 * 1024 * 1024 || request.max_output_bytes == 16384);
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
fn shared_host_is_fenced_by_snapshot_ancestry_not_the_marker() {
    for session in [Some(SESSION), None, Some("invalid")] {
        let probe = Probe::new([
            Ok("42 41 /usr/bin/zsh\n41 1 /opt/Codex App/bin/codex\n"),
            Ok("/opt/Codex App/bin/codex app-server --remote-control\n"),
        ]);
        let ActionResult::Completed(caller) = probe.identify(session) else {
            panic!("recognized host");
        };
        assert_eq!(caller.harness.as_str(), "codex");
        assert_eq!(
            caller.session.as_ref().map(ProviderSessionId::as_str),
            session.filter(|v| *v == SESSION)
        );
        assert!(!caller.permits_host_fallback());
        let calls = probe.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            &calls[0].0[3..],
            ["-A", "-o", "pid=,ppid=,comm="].map(OsString::from)
        );
        assert!(calls.iter().all(|(_, deadline)| *deadline == calls[0].1));
        assert!(probe.outputs.borrow().is_empty());
    }
}

#[test]
fn independent_invocation_preserves_host_verification_even_with_invalid_marker() {
    for session in [Some(SESSION), None, Some("invalid")] {
        let probe = Probe::new([
            Ok("42 41 /bin/zsh\n41 1 codex\n"),
            Ok("/opt/bin/codex --no-daemon\n"),
        ]);
        let ActionResult::Completed(caller) = probe.identify(session) else {
            panic!("independent host");
        };
        assert!(caller.permits_host_fallback());
        assert_eq!(
            caller.session.as_ref().map(ProviderSessionId::as_str),
            session.filter(|v| *v == SESSION)
        );
    }
}

#[test]
fn orphaned_markers_and_unrelated_codex_processes_do_not_disable_plain_shells() {
    for session in [None, Some(SESSION), Some("invalid")] {
        let probe = Probe::new([Ok("42 1 /bin/zsh\n99 1 codex\n")]);
        assert_eq!(probe.identify(session), ActionResult::Unsupported);
        assert_eq!(probe.calls.borrow().len(), 1);
    }
}

#[test]
fn failed_or_malformed_snapshots_only_fail_closed_when_a_marker_is_present() {
    for output in [
        Err(CommandFailure::Timeout),
        Err(CommandFailure::OutputLimit),
        Ok(""),
        Ok("invalid"),
        Ok("42 invalid /bin/zsh"),
        Ok("42 42 /bin/zsh"),
        Ok("42 1 /bin/zsh\n42 2 /bin/zsh"),
    ] {
        assert_eq!(
            Probe::new([output]).identify(Some(SESSION)),
            ActionResult::Failed(CallerObservationUnavailable)
        );
        assert_eq!(
            Probe::new([output]).identify(None),
            ActionResult::Unsupported
        );
    }
}

#[test]
fn many_ancestors_use_one_snapshot_and_shared_ancestors_still_override_independent_hits() {
    let probe = Probe::new([]);
    let snapshot = (42..80)
        .map(|pid| format!("{pid} {} /bin/zsh\n", pid + 1))
        .collect::<String>()
        + "80 1 codex\n";
    probe
        .outputs
        .borrow_mut()
        .extend([Ok(snapshot), Ok("codex --no-daemon".into())]);
    let ActionResult::Completed(caller) = probe.identify(None) else {
        panic!("independent ancestor");
    };
    assert!(caller.permits_host_fallback());
    assert_eq!(probe.calls.borrow().len(), 2);

    let nested = Probe::new([
        Ok("42 41 /bin/zsh\n41 40 codex\n40 1 codex\n"),
        Ok("codex --no-daemon"),
        Ok("codex -c setting=value app-server --remote-control"),
    ]);
    let ActionResult::Completed(caller) = nested.identify(None) else {
        panic!("shared outer host");
    };
    assert!(!caller.permits_host_fallback());
    assert_eq!(nested.calls.borrow().len(), 3);
}

#[test]
fn ancestry_depth_is_bounded_in_memory_without_more_ps_spawns() {
    let probe = Probe::new([]);
    probe.outputs.borrow_mut().push_back(Ok((42..107)
        .map(|pid| format!("{pid} {} /bin/zsh\n", pid + 1))
        .collect()));
    assert_eq!(
        probe.identify(Some(SESSION)),
        ActionResult::Failed(CallerObservationUnavailable)
    );
    assert_eq!(probe.calls.borrow().len(), 1);
}

#[test]
fn missing_ps_uses_shared_fixed_path_fallback_not_permission_or_timeout_retry() {
    let probe = Probe::new([
        Err(CommandFailure::Exit {
            code: Some(127),
            signal: None,
        }),
        Ok("42 1 /bin/zsh"),
    ]);
    assert_eq!(probe.identify(Some(SESSION)), ActionResult::Unsupported);
    let calls = probe.calls.borrow();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0[2], "/bin/ps");
    assert_eq!(calls[1].0[2], "/usr/bin/ps");
    assert_eq!(calls[0].1, calls[1].1);
    for failure in [
        CommandFailure::Timeout,
        CommandFailure::Exit {
            code: Some(126),
            signal: None,
        },
    ] {
        let probe = Probe::new([Err(failure)]);
        assert_eq!(
            probe.identify(Some(SESSION)),
            ActionResult::Failed(CallerObservationUnavailable)
        );
        assert_eq!(probe.calls.borrow().len(), 1);
    }
}
