use rusqlite::Connection;
use tmt_core::{
    binding::{
        BindingError, BindingRepository, Presence, bind_identity, list_presence, name_presence,
    },
    identity::{IdentityReader, Lifetime, create_or_resolve},
};

use super::super::super::StorageError;
use super::super::test_support::{Fixture, pane};
use super::endpoint::{FakeEndpoint, ProbeMode, server};

#[test]
fn marker_mismatch_detaches_binding_but_keeps_temporary_identity_offline() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let identity = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false)
        .unwrap()
        .identity;
    endpoint.pane_mut("%1").marker = None;

    let presence = name_presence(&mut storage, &mut endpoint, "Alice").unwrap();
    assert_eq!(presence.presence, Presence::Offline);
    assert!(presence.binding.is_none());
    assert_eq!(
        storage.find_identity("alice").unwrap().unwrap().id,
        identity.id
    );
    let entry = storage
        .with_binding_transaction(|records| records.entry_by_id(&identity.id))
        .unwrap()
        .unwrap();
    assert!(entry.binding.is_none());
    storage.close().unwrap();
}

#[test]
fn temporary_name_presence_death_commits_retirement_before_name_not_found() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let identity = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false)
        .unwrap()
        .identity;
    endpoint.probe_mode = ProbeMode::Dead;

    let error = name_presence(&mut storage, &mut endpoint, "Alice").unwrap_err();
    assert!(matches!(error, BindingError::NameNotFound(name) if name == "Alice"));
    assert!(storage.find_identity("alice").unwrap().is_none());
    let connection = Connection::open(&fixture.database).unwrap();
    assert!(
        connection
            .query_row(
                "SELECT retired_at_ms FROM identities WHERE id = ?",
                [&identity.id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap()
            .is_some()
    );
    storage.close().unwrap();
}

#[test]
fn saved_dead_binding_keeps_uuid_and_never_bound_temporary_stays_offline() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let saved = bind_identity(&mut storage, &mut endpoint, "%1", "Saved", true)
        .unwrap()
        .identity;
    endpoint.probe_mode = ProbeMode::Dead;

    let presence = name_presence(&mut storage, &mut endpoint, "Saved").unwrap();
    assert_eq!(presence.presence, Presence::Offline);
    assert!(presence.binding.is_none());
    assert_eq!(presence.identity.id, saved.id);
    assert_eq!(
        storage.find_identity("saved").unwrap().unwrap().id,
        saved.id
    );

    let temporary = create_or_resolve(&mut storage, "NeverBound", Lifetime::Temporary)
        .unwrap()
        .identity;
    let presence = name_presence(&mut storage, &mut endpoint, "NeverBound").unwrap();
    assert_eq!(presence.presence, Presence::Offline);
    assert!(presence.binding.is_none());
    assert_eq!(presence.identity.id, temporary.id);
    assert_eq!(
        storage.find_identity("neverbound").unwrap().unwrap().id,
        temporary.id
    );
    storage.close().unwrap();
}

#[test]
fn unknown_probe_preserves_binding_state() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    endpoint.probe_mode = ProbeMode::Unknown;

    let presence = name_presence(&mut storage, &mut endpoint, "Alice").unwrap();
    assert_eq!(presence.presence, Presence::Unknown);
    let entry = storage
        .with_binding_transaction(|records| {
            records.find_identity("alice").and_then(|identity| {
                identity.map_or(Ok(None), |identity| records.entry_by_id(&identity.id))
            })
        })
        .unwrap()
        .unwrap();
    assert!(entry.binding.is_some());
    storage.close().unwrap();
}

