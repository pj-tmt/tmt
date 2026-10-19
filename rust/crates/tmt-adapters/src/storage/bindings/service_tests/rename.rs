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

#[test]
fn automatic_naming_is_one_verified_transaction_and_keeps_binding_and_preferences() {
    use tmt_core::binding::session::{HarnessId, SessionPreferences};
    use tmt_core::binding::{BindingRepository, bind_auto_identity, name_auto_identity};
    for (launch_saved, naming_saved) in [(false, false), (false, true), (true, false)] {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let mut endpoint = FakeEndpoint::new(&["%1"]);
        let bound = bind_auto_identity(
            &mut storage,
            &mut endpoint,
            "%1",
            "claude-test",
            launch_saved,
        )
        .unwrap();
        let id = &bound.presence.identity.id;
        let preferences = SessionPreferences {
            channel: None,
            preferred_harness: Some(HarnessId::new("claude").unwrap()),
            remembered: None,
        };
        storage
            .with_binding_transaction::<_, crate::storage::StorageError>(|records| {
                records.set_session_preferences(id, &preferences)?;
                Ok(())
            })
            .unwrap();
        let named = name_auto_identity(&mut storage, &mut endpoint, "%1", "Reviewer", naming_saved)
            .unwrap()
            .unwrap();
        assert_eq!(&named.presence.identity.id, id);
        assert_eq!(named.presence.binding, bound.presence.binding);
        assert_eq!(
            named.presence.identity.lifetime,
            if launch_saved || naming_saved {
                Lifetime::Saved
            } else {
                Lifetime::Temporary
            }
        );
        assert_eq!(storage.session_preferences(id).unwrap(), preferences);
        assert!(storage.find_identity("claude-test").unwrap().is_none());
        assert!(
            name_auto_identity(&mut storage, &mut endpoint, "%1", "Another", false)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            endpoint.publish_calls, 1,
            "renaming never replaces the binding marker"
        );
        storage.close().unwrap();
    }
}

#[test]
fn automatic_name_conflict_or_unverified_marker_preserves_provenance_and_lifetime() {
    use tmt_core::binding::{bind_auto_identity, name_auto_identity};
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let before =
        bind_auto_identity(&mut storage, &mut endpoint, "%1", "claude-test", false).unwrap();
    create_or_resolve(&mut storage, "Taken", Lifetime::Saved).unwrap();
    assert!(matches!(
        name_auto_identity(&mut storage, &mut endpoint, "%1", "Taken", true),
        Err(Error::NameTaken(_))
    ));
    let marker = endpoint.pane_mut("%1").marker.take();
    assert!(matches!(
        name_auto_identity(&mut storage, &mut endpoint, "%1", "Reviewer", true),
        Err(Error::Unverified)
    ));
    assert_eq!(
        storage.find_identity("claude-test").unwrap(),
        Some(before.presence.identity.clone())
    );
    assert!(storage.find_identity("reviewer").unwrap().is_none());
    endpoint.pane_mut("%1").marker = marker;
    let named = name_auto_identity(&mut storage, &mut endpoint, "%1", "Reviewer", false)
        .unwrap()
        .unwrap();
    assert_eq!(named.presence.identity.id, before.presence.identity.id);
    assert_eq!(named.presence.identity.lifetime, Lifetime::Temporary);
    storage.close().unwrap();
}

#[test]
fn ordinary_names_that_look_generated_are_not_automatic() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    tmt_core::binding::bind_identity(
        &mut storage,
        &mut endpoint,
        "%1",
        "claude-012345678abc",
        false,
    )
    .unwrap();
    assert!(
        tmt_core::binding::name_auto_identity(&mut storage, &mut endpoint, "%1", "Reviewer", true)
            .unwrap()
            .is_none()
    );
    storage.close().unwrap();
}

