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
        launch_owner: None,
        state: RuntimeState::Running,
        last_transition: Some(SessionTransition::Resumed),
        key: Some(tmt_core::binding::session::ObservedSessionKey {
            incarnation: tmt_core::endpoint::ProcessIncarnation::new(101, "runtime-start").unwrap(),
            provider_session: None,
        }),
    };
    let preferences = SessionPreferences {
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
/// retirement (the third binding transaction of a refused `bind_identity`).
struct RacingStorage {
    inner: Storage,
    binding_transactions: usize,
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
        self.binding_transactions += 1;
        if self.binding_transactions == 3
            && let Some(race) = self.race.take()
        {
            race(&mut self.inner);
        }
        self.inner.with_binding_transaction(operation)
    }
}

#[test]
fn identity_bound_by_another_invocation_before_cleanup_is_not_retired() {
    let fixture = Fixture::new();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let server = endpoint.server.clone();
    let mut storage = RacingStorage {
        inner: fixture.open(),
        binding_transactions: 0,
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
    storage.binding_transactions = 0;

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
