use super::*;
use std::{cell::Cell, rc::Rc};
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

fn counting(withdrawn: &Rc<Cell<u32>>) -> Box<Counting> {
    Box::new(Counting {
        command: RuntimeCommand {
            executable: "agent".into(),
            args: vec!["--user".into(), "--planned".into()],
        },
        environment: vec![("TOKEN".into(), "private".into())],
        session: Some(ProviderSessionId::new("thread-1").unwrap()),
        withdrawn: Rc::clone(withdrawn),
    })
}

#[test]
fn a_held_lease_plans_the_spawn_and_without_one_the_users_command_runs_untouched() {
    let withdrawn = Rc::new(Cell::new(0));
    let mut lease = HeldLease::default();
    let user = user();
    assert_eq!(lease.command(&user), &user);
    assert!(lease.environment().is_empty());
    assert!(lease.provider_session().is_none());
    lease.hold(counting(&withdrawn));
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

/// Each path out of a launch that holds a lease, as `run_bound` can take them.
fn launch(withdrawn: &Rc<Cell<u32>>, path: &str) -> Result<u8, &'static str> {
    let mut lease = HeldLease::default();
    lease.hold(counting(withdrawn));
    let spawned: Result<(), &str> = match path {
        "spawn failure" => Err("spawn failed"),
        _ => Ok(()),
    };
    // An early return with `?` happens before any explicit withdrawal.
    spawned?;
    if path == "wait error" {
        // The launcher withdraws right after the wait, before reporting it.
        lease.withdraw();
        return Err("wait failed");
    }
    if path == "panic" {
        panic!("unexpected failure after the spawn");
    }
    if path == "normal exit" {
        lease.withdraw();
    }
    // "early return" falls through to the end of the scope without withdrawing.
    Ok(0)
}

#[test]
fn the_lease_is_withdrawn_exactly_once_on_every_path_out_of_a_launch() {
    for path in [
        "normal exit",
        "wait error",
        "spawn failure",
        "early return",
        "panic",
    ] {
        let withdrawn = Rc::new(Cell::new(0));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = launch(&withdrawn, path);
        }));
        assert_eq!(outcome.is_err(), path == "panic", "{path}");
        assert_eq!(withdrawn.get(), 1, "{path}");
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
