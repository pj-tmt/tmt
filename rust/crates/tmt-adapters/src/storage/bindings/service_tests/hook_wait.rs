use super::{
    super::test_support::Fixture,
    endpoint::{EndpointFailure, FakeEndpoint},
};
use crate::storage::{Storage, StorageError, StorageErrorCode};
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::{self, Binding, BindingEndpoint, BindingRepository, session::*},
    endpoint::{EndpointProbe, EndpointSnapshot, ProcessIncarnation, ServerEvidence},
    identity::{Identity, IdentityReader, Lifetime},
};

#[derive(Clone, Copy, Debug)]
enum Mutation {
    Bind,
    Unbind,
    Remove,
    NameAutomatic,
    FailedAutomatic,
}
impl Mutation {
    fn set_up(self, storage: &mut Storage, endpoint: &mut FakeEndpoint) -> Option<Binding> {
        match self {
            Self::Bind => None,
            Self::NameAutomatic | Self::FailedAutomatic => {
                binding::bind_auto_identity(storage, endpoint, "%1", "Target", false)
                    .unwrap()
                    .presence
                    .binding
            }
            Self::Unbind | Self::Remove => {
                binding::bind_identity(
                    storage,
                    endpoint,
                    "%1",
                    "Target",
                    matches!(self, Self::Remove),
                )
                .unwrap()
                .binding
            }
        }
    }
    fn run(
        self,
        storage: &mut Storage,
        endpoint: &mut impl BindingEndpoint<Error = EndpointFailure>,
        binding: Option<&Binding>,
    ) {
        match self {
            Self::Bind => {
                binding::bind_identity(storage, endpoint, "%1", "Target", false).unwrap();
            }
            Self::Unbind => {
                assert!(
                    binding::unbind_identity(storage, endpoint, "%1")
                        .unwrap()
                        .unwrap()
                        .retired
                );
            }
            Self::Remove => {
                binding::remove_identity(storage, endpoint, "Target", true).unwrap();
            }
            Self::NameAutomatic => {
                binding::name_auto_identity(storage, endpoint, "%1", "Named target", true)
                    .unwrap()
                    .unwrap();
            }
            Self::FailedAutomatic => {
                binding::retire_failed_auto_launch(storage, endpoint, binding.unwrap()).unwrap();
            }
        }
    }
    fn verify(self, storage: &mut Storage, endpoint: &mut FakeEndpoint) {
        let presence = binding::pane_presence(storage, endpoint, "%1").unwrap();
        match self {
            Self::Bind => assert_eq!(presence.identity.unwrap().name, "Target"),
            Self::NameAutomatic => {
                let identity = presence.identity.unwrap();
                assert_eq!(identity.name, "Named target");
                assert_eq!(identity.lifetime, Lifetime::Saved);
                assert!(
                    !storage
                        .with_binding_transaction(|r| r.is_auto_named(&identity.id))
                        .unwrap()
                );
            }
            _ => {
                assert!(presence.identity.is_none());
                assert!(storage.find_identity("target").unwrap().is_none());
            }
        }
    }
}

// Bind's initial snapshot is outside SQL, so gate its publish instead. Every
// other selected call is reached only inside the existing mutation transaction.
struct HeldHost<F> {
    inner: FakeEndpoint,
    mutation: Mutation,
    gate: Option<F>,
}
impl<F: FnOnce()> HeldHost<F> {
    fn hold(&mut self) {
        if let Some(gate) = self.gate.take() {
            gate();
        }
    }
}
impl<F: FnOnce()> BindingEndpoint for HeldHost<F> {
    type Error = EndpointFailure;
    fn begin_coordination(&mut self) {
        self.inner.begin_coordination();
    }
    fn budget_available(&self) -> bool {
        self.inner.budget_available()
    }
    fn current_host(&self) -> tmt_core::host::HostKind {
        self.inner.current_host()
    }
    fn current_snapshot(&mut self, panes: &[String]) -> Result<EndpointSnapshot, Self::Error> {
        if matches!(self.mutation, Mutation::NameAutomatic) {
            self.hold();
        }
        self.inner.current_snapshot(panes)
    }
    fn probe_binding(
        &mut self,
        server: &ServerEvidence,
        panes: &[String],
    ) -> Result<EndpointProbe, Self::Error> {
        self.inner.probe_binding(server, panes)
    }
    fn publish(&mut self, binding: &Binding, identity: &Identity) -> Result<(), Self::Error> {
        if matches!(self.mutation, Mutation::Bind) {
            self.hold();
        }
        self.inner.publish(binding, identity)
    }
    fn clear(&mut self, binding: &Binding) -> Result<bool, Self::Error> {
        self.hold();
        self.inner.clear(binding)
    }
    fn pane_incarnation(
        &mut self,
        server: &ServerEvidence,
        pid: u64,
    ) -> Result<Option<String>, Self::Error> {
        self.inner.pane_incarnation(server, pid)
    }
}

