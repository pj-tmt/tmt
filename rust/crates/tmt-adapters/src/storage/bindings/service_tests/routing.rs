use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use tmt_core::{
    binding::{BindingRepository, bind_identity, current_name_presence, pane_presence},
    identity::{IdentityReader, Lifetime, create_or_resolve},
};

use super::super::test_support::{Fixture, pane};
use super::endpoint::{FakeEndpoint, server};

fn binding_state(path: &Path, binding_id: &str) -> Option<(String, String, String)> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    connection
        .query_row(
            "SELECT server_id, socket_path, last_verified_at FROM bindings WHERE id = ?",
            [binding_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .unwrap()
}

#[test]
fn current_name_presence_returns_complete_active_binding_evidence() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let current_server = endpoint.server.clone();
    let bound = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let identity = bound.identity;
    let binding = bound.binding.unwrap();
    let current_calls = endpoint.current_calls;

    let observed = current_name_presence(&mut storage, &mut endpoint, "  aLiCe  ")
        .unwrap()
        .expect("active canonical name resolves on the current server");
    assert_eq!(observed.server, current_server);
    assert_eq!(observed.pane.id, "%1");
    assert_eq!(observed.pane.pane_pid, binding.pane_pid);
    assert_eq!(observed.identity, Some(identity.clone()));
    assert_eq!(observed.binding, Some(binding));
    assert_eq!(endpoint.current_calls, current_calls + 1);
    assert!(endpoint.probe_calls.is_empty());
    storage.close().unwrap();
}

#[test]
fn current_name_presence_unknown_or_pane_shaped_lookup_does_not_create_identity() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&[]);
    let before = storage
        .with_binding_transaction(|records| records.binding_entries())
        .unwrap();

    for name in ["Unknown", "   ", "%9", "main:1.0"] {
        assert!(
            current_name_presence(&mut storage, &mut endpoint, name)
                .unwrap()
                .is_none(),
            "lookup-only routing must not reinterpret or create '{name}'"
        );
    }

    let after = storage
        .with_binding_transaction(|records| records.binding_entries())
        .unwrap();
    assert!(before.is_empty());
    assert!(after.is_empty());
    assert_eq!(endpoint.current_calls, 0);
    assert!(storage.find_identity("unknown").unwrap().is_none());
    storage.close().unwrap();
}

#[test]
fn current_name_presence_rejects_foreign_socket_with_equal_server_uuid_without_mutation() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    let foreign = server("server-a", "/tmp/tmux-foreign.sock", 41, "foreign-start");
    let foreign_binding = storage
        .with_binding_transaction(|records| {
            records.insert_binding(&identity, &foreign, &pane("%1", 100))
        })
        .unwrap();
    let before_state = binding_state(&fixture.database, &foreign_binding.id).unwrap();

    let mut endpoint = FakeEndpoint::new(&["%1"]);
    endpoint.server = server("server-a", "/tmp/tmux-current.sock", 42, "current-start");
    assert!(
        current_name_presence(&mut storage, &mut endpoint, "Alice")
            .unwrap()
            .is_none()
    );
    assert!(endpoint.probe_calls.is_empty());

    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert_eq!(entry.identity, identity);
    assert_eq!(entry.binding, Some(foreign_binding));
    assert_eq!(
        binding_state(&fixture.database, &entry.binding.unwrap().id),
        Some(before_state)
    );
    storage.close().unwrap();
}

#[test]
fn current_name_presence_reads_another_hosts_binding_as_not_active_without_asking() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    let mut other = server("server-a", "/tmp/herdr.sock", 41, "other-start");
    other.host = tmt_core::host::HostKind::parse("herdr").unwrap();
    let other_binding = storage
        .with_binding_transaction(|records| {
            records.insert_binding(&identity, &other, &pane("w1-1", 100))
        })
        .unwrap();
    let before_state = binding_state(&fixture.database, &other_binding.id).unwrap();

    let mut endpoint = FakeEndpoint::new(&["%1"]);
    assert!(
        current_name_presence(&mut storage, &mut endpoint, "Alice")
            .unwrap()
            .is_none()
    );
    assert_eq!(endpoint.current_calls, 0);
    assert!(endpoint.probe_calls.is_empty());
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert_eq!(entry.binding, Some(other_binding));
    assert_eq!(
        binding_state(&fixture.database, &entry.binding.unwrap().id),
        Some(before_state)
    );
    storage.close().unwrap();
}

