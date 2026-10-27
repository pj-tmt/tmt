use rusqlite::{Connection, params, types::Value};
use tmt_core::{
    binding::{BindingError, remove_identity, unbind_identity},
    identity::{IdentityReader, Lifetime, create_or_resolve},
};

use super::super::test_support::Fixture;
use super::endpoint::{FakeEndpoint, ProbeMode};

fn state_counts(connection: &Connection, identity_id: &str) -> (i64, i64, i64, i64, i64, i64) {
    connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM role_profiles WHERE identity_id = ?),
                (SELECT COUNT(*) FROM identity_preambles WHERE identity_id = ?),
                (SELECT COUNT(*) FROM preamble_counters WHERE identity_id = ?),
                (SELECT COUNT(*) FROM request_attempts WHERE identity_id = ?),
                (SELECT COUNT(*) FROM request_responses WHERE request_id = ?),
                (SELECT COUNT(*) FROM request_attention_identities WHERE identity_id = ?)",
            params![
                identity_id,
                identity_id,
                identity_id,
                identity_id,
                format!("request-{identity_id}"),
                identity_id,
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap()
}

fn binding_rows(connection: &Connection, identity_id: &str) -> Vec<Vec<Value>> {
    let mut statement = connection
        .prepare("SELECT * FROM bindings WHERE identity_id = ? ORDER BY id")
        .unwrap();
    statement
        .query_map([identity_id], |row| {
            (0..row.as_ref().column_count())
                .map(|index| row.get(index))
                .collect::<rusqlite::Result<Vec<Value>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn seed_state(connection: &Connection, identity_id: &str) {
    let attempt_id = format!("attempt-{identity_id}");
    let request_id = format!("request-{identity_id}");
    connection
        .execute(
            "INSERT INTO role_profiles VALUES (?, 'role body', 'role-time')",
            [identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO identity_preambles VALUES (?, 'preamble body', 'preamble-time')",
            [identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO preamble_counters VALUES (?, 12, 123456789)",
            [identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_attention_identities VALUES (?, 7, 5)",
            [identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_attempts (
                attempt_id, request_id, identity_id, server_id, socket_path, server_pid,
                server_start_time, pane_id, pane_pid, wait_active, status, inject_preamble,
                cadence_reserved, prepared_at_ms, expires_at_ms, retention_days,
                retention_expires_at_ms
             ) VALUES (?, ?, ?, 'server', '/tmp/socket', 41, 'server-start', '%1', 99,
                0, 'sent', 0, 1, 100, 200, 7, 300)",
            params![attempt_id, request_id, identity_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO request_responses (
                request_id, attempt_id, server_id, socket_path, server_pid,
                server_start_time, pane_id, pane_pid, body, body_bytes, submitted_at_ms,
                response_expires_at_ms
             ) VALUES (?, ?, 'server', '/tmp/socket', 41, 'server-start', '%1', 99,
                'response body', 13, 150, 500)",
            params![request_id, attempt_id],
        )
        .unwrap();
}

#[test]
fn remove_unknown_binding_fails_closed_without_mutating_state() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let identity =
        tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true)
            .unwrap()
            .identity;
    seed_state(storage.connection().unwrap(), &identity.id);
    let rows = binding_rows(storage.connection().unwrap(), &identity.id);
    let counts = state_counts(storage.connection().unwrap(), &identity.id);
    endpoint.probe_mode = ProbeMode::Unknown;
    assert!(matches!(
        remove_identity(&mut storage, &mut endpoint, "Alice", true),
        Err(BindingError::Unverified)
    ));
    assert_eq!(endpoint.clear_calls, 0);
    assert_eq!(
        binding_rows(storage.connection().unwrap(), &identity.id),
        rows
    );
    assert_eq!(
        state_counts(storage.connection().unwrap(), &identity.id),
        counts
    );
    storage.close().unwrap();
}

#[test]
fn failed_post_commit_clear_keeps_retirement_and_stale_marker_is_reclaimed() {
    for (conditional, fail_with_error) in
        [(true, false), (true, true), (false, false), (false, true)]
    {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let mut endpoint = FakeEndpoint::new(&["%1"]);
        let original =
            tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true)
                .unwrap();
        let old_id = original.binding.unwrap().id;
        endpoint.conditional_clear = conditional;
        endpoint.clear_result = false;
        endpoint.clear_failure = fail_with_error;
        remove_identity(&mut storage, &mut endpoint, "Alice", true).unwrap();
        assert!(storage.find_identity("alice").unwrap().is_none());
        assert_eq!(
            endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
            old_id
        );
        endpoint.clear_failure = false;
        endpoint.clear_result = true;
        let successor =
            tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Successor", false)
                .unwrap();
        assert_eq!(
            endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
            successor.binding.unwrap().id
        );
        storage.close().unwrap();
    }
}

#[test]
fn clear_after_detach_never_erases_a_concurrent_successor() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let database = fixture.database.clone();
    endpoint.before_clear = Some(Box::new(move |host| {
        let mut other = crate::storage::Storage::open(&database).unwrap();
        let successor =
            tmt_core::binding::bind_identity(&mut other, host, "%1", "Successor", false).unwrap();
        assert_eq!(
            host.pane_mut("%1").marker.as_ref().unwrap().binding_id,
            successor.binding.unwrap().id
        );
        other.close().unwrap();
    }));
    unbind_identity(&mut storage, &mut endpoint, "%1")
        .unwrap()
        .unwrap();
    let successor = storage.find_identity("successor").unwrap().unwrap();
    assert_eq!(
        endpoint.pane_mut("%1").marker.as_ref().unwrap().identity_id,
        successor.id
    );
    storage.close().unwrap();
}

#[test]
fn unbind_refuses_a_replacement_created_after_its_snapshot() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true).unwrap();
    let old_marker = endpoint.pane_mut("%1").marker.clone();
    let database = fixture.database.clone();
    endpoint.after_current = Some(Box::new(move || {
        let mut other = crate::storage::Storage::open(&database).unwrap();
        let mut host = FakeEndpoint::new(&["%1"]);
        host.pane_mut("%1").marker = old_marker;
        unbind_identity(&mut other, &mut host, "%1")
            .unwrap()
            .unwrap();
        tmt_core::binding::bind_identity(&mut other, &mut host, "%1", "Successor", false).unwrap();
        other.close().unwrap();
    }));
    assert!(matches!(
        unbind_identity(&mut storage, &mut endpoint, "%1"),
        Err(BindingError::Unverified)
    ));
    assert!(storage.find_identity("successor").unwrap().is_some());
    assert_eq!(endpoint.clear_calls, 0);
    storage.close().unwrap();
}

#[test]
fn legacy_clear_reserves_the_pane_and_refuses_a_concurrent_successor() {
    for action in ["unbind", "remove", "failed-auto"] {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let mut endpoint = FakeEndpoint::new(&["%1"]);
        let original = if action == "failed-auto" {
            tmt_core::binding::bind_auto_identity(&mut storage, &mut endpoint, "%1", "Alice", false)
                .unwrap()
                .presence
        } else {
            tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true)
                .unwrap()
        };
        let binding = original.binding.unwrap();
        endpoint.conditional_clear = false;
        let database = fixture.database.clone();
        let old_id = binding.id.clone();
        endpoint.before_clear = Some(Box::new(move |host| {
            let mut other = crate::storage::Storage::open(&database).unwrap();
            let count: i64 = other
                .connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM binding_reservations WHERE target_binding_id = ?",
                    [&old_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "legacy clear must hold the pane reservation");
            let result =
                tmt_core::binding::bind_identity(&mut other, host, "%1", "Successor", false);
            assert!(matches!(
                result,
                Err(BindingError::PaneAlreadyBound) | Err(BindingError::Unverified)
            ));
            assert_eq!(
                host.pane_mut("%1").marker.as_ref().unwrap().binding_id,
                old_id
            );
            other.close().unwrap();
        }));
        match action {
            "unbind" => {
                unbind_identity(&mut storage, &mut endpoint, "%1")
                    .unwrap()
                    .unwrap();
            }
            "remove" => {
                remove_identity(&mut storage, &mut endpoint, "Alice", true).unwrap();
            }
            "failed-auto" => {
                tmt_core::binding::retire_failed_auto_launch(&mut storage, &mut endpoint, &binding)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            endpoint.before_clear.is_none(),
            "legacy clear was not reached"
        );
        let count: i64 = storage
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM binding_reservations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        let successor =
            tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Successor", false)
                .unwrap();
        assert_eq!(
            endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
            successor.binding.unwrap().id
        );
        storage.close().unwrap();
    }
}

#[test]
fn changed_identity_after_host_observation_refuses_removal() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let original =
        tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", true)
            .unwrap()
            .identity;
    let database = fixture.database.clone();
    endpoint.after_probe = Some(Box::new(move || {
        let mut other = crate::storage::Storage::open(&database).unwrap();
        tmt_core::binding::rename_identity::<_, super::endpoint::EndpointFailure>(
            &mut other, "Alice", "Renamed",
        )
        .unwrap();
        other.close().unwrap();
    }));
    assert!(matches!(
        remove_identity(&mut storage, &mut endpoint, "Alice", true),
        Err(BindingError::Unverified)
    ));
    assert_eq!(
        storage.find_identity("renamed").unwrap().unwrap().id,
        original.id
    );
    assert_eq!(endpoint.clear_calls, 0);
    storage.close().unwrap();
}

#[test]
fn unbind_retires_temporary_but_saved_offline_only_detaches() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
    let temporary =
        tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%1", "Temporary", false)
            .unwrap()
            .identity;
    let unbound = unbind_identity(&mut storage, &mut endpoint, "%1")
        .unwrap()
        .unwrap();
    assert!(unbound.retired);
    assert!(unbound.binding.is_some());
    assert!(storage.find_identity("temporary").unwrap().is_none());
    assert_eq!(unbound.identity.id, temporary.id);

    let saved = tmt_core::binding::bind_identity(&mut storage, &mut endpoint, "%2", "Saved", true)
        .unwrap()
        .identity;
    endpoint.pane_mut("%2").marker = None;
    let unbound = unbind_identity(&mut storage, &mut endpoint, "%2")
        .unwrap()
        .unwrap();
    assert!(!unbound.retired);
    assert!(unbound.binding.is_none());
    assert_eq!(unbound.identity.id, saved.id);
    assert!(storage.find_identity("saved").unwrap().is_some());
    assert_eq!(endpoint.clear_calls, 1);
    storage.close().unwrap();
}

