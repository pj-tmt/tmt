use rusqlite::Connection;
use tmt_core::{
    binding::{
        BindingError, BindingRecords, BindingRepository, BindingTargetEvidence, bind_identity,
        bind_identity_at, bind_identity_with_creation,
    },
    identity::{
        Identity, IdentityReader, IdentityRepository, IdentityWriter, Lifetime, create_or_resolve,
    },
};

use super::super::test_support::{Fixture, pane};
use super::endpoint::{EndpointFailure, FakeEndpoint};
use crate::storage::{Storage, StorageError};

#[test]
fn only_explicit_user_bind_can_replace_an_unchanged_legacy_marker() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2", "%3", "%4"]);
    let legacy = r#"{"version":1,"globalIdentity":{"name":"Old","canonicalName":"old"}}"#;
    for pane in ["%1", "%2", "%3", "%4"] {
        endpoint.legacy_markers.insert(pane.into(), legacy.into());
    }
    let bound = bind_identity(&mut storage, &mut endpoint, "%1", "Explicit", false).unwrap();
    assert_eq!(
        endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
        bound.binding.unwrap().id
    );
    assert!(!endpoint.legacy_markers.contains_key("%1"));
    let automatic =
        tmt_core::binding::bind_auto_identity(&mut storage, &mut endpoint, "%2", "auto", false);
    assert!(matches!(automatic, Err(BindingError::Unverified)));
    let runtime = tmt_core::binding::bind_runtime_identity_with_creation(
        &mut storage,
        &mut endpoint,
        "%3",
        "runtime",
        false,
    );
    assert!(matches!(runtime, Err(BindingError::Unverified)));
    let hook_identity = create_or_resolve(&mut storage, "Hook", Lifetime::Saved)
        .unwrap()
        .identity;
    let hook = tmt_core::binding::bind_existing_identity(
        &mut storage,
        &mut endpoint,
        "%4",
        &hook_identity.id,
        false,
    );
    assert!(matches!(hook, Err(BindingError::Unverified)));
    assert_eq!(
        endpoint.legacy_markers.get("%4").map(String::as_str),
        Some(legacy)
    );
    assert_eq!(
        endpoint.legacy_markers.get("%2").map(String::as_str),
        Some(legacy)
    );
    assert_eq!(
        endpoint.legacy_markers.get("%3").map(String::as_str),
        Some(legacy)
    );
    assert!(endpoint.pane_mut("%2").marker.is_none());
    assert!(endpoint.pane_mut("%3").marker.is_none());
    assert!(storage.find_identity("auto").unwrap().is_none());
    assert!(storage.find_identity("runtime").unwrap().is_none());
    storage.close().unwrap();
}

#[test]
fn changed_legacy_marker_refuses_explicit_bind_without_overwriting_it() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    endpoint
        .legacy_markers
        .insert("%1".into(), "old bytes".into());
    endpoint.replace_legacy_before_publish = Some("changed bytes".into());
    let result = bind_identity(&mut storage, &mut endpoint, "%1", "Explicit", false);
    assert!(matches!(result, Err(BindingError::Unverified)));
    assert_eq!(
        endpoint.legacy_markers.get("%1").map(String::as_str),
        Some("changed bytes")
    );
    assert!(endpoint.pane_mut("%1").marker.is_none());
    assert_eq!(endpoint.publish_calls, 0);
    storage.close().unwrap();
}

#[test]
fn competing_binds_recheck_free_observation_before_reserving() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut losing = FakeEndpoint::new(&["%1"]);
    let database = fixture.database.clone();
    let winner_publications = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&winner_publications);
    // Called after the loser observed both the free host pane and the empty
    // reservation table, but before it takes the reservation transaction.
    losing.after_owner = Some(Box::new(move || {
        let mut other = Storage::open(&database).unwrap();
        let mut winner = FakeEndpoint::new(&["%1"]);
        bind_identity(&mut other, &mut winner, "%1", "Winner", false).unwrap();
        recorded.store(winner.publish_calls, Ordering::SeqCst);
        other.close().unwrap();
    }));
    let result = bind_identity(&mut storage, &mut losing, "%1", "Loser", false);
    assert!(
        matches!(result, Err(BindingError::PaneAlreadyBound)),
        "{result:?}"
    );
    assert_eq!(winner_publications.load(Ordering::SeqCst), 1);
    assert_eq!(losing.publish_calls, 0);
    storage.close().unwrap();
}