fn launched_hook(storage: &mut Storage, endpoint: &mut FakeEndpoint, pane: &str) -> Binding {
    let mut bound = binding::bind_identity(storage, endpoint, pane, "Hook agent", true)
        .unwrap()
        .binding
        .unwrap();
    let next = bound
        .session
        .admit_launched(
            ObservedSessionKey {
                incarnation: ProcessIncarnation::new(200, "runtime-start").unwrap(),
                provider_session: None,
            },
            ProcessIncarnation::new(201, "owner-start").unwrap(),
            SessionTransition::Started,
            RuntimeLiveness::Alive,
            RuntimeLiveness::Alive,
        )
        .unwrap();
    storage
        .with_binding_transaction(|records| {
            records.set_session_state(&bound.id, &bound.session, &next)
        })
        .unwrap();
    bound.session = next;
    bound
}

fn resumed_hook(storage: &mut Storage, bound: &Binding) -> Result<bool, StorageError> {
    let event = crate::drivers::claude::decode_hook(
        br#"{"hook_event_name":"SessionStart","source":"resume","session_id":"waited-resume","model":"model-a"}"#,
    )
    .unwrap();
    let next = event
        .propose(
            &bound.session,
            &bound.session.key.as_ref().unwrap().incarnation,
            RuntimeLiveness::Alive,
        )
        .unwrap();
    storage.with_binding_transaction(|records| {
        // Same full binding fence as lifecycle admission, after lock acquisition.
        if records
            .entry_by_id(&bound.identity_id)?
            .and_then(|e| e.binding)
            .as_ref()
            != Some(bound)
        {
            return Ok(false);
        }
        assert!(records.set_session_state(&bound.id, &bound.session, &next)?);
        let mut preferences = records.session_preferences(&bound.identity_id)?;
        preferences.remember(
            HarnessId::new("claude").unwrap(),
            RuntimeMode::new("default").unwrap(),
            event.session.clone(),
        );
        records.set_session_preferences(&bound.identity_id, &preferences)
    })
}

#[test]
fn binding_publication_allows_a_waiting_hook() {
    mutation_admits_hook(Mutation::Bind);
}
#[test]
fn unbind_allows_a_waiting_hook() {
    mutation_admits_hook(Mutation::Unbind);
}
#[test]
fn removal_allows_a_waiting_hook() {
    mutation_admits_hook(Mutation::Remove);
}
#[test]
fn automatic_naming_allows_a_waiting_hook() {
    mutation_admits_hook(Mutation::NameAutomatic);
}
#[test]
fn failed_automatic_launch_cleanup_allows_a_waiting_hook() {
    mutation_admits_hook(Mutation::FailedAutomatic);
}

fn mutation_admits_hook(mutation: Mutation) {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%9"]);
    // Clearing %1 intentionally removes its ownership. A different live
    // binding is the valid positive control for hook writer admission.
    let hook_binding = launched_hook(&mut storage, &mut endpoint, "%9");
    let target = mutation.set_up(&mut storage, &mut endpoint);
    endpoint = thread::scope(|scope| {
        let (start_tx, start_rx) = mpsc::sync_channel(1);
        let (busy_tx, busy_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let database = &fixture.database;
        let hook_binding = &hook_binding;
        let worker = scope.spawn(move || {
            start_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            let mut hook =
                Storage::open_hook(database, Instant::now() + Duration::from_secs(2)).unwrap();
            let connection = hook.connection().unwrap();
            let wait_ms = connection
                .query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))
                .unwrap();
            // Prove this actual hook connection encountered the held writer,
            // then restore the production-configured bound for admission.
            connection.busy_timeout(Duration::ZERO).unwrap();
            let error = connection.execute_batch("BEGIN IMMEDIATE").unwrap_err();
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
            connection
                .busy_timeout(Duration::from_millis(wait_ms.try_into().unwrap()))
                .unwrap();
            busy_tx.send(()).unwrap();
            let result = resumed_hook(&mut hook, hook_binding);
            hook.close().unwrap();
            result_tx.send(result).unwrap();
        });
        let mut held = HeldHost {
            inner: endpoint,
            mutation,
            gate: Some(|| {
                start_tx.send(()).unwrap();
                busy_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                // Wait for a bounded result event, not a sleep: the old 50 ms
                // admission returns Busy here. The new writer must remain
                // pending until this host call returns and SQL commits.
                assert!(
                    matches!(
                        result_rx.recv_timeout(Duration::from_millis(100)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ),
                    "{mutation:?}: hook did not wait for host effect"
                );
            }),
        };
        mutation.run(&mut storage, &mut held, target.as_ref());
        assert!(
            held.gate.is_none(),
            "mutation did not reach its gated host effect"
        );
        assert!(
            result_rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap()
        );
        worker.join().unwrap();
        held.inner
    });
    mutation.verify(&mut storage, &mut endpoint);
    storage
        .with_binding_transaction::<_, StorageError>(|r| {
            let current = r
                .entry_by_id(&hook_binding.identity_id)?
                .unwrap()
                .binding
                .unwrap();
            assert_eq!(current.id, hook_binding.id);
            assert_eq!(
                current.session.last_transition,
                Some(SessionTransition::Resumed)
            );
            assert_eq!(
                current
                    .session
                    .key
                    .unwrap()
                    .provider_session
                    .unwrap()
                    .as_str(),
                "waited-resume"
            );
            assert_eq!(
                r.session_preferences(&hook_binding.identity_id)?
                    .remembered
                    .unwrap()
                    .provider_session
                    .as_str(),
                "waited-resume"
            );
            Ok(())
        })
        .unwrap();
    storage.close().unwrap();
}

