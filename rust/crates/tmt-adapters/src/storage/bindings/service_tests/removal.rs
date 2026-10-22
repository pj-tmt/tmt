use rusqlite::{Connection, params, types::Value};
use tmt_core::{
    binding::{BindingError, remove_identity, unbind_identity},
    identity::{IdentityReader, Lifetime, create_or_resolve},
};

use super::super::test_support::Fixture;
use super::endpoint::{EndpointFailure, FakeEndpoint, ProbeMode};

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
fn remove_unknown_or_unclearable_binding_fails_closed_without_mutating_state() {
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
    let error = remove_identity(&mut storage, &mut endpoint, "Alice", true).unwrap_err();
    assert!(matches!(error, BindingError::Unverified));
    assert_eq!(endpoint.clear_calls, 0);
    assert_eq!(
        binding_rows(storage.connection().unwrap(), &identity.id),
        rows
    );
    assert_eq!(
        state_counts(storage.connection().unwrap(), &identity.id),
        counts
    );

    endpoint.probe_mode = ProbeMode::Live;
    endpoint.clear_result = false;
    let error = remove_identity(&mut storage, &mut endpoint, "Alice", true).unwrap_err();
    assert!(matches!(error, BindingError::Unverified));
    assert_eq!(
        binding_rows(storage.connection().unwrap(), &identity.id),
        rows
    );
    assert_eq!(
        state_counts(storage.connection().unwrap(), &identity.id),
        counts
    );

    endpoint.clear_result = true;
    endpoint.clear_failure = true;
    let error = remove_identity(&mut storage, &mut endpoint, "Alice", true).unwrap_err();
    assert!(matches!(
        error,
        BindingError::Endpoint(EndpointFailure("clear failed"))
    ));
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
            let succeeds = matches!(case, "move" | "settled");
            assert_eq!(result.is_ok(), succeeds, "{saved}/{case}: {result:?}");
            let after =
                crate::storage::Storage::context_by_identity(&fixture.database, &identity.id, 0)
                    .unwrap()
                    .unwrap();
            assert_eq!(after.entry.identity, identity);
            assert_eq!(after.preferences, before.preferences, "{case}");
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
                assert_eq!(after.entry, before.entry, "{case}");
                if case == "partial" {
                    assert!(matches!(result, Err(BindingError::Partial(_))));
                } else {
                    assert_eq!(endpoint.clear_calls, clears, "{case}");
                }
            }
        }
    }
}
