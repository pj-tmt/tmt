use tmt_core::{
    binding::{BindingError, Presence, name_presence, remove_identity, rename_identity},
    identity::{IdentityReader, Lifetime, create_or_resolve},
};

use super::super::test_support::Fixture;
use super::endpoint::{EndpointFailure, FakeEndpoint};

type Error = BindingError<crate::storage::StorageError, EndpointFailure>;

#[test]
fn a_bound_identity_keeps_its_uuid_binding_and_state_under_a_new_name() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let bound =
        tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO identity_metadata (identity_id, key, value)
             VALUES (?, 'team', 'core')",
            [&bound.identity.id],
        )
        .unwrap();

    let renamed = rename_identity::<_, EndpointFailure>(&mut storage, "alice", "Ada").unwrap();
    assert!(renamed.changed());
    assert_eq!(renamed.previous.name, "Alice");
    assert_eq!(renamed.identity.id, bound.identity.id);
    assert_eq!(renamed.identity.name, "Ada");
    assert_eq!(renamed.identity.canonical_name, "ada");
    assert_eq!(renamed.identity.lifetime, Lifetime::Saved);
    assert_eq!(renamed.binding, bound.binding);
    assert_eq!(storage.find_identity("alice").unwrap(), None);
    let metadata: String = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT value FROM identity_metadata WHERE identity_id = ?",
            [&bound.identity.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(metadata, "core");

    // The pane marker still carries "Alice": the IDs keep the binding active.
    let presence = name_presence(&mut storage, &mut endpoint, "Ada").unwrap();
    assert_eq!(presence.presence, Presence::Active);
    assert_eq!(presence.identity.id, bound.identity.id);
    assert!(matches!(
        name_presence(&mut storage, &mut endpoint, "Alice"),
        Err(BindingError::NameNotFound(_))
    ));
    storage.close().unwrap();
}

#[test]
fn a_name_held_by_an_unretired_identity_is_refused_and_nothing_changes() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let alice = tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true)
        .unwrap()
        .identity;
    let offline = create_or_resolve(&mut storage, "Bob", Lifetime::Saved)
        .unwrap()
        .identity;
    for (old, new) in [("Alice", "BOB"), ("Bob", "alice")] {
        let error = rename_identity::<_, EndpointFailure>(&mut storage, old, new).unwrap_err();
        assert!(
            matches!(&error, Error::NameTaken(name) if name == new),
            "{error}"
        );
    }
    assert_eq!(storage.find_identity("alice").unwrap(), Some(alice));
    assert_eq!(storage.find_identity("bob").unwrap(), Some(offline));
    storage.close().unwrap();
}

#[test]
fn a_retired_name_is_free_and_a_case_only_rename_changes_the_display_name() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&[]);
    let retired = create_or_resolve(&mut storage, "Old", Lifetime::Saved)
        .unwrap()
        .identity;
    remove_identity(&mut storage, &mut endpoint, "Old", true).unwrap();
    let carol = create_or_resolve(&mut storage, "Carol", Lifetime::Temporary)
        .unwrap()
        .identity;

    let reused = rename_identity::<_, EndpointFailure>(&mut storage, "Carol", "old").unwrap();
    assert_eq!(reused.identity.id, carol.id);
    assert_ne!(reused.identity.id, retired.id);
    assert_eq!(reused.identity.lifetime, Lifetime::Temporary);

    let cased = rename_identity::<_, EndpointFailure>(&mut storage, "old", "OLD").unwrap();
    assert!(cased.changed());
    assert_eq!(cased.identity.name, "OLD");
    assert_eq!(cased.identity.canonical_name, "old");

    let same = rename_identity::<_, EndpointFailure>(&mut storage, "old", "OLD").unwrap();
    assert!(!same.changed());
    assert_eq!(same.identity, cased.identity);
    storage.close().unwrap();
}

#[test]
fn an_unknown_old_name_or_an_invalid_new_name_fails_before_any_write() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let dana = create_or_resolve(&mut storage, "Dana", Lifetime::Saved)
        .unwrap()
        .identity;
    assert!(matches!(
        rename_identity::<_, EndpointFailure>(&mut storage, "nobody", "Eve"),
        Err(Error::NameNotFound(name)) if name == "nobody"
    ));
    for invalid in ["", "%1", "bad\u{1}name"] {
        assert!(matches!(
            rename_identity::<_, EndpointFailure>(&mut storage, "Dana", invalid),
            Err(Error::InvalidName(_))
        ));
    }
    assert_eq!(storage.find_identity("dana").unwrap(), Some(dana));
    storage.close().unwrap();
}
