use super::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use tmt_core::{
    binding::session::{BindingSessionState, SessionTransition},
    endpoint::ServerEvidence,
    host::HostKind,
};

struct Counting {
    command: RuntimeCommand,
    environment: Vec<(OsString, OsString)>,
    session: Option<ProviderSessionId>,
    withdrawn: Rc<Cell<u32>>,
    started: Rc<RefCell<Vec<ProcessIncarnation>>>,
    refuse_foreground: bool,
    admitted: Rc<RefCell<Vec<ProcessIncarnation>>>,
    refuse_admission: bool,
}

impl ChannelEnrollment for Counting {
    fn command(&self) -> &RuntimeCommand {
        &self.command
    }

    fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    fn provider_session(&self) -> Option<&ProviderSessionId> {
        self.session.as_ref()
    }

    fn foreground_started(&mut self, foreground: &ProcessIncarnation) -> Result<(), ChannelError> {
        if self.refuse_foreground {
            return Err(ChannelError::Enrollment);
        }
        self.started.borrow_mut().push(foreground.clone());
        Ok(())
    }

    fn foreground_admitted(&mut self, foreground: &ProcessIncarnation) -> Result<(), ChannelError> {
        if self.refuse_admission {
            return Err(ChannelError::Enrollment);
        }
        self.admitted.borrow_mut().push(foreground.clone());
        Ok(())
    }

    fn withdraw(self: Box<Self>) {
        self.withdrawn.set(self.withdrawn.get() + 1);
    }
}

fn user() -> RuntimeCommand {
    RuntimeCommand {
        executable: "agent".into(),
        args: vec!["--user".into()],
    }
}

struct Watch {
    admitted: Rc<RefCell<Vec<ProcessIncarnation>>>,
    withdrawn: Rc<Cell<u32>>,
    started: Rc<RefCell<Vec<ProcessIncarnation>>>,
}

fn watch() -> Watch {
    Watch {
        admitted: Rc::new(RefCell::new(Vec::new())),
        withdrawn: Rc::new(Cell::new(0)),
        started: Rc::new(RefCell::new(Vec::new())),
    }
}

fn counting(watch: &Watch, refuse_foreground: bool) -> Box<Counting> {
    Box::new(Counting {
        command: RuntimeCommand {
            executable: "agent".into(),
            args: vec!["--user".into(), "--planned".into()],
        },
        environment: vec![("TOKEN".into(), "private".into())],
        session: Some(ProviderSessionId::new("thread-1").unwrap()),
        withdrawn: Rc::clone(&watch.withdrawn),
        started: Rc::clone(&watch.started),
        refuse_foreground,
        admitted: Rc::clone(&watch.admitted),
        refuse_admission: false,
    })
}

#[test]
fn a_held_lease_plans_the_spawn_and_without_one_the_users_command_runs_untouched() {
    let watch = watch();
    let mut lease = HeldLease::default();
    let user = user();
    assert_eq!(lease.command(&user), &user);
    assert!(lease.environment().is_empty());
    assert!(lease.provider_session().is_none());
    assert_eq!(lease.foreground_started(Some(&child())), None);
    lease.hold(counting(&watch, false));
    assert_eq!(lease.command(&user).args, ["--user", "--planned"]);
    assert_eq!(
        lease.environment(),
        [(OsString::from("TOKEN"), OsString::from("private"))]
    );
    assert_eq!(
        lease.provider_session().map(ProviderSessionId::as_str),
        Some("thread-1")
    );
}