#[test]
fn saved_remove_requires_confirmation_without_endpoint_io_and_force_removes_only_content() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&[]);
    let identity = create_or_resolve(&mut storage, "Saved", Lifetime::Saved)
        .unwrap()
        .identity;
    seed_state(storage.connection().unwrap(), &identity.id);
    let calls = (
        endpoint.begin_calls,
        endpoint.current_calls,
        endpoint.probe_calls.len(),
        endpoint.clear_calls,
    );

    let error = remove_identity(&mut storage, &mut endpoint, "Saved", false).unwrap_err();
    assert!(matches!(error, BindingError::ConfirmationRequired));
    assert_eq!(
        calls,
        (
            endpoint.begin_calls,
            endpoint.current_calls,
            endpoint.probe_calls.len(),
            endpoint.clear_calls
        )
    );
    let removed = remove_identity(&mut storage, &mut endpoint, "Saved", true).unwrap();
    assert_eq!(removed.identity.id, identity.id);
    assert!(removed.binding.is_none());
    assert_eq!(
        state_counts(storage.connection().unwrap(), &identity.id),
        (0, 0, 1, 1, 1, 1)
    );
    assert!(storage.find_identity("saved").unwrap().is_none());
    let tombstone: Option<i64> = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT retired_at_ms FROM identities WHERE id = ?",
            [&identity.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(tombstone.is_some());
    storage.close().unwrap();
}

