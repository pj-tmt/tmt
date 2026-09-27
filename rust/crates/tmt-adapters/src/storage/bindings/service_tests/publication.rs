use tmt_core::{
    binding::{
        BindingError, BindingRepository, BindingTargetEvidence, bind_identity, bind_identity_at,
    },
    identity::{IdentityReader, Lifetime, create_or_resolve},
};

use super::super::test_support::Fixture;
use super::endpoint::{EndpointFailure, FakeEndpoint};

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
        state: RuntimeState::Running,
        last_transition: Some(SessionTransition::Resumed),
        key: Some(tmt_core::binding::session::ObservedSessionKey {
            incarnation: tmt_core::binding::session::RuntimeIncarnation::new(101, "runtime-start")
                .unwrap(),
            provider_session: None,
        }),
    };
    let preferences = SessionPreferences {
        preferred_harness: Some(HarnessId::new("codex").unwrap()),
        remembered: Some(RememberedSession {
            harness: HarnessId::new("codex").unwrap(),
            mode: RuntimeMode::new("shared").unwrap(),
            provider_session: ProviderSessionId::new("remembered-session").unwrap(),
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
fn occupied_pane_leaves_new_identity_committed_without_binding() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let alice = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true)
        .unwrap()
        .identity;

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Bob", false).unwrap_err();
    assert!(matches!(error, BindingError::PaneAlreadyBound));
    let bob = storage.find_identity("bob").unwrap().unwrap();
    assert_ne!(alice.id, bob.id);
    let bob_entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&bob.id))
        .unwrap()
        .unwrap();
    assert!(bob_entry.binding.is_none());
    let alice_entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&alice.id))
        .unwrap()
        .unwrap();
    assert!(alice_entry.binding.is_some());
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
fn dead_pane_after_creation_cannot_be_revived_between_creation_and_publication() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    endpoint.hide_panes_after_current = Some(2);

    let error = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false).unwrap_err();
    assert!(matches!(error, BindingError::PaneNotFound(pane) if pane == "%1"));
    let identity = storage.find_identity("alice").unwrap().unwrap();
    assert_eq!(identity.lifetime, Lifetime::Temporary);
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert!(entry.binding.is_none());
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
    let identity = storage.find_identity("alice").unwrap().unwrap();
    assert!(
        storage
            .with_binding_transaction(|records| records.entry_by_id(&identity.id))
            .unwrap()
            .unwrap()
            .binding
            .is_none()
    );
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