/// Each path out of a launch that holds a lease, with the calls `run_bound` makes
/// at the same points: `foreground_started` right after the child is observed,
/// and a retiring call only for a failed spawn or a wait result passed through
/// `settle_wait`. The wait result's own settlement is tested on its own below.
fn launch(watch: &Watch, path: &str, refuse_foreground: bool) -> Result<u8, &'static str> {
    let mut lease = HeldLease::default();
    lease.hold(counting(watch, refuse_foreground));
    if path == "spawn failure" {
        lease.never_spawned();
        return Err("spawn failed");
    }
    let observed = child();
    let note = lease.foreground_started((path != "unobservable child").then_some(&observed));
    assert_eq!(
        note.is_some(),
        path == "unobservable child" || refuse_foreground,
        "{path}"
    );
    if path == "panic after spawn" {
        panic!("unexpected failure after the spawn");
    }
    if path == "early return after spawn" {
        return Err("returned early");
    }
    let waited = if path == "wait error" {
        Err("wait failed")
    } else {
        Ok(0)
    };
    lease.settle_wait(waited)
}

#[test]
fn only_a_wait_that_returned_retires_the_lease_and_the_result_passes_through() {
    for (waited, withdrawn) in [(Ok(7), 1), (Err("wait failed"), 0)] {
        let watch = watch();
        let mut lease = HeldLease::default();
        lease.hold(counting(&watch, false));
        assert_eq!(lease.settle_wait(waited), waited);
        assert_eq!(watch.withdrawn.get(), withdrawn, "{waited:?}");
    }
    // Without a lease there is nothing to retire and the result is still returned.
    assert_eq!(HeldLease::default().settle_wait::<_, ()>(Ok(3)), Ok(3));
}

#[test]
fn the_lease_is_retired_only_for_a_failed_spawn_or_a_reaped_child() {
    for (path, refuse, withdrawn, started) in [
        ("normal exit", false, 1, 1),
        ("spawn failure", false, 1, 0),
        // The child ran but its exact incarnation was not recorded; reaping it is
        // still a confirmed end.
        ("unobservable child", false, 1, 0),
        ("normal exit", true, 1, 0),
        // Nothing confirms the end of the foreground on these paths: the enrollment
        // is dropped, not retired.
        ("wait error", false, 0, 1),
        ("early return after spawn", false, 0, 1),
        ("panic after spawn", false, 0, 1),
    ] {
        let watch = watch();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = launch(&watch, path, refuse);
        }));
        assert_eq!(outcome.is_err(), path == "panic after spawn", "{path}");
        assert_eq!(watch.withdrawn.get(), withdrawn, "{path} refuse={refuse}");
        assert_eq!(
            watch.started.borrow().len(),
            started,
            "{path} refuse={refuse}"
        );
        if started == 1 {
            assert_eq!(watch.started.borrow()[0], child(), "{path}");
        }
    }
}

fn child() -> ProcessIncarnation {
    ProcessIncarnation::new(10, "child-start").unwrap()
}

fn key(incarnation: ProcessIncarnation, session: Option<&str>) -> ObservedSessionKey {
    ObservedSessionKey {
        incarnation,
        provider_session: session.map(|value| ProviderSessionId::new(value).unwrap()),
    }
}

#[test]
fn the_admitted_key_takes_a_lease_session_only_where_it_cannot_overwrite_evidence() {
    let lease = ProviderSessionId::new("thread-1").unwrap();
    let session = |key: Option<ObservedSessionKey>| key.and_then(|key| key.provider_session);
    // Nothing recorded for this child: the lease seeds the key.
    let fresh = admitted_key(None, &child(), None, Some(&lease)).unwrap();
    assert_eq!(fresh.incarnation, child());
    assert_eq!(session(Some(fresh)), Some(lease.clone()));
    // A hook that saw this child but no session yet: the lease fills it in.
    let attached = key(child(), None);
    assert_eq!(
        session(admitted_key(Some(&attached), &child(), None, Some(&lease))),
        Some(lease.clone())
    );
    // Matching hook evidence is preserved as it is.
    let matching = key(child(), Some("thread-1"));
    assert_eq!(
        admitted_key(Some(&matching), &child(), None, Some(&lease)),
        Some(matching)
    );
    // Conflicting hook evidence is never overwritten, and never admitted.
    let conflicting = key(child(), Some("thread-2"));
    assert_eq!(
        admitted_key(Some(&conflicting), &child(), None, Some(&lease)),
        None
    );
    // Evidence for another process is not this child's.
    let other = key(
        ProcessIncarnation::new(11, "other").unwrap(),
        Some("thread-2"),
    );
    let admitted = admitted_key(Some(&other), &child(), None, Some(&lease)).unwrap();
    assert_eq!(admitted.incarnation, child());
    assert_eq!(admitted.provider_session, Some(lease));
}

