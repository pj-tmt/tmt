use super::*;
use crate::caller_context::test_support::{World, cleanup_failure, environment};
use std::cell::Cell;
use tmt_core::identity::{Lifetime, create_or_resolve};

#[test]
fn stored_binding_operations_do_not_discover_the_callers_host() {
    let named = || Invocation::List {
        target: Some("Sender".into()),
        room: None,
        scope: ListScope::default(),
    };
    for request in [
        named(),
        Invocation::List {
            target: None,
            room: None,
            scope: ListScope::default(),
        },
        Invocation::List {
            target: None,
            room: None,
            scope: ListScope {
                lifetime: Some(Lifetime::Saved),
                ..ListScope::default()
            },
        },
        Invocation::List {
            target: None,
            room: Some("Team".into()),
            scope: ListScope::default(),
        },
        Invocation::Rename {
            old: "Sender".into(),
            new: "Renamed".into(),
        },
        Invocation::Remove {
            name: "Sender".into(),
            force: true,
        },
    ] {
        let world = World::new();
        let mut storage = world.storage();
        let original = create_or_resolve(&mut storage, "Sender", Lifetime::Saved)
            .unwrap()
            .identity;
        if matches!(&request, Invocation::List { room: Some(_), .. }) {
            use tmt_core::room::{self, MembershipChange, RoomRepository};
            let room = room::create(&mut storage, "Team".into()).unwrap();
            storage
                .change_meeting_membership(&room.id, &original.id, MembershipChange::Join)
                .unwrap();
        }
        storage.close().unwrap();
        let calls = Cell::new(0);
        let report = run_with(
            request.clone(),
            environment(),
            || Ok(world.paths.clone()),
            |_| {
                calls.set(calls.get() + 1);
                Err(cleanup_failure())
            },
        )
        .unwrap_or_else(|error| panic!("{request:?}: {}", error.message))
        .0;
        assert_eq!(calls.get(), 0, "{request:?}");
        let mut storage = world.storage();
        match (request, report) {
            (Invocation::Rename { .. }, Report::Renamed { result, .. }) => {
                assert_eq!(result.identity.id, original.id);
                assert_eq!(
                    storage.resolve_identity("Renamed").unwrap().unwrap().id,
                    original.id
                );
                assert!(storage.resolve_identity("Sender").unwrap().is_none());
            }
            (Invocation::Remove { .. }, Report::Removed(entry)) => {
                assert_eq!(entry.identity.id, original.id);
                assert!(storage.resolve_identity("Sender").unwrap().is_none());
            }
            (
                Invocation::List {
                    target: Some(_), ..
                },
                Report::Named { row, .. },
            ) => assert_eq!(row.identity.id, original.id),
            (Invocation::List { target: None, .. }, Report::Listed { rows, .. }) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].presence.identity.id, original.id);
            }
            _ => panic!("unexpected binding report"),
        }
        storage.close().unwrap();
    }
}

#[test]
fn caller_scoped_binding_operations_preserve_cleanup_error_and_state() {
    for request in [
        Invocation::Bind {
            name: "Sender".into(),
            save: false,
            pane: None,
        },
        Invocation::BindMarked {
            name: "Sender".into(),
            save: false,
        },
        Invocation::Whoami,
        Invocation::Unbind,
        Invocation::List {
            target: None,
            room: None,
            scope: ListScope {
                here: true,
                ..ListScope::default()
            },
        },
    ] {
        let world = World::new();
        let mut storage = world.storage();
        let original = create_or_resolve(&mut storage, "Sender", Lifetime::Saved)
            .unwrap()
            .identity;
        storage.close().unwrap();
        let error = run_with(
            request.clone(),
            environment(),
            || Ok(world.paths.clone()),
            |_| Err(cleanup_failure()),
        )
        .err()
        .expect("caller selection must fail");
        assert_eq!(
            error.document(),
            cleanup_failure().document(),
            "{request:?}"
        );
        assert_eq!(error.status, 1);
        let mut storage = world.storage();
        assert_eq!(
            storage.resolve_identity("Sender").unwrap().unwrap(),
            original
        );
        storage.close().unwrap();
    }
}

#[test]
fn explicit_add_preflight_does_not_discover_a_caller_even_when_it_refuses_legacy_order() {
    let world = World::new();
    let error = run_with(
        Invocation::Bind {
            name: "%1".into(),
            pane: Some("Sender".into()),
            save: false,
        },
        environment(),
        || Ok(world.paths.clone()),
        |_| panic!("explicit target acquired a caller host"),
    )
    .err()
    .expect("legacy argument order refuses");
    assert_eq!(error.code, "LEGACY_ADD_ORDER");
    assert!(
        !world.paths.database.exists(),
        "preflight must precede database effects"
    );
}