#[test]
fn refused_creation_keeps_an_identity_with_a_concurrent_reservation() {
    use tmt_core::{binding::BindingReservation, endpoint::ProcessIncarnation};
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut losing = FakeEndpoint::new(&["%1", "%2"]);
    let database = fixture.database.clone();
    losing.after_owner = Some(Box::new(move || {
        let mut other = Storage::open(&database).unwrap();
        let mut winner = FakeEndpoint::new(&["%1", "%2"]);
        let winner_binding = bind_identity(&mut other, &mut winner, "%1", "Winner", false)
            .unwrap()
            .binding
            .unwrap();
        let identity = other.find_identity("loser").unwrap().unwrap();
        let mut target = winner_binding;
        target.id = tmt_core::operation::new_operation_id();
        target.identity_id = identity.id;
        target.pane_id = "%2".into();
        other
            .with_binding_transaction::<_, StorageError>(|records| {
                records.begin_binding_reservation(&BindingReservation {
                    target,
                    old_binding_id: None,
                    expected_binding_id: None,
                    publisher: ProcessIncarnation::new(42, "other-publisher").unwrap(),
                    created_at_ms: 1_000,
                    expires_at_ms: 11_000,
                })
            })
            .unwrap();
        other.close().unwrap();
    }));
    assert!(matches!(
        bind_identity(&mut storage, &mut losing, "%1", "Loser", false),
        Err(BindingError::PaneAlreadyBound)
    ));
    let loser = storage
        .find_identity("loser")
        .unwrap()
        .expect("concurrent reservation keeps identity");
    assert!(
        storage
            .with_binding_transaction::<_, StorageError>(
                |records| records.reservation_by_identity(&loser.id)
            )
            .unwrap()
            .is_some()
    );
    assert_eq!(losing.publish_calls, 0);
    storage.close().unwrap();
}

#[test]
fn late_expired_publisher_refuses_successor_then_third_bind_reclaims() {
    use tmt_core::{binding::BindingReservation, endpoint::ProcessIncarnation};
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let old = bind_identity(&mut storage, &mut endpoint, "%1", "Old", true).unwrap();
    let old_binding = old.binding.unwrap();
    let old_marker = old_binding.marker(&old.identity);
    let expired = BindingReservation {
        target: old_binding.clone(),
        old_binding_id: None,
        expected_binding_id: None,
        publisher: ProcessIncarnation::new(99, "expired-publisher").unwrap(),
        created_at_ms: 100,
        expires_at_ms: 900,
    };
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.detach_binding(&old_binding.id)?;
            records.begin_binding_reservation(&expired)
        })
        .unwrap();
    endpoint.pane_mut("%1").marker = None;
    // The old publisher wins the empty-marker CAS after its lease expired,
    // just before the successor performs its own host CAS.
    endpoint.late_marker_before_publish = Some(old_marker.clone());
    let refused = bind_identity(&mut storage, &mut endpoint, "%1", "Successor", false);
    assert!(matches!(refused, Err(BindingError::Unverified)));
    assert_eq!(endpoint.pane_mut("%1").marker.as_ref(), Some(&old_marker));
    assert!(
        !storage
            .with_binding_transaction::<_, StorageError>(
                |records| records.binding_exists(&old_binding.id)
            )
            .unwrap()
    );
    let third = bind_identity(&mut storage, &mut endpoint, "%1", "Third", false).unwrap();
    assert_eq!(
        endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
        third.binding.unwrap().id
    );
    storage.close().unwrap();
}

#[test]
fn proven_dead_publisher_can_be_reclaimed_before_lease_expiry() {
    use tmt_core::{
        binding::{BindingReservation, session::RuntimeLiveness},
        endpoint::ProcessIncarnation,
    };
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let old = bind_identity(&mut storage, &mut endpoint, "%1", "Old", true).unwrap();
    let old_binding = old.binding.unwrap();
    let pending = BindingReservation {
        target: old_binding.clone(),
        old_binding_id: None,
        expected_binding_id: None,
        publisher: ProcessIncarnation::new(99, "dead").unwrap(),
        created_at_ms: 100,
        expires_at_ms: 11_000,
    };
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.detach_binding(&old_binding.id)?;
            records.begin_binding_reservation(&pending)
        })
        .unwrap();
    endpoint.publisher_liveness = RuntimeLiveness::Gone;
    let successor = bind_identity(&mut storage, &mut endpoint, "%1", "New", false).unwrap();
    assert_ne!(successor.binding.as_ref().unwrap().id, old_binding.id);
    assert_eq!(
        endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
        successor.binding.unwrap().id
    );
    assert!(storage.find_identity("old").unwrap().is_some());
    storage.close().unwrap();
}