#[test]
fn without_a_lease_the_admitted_key_keeps_its_existing_behavior() {
    let resumed = ProviderSessionId::new("remembered").unwrap();
    // Resume coordinates seed a fresh key.
    let seeded = admitted_key(None, &child(), Some(resumed.clone()), None).unwrap();
    assert_eq!(seeded.provider_session, Some(resumed.clone()));
    // A hook's key for this child stands, whatever was remembered.
    let hooked = key(child(), Some("from-hook"));
    assert_eq!(
        admitted_key(Some(&hooked), &child(), Some(resumed), None),
        Some(hooked)
    );
    // And a launch with no session at all stays without one.
    assert_eq!(
        admitted_key(None, &child(), None, None)
            .unwrap()
            .provider_session,
        None
    );
}

fn binding(session: BindingSessionState) -> Binding {
    Binding {
        id: "binding".into(),
        identity_id: "identity".into(),
        server: ServerEvidence {
            host: HostKind::Tmux,
            server_id: "server".into(),
            socket_path: "/tmp/tmux-test".into(),
            server_pid: 1,
            server_start_time: "start".into(),
        },
        pane_id: "%1".into(),
        pane_pid: 2,
        pane_incarnation: None,
        session,
    }
}

#[test]
fn a_claim_is_current_only_for_the_same_session_or_its_own_unknown_fence() {
    let running = BindingSessionState {
        last_transition: Some(SessionTransition::Started),
        state: RuntimeState::Running,
        key: Some(key(child(), None)),
        launch_owner: Some(ProcessIncarnation::new(5, "owner").unwrap()),
    };
    let claimed = binding(running.clone());
    assert!(claim_is_current(&binding(running.clone()), &claimed));
    // The launch's own fence: the same key and owner, state unknown.
    let fenced = BindingSessionState {
        state: RuntimeState::Unknown,
        ..running.clone()
    };
    assert!(claim_is_current(&binding(fenced), &claimed));
    // Another launch took the binding: another owner, or another runtime.
    for changed in [
        BindingSessionState {
            launch_owner: Some(ProcessIncarnation::new(6, "other").unwrap()),
            ..running.clone()
        },
        BindingSessionState {
            state: RuntimeState::Unknown,
            key: Some(key(ProcessIncarnation::new(11, "other").unwrap(), None)),
            ..running.clone()
        },
        BindingSessionState {
            state: RuntimeState::Ended,
            ..running
        },
    ] {
        assert!(!claim_is_current(&binding(changed), &claimed));
    }
}

struct Advertised {
    default: Cell<bool>,
    probes: Cell<u32>,
    outcome: u8,
}
impl RuntimeChannel for Advertised {
    fn enabled_by_default(&self) -> bool {
        self.default.get()
    }
    fn preflight(
        &self,
        command: &tmt_adapters::runtime::RuntimeCommand,
        working_directory: Option<&std::path::Path>,
        _: &std::path::Path,
        _: Instant,
    ) -> Result<Option<String>, ChannelError> {
        assert_eq!(command.executable, "agent");
        assert_eq!(command.args, [std::ffi::OsString::from("--fixture")]);
        if let Some(cwd) = working_directory {
            assert_eq!(cwd, std::path::Path::new("/launch"));
        }
        self.probes.set(self.probes.get() + 1);
        match self.outcome {
            0 => Ok(None),
            1 => Err(ChannelError::ProviderUnqualified {
                reason: "unqualified fixture build".into(),
            }),
            2 => Err(ChannelError::ProviderVersion {
                found: "fixture".into(),
            }),
            4 => Ok(Some("informational fixture advisory".into())),
            _ => Err(ChannelError::ProviderUnavailable),
        }
    }
    fn enroll(
        &self,
        _: &tmt_adapters::runtime::channel::ChannelPlan<'_>,
    ) -> Result<Box<dyn ChannelEnrollment>, ChannelError> {
        panic!("preflight must not enroll")
    }
    fn enrolled(
        &self,
        _: &std::path::Path,
        _: &str,
    ) -> Result<bool, tmt_adapters::runtime::channel::ChannelFault> {
        Ok(false)
    }
    fn enrolled_in_pane(
        &self,
        _: &std::path::Path,
        _: &tmt_adapters::runtime::channel::PaneAddress<'_>,
        _: Option<&str>,
        _: Instant,
    ) -> Result<
        tmt_adapters::runtime::channel::PaneEvidence,
        tmt_adapters::runtime::channel::EvidenceError,
    > {
        Ok(Default::default())
    }
}