#[test]
fn a_hook_waiting_for_unbind_cannot_restore_the_detached_binding() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%9"]);
    let old = launched_hook(&mut storage, &mut endpoint, "%9");
    endpoint = thread::scope(|scope| {
        let (start_tx, start_rx) = mpsc::sync_channel(1);
        let (busy_tx, busy_rx) = mpsc::sync_channel(1);
        let database = &fixture.database;
        let old = &old;
        let worker = scope.spawn(move || {
            start_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            let mut hook =
                Storage::open_hook(database, Instant::now() + Duration::from_secs(2)).unwrap();
            let c = hook.connection().unwrap();
            let wait_ms = c
                .query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))
                .unwrap();
            c.busy_timeout(Duration::ZERO).unwrap();
            assert_eq!(
                c.execute_batch("BEGIN IMMEDIATE")
                    .unwrap_err()
                    .sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
            c.busy_timeout(Duration::from_millis(wait_ms.try_into().unwrap()))
                .unwrap();
            busy_tx.send(()).unwrap();
            let result = resumed_hook(&mut hook, old);
            hook.close().unwrap();
            result
        });
        let mut held = HeldHost {
            inner: endpoint,
            mutation: Mutation::Unbind,
            gate: Some(|| {
                start_tx.send(()).unwrap();
                busy_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            }),
        };
        assert!(
            !binding::unbind_identity(&mut storage, &mut held, "%9")
                .unwrap()
                .unwrap()
                .retired
        );
        assert!(!worker.join().unwrap().unwrap());
        held.inner
    });
    let replacement =
        binding::bind_identity(&mut storage, &mut endpoint, "%9", "Replacement", true).unwrap();
    assert_ne!(replacement.binding.as_ref().unwrap().id, old.id);
    assert_eq!(
        replacement.binding.unwrap().session,
        BindingSessionState::default()
    );
    assert_eq!(
        binding::pane_presence(&mut storage, &mut endpoint, "%9")
            .unwrap()
            .identity
            .unwrap()
            .id,
        replacement.identity.id
    );
    assert!(
        storage
            .with_binding_transaction(|r| r.entry_by_id(&old.identity_id))
            .unwrap()
            .unwrap()
            .binding
            .is_none()
    );
    assert!(
        storage
            .session_preferences(&old.identity_id)
            .unwrap()
            .remembered
            .is_none()
    );
    storage.close().unwrap();
}

#[test]
fn exhausted_hook_budget_refuses_storage_without_mutation() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%9"]);
    let bound = launched_hook(&mut storage, &mut endpoint, "%9");
    let before = storage.session_preferences(&bound.identity_id).unwrap();
    let error = Storage::open_hook(&fixture.database, Instant::now())
        .err()
        .unwrap();
    assert_eq!(error.code, StorageErrorCode::Busy);
    assert_eq!(
        storage
            .with_binding_transaction(|r| r.entry_by_id(&bound.identity_id))
            .unwrap()
            .unwrap()
            .binding,
        Some(bound.clone())
    );
    assert_eq!(
        storage.session_preferences(&bound.identity_id).unwrap(),
        before
    );
    storage
        .with_binding_transaction::<_, StorageError>(|_| Ok(()))
        .unwrap();
    storage.close().unwrap();
}
