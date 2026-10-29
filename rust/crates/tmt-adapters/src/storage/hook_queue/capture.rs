//! Capture-specific payload and ingestion. Authority is proved before enqueue;
//! delayed ingestion places a final in that recorded batch, never a newer turn.
use super::super::{
    Storage, StorageError, bindings::BindingRows, capture_batches, classify, responses,
};
use super::invalid;
use crate::runtime::{
    driver_state,
    hook_protocol::{ActivityObservation, HookLaunch},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tmt_core::{
    binding::{
        BindingRecords,
        session::{
            ProviderSessionId,
            activity::{Event, EventKind},
        },
    },
    endpoint::ProcessIncarnation,
    responses::FinalText,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureResult {
    Committed,
    Duplicate,
    NotApplicable,
    NoAdmittedStart,
    RetiredIdentity,
    MissingBatch,
}
impl From<super::super::TurnCaptureOutcome> for CaptureResult {
    fn from(value: super::super::TurnCaptureOutcome) -> Self {
        use super::super::TurnCaptureOutcome as Outcome;
        match value {
            Outcome::Committed => Self::Committed,
            Outcome::Duplicate => Self::Duplicate,
            Outcome::NotApplicable => Self::NotApplicable,
            Outcome::NoAdmittedStart => Self::NoAdmittedStart,
        }
    }
}
impl CaptureResult {
    fn token(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Duplicate => "duplicate",
            Self::NotApplicable => "not_applicable",
            Self::NoAdmittedStart => "no_admitted_start",
            Self::RetiredIdentity => "retired_identity",
            Self::MissingBatch => "missing_batch",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureLearning {
    pub launch: HookLaunch,
    pub provider: String,
    pub step: Option<String>,
    pub detail: Option<String>,
    pub failure: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFollowUp {
    pub usage: bool,
    pub transcript: Option<PathBuf>,
    pub learning: Option<CaptureLearning>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeptFinal {
    text: String,
    original_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
}
impl KeptFinal {
    fn decode(&self) -> Result<FinalText, StorageError> {
        FinalText::from_kept(&self.text, self.original_bytes, self.digest.as_deref())
            .ok_or_else(|| invalid("Invalid queued final text"))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingCapture {
    identity: String,
    binding: String,
    binding_fingerprint: String,
    batch: Option<String>,
    #[serde(default)]
    provisional: bool,
    provider: String,
    session: String,
    pid: u64,
    start: String,
    turn: Option<String>,
    continuation: bool,
    model: Option<String>,
    interrupted: bool,
    response: Option<KeptFinal>,
    now_ms: u64,
    admitted: CaptureResult,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticFence {
    batch: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureEvent {
    version: u8,
    id: String,
    capture: Option<PendingCapture>,
    #[serde(default)]
    diagnostic_fence: Option<DiagnosticFence>,
    pub follow_up: CaptureFollowUp,
}
impl CaptureEvent {
    pub fn id(&self) -> &str {
        &self.id
    }
    /// A pre-upgrade active turn can lack an input batch. The queue keeps its
    /// provisional id across acknowledgments so two already-admitted Stops use
    /// one batch even if the first flusher finishes before the second enqueue.
    pub(super) fn provisional_key(&self) -> Option<String> {
        let c = self.capture.as_ref().filter(|c| c.provisional)?;
        let authority = serde_json::json!([
            c.identity, c.binding, c.provider, c.session, c.pid, c.start, c.turn
        ]);
        Some(tmt_core::content_digest::framed_sha256(
            b"tmt-hook-provisional-batch-v1",
            &serde_json::to_vec(&authority).expect("capture authority is serializable"),
        ))
    }
    pub(super) fn provisional_batch(&self) -> Option<&str> {
        self.capture
            .as_ref()
            .filter(|c| c.provisional)?
            .batch
            .as_deref()
    }
    pub(super) fn set_provisional_batch(&mut self, batch: String) {
        if let Some(c) = self.capture.as_mut().filter(|c| c.provisional) {
            c.batch = Some(batch);
        }
    }
    pub fn identity(&self) -> Option<&str> {
        self.capture
            .as_ref()
            .map(|c| c.identity.as_str())
            .or_else(|| {
                self.follow_up
                    .learning
                    .as_ref()
                    .map(|l| l.launch.identity_id.as_str())
            })
    }
    pub fn provider(&self) -> Option<&str> {
        self.capture.as_ref().map(|c| c.provider.as_str())
    }
    pub fn session(&self) -> Option<&str> {
        self.capture.as_ref().map(|c| c.session.as_str())
    }
    /// A delayed optional follow-up must not reintroduce an old Stop diagnostic
    /// or sample an old transcript into the activity of a newer input batch.
    pub fn is_current_batch(&self, database: &Path) -> Result<bool, StorageError> {
        let mut db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| classify(e, "Read queued follow-up batch"))?;
        db.busy_timeout(Duration::ZERO)
            .map_err(|e| classify(e, "Disable follow-up read writer wait"))?;
        let tx = db
            .transaction()
            .map_err(|e| classify(e, "Read queued follow-up snapshot"))?;
        super::super::migrations::require_current(&tx)?;
        current_event(&tx, self)
    }
    /// Read original binding authority and fresh preferences in one snapshot;
    /// the existing observation CAS protects that snapshot through telemetry.
    pub fn follow_up_context(
        &self,
        database: &Path,
    ) -> Result<Option<super::super::IdentityContextSnapshot>, StorageError> {
        let Some(c) = &self.capture else {
            return Ok(None);
        };
        let mut db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| classify(e, "Read capture follow-up context"))?;
        db.busy_timeout(Duration::ZERO)
            .map_err(|e| classify(e, "Disable capture follow-up writer wait"))?;
        let tx = db
            .transaction()
            .map_err(|e| classify(e, "Read capture follow-up context snapshot"))?;
        super::super::migrations::require_current(&tx)?;
        if !current_turn(&tx, c)? {
            return Ok(None);
        }
        let rows = BindingRows(&tx);
        let Some(entry) = rows.entry_by_id(&c.identity)? else {
            return Ok(None);
        };
        Ok(Some(super::super::IdentityContextSnapshot {
            entry,
            preferences: rows.session_preferences(&c.identity)?,
            role: None,
            role_truncated: false,
            requests: Default::default(),
        }))
    }
    pub fn admission(&self) -> CaptureResult {
        self.capture
            .as_ref()
            .map_or(CaptureResult::NotApplicable, |c| c.admitted)
    }
    pub fn interrupted(&self) -> bool {
        self.capture.as_ref().is_some_and(|c| c.interrupted)
    }
    /// A provider may supply a Stop transcript without an activity transition.
    /// Preserve its verified usage follow-up, without admitting capture or Idle.
    pub fn usage_only(mut self) -> Self {
        if let Some(c) = &mut self.capture {
            c.admitted = CaptureResult::NotApplicable;
            c.response = None;
            if c.provisional {
                c.batch = None;
                c.provisional = false;
            }
        }
        self
    }
    /// Direct fallback creates its own batch synchronously. Adopt it for the
    /// optional follow-up only while the committed Stop is still current.
    pub fn adopt_direct_batch(&mut self, database: &Path) -> Result<(), StorageError> {
        let Some(c) = self.capture.as_mut().filter(|c| c.provisional) else {
            return Ok(());
        };
        let mut db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| classify(e, "Read direct capture batch"))?;
        db.busy_timeout(Duration::ZERO)
            .map_err(|e| classify(e, "Disable direct follow-up writer wait"))?;
        let tx = db
            .transaction()
            .map_err(|e| classify(e, "Read direct follow-up snapshot"))?;
        super::super::migrations::require_current(&tx)?;
        let rows = BindingRows(&tx);
        let Some(binding) = rows
            .entry_by_id(&c.identity)?
            .and_then(|entry| entry.binding)
        else {
            return Ok(());
        };
        let preferences = rows.session_preferences(&c.identity)?;
        let activity = preferences
            .remembered
            .as_ref()
            .and_then(|r| r.state.as_ref())
            .and_then(driver_state::state_activity);
        if binding.id == c.binding
            && binding_fingerprint(&tx, &c.identity)? == c.binding_fingerprint
            && activity.as_ref().is_some_and(|a| {
                a.phase == tmt_core::binding::session::activity::ActivityPhase::Idle
                    && a.process.pid() == c.pid
                    && a.process.start_identity() == c.start
                    && a.session.as_str() == c.session
                    && a.turn.as_ref().map(|v| v.as_str()) == c.turn.as_deref()
                    && a.last_activity_ms == c.now_ms
            })
            && let Some(batch) = capture_batches::current(&tx, &c.identity)?
        {
            c.batch = Some(batch);
            c.provisional = false;
        }
        Ok(())
    }
    /// Capture a diagnostic-only Stop's launch and batch in one read snapshot.
    /// The existing launch-owner check still applies when the flusher commits it.
    pub fn diagnostic(database: &Path, follow_up: CaptureFollowUp) -> Result<Self, StorageError> {
        let learning = follow_up
            .learning
            .as_ref()
            .ok_or_else(|| invalid("Missing Stop diagnostic"))?;
        let launch = &learning.launch;
        let owner = launch
            .owner()
            .filter(|_| launch.valid())
            .ok_or_else(|| invalid("Invalid Stop launch"))?;
        let mut db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| classify(e, "Read Stop diagnostic authority"))?;
        db.busy_timeout(Duration::ZERO)
            .map_err(|e| classify(e, "Disable Stop diagnostic read writer wait"))?;
        let tx = db
            .transaction()
            .map_err(|e| classify(e, "Read Stop diagnostic snapshot"))?;
        super::super::migrations::require_current(&tx)?;
        let rows = BindingRows(&tx);
        let binding = rows
            .entry_by_id(&launch.identity_id)?
            .and_then(|e| e.binding);
        if binding.as_ref().is_none_or(|b| b.id != launch.binding_id)
            || rows
                .launch_observation(&launch.binding_id, &owner)?
                .is_none_or(|observation| observation.driver.as_str() != learning.provider)
        {
            return Err(invalid("Stop diagnostic launch changed before enqueue"));
        }
        let batch = capture_batches::current(&tx, &launch.identity_id)?;
        tx.commit()
            .map_err(|e| classify(e, "Finish Stop diagnostic snapshot"))?;
        Ok(Self {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            capture: None,
            diagnostic_fence: Some(DiagnosticFence { batch }),
            follow_up,
        })
    }
    pub(super) fn valid(&self) -> bool {
        self.version == 1
            && uuid::Uuid::parse_str(&self.id).is_ok()
            && match (&self.capture, &self.diagnostic_fence) {
                (Some(_), None) => true,
                (None, Some(fence)) => {
                    self.follow_up.learning.is_some()
                        && fence
                            .batch
                            .as_ref()
                            .is_none_or(|id| uuid::Uuid::parse_str(id).is_ok())
                }
                _ => false,
            }
            && self.capture.as_ref().is_none_or(|c| {
                uuid::Uuid::parse_str(&c.identity).is_ok()
                    && !c.binding.is_empty()
                    && c.binding_fingerprint.len() == 64
                    && c.binding_fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
                    && ProcessIncarnation::new(c.pid, &c.start).is_ok()
                    && ProviderSessionId::new(&c.session).is_ok()
                    && c.now_ms <= tmt_core::limits::MAX_JS_SAFE_INTEGER
                    && (!c.provisional
                        || c.batch
                            .as_ref()
                            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()))
                    && c.response.as_ref().is_none_or(|r| r.decode().is_ok())
                    && matches!(
                        c.admitted,
                        CaptureResult::Committed
                            | CaptureResult::Duplicate
                            | CaptureResult::NotApplicable
                            | CaptureResult::NoAdmittedStart
                    )
            })
            && self
                .follow_up
                .learning
                .as_ref()
                .is_none_or(|l| l.launch.valid())
    }
    /// Read-only admission shares one WAL snapshot with the batch locator.
    /// Queue data never turns an unverified environment locator into authority.
    pub fn prepare(
        database: &Path,
        capture: &super::super::TurnCapture<'_>,
        mut follow_up: CaptureFollowUp,
    ) -> Result<Self, StorageError> {
        let binding = capture
            .expected
            .entry
            .binding
            .as_ref()
            .ok_or_else(|| invalid("Capture has no binding"))?;
        if binding.session.state != tmt_core::binding::session::RuntimeState::Running
            || !matches!(
                capture.activity.event.kind,
                EventKind::TurnEnded | EventKind::ContinuationEnded
            )
        {
            return Err(invalid("Capture is not an admitted turn end"));
        }
        let mut db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| classify(e, "Read capture enqueue authority"))?;
        db.busy_timeout(Duration::ZERO)
            .map_err(|e| classify(e, "Disable capture read writer wait"))?;
        let tx = db
            .transaction()
            .map_err(|e| classify(e, "Read capture authority snapshot"))?;
        super::super::migrations::require_current(&tx)?;
        let rows = BindingRows(&tx);
        if rows
            .entry_by_id(&binding.identity_id)?
            .and_then(|e| e.binding)
            .as_ref()
            != Some(binding)
        {
            return Err(invalid("Capture binding changed before enqueue"));
        }
        let mut preferences = rows.session_preferences(&binding.identity_id)?;
        let remembered = preferences
            .remembered
            .as_ref()
            .ok_or_else(|| invalid("Capture session disappeared"))?;
        let expected = capture
            .expected
            .preferences
            .remembered
            .as_ref()
            .ok_or_else(|| invalid("Capture has no remembered session"))?;
        let mut selection = preferences.clone();
        selection.remembered.as_mut().unwrap().state = expected.state.clone();
        if selection != capture.expected.preferences
            || !binding.session.key.as_ref().is_some_and(|k| {
                k.incarnation == *capture.process
                    && k.provider_session.as_ref() == Some(&remembered.provider_session)
            })
        {
            return Err(invalid("Capture session changed before enqueue"));
        }
        let next = driver_state::after_activity(
            remembered.state.as_ref(),
            &remembered.provider_session,
            capture.process,
            &capture.activity.event,
            capture.activity.model.as_deref(),
            capture.now_ms,
        );
        let previous = remembered
            .state
            .as_ref()
            .and_then(driver_state::state_activity)
            .filter(|a| a.matches(&remembered.provider_session, capture.process));
        let mut batch = capture_batches::current(&tx, &binding.identity_id)?;
        let admitted = if next.is_some() {
            CaptureResult::Committed
        } else if capture.activity.event.kind == EventKind::TurnEnded
            && previous.as_ref().is_some_and(|a| {
                a.phase == tmt_core::binding::session::activity::ActivityPhase::Idle
                    && a.turn == capture.activity.event.turn
                    && capture.now_ms >= a.last_activity_ms
            })
        {
            if capture.response.is_some() && !capture.interrupted && batch.is_some() {
                CaptureResult::Committed
            } else {
                CaptureResult::Duplicate
            }
        } else if !capture.interrupted
            && previous
                .as_ref()
                .is_none_or(|a| a.turn != capture.activity.event.turn)
        {
            CaptureResult::NoAdmittedStart
        } else {
            CaptureResult::NotApplicable
        };
        let provisional = batch.is_none() && admitted == CaptureResult::Committed;
        if provisional {
            batch = Some(uuid::Uuid::new_v4().to_string());
        }
        let remembered = preferences.remembered.take().unwrap();
        if !follow_up.usage {
            follow_up.transcript = None;
        }
        if follow_up
            .transcript
            .as_ref()
            .is_some_and(|p| p.to_str().is_none_or(|s| s.len() > 4096))
        {
            follow_up.usage = false;
            follow_up.transcript = None;
        }
        let mut event = Self {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            diagnostic_fence: None,
            capture: Some(PendingCapture {
                identity: binding.identity_id.clone(),
                binding: binding.id.clone(),
                binding_fingerprint: binding_fingerprint(&tx, &binding.identity_id)?,
                batch,
                provisional,
                provider: remembered.harness.as_str().into(),
                session: remembered.provider_session.as_str().into(),
                pid: capture.process.pid(),
                start: capture.process.start_identity().into(),
                turn: capture
                    .activity
                    .event
                    .turn
                    .as_ref()
                    .map(|v| v.as_str().into()),
                continuation: capture.activity.event.kind == EventKind::ContinuationEnded,
                model: capture.activity.model.clone(),
                interrupted: capture.interrupted,
                response: capture
                    .response
                    .as_ref()
                    .filter(|_| !capture.interrupted && admitted == CaptureResult::Committed)
                    .map(|r| KeptFinal {
                        text: r.text.text().into(),
                        original_bytes: r.text.original_bytes(),
                        digest: r.text.digest().map(str::to_owned),
                    }),
                now_ms: capture.now_ms,
                admitted,
            }),
            follow_up,
        };
        // Optional telemetry locators cannot displace the supplied final from
        // its bounded durable envelope. Oversized telemetry stays silent.
        if event.follow_up.usage
            && serde_json::to_vec(&event)
                .map_err(|_| invalid("Encode capture metadata"))?
                .len()
                > super::ENTRY_LIMIT
        {
            event.follow_up.usage = false;
            event.follow_up.transcript = None;
        }
        tx.commit()
            .map_err(|e| classify(e, "Finish capture authority read"))?;
        Ok(event)
    }
}

impl Storage {
    pub(in crate::storage) fn prune_hook_receipts(
        &mut self,
        now: u64,
        limit: u64,
    ) -> Result<(), StorageError> {
        let mut maintenance = Self::open_hook_flusher(&self.path)?;
        let db = maintenance
            .connection
            .as_mut()
            .ok_or_else(|| invalid("Closed hook retention storage"))?;
        db.busy_timeout(Duration::ZERO)
            .map_err(|e| classify(e, "Disable hook retention writer wait"))?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| classify(e, "Begin hook retention"))?;
        let queue = super::HookQueue::begin_retention(&tx, now, limit)?;
        tx.commit()
            .map_err(|e| classify(e, "Commit hook receipt retention"))?;
        if let Some(queue) = queue {
            queue.finish_retention()?;
        }
        maintenance.close()
    }
    /// Existing compatible database only. SQLite alone waits for its writer;
    /// no schema upgrade, WAL retry loop or hook deadline is renewed here.
    pub fn open_hook_flusher(path: &Path) -> Result<Self, StorageError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| classify(e, "Open capture flusher storage"))?;
        connection
            .busy_timeout(Duration::from_secs(30))
            .map_err(|e| classify(e, "Set capture flusher writer wait"))?;
        super::super::migrations::require_current(&connection)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| classify(e, "Configure capture flusher foreign keys"))?;
        Ok(Self {
            path: path.into(),
            connection: Some(connection),
            capturing: false,
            hook_allowance: None,
        })
    }
    /// A completed Core transaction may wake the finite worker after a crash or
    /// failed spawn. Notification never ingests or changes the caller's snapshot.
    pub(in crate::storage) fn notify_pending_hook_captures(&self) -> Result<(), StorageError> {
        if !super::HookQueue::exists(&self.path) {
            return Ok(());
        }
        let deadline = self
            .hook_allowance
            .unwrap_or_else(|| std::time::Instant::now() + Duration::from_secs(3));
        if std::time::Instant::now() >= deadline {
            return Ok(());
        }
        let queue = match super::HookQueue::open(&self.path, Duration::ZERO) {
            Ok(queue) => queue,
            Err(error) if error.code == super::super::StorageErrorCode::IncompatibleSchema => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if queue.has_entries()? {
            queue
                .start_flusher(deadline)
                .map_err(|error| super::super::errors::classify_io(error, "Notify hook flusher"))?;
        }
        Ok(())
    }
    /// Explicit pre-snapshot callers ingest a bounded productive prefix.
    /// Follow-up telemetry remains flusher-owned.
    pub fn ingest_pending_hook_captures(&mut self) -> Result<(), StorageError> {
        self.ingest_pending_for_identity(None)
    }
    /// Digest must order its own pending capture before publication without
    /// waiting for an unrelated identity's backlog.
    pub fn ingest_pending_hook_captures_for_identity(
        &mut self,
        identity: &str,
    ) -> Result<(), StorageError> {
        self.ingest_pending_for_identity(Some(identity))
    }
    fn ingest_pending_for_identity(&mut self, identity: Option<&str>) -> Result<(), StorageError> {
        if !super::HookQueue::exists(&self.path) {
            return Ok(());
        }
        let queue = match super::HookQueue::open(&self.path, Duration::from_millis(100)) {
            Ok(queue) => queue,
            Err(error) if error.code == super::super::StorageErrorCode::IncompatibleSchema => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if !queue.has_entries()? {
            return Ok(());
        }
        let deadline = self
            .hook_allowance
            .unwrap_or_else(|| std::time::Instant::now() + Duration::from_secs(3));
        let _ = queue.start_flusher(deadline);
        for _ in 0..64 {
            let next = match identity {
                Some(identity) => queue.next_uncommitted_for_identity(identity)?,
                None => queue.next_uncommitted()?,
            };
            let Some(event) = next else {
                return Ok(());
            };
            self.ingest_hook_capture(&event)?;
            queue.mark_committed(&event)?;
        }
        let remaining = match identity {
            Some(identity) => queue.next_uncommitted_for_identity(identity)?,
            None => queue.next_uncommitted()?,
        };
        if remaining.is_some() {
            return Err(StorageError::new(
                super::super::StorageErrorCode::Busy,
                "Additional hook captures remain for the flusher",
            ));
        }
        Ok(())
    }
    pub fn try_capture_failure(
        path: &Path,
        expected: &super::super::IdentityContextSnapshot,
        reason: super::super::CaptureFailure,
        deadline: std::time::Instant,
    ) -> Result<(), StorageError> {
        let mut storage = Self::open_hook_flusher(path)?;
        storage
            .connection()?
            .busy_timeout(Duration::ZERO)
            .map_err(|e| classify(e, "Disable capture failure writer wait"))?;
        storage.record_capture_failure(
            expected,
            reason,
            crate::request_runtime::wall_time_ms(),
            deadline,
        )?;
        storage.close()
    }
    /// The terminal Stop diagnostic and its batch fence share one writer.
    /// A newer prompt can clear it without a delayed flusher restoring it later.
    pub fn update_hook_capture_learning(
        &mut self,
        event: &CaptureEvent,
        operation: impl FnOnce(
            &mut dyn BindingRecords<Error = StorageError>,
        ) -> Result<bool, StorageError>,
    ) -> Result<bool, StorageError> {
        super::super::identities::commit_immediate_transaction(
            self,
            "queued Stop diagnostic",
            |db| {
                if !current_event(db, event)? {
                    return Ok(false);
                }
                operation(&mut BindingRows(db))
            },
        )
    }
    pub fn ingest_hook_capture(
        &mut self,
        event: &CaptureEvent,
    ) -> Result<CaptureResult, StorageError> {
        super::super::identities::commit_immediate_transaction(self, "queued capture", |db| {
            ingest(db, event)
        })
    }
}

fn ingest(db: &Connection, event: &CaptureEvent) -> Result<CaptureResult, StorageError> {
    if !event.valid() {
        return Err(invalid("Invalid hook capture event"));
    }
    let receipt: Option<String> = db
        .query_row(
            "SELECT outcome FROM hook_capture_receipts WHERE event_id=?",
            [event.id()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| classify(e, "Read capture ingestion receipt"))?;
    if let Some(outcome) = receipt {
        return serde_json::from_value(serde_json::Value::String(outcome))
            .map_err(|_| invalid("Invalid capture receipt"));
    }
    let result = match &event.capture {
        Some(c) => commit(db, c)?,
        None => CaptureResult::NotApplicable,
    };
    db.execute("INSERT INTO hook_capture_receipts(event_id,identity_id,batch_id,outcome,committed_ms) VALUES(?,?,?,?,?)",params![event.id(),event.identity(),event.capture.as_ref().and_then(|c|c.batch.as_deref()),result.token(),crate::request_runtime::wall_time_ms() as i64]).map_err(|e|classify(e,"Commit capture ingestion receipt"))?;
    Ok(result)
}

fn binding_fingerprint(db: &Connection, identity: &str) -> Result<String, StorageError> {
    let sql = format!(
        "SELECT json_array({}) FROM bindings b WHERE b.identity_id=?",
        super::super::bindings::BINDING_COLUMNS
    );
    let value: String = db
        .query_row(&sql, [identity], |r| r.get(0))
        .map_err(|e| classify(e, "Read capture binding fence"))?;
    Ok(tmt_core::content_digest::sha256(value.as_bytes()))
}

fn current_event(db: &Connection, event: &CaptureEvent) -> Result<bool, StorageError> {
    match (&event.capture, &event.diagnostic_fence) {
        (Some(c), None) => current_turn(db, c),
        (None, Some(fence)) => {
            let Some(learning) = &event.follow_up.learning else {
                return Ok(false);
            };
            let launch = &learning.launch;
            let binding = BindingRows(db)
                .entry_by_id(&launch.identity_id)?
                .and_then(|e| e.binding);
            Ok(binding.as_ref().is_some_and(|b| b.id == launch.binding_id)
                && capture_batches::current(db, &launch.identity_id)? == fence.batch)
        }
        _ => Ok(false),
    }
}

fn current_turn(db: &Connection, c: &PendingCapture) -> Result<bool, StorageError> {
    let binding = BindingRows(db)
        .entry_by_id(&c.identity)?
        .and_then(|e| e.binding);
    Ok(binding.as_ref().is_some_and(|b| {
        b.id == c.binding
            && b.session.state == tmt_core::binding::session::RuntimeState::Running
            && b.session.key.as_ref().is_some_and(|k| {
                k.incarnation.pid() == c.pid
                    && k.incarnation.start_identity() == c.start
                    && k.provider_session
                        .as_ref()
                        .is_some_and(|s| s.as_str() == c.session)
            })
    }) && {
        let current = capture_batches::current(db, &c.identity)?;
        current == c.batch || (c.provisional && current.is_none())
    } && binding_fingerprint(db, &c.identity)? == c.binding_fingerprint)
}

fn commit(db: &Connection, c: &PendingCapture) -> Result<CaptureResult, StorageError> {
    let mut rows = BindingRows(db);
    let Some(_entry) = rows.entry_by_id(&c.identity)? else {
        return Ok(CaptureResult::RetiredIdentity);
    };
    let same_turn = current_turn(db, c)?;
    let batch_exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM capture_batches WHERE id=? AND identity_id=?)",
            params![c.batch, c.identity],
            |r| r.get(0),
        )
        .map_err(|e| classify(e, "Check queued capture batch"))?;
    if c.batch.is_none() && c.admitted != CaptureResult::Committed {
        return Ok(c.admitted);
    }
    if !batch_exists {
        if !c.provisional {
            return Ok(CaptureResult::MissingBatch);
        }
        // Only a provisional id represents a batch absent at admission. An
        // older turn is inserted closed, without displacing a newer batch.
        db.execute(
            "INSERT INTO capture_batches(id,identity_id,binding_id,driver,session_id,runtime_pid,runtime_start,provider_turn,created_ms,closed_ms) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params![c.batch, c.identity, c.binding, c.provider, c.session, c.pid as i64, c.start, c.turn, c.now_ms as i64, (!same_turn).then_some(c.now_ms as i64)],
        ).map_err(|e| classify(e, "Create provisional capture batch"))?;
    }
    if c.admitted != CaptureResult::Committed {
        return Ok(c.admitted);
    }
    let response = c
        .response
        .as_ref()
        .map(|response| {
            response
                .decode()
                .map(|text| responses::ResponseCapture { text })
        })
        .transpose()?;
    if !c.continuation
        && let Some(response) = response.as_ref()
        && responses::same_final_in_batch(db, c.batch.as_deref(), response)?
    {
        return Ok(CaptureResult::Duplicate);
    }
    if same_turn {
        let mut preferences = rows.session_preferences(&c.identity)?;
        if let Some(remembered) = preferences.remembered.as_mut().filter(|r| {
            r.harness.as_str() == c.provider && r.provider_session.as_str() == c.session
        }) {
            let process = ProcessIncarnation::new(c.pid, &c.start)
                .map_err(|_| invalid("Invalid queued process"))?;
            let activity = ActivityObservation {
                event: Event {
                    kind: if c.continuation {
                        EventKind::ContinuationEnded
                    } else {
                        EventKind::TurnEnded
                    },
                    turn: c
                        .turn
                        .as_ref()
                        .map(|t| {
                            ProviderSessionId::new(t).map_err(|_| invalid("Invalid queued turn"))
                        })
                        .transpose()?,
                },
                model: c.model.clone(),
            };
            if let Some(next) = driver_state::after_activity(
                remembered.state.as_ref(),
                &remembered.provider_session,
                &process,
                &activity.event,
                activity.model.as_deref(),
                c.now_ms,
            ) {
                remembered.state = Some(next);
                rows.set_session_preferences(&c.identity, &preferences)?;
            }
        }
    }
    let retained = response
        .as_ref()
        .map(|response| {
            responses::append_to_batch(
                db,
                &c.identity,
                &c.provider,
                &c.session,
                c.now_ms,
                response,
                c.batch.as_deref(),
            )
        })
        .transpose()?;
    // Settlement belongs only to the current turn. A historical final must not
    // close or change requests, passthrough, activity or Digest of a newer one.
    if same_turn {
        if c.continuation {
            super::super::requests::end_digest_continuation(db, &c.identity, c.now_ms)?;
        }
        super::super::requests::turns::close(db, &c.identity, c.now_ms)?;
        super::super::requests::passthrough::settle_turn(
            db,
            &c.identity,
            &c.session,
            c.turn.as_deref(),
            retained.zip(response.as_ref()).map(|(response, text)| {
                super::super::requests::passthrough::SettledFinal {
                    text: &text.text,
                    provider: &c.provider,
                    response,
                }
            }),
            c.now_ms,
        )?;
        if c.interrupted {
            capture_batches::close(db, &c.identity, c.now_ms)?;
        }
    }
    Ok(CaptureResult::Committed)
}