#[test]
fn unpublished_reservation_leaves_the_committed_identity_offline() {
    use tmt_core::{
        binding::{BindingReservation, Presence, session::RuntimeLiveness},
        endpoint::ProcessIncarnation,
    };
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let old = bind_identity(&mut storage, &mut endpoint, "%1", "Old", false).unwrap();
    let binding = old.binding.unwrap();
    let pending = BindingReservation {
        target: binding.clone(),
        old_binding_id: None,
        expected_binding_id: None,
        publisher: ProcessIncarnation::new(99, "dead").unwrap(),
        created_at_ms: 100,
        expires_at_ms: 11_000,
    };
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.detach_binding(&binding.id)?;
            records.begin_binding_reservation(&pending)
        })
        .unwrap();
    endpoint.pane_mut("%1").marker = None;
    endpoint.publisher_liveness = RuntimeLiveness::Gone;
    let presence = tmt_core::binding::name_presence(&mut storage, &mut endpoint, "Old").unwrap();
    assert_eq!(presence.presence, Presence::Offline);
    assert!(presence.binding.is_none());
    assert_eq!(
        storage.find_identity("old").unwrap().unwrap().id,
        old.identity.id
    );
    storage.close().unwrap();
}

#[test]
fn one_live_reservation_per_identity_blocks_a_second_target_without_publication() {
    use tmt_core::{binding::BindingReservation, endpoint::ProcessIncarnation};
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2", "%3"]);
    let old = bind_identity(&mut storage, &mut endpoint, "%1", "Old", true).unwrap();
    let old_binding = old.binding.unwrap();
    let mut target = old_binding.clone();
    target.id = tmt_core::operation::new_operation_id();
    target.pane_id = "%2".into();
    target.session = Default::default();
    let reservation = BindingReservation {
        target,
        old_binding_id: Some(old_binding.id.clone()),
        expected_binding_id: None,
        publisher: ProcessIncarnation::new(99, "live").unwrap(),
        created_at_ms: 100,
        expires_at_ms: 11_000,
    };
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            records.begin_binding_reservation(&reservation)
        })
        .unwrap();
    let calls = endpoint.publish_calls;
    let result = bind_identity(&mut storage, &mut endpoint, "%3", "Old", true);
    assert!(matches!(result, Err(BindingError::Unverified)));
    assert_eq!(endpoint.publish_calls, calls);
    assert_eq!(
        endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
        old_binding.id
    );
    storage.close().unwrap();
}

#[test]
fn detached_publisher_clears_only_its_own_marker_and_reports_unverified() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let database = fixture.database.clone();
    endpoint.after_publish = Some(Box::new(move || {
        let mut other = Storage::open(&database).unwrap();
        let identity = other.find_identity("late").unwrap().unwrap();
        other
            .with_binding_transaction::<_, StorageError>(|records| {
                let entry = records.entry_by_id(&identity.id)?.unwrap();
                assert!(entry.binding.is_none());
                let reservation = records.reservation_by_identity(&identity.id)?.unwrap();
                assert!(records.finish_binding_reservation(&reservation)?);
                Ok(())
            })
            .unwrap();
        other.close().unwrap();
    }));
    let result = bind_identity(&mut storage, &mut endpoint, "%1", "Late", false);
    assert!(matches!(result, Err(BindingError::Unverified)));
    assert!(endpoint.pane_mut("%1").marker.is_none());
    assert_eq!(endpoint.clear_calls, 1);
    assert!(storage.find_identity("late").unwrap().is_some());
    storage.close().unwrap();
}