#[test]
fn failed_automatic_spawn_retires_only_an_unchanged_temporary_binding() {
    use tmt_core::binding::{bind_auto_identity, name_auto_identity, retire_failed_auto_launch};
    for (save, named) in [(false, false), (true, false), (false, true)] {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let mut endpoint = FakeEndpoint::new(&["%1"]);
        let bound =
            bind_auto_identity(&mut storage, &mut endpoint, "%1", "claude-test", save).unwrap();
        if named {
            name_auto_identity(&mut storage, &mut endpoint, "%1", "Reviewer", false).unwrap();
        }
        retire_failed_auto_launch(
            &mut storage,
            &mut endpoint,
            bound.presence.binding.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(
            storage
                .find_active_identity_by_id(&bound.presence.identity.id)
                .unwrap()
                .is_some(),
            save || named
        );
        assert_eq!(endpoint.clear_calls, usize::from(!save && !named));
        storage.close().unwrap();
    }
}

#[test]
fn a_generated_name_collision_never_reconciles_or_retires_its_existing_holder() {
    use tmt_core::binding::bind_auto_identity;
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let original = tmt_core::binding::bind_identity(
        &mut storage,
        &mut endpoint,
        "%1",
        "claude-collision",
        false,
    )
    .unwrap();
    endpoint.pane_mut("%1").pane_pid += 1; // Conclusive old endpoint loss is still not ours to reconcile.
    assert!(
        bind_auto_identity(&mut storage, &mut endpoint, "%2", "claude-collision", false).is_err()
    );
    assert_eq!(
        storage.find_identity("claude-collision").unwrap(),
        Some(original.identity)
    );
    storage.close().unwrap();
}

#[test]
fn uuid_rename_cas_serializes_competing_names_and_preserves_the_winner() {
    use std::sync::{Arc, Barrier};
    use tmt_core::binding::{RenameOutcome, rename_identity_by_id};
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let alice = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    storage.close().unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = ["Ada", "Carol"]
        .into_iter()
        .map(|name| {
            let mut storage = fixture.open();
            let id = alice.id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let result =
                    rename_identity_by_id::<_, EndpointFailure>(&mut storage, &id, "Alice", name)
                        .unwrap();
                storage.close().unwrap();
                result
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    let applied = results
        .iter()
        .find_map(|r| match r {
            RenameOutcome::Applied(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, RenameOutcome::Applied(_)))
            .count(),
        1
    );
    let conflict = results
        .iter()
        .find_map(|r| match r {
            RenameOutcome::Conflict(i) => Some(i),
            _ => None,
        })
        .unwrap();
    assert_eq!(conflict, &applied.identity);
    let mut storage = fixture.open();
    assert_eq!(
        storage.find_identity_by_id(&alice.id).unwrap(),
        Some(applied.identity.clone())
    );
    storage.close().unwrap();
}

#[test]
fn uuid_rename_checks_exact_expectation_retirement_and_canonical_collision() {
    use tmt_core::binding::{BindingRepository, RenameOutcome, rename_identity_by_id};
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let alice = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    let bob = create_or_resolve(&mut storage, "Bob", Lifetime::Saved)
        .unwrap()
        .identity;
    assert_eq!(
        rename_identity_by_id::<_, EndpointFailure>(&mut storage, &alice.id, "alice", "Ada")
            .unwrap(),
        RenameOutcome::Conflict(alice.clone())
    );
    assert!(matches!(
        rename_identity_by_id::<_, EndpointFailure>(&mut storage, &alice.id, "Alice", "BOB"),
        Err(Error::NameTaken(_))
    ));
    assert_eq!(
        storage.find_identity_by_id(&alice.id).unwrap(),
        Some(alice.clone())
    );
    assert_eq!(storage.find_identity_by_id(&bob.id).unwrap(), Some(bob));
    let RenameOutcome::Applied(same) =
        rename_identity_by_id::<_, EndpointFailure>(&mut storage, &alice.id, "Alice", "Alice")
            .unwrap()
    else {
        panic!("same-name rename must apply")
    };
    assert!(!same.changed());
    assert_eq!(same.identity, alice);
    let RenameOutcome::Applied(cased) =
        rename_identity_by_id::<_, EndpointFailure>(&mut storage, &alice.id, "Alice", "ALICE")
            .unwrap()
    else {
        panic!("case change must apply")
    };
    assert!(cased.changed());
    assert_eq!(cased.identity.canonical_name, "alice");
    storage
        .with_binding_transaction(|rows| rows.retire_identity(&cased.identity, false))
        .unwrap();
    let successor = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    assert_eq!(
        rename_identity_by_id::<_, EndpointFailure>(&mut storage, &alice.id, "ALICE", "Ada")
            .unwrap(),
        RenameOutcome::NotFound
    );
    assert_eq!(
        storage.find_identity_by_id(&successor.id).unwrap(),
        Some(successor)
    );
    storage.close().unwrap();
}

#[test]
fn uuid_rename_preserves_bound_state_and_noop_cursor_and_rolls_back_storage_failure() {
    use tmt_core::binding::{RenameOutcome, rename_identity_by_id};
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let bound =
        tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    storage
        .connection()
        .unwrap()
        .execute(
            "INSERT INTO identity_metadata (identity_id,key,value) VALUES (?, 'team', 'core')",
            [&bound.identity.id],
        )
        .unwrap();
    let cursor = storage.change_cursor().unwrap();
    let RenameOutcome::Applied(same) = rename_identity_by_id::<_, EndpointFailure>(
        &mut storage,
        &bound.identity.id,
        "Alice",
        "Alice",
    )
    .unwrap() else {
        panic!("no-op must apply")
    };
    assert!(!same.changed());
    assert_eq!(storage.change_cursor().unwrap(), cursor);
    assert!(matches!(
        rename_identity_by_id::<_, EndpointFailure>(
            &mut storage,
            &bound.identity.id,
            "alice",
            "Ada"
        )
        .unwrap(),
        RenameOutcome::Conflict(_)
    ));
    assert_eq!(storage.change_cursor().unwrap(), cursor);
    storage.connection().unwrap().execute_batch("CREATE TRIGGER refuse_rename BEFORE UPDATE OF name ON identities BEGIN SELECT RAISE(ABORT, 'fixture refusal'); END;").unwrap();
    assert!(matches!(
        rename_identity_by_id::<_, EndpointFailure>(
            &mut storage,
            &bound.identity.id,
            "Alice",
            "Ada"
        ),
        Err(Error::Repository(_))
    ));
    assert_eq!(
        storage.find_identity_by_id(&bound.identity.id).unwrap(),
        Some(bound.identity.clone())
    );
    assert_eq!(storage.change_cursor().unwrap(), cursor);
    storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_rename;")
        .unwrap();
    let RenameOutcome::Applied(renamed) = rename_identity_by_id::<_, EndpointFailure>(
        &mut storage,
        &bound.identity.id,
        "Alice",
        "Ada",
    )
    .unwrap() else {
        panic!("rename must apply")
    };
    assert_eq!(renamed.identity.id, bound.identity.id);
    assert_eq!(renamed.identity.lifetime, Lifetime::Saved);
    assert_eq!(renamed.binding, bound.binding);
    assert!(storage.change_cursor().unwrap() > cursor);
    let metadata: String = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT value FROM identity_metadata WHERE identity_id = ? AND key = 'team'",
            [&bound.identity.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(metadata, "core");
    assert_eq!(
        name_presence(&mut storage, &mut endpoint, "Ada")
            .unwrap()
            .presence,
        Presence::Active
    );
    storage.close().unwrap();
}

#[test]
fn a_new_name_over_64_characters_is_refused_on_every_admission_path_and_a_held_one_still_resolves()
{
    use tmt_core::binding::{bind_auto_identity, bind_identity, name_auto_identity};
    use tmt_core::identity::find_by_name;
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let (sixty_four, sixty_five) = ("n".repeat(64), "n".repeat(65));

    // create
    assert!(matches!(
        create_or_resolve(&mut storage, &sixty_five, Lifetime::Saved),
        Err(tmt_core::identity::IdentityError::InvalidName(_))
    ));
    assert!(storage.find_identity(&sixty_five).unwrap().is_none());
    // add and name: a bind that creates the identity
    assert!(matches!(
        bind_identity(&mut storage, &mut endpoint, "%1", &sixty_five, true),
        Err(Error::InvalidName(_))
    ));
    let bound = bind_identity(&mut storage, &mut endpoint, "%1", &sixty_four, true).unwrap();
    assert_eq!(bound.identity.name, sixty_four);
    // rename
    assert!(matches!(
        rename_identity::<_, EndpointFailure>(&mut storage, &sixty_four, &sixty_five),
        Err(Error::InvalidName(_))
    ));
    assert_eq!(
        storage.find_identity(&sixty_four).unwrap().unwrap().id,
        bound.identity.id
    );
    // this: naming the auto-named caller
    bind_auto_identity(&mut storage, &mut endpoint, "%2", "claude-test", true).unwrap();
    assert!(matches!(
        name_auto_identity(&mut storage, &mut endpoint, "%2", &sixty_five, true),
        Err(Error::InvalidName(_))
    ));
    assert!(storage.find_identity("claude-test").unwrap().is_some());

    // An identity that already holds a longer name keeps it, lists, resolves and renames away.
    storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET name = ?1, canonical_name = ?1 WHERE id = ?2",
            [&sixty_five, &bound.identity.id],
        )
        .unwrap();
    assert!(
        storage
            .list_identities()
            .unwrap()
            .iter()
            .any(|identity| identity.name == sixty_five)
    );
    assert_eq!(
        find_by_name(&storage, &sixty_five).unwrap().unwrap().id,
        bound.identity.id
    );
    let renamed =
        rename_identity::<_, EndpointFailure>(&mut storage, &sixty_five, "Short").unwrap();
    assert_eq!(renamed.identity.id, bound.identity.id);
    storage.close().unwrap();
}
