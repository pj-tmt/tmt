//! Binding-owned startup rendezvous; all mutations share the binding writer transaction.
use super::{StorageError, errors::classify};
use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::{
    binding::session::{
        DriverState, HarnessId, ProviderSessionId, RuntimeMode, SessionTransition,
        launch::{
            LaunchObservation, LaunchStatus, MAXIMUM_PER_BINDING, PendingStart, RETENTION_MS,
            SessionLearningDiagnostic, SessionLearningReason, SessionLearningStep,
        },
    },
    endpoint::ProcessIncarnation,
    limits::MAX_JS_SAFE_INTEGER,
};

const COLUMNS: &str = "binding_id,owner_pid,owner_start,driver,created_at_ms,expires_at_ms,status,provider_pid,provider_start,provider_session,runtime_mode,observed_at_ms,step,reason,driver_state_version,driver_state,preferences_fingerprint,provider_transition";
fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<LaunchObservation> {
    let invalid = || rusqlite::Error::InvalidQuery;
    let process = |pid: i64, start: String| {
        ProcessIncarnation::new(u64::try_from(pid).map_err(|_| invalid())?, &start)
            .map_err(|_| invalid())
    };
    let time = |offset| u64::try_from(row.get::<_, i64>(offset)?).map_err(|_| invalid());
    let status = match row.get::<_, String>(6)?.as_str() {
        "intent" => LaunchStatus::Intent,
        "pending" => LaunchStatus::Pending,
        "admitted" => LaunchStatus::Admitted,
        "settled" => LaunchStatus::Settled,
        _ => return Err(invalid()),
    };
    let pending = match row.get::<_, Option<i64>>(7)? {
        None => None,
        Some(pid) => Some(PendingStart {
            process: process(pid, row.get(8)?)?,
            transition: match row.get::<_, String>(17)?.as_str() {
                "started" => SessionTransition::Started,
                "resumed" => SessionTransition::Resumed,
                "cleared" => SessionTransition::Cleared,
                "compacted" => SessionTransition::Compacted,
                "forked" => SessionTransition::Forked,
                "ended" => SessionTransition::Ended,
                _ => return Err(invalid()),
            },
            session: ProviderSessionId::new(&row.get::<_, String>(9)?).map_err(|_| invalid())?,
            mode: RuntimeMode::new(&row.get::<_, String>(10)?).map_err(|_| invalid())?,
            observed_at_ms: time(11)?,
            state: match (
                row.get::<_, Option<u16>>(14)?,
                row.get::<_, Option<String>>(15)?,
            ) {
                (None, None) => None,
                (Some(version), Some(document)) => {
                    Some(DriverState::new(version, &document).map_err(|_| invalid())?)
                }
                _ => return Err(invalid()),
            },
        }),
    };
    let diagnostic = match (
        row.get::<_, Option<String>>(12)?,
        row.get::<_, Option<String>>(13)?,
    ) {
        (None, None) => None,
        (Some(step), Some(reason)) => Some(SessionLearningDiagnostic {
            step: SessionLearningStep::parse(&step).ok_or_else(invalid)?,
            reason: SessionLearningReason::parse(&reason).ok_or_else(invalid)?,
        }),
        _ => return Err(invalid()),
    };
    Ok(LaunchObservation {
        binding_id: row.get(0)?,
        preferences_fingerprint: row.get(16)?,
        owner: process(row.get(1)?, row.get(2)?)?,
        driver: HarnessId::new(&row.get::<_, String>(3)?).map_err(|_| invalid())?,
        created_at_ms: time(4)?,
        expires_at_ms: time(5)?,
        status,
        pending,
        diagnostic,
    })
}

pub(super) fn read(
    db: &Connection,
    binding: &str,
    owner: &ProcessIncarnation,
) -> Result<Option<LaunchObservation>, StorageError> {
    db.query_row(&format!("SELECT {COLUMNS} FROM launch_observations WHERE binding_id=? AND owner_pid=? AND owner_start=?"),
        params![binding,owner.pid() as i64,owner.start_identity()],decode).optional()
        .map_err(|e|classify(e,"Read launch observation"))
}

