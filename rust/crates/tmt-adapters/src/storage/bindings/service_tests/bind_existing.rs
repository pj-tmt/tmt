//! Binding an identity already chosen by its UUID never looks its name up again (#4760).
use tmt_core::{
    binding::{
        BindingError, bind_existing_identity, bind_identity, remove_identity, rename_identity,
    },
    identity::{IdentityReader, Lifetime, create_or_resolve},
};

use super::super::test_support::Fixture;
use super::endpoint::{EndpointFailure, FakeEndpoint};

#[test]
fn a_rename_after_the_uuid_was_chosen_cannot_retarget_the_bind() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let chosen = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    rename_identity::<_, EndpointFailure>(&mut storage, "Alice", "Ada").unwrap();
    // The old name now belongs to somebody else.
    let namesake = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    assert_ne!(namesake.id, chosen.id);

    let bound =
        bind_existing_identity(&mut storage, &mut endpoint, "%1", &chosen.id, false).unwrap();
    assert!(!bound.created);
    assert_eq!(bound.presence.identity.id, chosen.id);
    assert_eq!(bound.presence.identity.name, "Ada");
    let binding = bound
        .presence
        .binding
        .expect("the chosen identity is bound");
    assert_eq!(binding.identity_id, chosen.id);
    assert_eq!(binding.pane_id, "%1");
    // Nothing happened to the namesake, and nobody was created.
    assert_eq!(storage.find_identity("alice").unwrap(), Some(namesake));
    storage.close().unwrap();
}

#[test]
fn an_unknown_or_retired_id_is_refused_and_binds_nothing() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let retired = bind_identity(&mut storage, &mut endpoint, "%2", "Gone", true)
        .unwrap()
        .identity;
    remove_identity(&mut storage, &mut endpoint, "Gone", true).unwrap();

    for id in [retired.id.as_str(), "7f0c2f2e-0000-4000-8000-000000000000"] {
        let error =
            bind_existing_identity(&mut storage, &mut endpoint, "%1", id, true).unwrap_err();
        assert!(
            matches!(&error, BindingError::NameNotFound(named) if named == id),
            "{error:?}"
        );
    }
    let bindings: i64 = storage
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM bindings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(bindings, 0, "a refused bind leaves no binding behind");
    storage.close().unwrap();
}

#[test]
fn only_a_requested_save_promotes_a_temporary_identity() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let kept = create_or_resolve(&mut storage, "Kept", Lifetime::Temporary)
        .unwrap()
        .identity;
    let saved = create_or_resolve(&mut storage, "Saved", Lifetime::Temporary)
        .unwrap()
        .identity;

    let bound = bind_existing_identity(&mut storage, &mut endpoint, "%1", &kept.id, false).unwrap();
    assert_eq!(bound.presence.identity.lifetime, Lifetime::Temporary);
    let bound = bind_existing_identity(&mut storage, &mut endpoint, "%2", &saved.id, true).unwrap();
    assert_eq!(bound.presence.identity.lifetime, Lifetime::Saved);
    assert_eq!(
        storage.find_identity("saved").unwrap().unwrap().lifetime,
        Lifetime::Saved
    );
    storage.close().unwrap();
}
