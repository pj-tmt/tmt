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
    withdrawn: Rc<Cell<u32>>,
    started: Rc<RefCell<Vec<ProcessIncarnation>>>,
}

fn watch() -> Watch {
    Watch {
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