pub(super) fn begin(
    db: &Connection,
    binding: &str,
    owner: &ProcessIncarnation,
    driver: &HarnessId,
    now: u64,
) -> Result<bool, StorageError> {
    if now > MAX_JS_SAFE_INTEGER {
        return Err(classify(
            rusqlite::Error::InvalidQuery,
            "Validate launch observation time",
        ));
    }
    db.execute(
        "DELETE FROM launch_observations WHERE expires_at_ms<=?",
        [now as i64],
    )
    .map_err(|e| classify(e, "Expire launch observations"))?;
    if let Some(current) = read(db, binding, owner)? {
        return Ok(current.driver == *driver && current.status == LaunchStatus::Intent);
    }
    db.execute("DELETE FROM launch_observations WHERE binding_id=? AND (owner_pid,owner_start) IN
        (SELECT owner_pid,owner_start FROM launch_observations WHERE binding_id=? ORDER BY created_at_ms DESC,owner_pid DESC,owner_start DESC LIMIT -1 OFFSET ?)",
        params![binding,binding,MAXIMUM_PER_BINDING as i64-1]).map_err(|e|classify(e,"Bound launch observation retention"))?;
    let fingerprint = preferences_fingerprint(db, binding)?;
    db.execute("INSERT INTO launch_observations(binding_id,owner_pid,owner_start,driver,created_at_ms,expires_at_ms,step,reason,preferences_fingerprint)
        VALUES(?,?,?,?,?,?, 'input','not_admitted',?)",params![binding,owner.pid() as i64,owner.start_identity(),driver.as_str(),now as i64,
            now.saturating_add(RETENTION_MS).min(MAX_JS_SAFE_INTEGER) as i64,fingerprint])
        .map(|n|n==1).map_err(|e|classify(e,"Record launch intent"))
}

pub(super) fn replace(
    db: &Connection,
    expected: &LaunchObservation,
    next: &LaunchObservation,
) -> Result<bool, StorageError> {
    if (expected.status == LaunchStatus::Settled && next.status != LaunchStatus::Admitted)
        || (expected.status == LaunchStatus::Admitted
            && !matches!(next.status, LaunchStatus::Admitted | LaunchStatus::Settled))
        || (next.status == LaunchStatus::Pending) != next.pending.is_some()
        || (expected.pending.is_some()
            && next.pending.is_some()
            && expected.pending != next.pending
            && !expected
                .pending
                .as_ref()
                .zip(next.pending.as_ref())
                .is_some_and(|(old, new)| {
                    let mut ended = old.clone();
                    ended.transition = SessionTransition::Ended;
                    new.transition == SessionTransition::Ended && *new == ended
                }))
    {
        return Ok(false);
    }
    if expected.preferences_fingerprint != next.preferences_fingerprint
        || expected.binding_id != next.binding_id
        || expected.owner != next.owner
        || expected.driver != next.driver
        || expected.created_at_ms != next.created_at_ms
        || expected.expires_at_ms != next.expires_at_ms
        || read(db, &expected.binding_id, &expected.owner)?.as_ref() != Some(expected)
    {
        return Ok(false);
    }
    let status = match next.status {
        LaunchStatus::Intent => "intent",
        LaunchStatus::Pending => "pending",
        LaunchStatus::Admitted => "admitted",
        LaunchStatus::Settled => "settled",
    };
    let pending = next.pending.as_ref();
    let state = pending.and_then(|p| p.state.as_ref());
    db.execute("UPDATE launch_observations SET status=?,provider_pid=?,provider_start=?,provider_session=?,runtime_mode=?,observed_at_ms=?,step=?,reason=?,driver_state_version=?,driver_state=?,provider_transition=?
        WHERE binding_id=? AND owner_pid=? AND owner_start=?", params![status,pending.map(|p|p.process.pid() as i64),pending.map(|p|p.process.start_identity()),
        pending.map(|p|p.session.as_str()),pending.map(|p|p.mode.as_str()),pending.map(|p|p.observed_at_ms as i64),
        next.diagnostic.map(|d|d.step.as_str()),next.diagnostic.map(|d|d.reason.as_str()),state.map(DriverState::version),state.map(DriverState::document),pending.map(|p|match p.transition {SessionTransition::Started=>"started",SessionTransition::Resumed=>"resumed",SessionTransition::Cleared=>"cleared",SessionTransition::Compacted=>"compacted",SessionTransition::Forked=>"forked",SessionTransition::Ended=>"ended"}),
        next.binding_id,next.owner.pid() as i64,next.owner.start_identity()])
        .map(|n|n==1).map_err(|e|classify(e,"Commit launch observation"))
}