#[test]
fn default_policy_follows_only_the_advertised_driver_default() {
    let port = Advertised {
        default: Cell::new(false),
        probes: Cell::new(0),
        outcome: 0,
    };
    let prepare = |mode| {
        super::prepare(
            Some(&port),
            mode,
            &RuntimeCommand {
                executable: "agent".into(),
                args: vec!["--fixture".into()],
            },
            Some(std::path::Path::new("/launch")),
            PathBuf::from("/fixture"),
        )
    };
    assert!(prepare(ChannelMode::Default).unwrap().channel.is_none());
    assert_eq!(port.probes.get(), 0);
    port.default.set(true);
    assert!(prepare(ChannelMode::Default).unwrap().channel.is_some());
    assert_eq!(port.probes.get(), 1);
    assert!(prepare(ChannelMode::Disabled).unwrap().channel.is_none());
    assert_eq!(port.probes.get(), 1);
    port.default.set(false);
    assert!(prepare(ChannelMode::Required).unwrap().channel.is_some());
    assert_eq!(port.probes.get(), 2);
}

#[test]
fn unavailable_and_advisory_preflight_fall_back_only_under_default() {
    for (outcome, code) in [
        (1, "CHANNEL_PROVIDER_UNSUPPORTED"),
        (2, "CHANNEL_PROVIDER_UNSUPPORTED"),
        (3, "CHANNEL_UNAVAILABLE"),
    ] {
        let port = Advertised {
            default: Cell::new(true),
            probes: Cell::new(0),
            outcome,
        };
        let prepared = prepare(
            Some(&port),
            ChannelMode::Default,
            &RuntimeCommand {
                executable: "agent".into(),
                args: vec!["--fixture".into()],
            },
            Some(std::path::Path::new("/launch")),
            "/fixture".into(),
        )
        .unwrap();
        assert!(prepared.channel.is_none());
        assert!(prepared.notice.is_some());
        let error = prepare(
            Some(&port),
            ChannelMode::Required,
            &RuntimeCommand {
                executable: "agent".into(),
                args: vec!["--fixture".into()],
            },
            Some(std::path::Path::new("/launch")),
            "/fixture".into(),
        )
        .err()
        .unwrap();
        assert_eq!(error.code, code);
    }
    let error = prepare(
        None,
        ChannelMode::Required,
        &RuntimeCommand {
            executable: "agent".into(),
            args: vec!["--fixture".into()],
        },
        Some(std::path::Path::new("/launch")),
        "/fixture".into(),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "CHANNEL_UNSUPPORTED");
    let absent = prepare(
        None,
        ChannelMode::Default,
        &RuntimeCommand {
            executable: "agent".into(),
            args: vec!["--fixture".into()],
        },
        Some(std::path::Path::new("/launch")),
        "/fixture".into(),
    )
    .unwrap();
    assert!(absent.channel.is_none() && absent.notice.is_none());
    assert_eq!(
        paste_notice("worker\nname", "build\nunknown\rhere"),
        "worker name uses paste delivery: build unknown here"
    );
}