#[test]
fn dead_temporary_binding_retires_before_fresh_name_creation() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let old = bind_identity(&mut storage, &mut endpoint, "%1", "Alice", false)
        .unwrap()
        .identity;
    endpoint.probe_mode = ProbeMode::Dead;

    let fresh = bind_identity(&mut storage, &mut endpoint, "%2", "Alice", false)
        .unwrap()
        .identity;
    assert_ne!(old.id, fresh.id);
    assert!(
        storage
            .find_identity("alice")
            .unwrap()
            .is_some_and(|identity| identity.id == fresh.id)
    );
    let connection = Connection::open(&fixture.database).unwrap();
    assert!(
        connection
            .query_row(
                "SELECT retired_at_ms FROM identities WHERE id = ?",
                [&old.id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap()
            .is_some()
    );
    storage.close().unwrap();
}

#[test]
fn list_groups_servers_orders_identities_and_preserves_rows_when_budget_is_exhausted() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let server_a = server("server-a", "/tmp/tmux-a.sock", 41, "start-a");
    let server_b = server("server-b", "/tmp/tmux-b.sock", 42, "start-b");
    let alpha = create_or_resolve(&mut storage, "Alpha", Lifetime::Saved)
        .unwrap()
        .identity;
    let bravo = create_or_resolve(&mut storage, "Bravo", Lifetime::Saved)
        .unwrap()
        .identity;
    let charlie = create_or_resolve(&mut storage, "Charlie", Lifetime::Saved)
        .unwrap()
        .identity;
    let bindings = storage
        .with_binding_transaction(|records| {
            Ok::<_, StorageError>(vec![
                records.insert_binding(&charlie, &server_a, &pane("%1", 101))?,
                records.insert_binding(&alpha, &server_a, &pane("%2", 102))?,
                records.insert_binding(&bravo, &server_b, &pane("%3", 103))?,
            ])
        })
        .unwrap();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2", "%3"]);
    endpoint.server = server_a.clone();
    endpoint.pane_mut("%1").pane_pid = 101;
    endpoint.pane_mut("%2").pane_pid = 102;
    endpoint.pane_mut("%3").pane_pid = 103;
    for (identity, binding) in [
        (&charlie, &bindings[0]),
        (&alpha, &bindings[1]),
        (&bravo, &bindings[2]),
    ] {
        endpoint.pane_mut(&binding.pane_id).marker = Some(binding.marker(identity));
    }

    let result = list_presence(
        &mut storage,
        &mut endpoint,
        Some(tmt_core::host::ServerSelector {
            host: tmt_core::host::HostKind::Tmux,
            socket: server_b.socket_path.as_str(),
        }),
    )
    .unwrap();
    assert_eq!(
        result
            .iter()
            .map(|presence| presence.identity.canonical_name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "bravo", "charlie"]
    );
    assert!(
        result
            .iter()
            .all(|presence| presence.presence == Presence::Active)
    );
    assert_eq!(endpoint.probe_calls.len(), 2);
    assert_eq!(
        endpoint.probe_calls,
        vec![
            ("server-b".into(), vec!["%3".into()]),
            ("server-a".into(), vec!["%2".into(), "%1".into()]),
        ]
    );

    endpoint.expire_after_probe = Some(endpoint.probe_calls.len() + 1);
    let probe_count = endpoint.probe_calls.len();
    let result = list_presence(
        &mut storage,
        &mut endpoint,
        Some(tmt_core::host::ServerSelector {
            host: tmt_core::host::HostKind::Tmux,
            socket: server_b.socket_path.as_str(),
        }),
    )
    .unwrap();
    assert_eq!(endpoint.probe_calls.len(), probe_count + 1);
    assert_eq!(
        result
            .iter()
            .find(|presence| presence.identity.canonical_name == "bravo")
            .unwrap()
            .presence,
        Presence::Active
    );
    assert_eq!(
        result
            .iter()
            .filter(|presence| presence.presence == Presence::Unknown)
            .count(),
        2
    );
    let entries = storage
        .with_binding_transaction(|records| records.binding_entries())
        .unwrap();
    assert!(entries.iter().all(|entry| entry.binding.is_some()));
    storage.close().unwrap();
}