/// Moving replaces only the binding: even a temporary identity keeps its
/// UUID, lifetime, preferences and durable exchange/profile state.
#[test]
fn moving_an_identity_preserves_its_state_and_fences_the_captured_records() {
    use tmt_core::{
        binding::{
            self, BindingRepository, BindingTargetEvidence,
            session::{
                HarnessId, ObservedSessionKey, ProviderSessionId, RuntimeLiveness, RuntimeMode,
                SessionPreferences, SessionTransition,
            },
        },
        endpoint::ProcessIncarnation,
    };
    for saved in [false, true] {
        for case in [
            "move",
            "settled",
            "settled-key",
            "settled-owner",
            "settled-unknown",
            "settled-transition",
            "preferences",
            "binding",
            "occupied",
            "partial",
            "concurrent-preferences",
            "concurrent-binding-change",
        ] {
            let fixture = Fixture::new();
            let mut storage = fixture.open();
            let mut endpoint = FakeEndpoint::new(&["%1", "%2"]);
            let bound =
                binding::bind_identity(&mut storage, &mut endpoint, "%1", "Alice", saved).unwrap();
            let identity = bound.identity;
            seed_state(storage.connection().unwrap(), &identity.id);
            let counts = state_counts(storage.connection().unwrap(), &identity.id);
            let process = ProcessIncarnation::new(200, "runtime").unwrap();
            let owner = ProcessIncarnation::new(201, "wrapper").unwrap();
            let mut preferences = SessionPreferences {
                channel: Some(false),
                preferred_harness: Some(HarnessId::new("claude").unwrap()),
                ..Default::default()
            };
            preferences.remember(
                HarnessId::new("claude").unwrap(),
                RuntimeMode::new("default").unwrap(),
                ProviderSessionId::new("s-1").unwrap(),
            );
            storage
                .with_binding_transaction::<_, crate::storage::StorageError>(|records| {
                    let binding = bound.binding.as_ref().unwrap();
                    let next = binding
                        .session
                        .admit_launched(
                            ObservedSessionKey {
                                incarnation: process.clone(),
                                provider_session: Some(ProviderSessionId::new("s-1").unwrap()),
                            },
                            owner.clone(),
                            SessionTransition::Started,
                            RuntimeLiveness::Alive,
                            RuntimeLiveness::Alive,
                        )
                        .unwrap();
                    records.set_session_state(&binding.id, &binding.session, &next)?;
                    records.set_session_preferences(&identity.id, &preferences)
                })
                .unwrap();
            let expected =
                crate::storage::Storage::context_by_identity(&fixture.database, &identity.id, 0)
                    .unwrap()
                    .unwrap();
            let target = BindingTargetEvidence {
                server: endpoint.server.clone(),
                pane_id: "%2".into(),
                pane_pid: 100,
                pane_incarnation: None,
                discovery: None,
            };
            // Deterministically schedule the old wrapper's exit transaction
            // after the move's post-stop snapshot and before its final CAS.
            let captured = expected.entry.binding.as_ref().unwrap();
            let ended = captured
                .session
                .record_launched_exit(captured.session.key.clone().unwrap(), owner.clone())
                .unwrap();
            if case.starts_with("settled") {
                let mut next = ended.clone();
                match case {
                    "settled-key" => {
                        next.key.as_mut().unwrap().provider_session =
                            Some(ProviderSessionId::new("other-session").unwrap())
                    }
                    "settled-owner" => {
                        next.launch_owner =
                            Some(ProcessIncarnation::new(202, "other-wrapper").unwrap())
                    }
                    "settled-unknown" => {
                        next.state = tmt_core::binding::session::RuntimeState::Unknown
                    }
                    "settled-transition" => next.last_transition = Some(SessionTransition::Started),
                    _ => (),
                }
                storage
                    .with_binding_transaction::<_, crate::storage::StorageError>(|records| {
                        assert!(records.set_session_state(
                            &captured.id,
                            &captured.session,
                            &next
                        )?);
                        Ok(())
                    })
                    .unwrap();
            }
            match case {
                "preferences" => {
                    storage
                        .with_binding_transaction::<_, crate::storage::StorageError>(|records| {
                            let mut changed = preferences.clone();
                            changed.channel = Some(true);
                            records.set_session_preferences(&identity.id, &changed)
                        })
                        .unwrap();
                }
                "binding" => {
                    storage
                        .with_binding_transaction::<_, crate::storage::StorageError>(|records| {
                            records.detach_binding(&expected.entry.binding.as_ref().unwrap().id)?;
                            let server = endpoint.server.clone();
                            records.insert_binding(&identity, &server, endpoint.pane_mut("%1"))?;
                            Ok(())
                        })
                        .unwrap();
                }
                "occupied" => {
                    binding::bind_identity(&mut storage, &mut endpoint, "%2", "Bob", true).unwrap();
                }
                "partial" => {
                    endpoint.publish_failure = true;
                }
                "concurrent-binding-change" => {
                    let database = fixture.database.clone();
                    let binding = captured.clone();
                    endpoint.after_publish = Some(Box::new(move || {
                        let mut other = crate::storage::Storage::open(&database).unwrap();
                        other
                            .with_binding_transaction::<_, crate::storage::StorageError>(
                                |records| {
                                    let mut changed = binding.session.clone();
                                    changed.state =
                                        tmt_core::binding::session::RuntimeState::Unknown;
                                    assert!(records.set_session_state(
                                        &binding.id,
                                        &binding.session,
                                        &changed
                                    )?);
                                    Ok(())
                                },
                            )
                            .unwrap();
                        other.close().unwrap();
                    }));
                }
                "concurrent-preferences" => {
                    let database = fixture.database.clone();
                    let identity_id = identity.id.clone();
                    endpoint.after_publish = Some(Box::new(move || {
                        let mut other = crate::storage::Storage::open(&database).unwrap();
                        other
                            .with_binding_transaction::<_, crate::storage::StorageError>(
                                |records| {
                                    let entry = records.entry_by_id(&identity_id)?.unwrap();
                                    assert_eq!(
                                        entry.binding.unwrap().pane_id,
                                        "%1",
                                        "old binding remains owned until target activation"
                                    );
                                    let mut preferences =
                                        records.session_preferences(&identity_id)?;
                                    preferences.channel = Some(true);
                                    records.set_session_preferences(&identity_id, &preferences)
                                },
                            )
                            .unwrap();
                        other.close().unwrap();
                    }));
                }
                _ => (),
            }
            let before =
                crate::storage::Storage::context_by_identity(&fixture.database, &identity.id, 0)
                    .unwrap()
                    .unwrap();
            let clears = endpoint.clear_calls;
            let result = binding::move_identity(
                &mut storage,
                &mut endpoint,
                &expected.entry,
                &expected.preferences,
                &process,
                &target,
            );
            let succeeds = matches!(case, "move" | "settled" | "concurrent-preferences");
            assert_eq!(result.is_ok(), succeeds, "{saved}/{case}: {result:?}");
            let after =
                crate::storage::Storage::context_by_identity(&fixture.database, &identity.id, 0)
                    .unwrap()
                    .unwrap();
            assert_eq!(after.entry.identity, identity);
            let mut expected_preferences = before.preferences.clone();
            if case == "concurrent-preferences" {
                expected_preferences.channel = Some(true);
            }
            assert_eq!(after.preferences, expected_preferences, "{case}");
            assert_eq!(
                state_counts(storage.connection().unwrap(), &identity.id),
                counts,
                "{case}"
            );
            if succeeds {
                let binding = after.entry.binding.as_ref().unwrap();
                assert_eq!(binding.pane_id, "%2");
                assert_ne!(binding.id, captured.id);
                // A still-later exit settlement cannot touch the new binding.
                storage
                    .with_binding_transaction::<_, crate::storage::StorageError>(|records| {
                        assert!(!records.set_session_state(
                            &captured.id,
                            &captured.session,
                            &ended
                        )?);
                        Ok(())
                    })
                    .unwrap();
                let settled = crate::storage::Storage::context_by_identity(
                    &fixture.database,
                    &identity.id,
                    0,
                )
                .unwrap()
                .unwrap();
                assert_eq!(settled.entry, after.entry);
                assert_eq!(settled.preferences, after.preferences);
                assert!(endpoint.pane_mut("%1").marker.is_none());
                assert_eq!(
                    endpoint.pane_mut("%2").marker.as_ref().unwrap().identity_id,
                    identity.id
                );
            } else {
                if case == "concurrent-binding-change" {
                    assert_eq!(after.entry.binding.as_ref().unwrap().id, captured.id);
                    assert_eq!(endpoint.clear_calls, clears + 1);
                    assert!(endpoint.pane_mut("%2").marker.is_none());
                    assert_eq!(
                        endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
                        captured.id
                    );
                } else {
                    assert_eq!(after.entry, before.entry, "{case}");
                }
                if case == "partial" {
                    assert!(matches!(result, Err(BindingError::Partial(_))));
                    assert_eq!(
                        endpoint.clear_calls, clears,
                        "failed target publication must not clear the old pane"
                    );
                    assert_eq!(
                        endpoint.pane_mut("%1").marker.as_ref().unwrap().binding_id,
                        captured.id
                    );
                } else if case != "concurrent-binding-change" {
                    assert_eq!(endpoint.clear_calls, clears, "{case}");
                }
            }
        }
    }
}

#[test]
fn an_identity_uuid_removes_the_identity_and_a_retired_uuid_is_not_found() {
    let fixture = Fixture::new();
    let mut storage = fixture.open();
    let mut endpoint = FakeEndpoint::new(&["%1"]);
    let alice = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;

    assert!(matches!(
        remove_identity(&mut storage, &mut endpoint, &alice.id, false),
        Err(BindingError::ConfirmationRequired)
    ));
    let removed = remove_identity(&mut storage, &mut endpoint, &alice.id, true).unwrap();
    assert_eq!(removed.identity.id, alice.id);
    assert_eq!(storage.find_identity("alice").unwrap(), None);

    match remove_identity(&mut storage, &mut endpoint, &alice.id, true) {
        Err(BindingError::NameNotFound(named)) => assert_eq!(named, alice.id),
        other => panic!("{other:?}"),
    }
    storage.close().unwrap();
}