impl super::Storage {
    /// Stored startup diagnostics only. Never probes a process, reconciles a
    /// binding, or supplies presence/delivery authority or resume coordinates.
    pub fn identity_session_learning(
        &self,
        identity: &str,
        now: u64,
    ) -> Result<Option<(tmt_core::binding::Binding, LaunchObservation)>, StorageError> {
        use tmt_core::binding::BindingRecords;
        let transaction = self
            .connection()?
            .unchecked_transaction()
            .map_err(|e| classify(e, "Begin session learning projection"))?;
        let result = match super::bindings::BindingRows(&transaction)
            .entry_by_id(identity)?
            .and_then(|entry| entry.binding)
        {
            Some(binding) => {
                latest(&transaction, &binding.id, now)?.map(|observed| (binding, observed))
            }
            None => None,
        };
        transaction
            .commit()
            .map_err(|e| classify(e, "Finish session learning projection"))?;
        Ok(result)
    }
    pub fn session_learning(
        &self,
        binding: &str,
        now: u64,
    ) -> Result<Option<LaunchObservation>, StorageError> {
        latest(self.connection()?, binding, now)
    }
}
pub(super) fn latest(
    db: &Connection,
    binding: &str,
    now: u64,
) -> Result<Option<LaunchObservation>, StorageError> {
    if now > MAX_JS_SAFE_INTEGER {
        return Err(classify(
            rusqlite::Error::InvalidQuery,
            "Validate session learning observation time",
        ));
    }
    db.query_row(&format!("SELECT {COLUMNS} FROM launch_observations WHERE binding_id=? AND expires_at_ms>? ORDER BY created_at_ms DESC,owner_pid DESC,owner_start DESC LIMIT 1"),
        params![binding,now as i64],decode).optional().map_err(|e|classify(e,"Read stored session learning diagnostic"))
}

