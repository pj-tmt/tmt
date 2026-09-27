//! Binding-transaction-owned runtime observations and identity preferences.

use rusqlite::{Connection, OptionalExtension, Row, params};
use tmt_core::binding::session::{
    BindingSessionState, HarnessId, ObservedSessionKey, ProviderSessionId, RememberedSession,
    RuntimeIncarnation, RuntimeMode, RuntimeState, SessionPreferences, SessionTransition,
};

use super::super::{StorageError, errors::classify};

pub(super) fn decode_state(row: &Row<'_>, offset: usize) -> rusqlite::Result<BindingSessionState> {
    let state = match row.get::<_, String>(offset)?.as_str() {
        "unknown" => RuntimeState::Unknown,
        "running" => RuntimeState::Running,
        "ended" => RuntimeState::Ended,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let last_transition = row
        .get::<_, Option<String>>(offset + 1)?
        .map(|value| match value.as_str() {
            "started" => Ok(SessionTransition::Started),
            "resumed" => Ok(SessionTransition::Resumed),
            "cleared" => Ok(SessionTransition::Cleared),
            "compacted" => Ok(SessionTransition::Compacted),
            "forked" => Ok(SessionTransition::Forked),
            "ended" => Ok(SessionTransition::Ended),
            _ => Err(rusqlite::Error::InvalidQuery),
        })
        .transpose()?;
    let key = match (
        row.get::<_, Option<i64>>(offset + 2)?,
        row.get::<_, Option<String>>(offset + 3)?,
        row.get::<_, Option<String>>(offset + 4)?,
    ) {
        (None, None, None) => None,
        (Some(pid), Some(start), session) => Some(ObservedSessionKey {
            incarnation: RuntimeIncarnation::new(
                u64::try_from(pid).map_err(|_| rusqlite::Error::InvalidQuery)?,
                &start,
            )
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
            provider_session: session
                .map(|value| {
                    ProviderSessionId::new(&value).map_err(|_| rusqlite::Error::InvalidQuery)
                })
                .transpose()?,
        }),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(BindingSessionState {
        last_transition,
        state,
        key,
    })
}

pub(super) fn preferences(
    connection: &Connection,
    identity_id: &str,
) -> Result<SessionPreferences, StorageError> {
    connection.query_row(
        "SELECT p.preferred_harness, p.remembered_harness, p.runtime_mode, p.provider_session_id FROM identity_session_preferences p
         JOIN identities i ON i.id = p.identity_id WHERE p.identity_id = ? AND i.retired_at_ms IS NULL",
        [identity_id],
        |row| {
            let preferred_harness = row.get::<_, Option<String>>(0)?
                .map(|value| HarnessId::new(&value).map_err(|_| rusqlite::Error::InvalidQuery))
                .transpose()?;
            let remembered = match (row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?) {
                (None, None, None) => None,
                (Some(harness), Some(mode), Some(session)) => Some(RememberedSession {
                    harness: HarnessId::new(&harness).map_err(|_| rusqlite::Error::InvalidQuery)?,
                    mode: RuntimeMode::new(&mode).map_err(|_| rusqlite::Error::InvalidQuery)?,
                    provider_session: ProviderSessionId::new(&session).map_err(|_| rusqlite::Error::InvalidQuery)?,
                }),
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            Ok(SessionPreferences { preferred_harness, remembered })
        },
    ).optional().map(|value| value.unwrap_or_default())
        .map_err(|error| classify(error, "Read session preferences"))
}

pub(super) fn set_preferences(
    connection: &Connection,
    identity_id: &str,
    value: &SessionPreferences,
) -> Result<bool, StorageError> {
    connection
        .execute(
            "INSERT INTO identity_session_preferences (identity_id, preferred_harness, remembered_harness, runtime_mode, provider_session_id)
         SELECT id, ?, ?, ?, ? FROM identities WHERE id = ? AND retired_at_ms IS NULL
         ON CONFLICT(identity_id) DO UPDATE SET preferred_harness = excluded.preferred_harness,
             remembered_harness = excluded.remembered_harness, runtime_mode = excluded.runtime_mode,
             provider_session_id = excluded.provider_session_id",
            params![
                value.preferred_harness.as_ref().map(HarnessId::as_str),
                value.remembered.as_ref().map(|session| session.harness.as_str()),
                value.remembered.as_ref().map(|session| session.mode.as_str()),
                value.remembered.as_ref().map(|session| session.provider_session.as_str()),
                identity_id
            ],
        )
        .map(|changed| changed == 1)
        .map_err(|error| classify(error, "Write session preferences"))
}

pub(super) fn set_state(
    connection: &Connection,
    binding_id: &str,
    expected: &BindingSessionState,
    value: &BindingSessionState,
) -> Result<bool, StorageError> {
    let stored_pid = |value: &BindingSessionState| {
        value
            .key
            .as_ref()
            .map(|key| super::stored_process_id(key.incarnation.pid()))
            .transpose()
            .map_err(|error| classify(error, "Validate runtime PID"))
    };
    let next_pid = stored_pid(value)?;
    let expected_pid = stored_pid(expected)?;
    let state = |value| match value {
        RuntimeState::Unknown => "unknown",
        RuntimeState::Running => "running",
        RuntimeState::Ended => "ended",
    };
    let transition = |value: Option<SessionTransition>| {
        value.map(|transition| match transition {
            SessionTransition::Started => "started",
            SessionTransition::Resumed => "resumed",
            SessionTransition::Cleared => "cleared",
            SessionTransition::Compacted => "compacted",
            SessionTransition::Forked => "forked",
            SessionTransition::Ended => "ended",
        })
    };
    connection.execute(
        "UPDATE bindings SET runtime_state = ?, last_transition = ?, runtime_pid = ?, runtime_start_identity = ?, observed_provider_session_id = ?
         WHERE id = ? AND runtime_state = ? AND last_transition IS ? AND runtime_pid IS ? AND runtime_start_identity IS ? AND observed_provider_session_id IS ?
         AND EXISTS (SELECT 1 FROM identities i WHERE i.id = bindings.identity_id AND i.retired_at_ms IS NULL)",
        params![state(value.state), transition(value.last_transition),
            next_pid,
            value.key.as_ref().map(|key| key.incarnation.start_identity()),
            value.key.as_ref().and_then(|key| key.provider_session.as_ref()).map(ProviderSessionId::as_str),
            binding_id, state(expected.state), transition(expected.last_transition),
            expected_pid,
            expected.key.as_ref().map(|key| key.incarnation.start_identity()),
            expected.key.as_ref().and_then(|key| key.provider_session.as_ref()).map(ProviderSessionId::as_str)],
    ).map(|changed| changed == 1).map_err(|error| classify(error, "Write binding session state"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::bindings::test_support::{Fixture, pane};
    use tmt_core::{
        binding::BindingRepository,
        endpoint::ServerEvidence,
        identity::{Lifetime, create_or_resolve},
    };

    fn server() -> ServerEvidence {
        ServerEvidence {
            server_id: "11111111-1111-4111-8111-111111111111".into(),
            socket_path: "/tmp/session-fixture.sock".into(),
            server_pid: 100,
            server_start_time: "fixture-start".into(),
        }
    }

    fn remembered() -> SessionPreferences {
        SessionPreferences {
            preferred_harness: Some(HarnessId::new("claude").unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new("claude").unwrap(),
                mode: RuntimeMode::new("default").unwrap(),
                provider_session: ProviderSessionId::new("saved-session").unwrap(),
            }),
        }
    }

    #[test]
    fn changing_preferred_harness_does_not_relabel_a_remembered_session() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Preferences", Lifetime::Saved)
            .unwrap()
            .identity;
        let mut preferences = remembered();
        preferences.preferred_harness = Some(HarnessId::new("codex").unwrap());
        storage
            .with_binding_transaction(|records| {
                assert!(records.set_session_preferences(&identity.id, &preferences)?);
                let restored = records.session_preferences(&identity.id)?;
                assert_eq!(restored, preferences);
                let session = restored.remembered.unwrap();
                assert_eq!(session.harness.as_str(), "claude");
                assert_eq!(session.mode.as_str(), "default");
                assert_eq!(session.provider_session.as_str(), "saved-session");
                Ok::<_, StorageError>(())
            })
            .unwrap();
        let connection = Connection::open(&fixture.database).unwrap();
        assert!(connection.execute("UPDATE identity_session_preferences SET remembered_harness = NULL WHERE identity_id = ?", [&identity.id]).is_err());
        storage
            .with_binding_transaction(|records| {
                assert_eq!(records.session_preferences(&identity.id)?, preferences);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        storage.close().unwrap();
    }

    #[test]
    fn session_compare_and_set_rejects_stale_events_on_the_same_binding() {
        use tmt_core::binding::session::RuntimeLiveness;
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Events", Lifetime::Saved)
            .unwrap()
            .identity;
        let initial = BindingSessionState::default();
        let key = ObservedSessionKey {
            incarnation: RuntimeIncarnation::new(123, "start-a").unwrap(),
            provider_session: Some(ProviderSessionId::new("session-a").unwrap()),
        };
        let running = initial
            .admit(
                key.clone(),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        let cleared = running
            .transition(
                &key,
                SessionTransition::Cleared,
                Some(ProviderSessionId::new("session-b").unwrap()),
            )
            .unwrap();
        let binding = storage
            .with_binding_transaction(|records| {
                let binding = records.insert_binding(&identity, &server(), &pane("%4", 104))?;
                assert!(records.set_session_state(&binding.id, &initial, &running)?);
                assert!(records.set_session_state(&binding.id, &running, &cleared)?);
                Ok::<_, StorageError>(binding)
            })
            .unwrap();
        storage.close().unwrap();
        let mut storage = fixture.open();
        storage
            .with_binding_transaction(|records| {
                let stale_end = running
                    .transition(&key, SessionTransition::Ended, None)
                    .unwrap();
                assert!(!records.set_session_state(&binding.id, &running, &stale_end)?);
                assert!(!records.set_session_state(&binding.id, &initial, &running)?);
                assert_eq!(
                    records
                        .entry_by_id(&identity.id)?
                        .unwrap()
                        .binding
                        .unwrap()
                        .session,
                    cleared
                );
                let ended = cleared
                    .transition(
                        cleared.key.as_ref().unwrap(),
                        SessionTransition::Ended,
                        None,
                    )
                    .unwrap();
                assert!(records.set_session_state(&binding.id, &cleared, &ended)?);
                assert_eq!(
                    records
                        .entry_by_id(&identity.id)?
                        .unwrap()
                        .binding
                        .unwrap()
                        .session,
                    ended
                );
                Ok::<_, StorageError>(())
            })
            .unwrap();
        storage.close().unwrap();
    }

    #[test]
    fn preferences_survive_reopen_and_rebind_but_observations_do_not_move() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Saved", Lifetime::Saved)
            .unwrap()
            .identity;
        let binding = storage
            .with_binding_transaction(|records| {
                let binding = records.insert_binding(&identity, &server(), &pane("%1", 101))?;
                assert_eq!(binding.session, BindingSessionState::default());
                assert!(records.set_session_preferences(&identity.id, &remembered())?);
                assert!(records.set_session_state(
                    &binding.id,
                    &BindingSessionState::default(),
                    &BindingSessionState {
                        state: RuntimeState::Running,
                        last_transition: Some(SessionTransition::Started),
                        key: Some(ObservedSessionKey {
                            incarnation: RuntimeIncarnation::new(101, "runtime-start").unwrap(),
                            provider_session: None,
                        }),
                    }
                )?);
                Ok::<_, StorageError>(binding)
            })
            .unwrap();
        storage.close().unwrap();
        let mut storage = fixture.open();
        storage
            .with_binding_transaction(|records| {
                assert_eq!(records.session_preferences(&identity.id)?, remembered());
                let observed = records.entry_by_id(&identity.id)?.unwrap().binding.unwrap();
                assert_eq!(observed.session.state, RuntimeState::Running);
                records.touch_binding(&binding.id)?;
                assert_eq!(
                    records
                        .entry_by_id(&identity.id)?
                        .unwrap()
                        .binding
                        .unwrap()
                        .session,
                    observed.session
                );
                records.detach_binding(&binding.id)?;
                let replacement = records.insert_binding(&identity, &server(), &pane("%2", 102))?;
                assert_ne!(replacement.id, binding.id);
                assert_eq!(replacement.session, BindingSessionState::default());
                assert_eq!(records.session_preferences(&identity.id)?, remembered());
                // A delayed observation for the removed binding cannot update its replacement.
                assert!(!records.set_session_state(
                    &binding.id,
                    &BindingSessionState::default(),
                    &observed.session
                )?);
                assert_eq!(
                    records.entry_by_id(&identity.id)?.unwrap().binding.unwrap(),
                    replacement
                );
                Ok::<_, StorageError>(())
            })
            .unwrap();
        storage.close().unwrap();
    }

    #[test]
    fn retirement_hides_preferences_without_deleting_or_resurrecting_them() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Temporary", Lifetime::Temporary)
            .unwrap()
            .identity;
        storage
            .with_binding_transaction(|records| {
                assert!(records.set_session_preferences(&identity.id, &remembered())?);
                records.retire_identity(&identity, false)?;
                assert_eq!(
                    records.session_preferences(&identity.id)?,
                    SessionPreferences::default()
                );
                assert!(
                    !records
                        .set_session_preferences(&identity.id, &SessionPreferences::default())?
                );
                Ok::<_, StorageError>(())
            })
            .unwrap();
        let replacement = create_or_resolve(&mut storage, "Temporary", Lifetime::Temporary)
            .unwrap()
            .identity;
        assert_ne!(replacement.id, identity.id);
        storage
            .with_binding_transaction(|records| {
                assert_eq!(
                    records.session_preferences(&replacement.id)?,
                    SessionPreferences::default()
                );
                Ok::<_, StorageError>(())
            })
            .unwrap();
        let connection = Connection::open(&fixture.database).unwrap();
        let retained: String = connection.query_row(
            "SELECT provider_session_id FROM identity_session_preferences WHERE identity_id = ?",
            [&identity.id], |row| row.get(0),
        ).unwrap();
        assert_eq!(retained, "saved-session");
        storage.close().unwrap();
    }

    #[test]
    fn preferences_and_observations_roll_back_with_the_existing_transaction() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Rollback", Lifetime::Saved)
            .unwrap()
            .identity;
        let binding = storage
            .with_binding_transaction(|records| {
                records.insert_binding(&identity, &server(), &pane("%3", 103))
            })
            .unwrap();
        let result = storage.with_binding_transaction(|records| {
            records.set_session_preferences(&identity.id, &remembered())?;
            records.set_session_state(
                &binding.id,
                &BindingSessionState::default(),
                &BindingSessionState {
                    state: RuntimeState::Ended,
                    last_transition: Some(SessionTransition::Ended),
                    key: Some(ObservedSessionKey {
                        incarnation: RuntimeIncarnation::new(103, "runtime-start").unwrap(),
                        provider_session: None,
                    }),
                },
            )?;
            Err::<(), _>(StorageError::new(
                crate::storage::StorageErrorCode::Unknown,
                "fixture rollback",
            ))
        });
        assert_eq!(result.unwrap_err().message, "fixture rollback");
        storage
            .with_binding_transaction(|records| {
                assert_eq!(
                    records.session_preferences(&identity.id)?,
                    SessionPreferences::default()
                );
                assert_eq!(
                    records.entry_by_id(&identity.id)?.unwrap().binding.unwrap(),
                    binding
                );
                Ok::<_, StorageError>(())
            })
            .unwrap();
        storage.close().unwrap();
    }
}