#[test]
fn unavailable_cwd_does_not_change_channel_selection() {
    let port = Advertised {
        default: Cell::new(true),
        probes: Cell::new(0),
        outcome: 0,
    };
    let command = RuntimeCommand {
        executable: "agent".into(),
        args: vec!["--fixture".into()],
    };
    for mode in [ChannelMode::Default, ChannelMode::Required] {
        let prepared = prepare(Some(&port), mode, &command, None, "/fixture".into()).unwrap();
        assert!(prepared.channel.is_some());
        assert!(prepared.notice.is_none());
    }
    assert_eq!(port.probes.get(), 2);
}

#[test]
fn informational_advisory_does_not_gate_an_available_driver() {
    let port = Advertised {
        default: Cell::new(true),
        probes: Cell::new(0),
        outcome: 4,
    };
    for mode in [ChannelMode::Default, ChannelMode::Required] {
        let prepared = prepare(
            Some(&port),
            mode,
            &RuntimeCommand {
                executable: "agent".into(),
                args: vec!["--fixture".into()],
            },
            Some(std::path::Path::new("/launch")),
            "/fixture".into(),
        )
        .unwrap();
        assert!(prepared.channel.is_some());
        assert!(prepared.notice.is_none());
    }
}

#[test]
fn unsupported_enrollment_keeps_the_driver_reason_and_stable_code() {
    for reason in [
        "This command has no channel support.",
        "Claude channel enrollment on resume is not supported yet; resume without --channel",
    ] {
        let error = ChannelError::Unsupported(reason);
        assert_eq!(error.to_string(), reason);
        let failure = enrollment_failure(error);
        assert_eq!(failure.code, "CHANNEL_UNSUPPORTED");
        assert_eq!(failure.message, reason);
    }
}

#[test]
fn admitted_callback_passes_original_child_and_failure_keeps_enrollment() {
    assert!(HeldLease::default().foreground_admitted(&child()).is_none());
    let watch = watch();
    let mut enrollment = counting(&watch, false);
    enrollment.refuse_admission = true;
    let mut lease = HeldLease::default();
    lease.hold(enrollment);
    assert!(
        lease
            .foreground_admitted(&child())
            .unwrap()
            .contains("stays unready")
    );
    assert_eq!(watch.withdrawn.get(), 0);
    assert_eq!(lease.command(&user()).args, ["--user", "--planned"]);
    assert!(lease.provider_session().is_some());
    drop(lease);
    assert_eq!(watch.withdrawn.get(), 0);

    let mut lease = HeldLease::default();
    lease.hold(counting(&watch, false));
    assert!(lease.foreground_admitted(&child()).is_none());
    assert_eq!(*watch.admitted.borrow(), [child()]);
}

#[test]
fn exact_resume_reuses_preference_unless_a_flag_overrides_it() {
    for (remembered, expected) in [
        (None, ChannelMode::Default),
        (Some(true), ChannelMode::Required),
        (Some(false), ChannelMode::Disabled),
    ] {
        assert_eq!(
            resume_mode(ChannelMode::Default, true, remembered),
            expected
        );
        assert_eq!(
            resume_mode(ChannelMode::Default, false, remembered),
            ChannelMode::Default
        );
        for explicit in [ChannelMode::Required, ChannelMode::Disabled] {
            assert_eq!(resume_mode(explicit, true, remembered), explicit);
        }
    }
}

#[test]
fn explicit_channel_requires_the_experimental_setting() {
    let error = require_enabled(ChannelMode::Required, false).unwrap_err();
    assert_eq!(error.code, "CHANNEL_DISABLED");
    assert_eq!(
        error.message,
        "Message channels require experimental.channel=true."
    );
    assert_eq!(error.status, 1);
    for mode in [
        ChannelMode::Default,
        ChannelMode::Disabled,
        ChannelMode::Required,
    ] {
        assert!(require_enabled(mode, true).is_ok());
    }
    for mode in [ChannelMode::Default, ChannelMode::Disabled] {
        assert!(require_enabled(mode, false).is_ok());
    }
}