#[test]
fn current_name_presence_detaches_stale_marker_but_preserves_saved_identity() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let bound = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let identity = bound.identity;
    endpoint.pane_mut("%1").marker = None;

    assert!(
        current_name_presence(&mut storage, &mut endpoint, "Alice")
            .unwrap()
            .is_none()
    );
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert_eq!(entry.identity, identity);
    assert!(entry.binding.is_none());
    storage.close().unwrap();
}

#[test]
fn current_name_presence_unknown_server_evidence_preserves_binding() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let bound = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let identity = bound.identity;
    let binding = bound.binding.unwrap();
    let before_state = binding_state(&fixture.database, &binding.id).unwrap();
    endpoint.server.server_id = "server-b".into();

    assert!(
        current_name_presence(&mut storage, &mut endpoint, "Alice")
            .unwrap()
            .is_none()
    );
    assert!(endpoint.probe_calls.is_empty());
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert_eq!(entry.binding, Some(binding));
    assert_eq!(
        binding_state(&fixture.database, &entry.binding.unwrap().id),
        Some(before_state)
    );
    storage.close().unwrap();
}

#[test]
fn current_name_presence_scopes_observation_without_reconciling_unrelated_rows() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let alice = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true)
        .unwrap()
        .identity;
    let bob = bind_identity(&mut storage, &mut endpoint, "%2", "Bob", true)
        .unwrap()
        .identity;
    let bob_entry_before = storage
        .with_binding_transaction(|records| records.entry_by_id(&bob.id))
        .unwrap()
        .unwrap();
    let bob_binding_id = bob_entry_before.binding.as_ref().unwrap().id.clone();
    let bob_state_before = binding_state(&fixture.database, &bob_binding_id).unwrap();
    endpoint.pane_mut("%2").marker = None;

    let observed = current_name_presence(&mut storage, &mut endpoint, "Alice")
        .unwrap()
        .expect("Alice remains active");
    assert_eq!(observed.identity.unwrap().id, alice.id);

    let bob_entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&bob.id))
        .unwrap()
        .unwrap();
    assert!(bob_entry.binding.is_some());
    assert_eq!(
        binding_state(&fixture.database, &bob_binding_id),
        Some(bob_state_before)
    );
    assert!(endpoint.probe_calls.is_empty());
    storage.close().unwrap();
}

#[test]
fn pane_presence_retains_server_pane_identity_and_binding_evidence() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let current_server = endpoint.server.clone();
    let bound = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let identity = bound.identity;
    let binding = bound.binding.unwrap();

    let active = pane_presence(&mut storage, &mut endpoint, "%1").unwrap();
    assert_eq!(active.server, current_server);
    assert_eq!(active.pane.id, "%1");
    assert_eq!(active.identity, Some(identity.clone()));
    assert_eq!(active.binding, Some(binding));

    let unbound = pane_presence(&mut storage, &mut endpoint, "%2").unwrap();
    assert_eq!(unbound.server, current_server);
    assert_eq!(unbound.pane.id, "%2");
    assert_eq!(unbound.identity, None);
    assert_eq!(unbound.binding, None);
    storage.close().unwrap();
}

#[test]
fn pane_presence_stale_marker_returns_pane_evidence_and_detaches_binding() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let bound = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let identity = bound.identity;
    endpoint.pane_mut("%1").marker = None;

    let observed = pane_presence(&mut storage, &mut endpoint, "%1").unwrap();
    assert_eq!(observed.server, endpoint.server);
    assert_eq!(observed.pane.id, "%1");
    assert_eq!(observed.identity, None);
    assert_eq!(observed.binding, None);
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert!(entry.binding.is_none());
    storage.close().unwrap();
}
