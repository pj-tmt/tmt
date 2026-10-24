//! Binding-transaction-owned runtime observations and identity preferences.

use rusqlite::{Connection, OptionalExtension, Row, params};
use tmt_core::binding::session::{
    BindingSessionState, DriverState, HarnessId, ObservedSessionKey, ProviderSessionId,
    RememberedSession, RetainedSession, RuntimeMode, RuntimeState, SessionPreferences,
    SessionTransition, StartRefusal, StartRefusalReason,
};
use tmt_core::endpoint::ProcessIncarnation;

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
    let launch_owner = match (
        row.get::<_, Option<i64>>(offset + 5)?,
        row.get::<_, Option<String>>(offset + 6)?,
    ) {
        (None, None) => None,
        (Some(pid), Some(start)) => Some(
            ProcessIncarnation::new(
                u64::try_from(pid).map_err(|_| rusqlite::Error::InvalidQuery)?,
                &start,
            )
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        ),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let key = match (
        row.get::<_, Option<i64>>(offset + 2)?,
        row.get::<_, Option<String>>(offset + 3)?,
        row.get::<_, Option<String>>(offset + 4)?,
    ) {
        (None, None, None) if launch_owner.is_none() => None,
        (Some(pid), Some(start), session) => Some(ObservedSessionKey {
            incarnation: ProcessIncarnation::new(
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
        start_refusal: decode_refusal(row.get(offset + 9)?)?,
        launch_owner,
        last_transition,
        state,
        key,
    })
}

fn encode_refusal(value: Option<&StartRefusal>) -> Option<String> {
    value.map(|value| {
        serde_json::json!({"driver": value.harness.as_str(),
        "session": value.session.as_str(), "reason": value.reason.as_str(),
        "atMs": value.observed_at_ms})
        .to_string()
    })
}

fn decode_refusal(value: Option<String>) -> rusqlite::Result<Option<StartRefusal>> {
    value
        .map(|value| {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Wire {
                driver: String,
                session: String,
                reason: String,
                at_ms: u64,
            }
            let invalid = || rusqlite::Error::InvalidQuery;
            let wire: Wire = serde_json::from_str(&value).map_err(|_| invalid())?;
            if wire.at_ms > tmt_core::limits::MAX_JS_SAFE_INTEGER {
                return Err(invalid());
            }
            Ok(StartRefusal {
                harness: HarnessId::new(&wire.driver).map_err(|_| invalid())?,
                session: ProviderSessionId::new(&wire.session).map_err(|_| invalid())?,
                reason: StartRefusalReason::parse(&wire.reason).ok_or_else(invalid)?,
                observed_at_ms: wire.at_ms,
            })
        })
        .transpose()
}

pub(super) fn preferences(
    connection: &Connection,
    identity_id: &str,
) -> Result<SessionPreferences, StorageError> {
    if connection.is_autocommit() {
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| classify(error, "Begin session preference snapshot"))?;
        let value = preferences(&transaction, identity_id)?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Close session preference snapshot"))?;
        return Ok(value);
    }
    let mut preferences = connection.query_row(
        "SELECT p.preferred_harness, p.remembered_harness, p.runtime_mode, p.provider_session_id,
                p.driver_state_version, p.driver_state, p.stale_at_ms, p.resume_pending_at_ms, p.channel FROM identity_session_preferences p
         JOIN identities i ON i.id = p.identity_id WHERE p.identity_id = ? AND i.retired_at_ms IS NULL",
        [identity_id],
        |row| decode_preferences(row, 0),
    ).optional().map(|value| value.unwrap_or_default())
        .map_err(|error| classify(error, "Read session preferences"))?;
    let mut query = connection
        .prepare(
            "SELECT harness, harness, runtime_mode, provider_session_id, driver_state_version,
         driver_state, stale_at_ms, resume_pending_at_ms, channel FROM identity_resume_records r JOIN identities i ON i.id = r.identity_id
         WHERE r.identity_id = ? AND i.retired_at_ms IS NULL ORDER BY harness",
        )
        .map_err(|error| classify(error, "Read retained driver sessions"))?;
    preferences.retained = query
        .query_map([identity_id], |row| {
            let value = decode_preferences(row, 0)?;
            Ok(RetainedSession {
                session: value.remembered.ok_or(rusqlite::Error::InvalidQuery)?,
                channel: value.channel,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|error| classify(error, "Decode retained driver sessions"))?;
    Ok(preferences)
}

pub(super) fn decode_preferences(
    row: &Row<'_>,
    offset: usize,
) -> rusqlite::Result<SessionPreferences> {
    let invalid = |_| rusqlite::Error::InvalidQuery;
    let preferred_harness = row
        .get::<_, Option<String>>(offset)?
        .map(|value| HarnessId::new(&value).map_err(invalid))
        .transpose()?;
    let remembered = match (
        row.get::<_, Option<String>>(offset + 1)?,
        row.get::<_, Option<String>>(offset + 2)?,
        row.get::<_, Option<String>>(offset + 3)?,
    ) {
        (None, None, None) => None,
        (Some(harness), Some(mode), Some(session)) => Some(RememberedSession {
            harness: HarnessId::new(&harness).map_err(invalid)?,
            mode: RuntimeMode::new(&mode).map_err(invalid)?,
            provider_session: ProviderSessionId::new(&session).map_err(invalid)?,
            state: match (
                row.get::<_, Option<u16>>(offset + 4)?,
                row.get::<_, Option<String>>(offset + 5)?,
            ) {
                (None, None) => None,
                (Some(version), Some(document)) => {
                    Some(DriverState::new(version, &document).map_err(invalid)?)
                }
                _ => return Err(rusqlite::Error::InvalidQuery),
            },
            stale_at_ms: time(row, offset + 6)?,
            resume_pending_at_ms: time(row, offset + 7)?,
        }),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(SessionPreferences {
        retained: Vec::new(),
        preferred_harness,
        remembered,
        channel: row.get(offset + 8)?,
    })
}

fn time(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()
}

fn stored_time(value: Option<u64>) -> Result<Option<i64>, StorageError> {
    value
        .map(|value| i64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()
        .map_err(|error| classify(error, "Validate session time"))
}

pub(super) fn set_preferences(
    connection: &Connection,
    identity_id: &str,
    value: &SessionPreferences,
) -> Result<bool, StorageError> {
    if value.remembered.as_ref().is_some_and(|active| {
        value
            .retained
            .iter()
            .any(|record| record.session.harness == active.harness)
    }) {
        return Err(classify(
            rusqlite::Error::InvalidQuery,
            "Duplicate current driver session",
        ));
    }
    let retained_changed = preferences(connection, identity_id)?.retained != value.retained;
    let remembered = value.remembered.as_ref();
    let state = remembered.and_then(|session| session.state.as_ref());
    let stale_at_ms = stored_time(remembered.and_then(|session| session.stale_at_ms))?;
    let resume_pending_at_ms =
        stored_time(remembered.and_then(|session| session.resume_pending_at_ms))?;
    let changed = connection
        .execute(
            "INSERT INTO identity_session_preferences (identity_id, preferred_harness, remembered_harness, runtime_mode,
             provider_session_id, driver_state_version, driver_state, stale_at_ms, resume_pending_at_ms, channel)
         SELECT id, ?, ?, ?, ?, ?, ?, ?, ?, ? FROM identities WHERE id = ? AND retired_at_ms IS NULL
         ON CONFLICT(identity_id) DO UPDATE SET preferred_harness = excluded.preferred_harness,
             remembered_harness = excluded.remembered_harness, runtime_mode = excluded.runtime_mode,
             provider_session_id = excluded.provider_session_id,
             driver_state_version = excluded.driver_state_version, driver_state = excluded.driver_state,
             stale_at_ms = excluded.stale_at_ms, resume_pending_at_ms = excluded.resume_pending_at_ms, channel = excluded.channel",
            params![
                value.preferred_harness.as_ref().map(HarnessId::as_str),
                remembered.map(|session| session.harness.as_str()),
                remembered.map(|session| session.mode.as_str()),
                remembered.map(|session| session.provider_session.as_str()),
                state.map(DriverState::version),
                state.map(DriverState::document),
                stale_at_ms,
                resume_pending_at_ms,
                value.channel,
                identity_id
            ],
        )
        .map(|changed| changed == 1)
        .map_err(|error| classify(error, "Write session preferences"))?;
    if changed && retained_changed {
        connection
            .execute(
                "DELETE FROM identity_resume_records WHERE identity_id = ?",
                [identity_id],
            )
            .map_err(|error| classify(error, "Replace retained driver sessions"))?;
        for record in &value.retained {
            let session = &record.session;
            let state = session.state.as_ref();
            connection.execute("INSERT INTO identity_resume_records
                (identity_id,harness,runtime_mode,provider_session_id,driver_state_version,driver_state,stale_at_ms,resume_pending_at_ms,channel)
                VALUES (?,?,?,?,?,?,?,?,?)", params![identity_id,session.harness.as_str(),session.mode.as_str(),
                    session.provider_session.as_str(),state.map(DriverState::version),state.map(DriverState::document),
                    stored_time(session.stale_at_ms)?,stored_time(session.resume_pending_at_ms)?,record.channel])
                .map_err(|error| classify(error, "Write retained driver session"))?;
        }
    }
    Ok(changed)
}

/// Clears the remembered session and its driver state. Retirement calls this
/// in the retiring transaction, whatever the lifetime.
pub(super) fn forget(connection: &Connection, identity_id: &str) -> Result<(), StorageError> {
    connection
        .execute(
            "DELETE FROM identity_resume_records WHERE identity_id = ?",
            [identity_id],
        )
        .map_err(|error| classify(error, "Clear retired driver sessions"))?;
    connection
        .execute(
            "UPDATE identity_session_preferences SET remembered_harness = NULL, runtime_mode = NULL,
             provider_session_id = NULL, driver_state_version = NULL, driver_state = NULL, stale_at_ms = NULL,
             resume_pending_at_ms = NULL, channel = NULL WHERE identity_id = ?",
            [identity_id],
        )
        .map(drop)
        .map_err(|error| classify(error, "Clear retired runtime session"))
}

pub(super) fn purge_unregistered(
    connection: &Connection,
    registered: &[&str],
) -> Result<Vec<super::PurgedSession>, StorageError> {
    let orphaned = {
        let mut statement = connection
            .prepare(
                "SELECT p.identity_id, i.name, p.remembered_harness FROM identity_session_preferences p
                 JOIN identities i ON i.id = p.identity_id
                 WHERE p.remembered_harness IS NOT NULL
                 UNION SELECT r.identity_id, i.name, r.harness FROM identity_resume_records r
                 JOIN identities i ON i.id = r.identity_id ORDER BY 1, 3",
            )
            .map_err(|error| classify(error, "Find remembered sessions"))?;
        statement
            .query_map([], |row| {
                Ok(super::PurgedSession {
                    identity_id: row.get(0)?,
                    name: row.get(1)?,
                    harness: row.get(2)?,
                })
            })
            .map_err(|error| classify(error, "Read remembered sessions"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| classify(error, "Decode remembered sessions"))?
            .into_iter()
            .filter(|purged| !registered.contains(&purged.harness.as_str()))
            .collect::<Vec<_>>()
    };
    let identities = orphaned
        .iter()
        .map(|item| item.identity_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for identity in identities {
        let mut value = preferences(connection, identity)?;
        if value
            .remembered
            .as_ref()
            .is_some_and(|session| !registered.contains(&session.harness.as_str()))
        {
            value.remembered = None;
            value.channel = None;
        }
        value
            .retained
            .retain(|record| registered.contains(&record.session.harness.as_str()));
        set_preferences(connection, identity, &value)?;
    }
    Ok(orphaned)
}

pub(super) fn set_state(
    connection: &Connection,
    binding_id: &str,
    expected: &BindingSessionState,
    value: &BindingSessionState,
) -> Result<bool, StorageError> {
    for state in [expected, value] {
        if state
            .start_refusal
            .as_ref()
            .is_some_and(|v| v.observed_at_ms > tmt_core::limits::MAX_JS_SAFE_INTEGER)
        {
            return Err(classify(
                rusqlite::Error::InvalidQuery,
                "Validate runtime start observation time",
            ));
        }
    }
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
    let owner_pid = |value: &BindingSessionState| {
        value
            .launch_owner
            .as_ref()
            .map(|owner| super::stored_process_id(owner.pid()))
            .transpose()
            .map_err(|error| classify(error, "Validate launch owner PID"))
    };
    let next_owner_pid = owner_pid(value)?;
    let expected_owner_pid = owner_pid(expected)?;
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
    let changed = connection.execute(
        "UPDATE bindings SET runtime_state = ?, last_transition = ?, runtime_pid = ?, runtime_start_identity = ?, observed_provider_session_id = ?, launch_owner_pid = ?, launch_owner_start_identity = ?, runtime_start_refusal = ?
         WHERE id = ? AND runtime_state = ? AND last_transition IS ? AND runtime_pid IS ? AND runtime_start_identity IS ? AND observed_provider_session_id IS ? AND launch_owner_pid IS ? AND launch_owner_start_identity IS ? AND runtime_start_refusal IS ?
         AND EXISTS (SELECT 1 FROM identities i WHERE i.id = bindings.identity_id AND i.retired_at_ms IS NULL)",
        params![state(value.state), transition(value.last_transition),
            next_pid,
            value.key.as_ref().map(|key| key.incarnation.start_identity()),
            value.key.as_ref().and_then(|key| key.provider_session.as_ref()).map(ProviderSessionId::as_str),
            next_owner_pid,
            value.launch_owner.as_ref().map(ProcessIncarnation::start_identity),
            encode_refusal(value.start_refusal.as_ref()),
            binding_id, state(expected.state), transition(expected.last_transition),
            expected_pid,
            expected.key.as_ref().map(|key| key.incarnation.start_identity()),
            expected.key.as_ref().and_then(|key| key.provider_session.as_ref()).map(ProviderSessionId::as_str),
            expected_owner_pid,
            expected.launch_owner.as_ref().map(ProcessIncarnation::start_identity),
            encode_refusal(expected.start_refusal.as_ref())],
    ).map_err(|error| classify(error, "Write binding session state"))? == 1;
    // A Stop continuation belongs to the provider process and session that was running. Once
    // either ends or is replaced (SessionEnd, a crash seen as a process fact, a clear or a new
    // launch), no Stop will ever close it, so it must not keep the seat reading as busy (#4858).
    if changed && (value.state != RuntimeState::Running || run_replaced(expected, value)) {
        end_continuation_of_binding(connection, binding_id)?;
    }
    Ok(changed)
}

/// A different process, or a different provider session than one already known. A thread ID
/// disclosed late is the same run.
fn run_replaced(expected: &BindingSessionState, value: &BindingSessionState) -> bool {
    match (&expected.key, &value.key) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(old), Some(new)) => {
            old.incarnation != new.incarnation
                || (old.provider_session.is_some() && old.provider_session != new.provider_session)
        }
    }
}

/// The run a binding's Digest continuation belonged to is over.
pub(super) fn end_continuation_of_binding(
    connection: &Connection,
    binding_id: &str,
) -> Result<(), StorageError> {
    let identity = connection
        .query_row(
            "SELECT identity_id FROM bindings WHERE id = ?",
            [binding_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| classify(error, "Read the binding of a Stop continuation"))?;
    identity.map_or(Ok(()), |identity| {
        end_continuation_of_identity(connection, &identity)
    })
}

pub(super) fn end_continuation_of_identity(
    connection: &Connection,
    identity_id: &str,
) -> Result<(), StorageError> {
    super::super::requests::end_digest_continuation(
        connection,
        identity_id,
        crate::request_runtime::wall_time_ms(),
    )
    .map(|_| ())
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
            host: tmt_core::host::HostKind::Tmux,
            server_id: "11111111-1111-4111-8111-111111111111".into(),
            socket_path: "/tmp/session-fixture.sock".into(),
            server_pid: 100,
            server_start_time: "fixture-start".into(),
        }
    }

    fn remembered() -> SessionPreferences {
        SessionPreferences {
            retained: Vec::new(),
            channel: None,
            preferred_harness: Some(HarnessId::new("claude").unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new("claude").unwrap(),
                mode: RuntimeMode::new("default").unwrap(),
                provider_session: ProviderSessionId::new("saved-session").unwrap(),
                state: None,
                stale_at_ms: None,
                resume_pending_at_ms: None,
            }),
        }
    }

    #[test]
    fn refused_starts_round_trip_fence_stale_cas_and_clear_on_admission_or_rebind() {
        use tmt_core::binding::session::RuntimeLiveness;
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Refused", Lifetime::Saved)
            .unwrap()
            .identity;
        let binding = storage
            .with_binding_transaction(|records| {
                records.insert_binding(&identity, &server(), &pane("%4", 104))
            })
            .unwrap();
        let initial = binding.session.clone();
        let mut refused = initial.clone();
        refused.start_refusal = Some(StartRefusal {
            harness: HarnessId::new("codex").unwrap(),
            session: ProviderSessionId::new("unadmitted-thread").unwrap(),
            reason: StartRefusalReason::DriverMismatch,
            observed_at_ms: 100,
        });
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                assert!(records.set_session_state(&binding.id, &initial, &refused)?);
                assert!(!records.set_session_state(&binding.id, &initial, &initial)?);
                Ok(())
            })
            .unwrap();
        storage.close().unwrap();
        let mut storage = fixture.open();
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                assert_eq!(
                    records
                        .entry_by_id(&identity.id)?
                        .unwrap()
                        .binding
                        .unwrap()
                        .session,
                    refused
                );
                let key = ObservedSessionKey {
                    incarnation: ProcessIncarnation::new(123, "new-process").unwrap(),
                    provider_session: Some(ProviderSessionId::new("admitted").unwrap()),
                };
                let admitted = refused
                    .admit(key, SessionTransition::Started, RuntimeLiveness::Alive)
                    .unwrap();
                assert!(admitted.start_refusal.is_none());
                assert!(records.set_session_state(&binding.id, &refused, &admitted)?);
                assert!(!records.set_session_state(&binding.id, &refused, &refused)?);
                assert!(records.set_session_state(&binding.id, &admitted, &refused)?);
                records.detach_binding(&binding.id)?;
                let rebound = records.insert_binding(&identity, &server(), &pane("%5", 105))?;
                assert!(rebound.session.start_refusal.is_none());
                assert_ne!(rebound.id, binding.id);
                Ok(())
            })
            .unwrap();
        storage.close().unwrap();
    }

    #[test]
    fn inactive_driver_records_survive_reopen_and_a_current_session_clear() {
        let root = crate::test_support::TestDirectory::new();
        let path = root.path.join("drivers.db");
        let mut storage = crate::storage::Storage::open(&path).unwrap();
        let identity = create_or_resolve(&mut storage, "Switcher", Lifetime::Saved)
            .unwrap()
            .identity;
        let mut value = remembered();
        value.channel = Some(true);
        let codex = HarnessId::new("codex").unwrap();
        value.launched(&codex);
        value.remember(
            codex,
            RuntimeMode::new("embedded").unwrap(),
            ProviderSessionId::new("codex-thread").unwrap(),
        );
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                records.set_session_preferences(&identity.id, &value)?;
                Ok(())
            })
            .unwrap();
        storage.close().unwrap();
        let mut storage = crate::storage::Storage::open(&path).unwrap();
        assert_eq!(storage.session_preferences(&identity.id).unwrap(), value);
        value.remembered = None;
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                records.set_session_preferences(&identity.id, &value)?;
                Ok(())
            })
            .unwrap();
        let mut restored = storage.session_preferences(&identity.id).unwrap();
        restored.launched(&HarnessId::new("claude").unwrap());
        assert_eq!(restored.remembered, remembered().remembered);
        assert_eq!(restored.channel, Some(true));
        forget(storage.connection().unwrap(), &identity.id).unwrap();
        assert!(
            storage
                .session_preferences(&identity.id)
                .unwrap()
                .retained
                .is_empty()
        );
        storage.close().unwrap();
    }

    #[test]
    fn unchanged_retained_records_do_not_advance_the_cursor_and_duplicate_writes_roll_back() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Driver records", Lifetime::Saved)
            .unwrap()
            .identity;
        let mut value = remembered();
        value.launched(&HarnessId::new("codex").unwrap());
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                records.set_session_preferences(&identity.id, &value)?;
                Ok(())
            })
            .unwrap();
        let cursor = storage.change_cursor().unwrap();
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                records.set_session_preferences(&identity.id, &value)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(storage.change_cursor().unwrap(), cursor);
        let mut invalid = value.clone();
        invalid.retained.push(invalid.retained[0].clone());
        assert!(
            storage
                .with_binding_transaction::<_, StorageError>(|records| {
                    records.set_session_preferences(&identity.id, &invalid)?;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(storage.session_preferences(&identity.id).unwrap(), value);
        assert_eq!(storage.change_cursor().unwrap(), cursor);
        storage.close().unwrap();
    }

    #[test]
    fn channel_preference_round_trips_and_forget_clears_it() {
        let root = crate::test_support::TestDirectory::new();
        let path = root.path.join("channel.db");
        let mut storage = crate::storage::Storage::open(&path).unwrap();
        let identity = create_or_resolve(&mut storage, "Channel owner", Lifetime::Saved)
            .unwrap()
            .identity;
        for channel in [Some(true), Some(false), None] {
            let mut value = remembered();
            value.channel = channel;
            storage
                .with_binding_transaction::<_, StorageError>(|records| {
                    records.set_session_preferences(&identity.id, &value)?;
                    assert_eq!(records.session_preferences(&identity.id)?, value);
                    Ok(())
                })
                .unwrap();
            storage.close().unwrap();
            storage = crate::storage::Storage::open(&path).unwrap();
            assert_eq!(
                storage.session_preferences(&identity.id).unwrap().channel,
                channel
            );
        }
        let mut value = remembered();
        value.channel = Some(true);
        storage
            .with_binding_transaction::<_, StorageError>(|records| {
                records.set_session_preferences(&identity.id, &value)
            })
            .unwrap();
        forget(storage.connection().unwrap(), &identity.id).unwrap();
        assert_eq!(
            storage.session_preferences(&identity.id).unwrap().channel,
            None
        );
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

    /// A Stop continuation is closed by that run's own Stop. When the run is gone first, nothing
    /// will ever close it, and the seat must not read as busy forever (#4858).
    #[test]
    fn a_stop_continuation_ends_with_the_provider_run_that_started_it() {
        use tmt_core::binding::session::RuntimeLiveness;
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Continuing", Lifetime::Saved)
            .unwrap()
            .identity;
        let key = |pid, start: &str, session: Option<&str>| ObservedSessionKey {
            incarnation: ProcessIncarnation::new(pid, start).unwrap(),
            provider_session: session.map(|id| ProviderSessionId::new(id).unwrap()),
        };
        let run = |key: ObservedSessionKey| {
            BindingSessionState::default()
                .admit(key, SessionTransition::Started, RuntimeLiveness::Alive)
                .unwrap()
        };
        let hand_over = |storage: &crate::storage::Storage| {
            storage
                .connection()
                .unwrap()
                .execute(
                    "INSERT INTO focus_checklists (id,identity_id,attempt_token,through_sequence,state,created_at_ms,opportunity,sealed_count,settled_at_ms)
                     VALUES (lower(hex(randomblob(8))),?1,lower(hex(randomblob(8))),1,'delivered',1000,'turn_boundary',1,1000)",
                    [&identity.id],
                )
                .unwrap();
        };
        let open = |storage: &mut crate::storage::Storage| {
            storage.digest_continuation_open(&identity.id).unwrap()
        };
        let first = run(key(500, "start-a", None));
        let binding = storage
            .with_binding_transaction(|records| {
                let binding = records.insert_binding(&identity, &server(), &pane("%7", 107))?;
                assert!(records.set_session_state(
                    &binding.id,
                    &BindingSessionState::default(),
                    &first
                )?);
                Ok::<_, StorageError>(binding)
            })
            .unwrap();

        // A thread ID disclosed late is the same run: the continuation stays open.
        hand_over(&storage);
        assert!(open(&mut storage));
        let disclosed = run(key(500, "start-a", Some("session-a")));
        storage
            .with_binding_transaction(|records| {
                assert!(records.set_session_state(&binding.id, &first, &disclosed)?);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        assert!(open(&mut storage));

        // SessionEnd: the run is over.
        let ended = disclosed
            .transition(
                disclosed.key.as_ref().unwrap(),
                SessionTransition::Ended,
                None,
            )
            .unwrap();
        storage
            .with_binding_transaction(|records| {
                assert!(records.set_session_state(&binding.id, &disclosed, &ended)?);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        assert!(!open(&mut storage));

        // A launch that replaces a run nobody closed (a crash with no SessionEnd) ends it too,
        // and so does a clear that moves the same process to another provider session.
        let second = run(key(600, "start-b", Some("session-b")));
        storage
            .with_binding_transaction(|records| {
                assert!(records.set_session_state(&binding.id, &ended, &second)?);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        hand_over(&storage);
        assert!(open(&mut storage));
        let cleared = run(key(600, "start-b", Some("session-c")));
        storage
            .with_binding_transaction(|records| {
                assert!(records.set_session_state(&binding.id, &second, &cleared)?);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        assert!(!open(&mut storage));
        hand_over(&storage);
        let relaunched = run(key(700, "start-c", Some("session-d")));
        storage
            .with_binding_transaction(|records| {
                assert!(records.set_session_state(&binding.id, &cleared, &relaunched)?);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        assert!(!open(&mut storage));

        // The binding going away ends it as well.
        hand_over(&storage);
        assert!(open(&mut storage));
        storage
            .with_binding_transaction(|records| {
                records.detach_binding(&binding.id)?;
                Ok::<_, StorageError>(())
            })
            .unwrap();
        assert!(!open(&mut storage));
        storage.close().unwrap();
    }

    #[test]
    fn launch_owner_round_trips_and_participates_in_exact_state_comparison() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Launched", Lifetime::Saved)
            .unwrap()
            .identity;
        let state = BindingSessionState::default()
            .admit_launched(
                ObservedSessionKey {
                    incarnation: ProcessIncarnation::new(500, "child-start").unwrap(),
                    provider_session: None,
                },
                ProcessIncarnation::new(400, "owner-start").unwrap(),
                SessionTransition::Started,
                tmt_core::binding::session::RuntimeLiveness::Alive,
                tmt_core::binding::session::RuntimeLiveness::Alive,
            )
            .unwrap();
        let binding = storage
            .with_binding_transaction(|records| {
                let binding = records.insert_binding(&identity, &server(), &pane("%5", 105))?;
                assert!(records.set_session_state(
                    &binding.id,
                    &BindingSessionState::default(),
                    &state
                )?);
                Ok::<_, StorageError>(binding)
            })
            .unwrap();
        storage.close().unwrap();
        let mut storage = fixture.open();
        storage
            .with_binding_transaction(|records| {
                assert_eq!(
                    records
                        .entry_by_id(&identity.id)?
                        .unwrap()
                        .binding
                        .unwrap()
                        .session,
                    state
                );
                let mut stale = state.clone();
                stale.launch_owner = Some(ProcessIncarnation::new(400, "old-owner-start").unwrap());
                let ended = state
                    .transition(state.key.as_ref().unwrap(), SessionTransition::Ended, None)
                    .unwrap();
                assert!(!records.set_session_state(&binding.id, &stale, &ended)?);
                stale.launch_owner = None;
                assert!(!records.set_session_state(&binding.id, &stale, &ended)?);
                assert_eq!(
                    records
                        .entry_by_id(&identity.id)?
                        .unwrap()
                        .binding
                        .unwrap()
                        .session,
                    state
                );
                assert!(records.set_session_state(&binding.id, &state, &ended)?);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        storage.close().unwrap();
    }

    #[test]
    fn delayed_launch_owner_attachment_preserves_the_committed_provider_end() {
        use tmt_core::binding::session::RuntimeLiveness;
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Delayed", Lifetime::Saved)
            .unwrap()
            .identity;
        let key = ObservedSessionKey {
            incarnation: ProcessIncarnation::new(500, "child-start").unwrap(),
            provider_session: Some(ProviderSessionId::new("session-a").unwrap()),
        };
        let owner = ProcessIncarnation::new(400, "owner-start").unwrap();
        let running = BindingSessionState::default()
            .admit(
                key.clone(),
                SessionTransition::Started,
                RuntimeLiveness::Alive,
            )
            .unwrap();
        let ended = running
            .transition(&key, SessionTransition::Ended, None)
            .unwrap();
        let binding = storage
            .with_binding_transaction(|records| {
                let binding = records.insert_binding(&identity, &server(), &pane("%6", 106))?;
                assert!(records.set_session_state(
                    &binding.id,
                    &BindingSessionState::default(),
                    &running
                )?);
                assert!(records.set_session_state(&binding.id, &running, &ended)?);
                Ok::<_, StorageError>(binding)
            })
            .unwrap();
        storage.close().unwrap();
        let mut storage = fixture.open();
        storage
            .with_binding_transaction(|records| {
                let current = records
                    .entry_by_id(&identity.id)?
                    .unwrap()
                    .binding
                    .unwrap()
                    .session;
                assert_eq!(current, ended);
                let attached = current
                    .admit_launched(
                        key.clone(),
                        owner.clone(),
                        SessionTransition::Started,
                        RuntimeLiveness::Alive,
                        RuntimeLiveness::Alive,
                    )
                    .unwrap();
                assert!(!records.set_session_state(&binding.id, &running, &attached)?);
                assert!(records.set_session_state(&binding.id, &current, &attached)?);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        storage.close().unwrap();
        let mut storage = fixture.open();
        let saved = storage
            .with_binding_transaction(|records| {
                Ok::<_, StorageError>(
                    records
                        .entry_by_id(&identity.id)?
                        .unwrap()
                        .binding
                        .unwrap()
                        .session,
                )
            })
            .unwrap();
        assert_eq!(saved.key, Some(key));
        assert_eq!(saved.launch_owner, Some(owner));
        assert_eq!(saved.state, RuntimeState::Unknown);
        assert_eq!(saved.last_transition, Some(SessionTransition::Ended));
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
            incarnation: ProcessIncarnation::new(123, "start-a").unwrap(),
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
                        start_refusal: None,
                        launch_owner: None,
                        state: RuntimeState::Running,
                        last_transition: Some(SessionTransition::Started),
                        key: Some(ObservedSessionKey {
                            incarnation: ProcessIncarnation::new(101, "runtime-start").unwrap(),
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

    fn with_state(mut preferences: SessionPreferences) -> SessionPreferences {
        let session = preferences.remembered.as_mut().unwrap();
        session.state = Some(DriverState::new(2, r#"{"model":"opus"}"#).unwrap());
        session.stale_at_ms = Some(1_700_000_000_000);
        preferences
    }

    fn stored_session(fixture: &Fixture, identity_id: &str) -> (Option<String>, Option<String>) {
        Connection::open(&fixture.database)
            .unwrap()
            .query_row(
                "SELECT preferred_harness, coalesce(provider_session_id, driver_state,
                 CAST(stale_at_ms AS TEXT)) FROM identity_session_preferences WHERE identity_id = ?",
                [identity_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn retirement_clears_the_remembered_session_for_both_lifetimes() {
        // Deliberate change (#420): retirement used to hide the retained session;
        // it now deletes the session and driver state in the retiring transaction.
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        for (name, lifetime, remove_content) in [
            ("Temporary", Lifetime::Temporary, false),
            ("Saved", Lifetime::Saved, true),
        ] {
            let identity = create_or_resolve(&mut storage, name, lifetime)
                .unwrap()
                .identity;
            storage
                .with_binding_transaction(|records| {
                    assert!(
                        records.set_session_preferences(&identity.id, &with_state(remembered()))?
                    );
                    records.retire_identity(&identity, remove_content)?;
                    assert_eq!(
                        records.session_preferences(&identity.id)?,
                        SessionPreferences::default()
                    );
                    assert!(
                        !records.set_session_preferences(&identity.id, &remembered())?,
                        "a retired identity cannot be given a session again"
                    );
                    Ok::<_, StorageError>(())
                })
                .unwrap();
            assert_eq!(
                stored_session(&fixture, &identity.id),
                (Some("claude".into()), None),
                "{name}: only the launch preference remains"
            );
            let replacement = create_or_resolve(&mut storage, name, lifetime)
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
        }
        storage.close().unwrap();
    }

    #[test]
    fn driver_state_and_stale_marks_round_trip_within_their_constraints() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let identity = create_or_resolve(&mut storage, "Stateful", Lifetime::Saved)
            .unwrap()
            .identity;
        let preferences = with_state(remembered());
        storage
            .with_binding_transaction(|records| {
                assert!(records.set_session_preferences(&identity.id, &preferences)?);
                assert_eq!(records.session_preferences(&identity.id)?, preferences);
                Ok::<_, StorageError>(())
            })
            .unwrap();
        storage.close().unwrap();
        let connection = Connection::open(&fixture.database).unwrap();
        for update in [
            "UPDATE identity_session_preferences SET driver_state = NULL",
            "UPDATE identity_session_preferences SET driver_state_version = 0",
            "UPDATE identity_session_preferences SET driver_state = zeroblob(8)",
            "UPDATE identity_session_preferences SET driver_state = printf('%.*c', 1025, 'x')",
            "UPDATE identity_session_preferences SET remembered_harness = NULL,
             runtime_mode = NULL, provider_session_id = NULL",
        ] {
            assert!(connection.execute(update, []).is_err(), "{update}");
        }
        connection
            .execute(
                "UPDATE identity_session_preferences SET driver_state = NULL,
                 driver_state_version = NULL, stale_at_ms = NULL",
                [],
            )
            .unwrap();
        let mut storage = fixture.open();
        storage
            .with_binding_transaction(|records| {
                assert_eq!(records.session_preferences(&identity.id)?, remembered());
                Ok::<_, StorageError>(())
            })
            .unwrap();
        storage.close().unwrap();
    }

    #[test]
    fn sessions_of_unregistered_drivers_are_purged_with_their_state() {
        let fixture = Fixture::new();
        let mut storage = fixture.open();
        let mut ids = Vec::new();
        for (name, harness) in [("Kept", "claude"), ("Orphan", "removed-driver")] {
            let identity = create_or_resolve(&mut storage, name, Lifetime::Saved)
                .unwrap()
                .identity;
            let mut preferences = with_state(remembered());
            preferences.remembered.as_mut().unwrap().harness = HarnessId::new(harness).unwrap();
            storage
                .with_binding_transaction(|records| {
                    records.set_session_preferences(&identity.id, &preferences)
                })
                .unwrap();
            ids.push(identity.id);
        }
        assert_eq!(
            storage
                .purge_unregistered_sessions(&["claude", "codex"])
                .unwrap(),
            [crate::storage::PurgedSession {
                identity_id: ids[1].clone(),
                name: "Orphan".into(),
                harness: "removed-driver".into(),
            }]
        );
        assert!(
            storage
                .purge_unregistered_sessions(&["claude", "codex"])
                .unwrap()
                .is_empty()
        );
        storage.close().unwrap();
        assert_eq!(
            stored_session(&fixture, &ids[1]),
            (Some("claude".into()), None)
        );
        assert_eq!(
            stored_session(&fixture, &ids[0]),
            (Some("claude".into()), Some("saved-session".into()))
        );
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
                    start_refusal: None,
                    launch_owner: None,
                    state: RuntimeState::Ended,
                    last_transition: Some(SessionTransition::Ended),
                    key: Some(ObservedSessionKey {
                        incarnation: ProcessIncarnation::new(103, "runtime-start").unwrap(),
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