#[test]
fn temporary_binding_save_reuses_uuid_and_never_downgrades() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);

    let first = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false).unwrap();
    let second = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let third = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false).unwrap();

    assert_eq!(first.identity.id, second.identity.id);
    assert_eq!(second.identity, third.identity);
    assert_eq!(second.identity.lifetime, Lifetime::Saved);
    assert_eq!(
        first.binding.as_ref().unwrap().id,
        second.binding.as_ref().unwrap().id
    );
    assert_eq!(endpoint.publish_calls, 1);
    storage.close().unwrap();
}

#[test]
fn same_interface_rebind_retains_runtime_observation_and_preferences() {
    use tmt_core::binding::session::{
        BindingSessionState, HarnessId, ProviderSessionId, RememberedSession, RuntimeMode,
        RuntimeState, SessionPreferences, SessionTransition,
    };
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let first = bind_identity(&mut storage, &mut endpoint, "%1", "Session", true).unwrap();
    let state = BindingSessionState {
        plain_program: None,
        start_refusal: None,
        launch_owner: None,
        state: RuntimeState::Running,
        last_transition: Some(SessionTransition::Resumed),
        key: Some(tmt_core::binding::session::ObservedSessionKey {
            incarnation: tmt_core::endpoint::ProcessIncarnation::new(101, "runtime-start").unwrap(),
            provider_session: None,
        }),
    };
    let preferences = SessionPreferences {
        retained: Vec::new(),
        channel: None,
        preferred_harness: Some(HarnessId::new("codex").unwrap()),
        remembered: Some(RememberedSession {
            harness: HarnessId::new("codex").unwrap(),
            mode: RuntimeMode::new("shared").unwrap(),
            provider_session: ProviderSessionId::new("remembered-session").unwrap(),
            state: None,
            stale_at_ms: None,
            resume_pending_at_ms: None,
        }),
    };
    storage
        .with_binding_transaction(|records| {
            records.set_session_state(
                &first.binding.as_ref().unwrap().id,
                &BindingSessionState::default(),
                &state,
            )?;
            records.set_session_preferences(&first.identity.id, &preferences)?;
            Ok::<_, crate::storage::StorageError>(())
        })
        .unwrap();
    let rebound = bind_identity(&mut storage, &mut endpoint, "%1", "Session", true).unwrap();
    assert_eq!(
        rebound.binding.as_ref().unwrap().id,
        first.binding.as_ref().unwrap().id
    );
    assert_eq!(rebound.binding.unwrap().session, state);
    assert_eq!(endpoint.publish_calls, 1);
    storage
        .with_binding_transaction(|records| {
            assert_eq!(
                records.session_preferences(&first.identity.id)?,
                preferences
            );
            Ok::<_, crate::storage::StorageError>(())
        })
        .unwrap();
    storage.close().unwrap();
}

#[test]
fn occupied_pane_retires_the_identity_it_created_and_keeps_the_occupant() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let alice = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true)
        .unwrap()
        .identity;

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Bob", false).unwrap_err();
    assert!(matches!(error, BindingError::PaneAlreadyBound));
    // The refusal is deterministic: nothing may keep a never-bound temporary row.
    assert!(storage.find_identity("bob").unwrap().is_none());
    let alice_entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&alice.id))
        .unwrap()
        .unwrap();
    assert!(alice_entry.binding.is_some());

    // The name is free again: a later bind in a free pane gets a fresh identity.
    let retry =
        bind_identity_with_creation(&mut storage, &mut endpoint, "%2", "Bob", false).unwrap();
    assert!(retry.created);
    assert_eq!(retry.presence.identity.lifetime, Lifetime::Temporary);
    assert!(retry.presence.binding.is_some());
    storage.close().unwrap();
}

#[test]
fn refusal_keeps_identities_it_did_not_create_or_that_are_saved() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let existing = create_or_resolve(&mut storage, "Carol", Lifetime::Temporary)
        .unwrap()
        .identity;

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Carol", false).unwrap_err();
    assert!(matches!(error, BindingError::PaneAlreadyBound));
    assert_eq!(
        storage.find_identity("carol").unwrap().unwrap().id,
        existing.id
    );

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Dave", true).unwrap_err();
    assert!(matches!(error, BindingError::PaneAlreadyBound));
    assert_eq!(
        storage.find_identity("dave").unwrap().unwrap().lifetime,
        Lifetime::Saved
    );

    // A name that is active elsewhere is refused without touching that identity.
    let error = bind_identity(&mut storage, &mut endpoint, "%2", "Alice", true).unwrap_err();
    assert!(matches!(error, BindingError::NameAlreadyActive));
    let alice = storage.find_identity("alice").unwrap().unwrap();
    assert!(
        storage
            .with_binding_transaction(|records| records.entry_by_id(&alice.id))
            .unwrap()
            .unwrap()
            .binding
            .is_some()
    );
    storage.close().unwrap();
}