/// Canonical stored values, including inactive driver records, fence a pending
/// event against any concurrent preference edit without persisting a second copy.
fn preferences_fingerprint(db: &Connection, binding: &str) -> Result<String, StorageError> {
    let mut values = Vec::new();
    for sql in [
        "SELECT p.* FROM identity_session_preferences p JOIN bindings b ON b.identity_id=p.identity_id WHERE b.id=?",
        "SELECT r.* FROM identity_resume_records r JOIN bindings b ON b.identity_id=r.identity_id WHERE b.id=? ORDER BY r.harness",
    ] {
        let mut statement = db
            .prepare(sql)
            .map_err(|e| classify(e, "Prepare launch preference fence"))?;
        let rows = statement
            .query_map([binding], |row| {
                (0..row.as_ref().column_count())
                    .map(|i| match row.get_ref(i)? {
                        rusqlite::types::ValueRef::Null => Ok(serde_json::Value::Null),
                        rusqlite::types::ValueRef::Integer(value) => Ok(serde_json::json!(value)),
                        rusqlite::types::ValueRef::Text(value) => std::str::from_utf8(value)
                            .map(|v| serde_json::json!(v))
                            .map_err(|_| rusqlite::Error::InvalidQuery),
                        _ => Err(rusqlite::Error::InvalidQuery),
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| classify(e, "Read launch preference fence"))?;
        values.push(rows);
    }
    Ok(tmt_core::content_digest::sha256(
        serde_json::to_string(&values)
            .expect("stored strings and integers serialize")
            .as_bytes(),
    ))
}
pub(super) fn preferences_match(
    db: &Connection,
    observation: &LaunchObservation,
) -> Result<bool, StorageError> {
    let exists: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM bindings b JOIN identities i ON i.id=b.identity_id WHERE b.id=? AND i.retired_at_ms IS NULL)",[&observation.binding_id],|row|row.get(0)).map_err(|e|classify(e,"Check launch preference binding"))?;
    Ok(exists
        && preferences_fingerprint(db, &observation.binding_id)?
            == observation.preferences_fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        storage::{Storage, StorageErrorCode},
        test_support::TestDirectory,
    };
    use tmt_core::{
        binding::BindingRepository,
        endpoint::{PaneObservation, ServerEvidence},
        host::HostKind,
        identity::{Lifetime, create_or_resolve},
    };

    #[test]
    fn pending_evidence_rolls_back_with_admission_and_never_replaces_another_session() {
        let directory = TestDirectory::new();
        let path = directory.path.join("state.db");
        let mut storage = Storage::open(&path).unwrap();
        let identity = create_or_resolve(&mut storage, "Alpha", Lifetime::Saved)
            .unwrap()
            .identity;
        let binding = storage
            .with_binding_transaction(|records| {
                records.insert_binding(
                    &identity,
                    &ServerEvidence {
                        host: HostKind::Tmux,
                        server_id: "server".into(),
                        socket_path: "/tmp/private".into(),
                        server_pid: 41,
                        server_start_time: "server".into(),
                    },
                    &PaneObservation {
                        id: "%3".into(),
                        target: None,
                        cwd: None,
                        command: "sh".into(),
                        pane_pid: 51,
                        pane_incarnation: None,
                        suggested_name: None,
                        marker: None,
                        location: None,
                        agent_name: None,
                    },
                )
            })
            .unwrap();
        let owner = ProcessIncarnation::new(61, "owner").unwrap();
        let driver = HarnessId::new("claude").unwrap();
        assert!(
            storage
                .with_binding_transaction(
                    |records| records.begin_launch_observation(&binding, &owner, &driver, 100)
                )
                .unwrap()
        );
        let intent = storage
            .with_binding_transaction(|records| records.launch_observation(&binding.id, &owner))
            .unwrap()
            .unwrap();
        assert_eq!(intent.status, LaunchStatus::Intent);
        assert!(
            storage
                .with_binding_transaction(|records| records.launch_preferences_match(&intent))
                .unwrap()
        );
        let changed = storage.with_binding_transaction::<_, StorageError>(|records| {
            let mut preferences = records.session_preferences(&identity.id)?;
            preferences.preferred_harness = Some(HarnessId::new("codex").unwrap());
            assert!(records.set_session_preferences(&identity.id, &preferences)?);
            assert!(
                !records.launch_preferences_match(&intent)?,
                "a provisional event cannot overwrite a concurrent preference edit"
            );
            Err::<(), _>(StorageError::new(
                StorageErrorCode::Unknown,
                "roll back preference control",
            ))
        });
        assert!(changed.is_err());
        assert!(
            storage
                .with_binding_transaction(|records| records.launch_preferences_match(&intent))
                .unwrap()
        );

        let mut pending = intent.clone();
        pending.status = LaunchStatus::Pending;
        pending.pending = Some(PendingStart {
            process: ProcessIncarnation::new(71, "provider").unwrap(),
            transition: SessionTransition::Started,
            session: ProviderSessionId::new("session").unwrap(),
            mode: RuntimeMode::new("default").unwrap(),
            state: None,
            observed_at_ms: 110,
        });
        let failed = storage.with_binding_transaction::<_, StorageError>(|records| {
            assert!(records.set_launch_observation(&intent, &pending)?);
            Err::<(), _>(StorageError::new(
                StorageErrorCode::Unknown,
                "injected transaction abort",
            ))
        });
        assert!(failed.is_err());
        assert_eq!(
            storage
                .with_binding_transaction(|records| records.launch_observation(&binding.id, &owner))
                .unwrap(),
            Some(intent.clone())
        );
        assert!(
            storage
                .with_binding_transaction(
                    |records| records.set_launch_observation(&intent, &pending)
                )
                .unwrap()
        );
        let mut conflicting = pending.clone();
        conflicting.pending.as_mut().unwrap().session = ProviderSessionId::new("other").unwrap();
        assert!(
            !storage
                .with_binding_transaction(
                    |records| records.set_launch_observation(&pending, &conflicting)
                )
                .unwrap()
        );
        assert!(
            storage
                .session_preferences(&identity.id)
                .unwrap()
                .remembered
                .is_none()
        );
        assert_eq!(
            storage
                .with_binding_transaction(|records| records.entry_by_id(&identity.id))
                .unwrap()
                .unwrap()
                .binding,
            Some(binding.clone())
        );
        let mut ended = pending.clone();
        ended.pending.as_mut().unwrap().transition = SessionTransition::Ended;
        assert!(storage.with_binding_transaction(|records|records.set_launch_observation(&pending,&ended)).unwrap());
        assert!(!storage.with_binding_transaction(|records|records.set_launch_observation(&ended,&pending)).unwrap(),"a later start cannot undo the retained end");
        pending = ended;
        let mut consumed = pending.clone();
        consumed.status = LaunchStatus::Admitted;
        consumed.pending = None;
        consumed.diagnostic = None;
        assert!(
            storage
                .with_binding_transaction(
                    |records| records.set_launch_observation(&pending, &consumed)
                )
                .unwrap()
        );
        assert!(
            !storage
                .with_binding_transaction(
                    |records| records.set_launch_observation(&consumed, &pending)
                )
                .unwrap(),
            "late hooks cannot resurrect consumed evidence"
        );
        for pid in 100..111 {
            let another = ProcessIncarnation::new(pid, "another-owner").unwrap();
            assert!(
                storage
                    .with_binding_transaction(|records| records
                        .begin_launch_observation(&binding, &another, &driver, 150))
                    .unwrap()
            );
            assert!(
                storage
                    .connection()
                    .unwrap()
                    .query_row("SELECT count(*) FROM launch_observations", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap()
                    <= MAXIMUM_PER_BINDING as i64
            );
        }
        let later = ProcessIncarnation::new(200, "later-owner").unwrap();
        assert!(
            storage
                .with_binding_transaction(|records| records.begin_launch_observation(
                    &binding,
                    &later,
                    &driver,
                    RETENTION_MS + 200
                ))
                .unwrap()
        );
        assert_eq!(
            storage
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM launch_observations", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1,
            "the next bounded mutation expires old rows"
        );
        storage
            .with_binding_transaction(|records| records.detach_binding(&binding.id))
            .unwrap();
        assert!(
            storage
                .with_binding_transaction(|records| records.launch_observation(&binding.id, &owner))
                .unwrap()
                .is_none()
        );
    }
}
