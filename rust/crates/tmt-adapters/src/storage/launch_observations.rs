//! Binding-owned startup rendezvous; all mutations share the binding writer transaction.
use super::{StorageError, errors::classify};
use rusqlite::{Connection, OptionalExtension, params};
use tmt_core::{
    binding::session::{
        DriverState, HarnessId, ProviderSessionId, RuntimeMode,
        launch::{
            LaunchObservation, LaunchStatus, MAXIMUM_PER_BINDING, PendingStart, RETENTION_MS,
            SessionLearningDiagnostic, SessionLearningReason, SessionLearningStep,
        },
    },
    endpoint::ProcessIncarnation,
    limits::MAX_JS_SAFE_INTEGER,
};

const COLUMNS: &str = "binding_id,owner_pid,owner_start,driver,created_at_ms,expires_at_ms,status,provider_pid,provider_start,provider_session,runtime_mode,observed_at_ms,step,reason,driver_state_version,driver_state";
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
    db.execute("INSERT INTO launch_observations(binding_id,owner_pid,owner_start,driver,created_at_ms,expires_at_ms,step,reason)
        VALUES(?,?,?,?,?,?, 'input','not_admitted')",params![binding,owner.pid() as i64,owner.start_identity(),driver.as_str(),now as i64,
            now.saturating_add(RETENTION_MS).min(MAX_JS_SAFE_INTEGER) as i64])
        .map(|n|n==1).map_err(|e|classify(e,"Record launch intent"))
}

pub(super) fn replace(
    db: &Connection,
    expected: &LaunchObservation,
    next: &LaunchObservation,
) -> Result<bool, StorageError> {
    if matches!(
        expected.status,
        LaunchStatus::Admitted | LaunchStatus::Settled
    ) || (next.status == LaunchStatus::Pending) != next.pending.is_some()
        || (expected.pending.is_some()
            && next.pending.is_some()
            && expected.pending != next.pending)
    {
        return Ok(false);
    }
    if expected.binding_id != next.binding_id
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
    db.execute("UPDATE launch_observations SET status=?,provider_pid=?,provider_start=?,provider_session=?,runtime_mode=?,observed_at_ms=?,step=?,reason=?,driver_state_version=?,driver_state=?
        WHERE binding_id=? AND owner_pid=? AND owner_start=?", params![status,pending.map(|p|p.process.pid() as i64),pending.map(|p|p.process.start_identity()),
        pending.map(|p|p.session.as_str()),pending.map(|p|p.mode.as_str()),pending.map(|p|p.observed_at_ms as i64),
        next.diagnostic.map(|d|d.step.as_str()),next.diagnostic.map(|d|d.reason.as_str()),state.map(DriverState::version),state.map(DriverState::document),
        next.binding_id,next.owner.pid() as i64,next.owner.start_identity()])
        .map(|n|n==1).map_err(|e|classify(e,"Commit launch observation"))
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
        let mut pending = intent.clone();
        pending.status = LaunchStatus::Pending;
        pending.pending = Some(PendingStart {
            process: ProcessIncarnation::new(71, "provider").unwrap(),
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