type Race = Box<dyn FnOnce(&mut Storage)>;

/// Runs `race` once in the window between a refused bind and its compensating
/// retirement, triggered by the refused transaction rather than a transaction count.
struct RacingStorage {
    inner: Storage,
    refused: bool,
    race: Option<Race>,
}

impl IdentityReader for RacingStorage {
    type Error = StorageError;

    fn find_identity(&self, canonical_name: &str) -> Result<Option<Identity>, Self::Error> {
        self.inner.find_identity(canonical_name)
    }

    fn list_identities(&self) -> Result<Vec<Identity>, Self::Error> {
        self.inner.list_identities()
    }
}

impl IdentityRepository for RacingStorage {
    fn with_identity_transaction<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn IdentityWriter<Error = Self::Error>) -> Result<T, Self::Error>,
    ) -> Result<T, Self::Error> {
        self.inner.with_identity_transaction(operation)
    }
}

impl BindingRepository for RacingStorage {
    fn with_binding_transaction<T, E: From<Self::Error>>(
        &mut self,
        operation: impl FnOnce(&mut dyn BindingRecords<Error = Self::Error>) -> Result<T, E>,
    ) -> Result<T, E> {
        if self.refused
            && let Some(race) = self.race.take()
        {
            race(&mut self.inner);
        }
        let result = self.inner.with_binding_transaction(operation);
        self.refused |= result.is_err();
        result
    }
}

#[test]
fn identity_bound_by_another_invocation_before_cleanup_is_not_retired() {
    let fixture = Fixture::new();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let server = endpoint.server.clone();
    let mut storage = RacingStorage {
        inner: fixture.open(),
        refused: false,
        race: Some(Box::new(move |storage| {
            let bob = storage.find_identity("bob").unwrap().unwrap();
            storage
                .with_binding_transaction(|records| {
                    records.insert_binding(&bob, &server, &pane("%2", 100))
                })
                .unwrap();
        })),
    };
    bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    storage.refused = false;

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Bob", false).unwrap_err();
    assert!(matches!(error, BindingError::PaneAlreadyBound));
    let bob = storage.find_identity("bob").unwrap().unwrap();
    assert!(
        storage
            .with_binding_transaction(|records| records.entry_by_id(&bob.id))
            .unwrap()
            .unwrap()
            .binding
            .is_some()
    );
    storage.inner.close().unwrap();
}

#[test]
fn failed_retirement_is_reported_beside_the_original_error() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    Connection::open(&fixture.database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER block_retirement BEFORE UPDATE OF retired_at_ms ON identities
             BEGIN SELECT RAISE(ABORT, 'retirement blocked'); END;",
        )
        .unwrap();

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Bob", false).unwrap_err();
    let BindingError::CleanupFailed { error, .. } = error else {
        panic!("expected the cleanup failure to accompany the bind error");
    };
    assert!(matches!(*error, BindingError::PaneAlreadyBound));
    // Nothing was hidden: the identity is still there for `tmt rm`.
    assert!(storage.find_identity("bob").unwrap().is_some());
    storage.close().unwrap();
}

const LEFTOVER_BINDING: &str = "11111111-1111-4111-8111-111111111111";
const LEFTOVER_IDENTITY: &str = "22222222-2222-4222-8222-222222222222";

fn leftover_marker(server_id: &str, pane_pid: u64) -> tmt_core::endpoint::BindingMarker {
    tmt_core::endpoint::BindingMarker {
        name: "Ghost".into(),
        canonical_name: "ghost".into(),
        identity_id: LEFTOVER_IDENTITY.into(),
        binding_id: LEFTOVER_BINDING.into(),
        server_id: server_id.into(),
        pane_pid,
    }
}

#[test]
fn a_respawned_pane_binds_its_name_again_over_the_dead_processs_marker() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let first = bind_identity(&mut storage, &mut endpoint, "%1", "Respawn", true).unwrap();
    let old = first.binding.unwrap();
    // respawn-pane keeps the pane and its options but starts another process.
    endpoint.pane_mut("%1").pane_pid += 1;
    let again = bind_identity(&mut storage, &mut endpoint, "%1", "Respawn", true).unwrap();
    let new = again.binding.unwrap();
    assert_ne!(new.id, old.id);
    assert_eq!(new.pane_pid, endpoint.pane_mut("%1").pane_pid);
    let marker = endpoint.pane_mut("%1").marker.clone().unwrap();
    assert_eq!((marker.binding_id, marker.pane_pid), (new.id, new.pane_pid));
    storage.close().unwrap();
}

#[test]
fn a_leftover_marker_of_another_process_is_replaced_only_when_nothing_owns_its_binding() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2", "%3"]);
    let server = endpoint.server.server_id.clone();
    let pid = endpoint.pane_mut("%1").pane_pid;
    // No row owns the binding and the writing process is not the pane's: reclaimable.
    endpoint.pane_mut("%1").marker = Some(leftover_marker(&server, pid + 100));
    let bound = bind_identity(&mut storage, &mut endpoint, "%1", "Fresh", false).unwrap();
    let binding = bound.binding.unwrap();
    assert_eq!(
        endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
        binding.id
    );

    // The same marker copied onto a pane while its binding is still active is not.
    let owned = endpoint.pane_mut("%1").marker.clone().unwrap();
    endpoint.pane_mut("%2").marker = Some(owned.clone());
    let refused = bind_identity(&mut storage, &mut endpoint, "%2", "Copy", false);
    assert!(
        matches!(refused, Err(BindingError::Unverified)),
        "{refused:?}"
    );
    assert_eq!(endpoint.pane_mut("%2").marker.as_ref(), Some(&owned));

    // Another server's marker is never reclaimed.
    let foreign = leftover_marker("another-server", pid);
    endpoint.pane_mut("%3").marker = Some(foreign.clone());
    let refused = bind_identity(&mut storage, &mut endpoint, "%3", "Foreign", false);
    assert!(
        matches!(refused, Err(BindingError::Unverified)),
        "{refused:?}"
    );
    assert_eq!(endpoint.pane_mut("%3").marker.as_ref(), Some(&foreign));
    storage.close().unwrap();
}

#[test]
fn publish_and_verification_failures_rollback_binding_but_keep_identity() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    endpoint.publish_failure = true;
    let error = bind_identity(&mut storage, &mut endpoint, "%1", "PublishFail", false).unwrap_err();
    assert!(matches!(
        error,
        BindingError::Endpoint(EndpointFailure("publish failed"))
    ));
    let identity = storage.find_identity("publishfail").unwrap().unwrap();
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert!(entry.binding.is_none());

    endpoint.publish_failure = false;
    endpoint.verification_failure = true;
    let error = bind_identity(&mut storage, &mut endpoint, "%2", "VerifyFail", false).unwrap_err();
    assert!(matches!(error, BindingError::Unverified));
    let identity = storage.find_identity("verifyfail").unwrap().unwrap();
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert!(entry.binding.is_none());
    storage.close().unwrap();
}

#[test]
fn preflight_missing_or_invalid_name_does_not_create_identity_or_publish() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut missing = FakeEndpoint::new(&[]);
    let error = bind_identity(&mut storage, &mut missing, "%9", "Missing", false).unwrap_err();
    assert!(matches!(error, BindingError::PaneNotFound(pane) if pane == "%9"));
    assert!(storage.find_identity("missing").unwrap().is_none());
    assert_eq!(missing.publish_calls, 0);

    let mut invalid = FakeEndpoint::new(&["%1"]);
    let error = bind_identity(&mut storage, &mut invalid, "%1", "   ", false).unwrap_err();
    assert!(matches!(error, BindingError::InvalidName(_)));
    assert!(storage.find_identity("").unwrap().is_none());
    assert_eq!(invalid.publish_calls, 0);
    storage.close().unwrap();
}

#[test]
fn failed_saved_publication_keeps_separately_committed_promotion() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let original = create_or_resolve(&mut storage, "Promote", Lifetime::Temporary)
        .unwrap()
        .identity;
    endpoint.publish_failure = true;

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Promote", true).unwrap_err();
    assert!(matches!(
        error,
        BindingError::Endpoint(EndpointFailure("publish failed"))
    ));
    let promoted = storage.find_identity("promote").unwrap().unwrap();
    assert_eq!(promoted.id, original.id);
    assert_eq!(promoted.lifetime, Lifetime::Saved);
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&promoted.id))
        .unwrap()
        .unwrap();
    assert!(entry.binding.is_none());
    assert_eq!(endpoint.publish_calls, 0);
    storage.close().unwrap();
}

#[test]
fn dead_pane_after_creation_cannot_be_revived_and_leaves_no_identity() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    endpoint.hide_panes_after_current = Some(2);

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false).unwrap_err();
    assert!(matches!(error, BindingError::PaneNotFound(pane) if pane == "%1"));
    // A pane lost between creation and publication is a deterministic refusal.
    assert!(storage.find_identity("alice").unwrap().is_none());
    assert_eq!(endpoint.publish_calls, 0);
    storage.close().unwrap();
}

#[test]
fn frozen_target_rejects_pane_id_reuse_before_publication() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let target = BindingTargetEvidence {
        server: endpoint.server.clone(),
        pane_id: "%1".into(),
        pane_pid: endpoint.pane_mut("%1").pane_pid,
        pane_incarnation: None,
        discovery: None,
    };
    endpoint.replace_pane_after_current = Some(2);

    let error = bind_identity_at(
        &mut storage,
        &mut endpoint,
        "%1",
        Some(&target),
        "Alice",
        false,
    )
    .unwrap_err();
    assert!(matches!(error, BindingError::TargetChanged(pane) if pane == "%1"));
    assert_eq!(endpoint.publish_calls, 0);
    assert!(storage.find_identity("alice").unwrap().is_none());
    storage.close().unwrap();
}

#[test]
fn a_bind_records_the_observed_pane_incarnation_or_leaves_it_unknown() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    endpoint.pane_mut("%1").pane_pid = 101;
    endpoint
        .starts
        .insert(101, "ps-v1:Thu Oct 1 09:00:00 2026".into());

    bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false).unwrap();
    bind_identity(&mut storage, &mut endpoint, "%2", "Bob", false).unwrap();
    // Observed once per new binding, never by the reads around it.
    assert_eq!(endpoint.incarnation_calls, 2);
    let mut stored = |name: &str| {
        let identity = storage.find_identity(name).unwrap().unwrap();
        storage
            .with_binding_transaction(|records| records.entry_by_id(&identity.id))
            .unwrap()
            .unwrap()
            .binding
            .unwrap()
            .pane_incarnation
    };
    assert_eq!(
        stored("alice").as_deref(),
        Some("ps-v1:Thu Oct 1 09:00:00 2026")
    );
    assert_eq!(stored("bob"), None, "an unobserved start stays unknown");
    // Binding the same pane again keeps the binding; nothing is observed.
    bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false).unwrap();
    assert_eq!(endpoint.incarnation_calls, 2);
    storage.close().unwrap();
}

#[test]
fn a_failed_pane_incarnation_observation_still_binds_with_an_unknown_start() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    endpoint.pane_mut("%1").pane_pid = 101;
    endpoint
        .starts
        .insert(101, "ps-v1:Thu Oct 1 09:00:00 2026".into());
    endpoint.incarnation_failure = true;

    bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false).unwrap();
    assert_eq!(endpoint.incarnation_calls, 1);
    assert_eq!(endpoint.publish_calls, 1, "the binding was published");
    let identity = storage.find_identity("alice").unwrap().unwrap();
    let binding = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap()
        .binding
        .unwrap();
    assert_eq!(binding.pane_id, "%1");
    assert_eq!(binding.pane_incarnation, None);
    storage.close().unwrap();
}

#[test]
fn expiry_after_publication_rolls_back_binding_without_losing_identity() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    endpoint.expire_after_publish = true;

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false).unwrap_err();
    assert!(matches!(error, BindingError::Deadline));
    assert_eq!(endpoint.publish_calls, 1);
    let identity = storage.find_identity("alice").unwrap().unwrap();
    assert_eq!(identity.lifetime, Lifetime::Temporary);
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert!(entry.binding.is_none());
    storage.close().unwrap();
}
