//! Queue regressions exercise real private SQLite files and durable state.
use super::super::{
    ResponseCapture, Storage, TurnCapture,
    bindings::BindingRows,
    capture_batches::{begin, current},
};
use super::*;
use crate::{runtime::lifecycle::RuntimeLifecycle, test_support::TestDirectory};
use std::time::Instant;
use tmt_core::{
    binding::{
        BindingRecords, BindingRepository,
        session::{
            ProviderSessionId,
            activity::{ActivityPhase, Event, EventKind},
        },
    },
    endpoint::ProcessIncarnation,
    identity::Lifetime,
};
struct World {
    _directory: TestDirectory,
    path: std::path::PathBuf,
    storage: Storage,
    agent: String,
    process: ProcessIncarnation,
    now: u64,
}
impl World {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let path = directory.path.join("capture.db");
        let mut storage = Storage::open(&path).unwrap();
        let agent = tmt_core::identity::create_or_resolve(&mut storage, "Agent", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        let now = crate::request_runtime::wall_time_ms();
        storage.connection().unwrap().execute("INSERT INTO bindings(id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at,runtime_state,runtime_pid,runtime_start_identity,observed_provider_session_id)
            VALUES('11111111-1111-4111-8111-111111111111',?1,'tmux','%1','server','/private/fake.sock',10,'server-start',11,'t','t','running',12,'provider-start','fixture-session')",[&agent]).unwrap();
        storage.connection().unwrap().execute("INSERT INTO identity_session_preferences(identity_id,remembered_harness,runtime_mode,provider_session_id) VALUES(?1,'claude','plain','fixture-session')",[&agent]).unwrap();
        let process = ProcessIncarnation::new(12, "provider-start").unwrap();
        let mut preferences = BindingRows(storage.connection().unwrap())
            .session_preferences(&agent)
            .unwrap();
        preferences.remembered.as_mut().unwrap().state =
            crate::runtime::driver_state::after_activity(
                None,
                &ProviderSessionId::new("fixture-session").unwrap(),
                &process,
                &Event {
                    kind: EventKind::TurnStarted,
                    turn: None,
                },
                None,
                now,
            );
        BindingRows(storage.connection().unwrap())
            .set_session_preferences(&agent, &preferences)
            .unwrap();
        let snapshot = Storage::context_by_identity(&path, &agent, now)
            .unwrap()
            .unwrap();
        begin(
            storage.connection().unwrap(),
            snapshot.entry.binding.as_ref().unwrap(),
            preferences.remembered.as_ref().unwrap(),
            None,
            now,
        )
        .unwrap();
        Self {
            _directory: directory,
            path,
            storage,
            agent,
            process,
            now,
        }
    }
}

impl World {
    fn event(&self, text: &str, continuation: bool) -> CaptureEvent {
        self.event_with_follow_up(text, continuation, CaptureFollowUp::default())
    }
    fn event_with_follow_up(
        &self,
        text: &str,
        continuation: bool,
        follow_up: CaptureFollowUp,
    ) -> CaptureEvent {
        let lifecycle = crate::drivers::claude::ClaudeLifecycle;
        let payload=serde_json::json!({"hook_event_name":"Stop","session_id":"fixture-session","stop_hook_active":continuation,"last_assistant_message":text}).to_string();
        let turn = lifecycle.decode_turn(payload.as_bytes()).unwrap();
        let activity = lifecycle.decode_activity(payload.as_bytes()).unwrap();
        let expected = Storage::context_by_identity(&self.path, &self.agent, self.now)
            .unwrap()
            .unwrap();
        CaptureEvent::prepare(
            &self.path,
            &TurnCapture {
                expected: &expected,
                process: &self.process,
                activity: &activity,
                response: Some(ResponseCapture {
                    text: turn.final_text.unwrap(),
                }),
                interrupted: false,
                now_ms: self.now + 1,
                deadline: Instant::now() + Duration::from_secs(2),
            },
            follow_up,
        )
        .unwrap()
    }
    fn final_count(&self) -> i64 {
        self.storage
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM recent_responses", [], |r| r.get(0))
            .unwrap()
    }
    fn phase(&self) -> ActivityPhase {
        let p = BindingRows(self.storage.connection().unwrap())
            .session_preferences(&self.agent)
            .unwrap();
        crate::runtime::driver_state::state_activity(p.remembered.unwrap().state.as_ref().unwrap())
            .unwrap()
            .phase
    }
    fn new_prompt(&mut self) -> String {
        self.now += 10;
        let mut p = BindingRows(self.storage.connection().unwrap())
            .session_preferences(&self.agent)
            .unwrap();
        p.remembered.as_mut().unwrap().state = crate::runtime::driver_state::after_activity(
            None,
            &ProviderSessionId::new("fixture-session").unwrap(),
            &self.process,
            &Event {
                kind: EventKind::TurnStarted,
                turn: None,
            },
            None,
            self.now,
        );
        BindingRows(self.storage.connection().unwrap())
            .set_session_preferences(&self.agent, &p)
            .unwrap();
        let b = BindingRows(self.storage.connection().unwrap())
            .entry_by_id(&self.agent)
            .unwrap()
            .unwrap()
            .binding
            .unwrap();
        begin(
            self.storage.connection().unwrap(),
            &b,
            p.remembered.as_ref().unwrap(),
            None,
            self.now,
        )
        .unwrap()
    }
}

#[test]
fn enqueue_does_not_wait_for_the_main_writer_and_replay_after_commit_is_a_noop() {
    let mut w = World::new();
    let db = Connection::open(&w.path).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let event = w.event("the supplied final", false);
    let queue = HookQueue::open(&w.path, Duration::from_millis(100)).unwrap();
    queue.enqueue(&event).unwrap();
    assert_eq!(w.final_count(), 0);
    assert_eq!(w.phase(), ActivityPhase::Working);
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        w.storage
            .ingest_hook_capture(&queue.next().unwrap().unwrap())
            .unwrap(),
        CaptureResult::Committed
    );
    // Crash between main commit and queue acknowledgment: replay the durable row.
    assert_eq!(
        w.storage
            .ingest_hook_capture(&queue.next().unwrap().unwrap())
            .unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.final_count(), 1);
    assert_eq!(w.phase(), ActivityPhase::Idle);
    let text: String = w
        .storage
        .connection()
        .unwrap()
        .query_row("SELECT text FROM recent_responses", [], |r| r.get(0))
        .unwrap();
    assert_eq!(text, "the supplied final");
    queue.acknowledge(&event).unwrap();
    assert!(queue.next().unwrap().is_none());
}
#[test]
fn a_first_stop_without_an_input_batch_keeps_its_final_and_settles_activity() {
    let mut w = World::new();
    w.storage
        .connection()
        .unwrap()
        .execute("DELETE FROM capture_batches", [])
        .unwrap();
    let first = w.event("first standalone answer", false);
    let retry = w.event("first standalone answer", false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&first).unwrap();
    queue.enqueue(&retry).unwrap();
    let first = queue.next().unwrap().unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&first).unwrap(),
        CaptureResult::Committed
    );
    queue.acknowledge(&first).unwrap();
    let retry = queue.next().unwrap().unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&retry).unwrap(),
        CaptureResult::Duplicate
    );
    assert_eq!(w.final_count(), 1);
    assert_eq!(w.phase(), ActivityPhase::Idle);
    let batch: Option<String> = w
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT batch_id FROM capture_entries WHERE kind='final'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        batch.is_some(),
        "standalone finals must have an input batch"
    );
    let page = w
        .storage
        .conversation_timeline(&w.agent, 50, None, w.now + 100)
        .unwrap();
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page["entries"][0]["kind"], "final");
    assert_eq!(page["responses"][0]["text"], "first standalone answer");
}

#[test]
fn a_first_stop_committing_between_retry_admission_and_enqueue_still_dedupes() {
    let mut w = World::new();
    w.storage
        .connection()
        .unwrap()
        .execute("DELETE FROM capture_batches", [])
        .unwrap();
    let first = w.event("first standalone answer", false);
    let retry = w.event("first standalone answer", false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&first).unwrap();
    let first = queue.next().unwrap().unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&first).unwrap(),
        CaptureResult::Committed
    );
    queue.acknowledge(&first).unwrap();
    queue.enqueue(&retry).unwrap();
    let retry = queue.next().unwrap().unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&retry).unwrap(),
        CaptureResult::Duplicate
    );
    assert_eq!(w.final_count(), 1);
    assert_eq!(w.phase(), ActivityPhase::Idle);
}

#[test]
fn direct_fallback_adopts_only_its_own_new_batch_for_follow_up() {
    let mut w = World::new();
    w.storage
        .connection()
        .unwrap()
        .execute("DELETE FROM capture_batches", [])
        .unwrap();
    let mut queued = w.event("direct answer", false);
    let lifecycle = crate::drivers::claude::ClaudeLifecycle;
    let payload = br#"{"hook_event_name":"Stop","session_id":"fixture-session","last_assistant_message":"direct answer"}"#;
    let turn = lifecycle.decode_turn(payload).unwrap();
    let activity = lifecycle.decode_activity(payload).unwrap();
    let expected = Storage::context_by_identity(&w.path, &w.agent, w.now)
        .unwrap()
        .unwrap();
    assert_eq!(
        w.storage
            .capture_turn_end(TurnCapture {
                expected: &expected,
                process: &w.process,
                activity: &activity,
                response: Some(ResponseCapture {
                    text: turn.final_text.unwrap()
                }),
                interrupted: false,
                now_ms: w.now + 1,
                deadline: Instant::now() + Duration::from_secs(2),
            })
            .unwrap(),
        super::super::TurnCaptureOutcome::Committed
    );
    queued.adopt_direct_batch(&w.path).unwrap();
    assert!(queued.is_current_batch(&w.path).unwrap());
    w.new_prompt();
    assert!(!queued.is_current_batch(&w.path).unwrap());
}

#[test]
fn a_late_first_stop_without_an_input_batch_keeps_its_final_without_idling_the_new_turn() {
    let mut w = World::new();
    w.storage
        .connection()
        .unwrap()
        .execute("DELETE FROM capture_batches", [])
        .unwrap();
    let event = w.event("standalone old turn", false);
    let newer = w.new_prompt();
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.final_count(), 1);
    assert_eq!(w.phase(), ActivityPhase::Working);
    assert_eq!(
        current(w.storage.connection().unwrap(), &w.agent)
            .unwrap()
            .unwrap(),
        newer
    );
    let batch: Option<String> = w
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT batch_id FROM capture_entries WHERE kind='final'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(batch.is_some());
    assert_ne!(batch.unwrap(), newer);
}

#[test]
fn a_rebind_after_a_first_stop_without_an_input_batch_keeps_its_final() {
    let mut w = World::new();
    w.storage
        .connection()
        .unwrap()
        .execute("DELETE FROM capture_batches", [])
        .unwrap();
    let event = w.event("standalone restored answer", false);
    w.storage
        .connection()
        .unwrap()
        .execute("UPDATE bindings SET id='restored-binding'", [])
        .unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.final_count(), 1);
    assert_eq!(w.phase(), ActivityPhase::Working);
}

#[test]
fn a_late_final_keeps_its_input_batch_and_never_idles_a_new_prompt() {
    let mut w = World::new();
    let event = w.event("old turn answer", false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&event).unwrap();
    let event = queue.next().unwrap().unwrap();
    let old = current(w.storage.connection().unwrap(), &w.agent)
        .unwrap()
        .unwrap();
    let newer = w.new_prompt();
    assert_ne!(old, newer);
    let db = w.storage.connection().unwrap();
    for request in ["new-request", "new-person-input"] {
        db.execute("INSERT INTO request_attempts(attempt_id,request_id,route_kind,recipient_identity_id,originator_kind,originator_identity_id,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms)
            VALUES(?1,?1,'inbox',?2,'explicit',?2,0,'sent',0,0,?3,?4,?4)", params![request,w.agent,w.now as i64,(w.now+60_000) as i64]).unwrap();
    }
    db.execute(
        "INSERT INTO request_turns(request_id,started_ms) VALUES('new-request',?)",
        [w.now as i64],
    )
    .unwrap();
    db.execute("INSERT INTO request_passthrough(request_id,delivered_at_ms,state,bound_session,bound_at_ms,folded)
        VALUES('new-person-input',?1,'bound','fixture-session',?1,1)", [w.now as i64]).unwrap();
    let unsettled = |db: &Connection| {
        db.query_row("SELECT json_array(t.started_ms,t.ended_ms,p.state,p.settled_at_ms,p.answer_response)
        FROM request_turns t CROSS JOIN request_passthrough p WHERE t.request_id='new-request' AND p.request_id='new-person-input'", [], |row|row.get::<_,String>(0)).unwrap()
    };
    let before = unsettled(db);
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.phase(), ActivityPhase::Working);
    assert_eq!(
        current(w.storage.connection().unwrap(), &w.agent)
            .unwrap()
            .unwrap(),
        newer
    );
    let batch: String = w
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT batch_id FROM capture_entries WHERE kind='final'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(batch, old);
    assert_eq!(
        unsettled(w.storage.connection().unwrap()),
        before,
        "an older final cannot settle the newer request or passthrough"
    );
}
#[test]
fn a_rebind_after_verified_enqueue_keeps_the_final_without_changing_new_activity() {
    let mut w = World::new();
    let event = w.event("survives restore", false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&event).unwrap();
    let event = queue.next().unwrap().unwrap();
    w.storage
        .connection()
        .unwrap()
        .execute("UPDATE bindings SET id='restored-binding'", [])
        .unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.final_count(), 1);
    assert_eq!(w.phase(), ActivityPhase::Working);
}
#[test]
fn a_distinct_final_survives_a_missing_prompt_after_the_previous_stop() {
    let mut w = World::new();
    let first = w.event("the previous turn's final", false);
    assert_eq!(
        w.storage.ingest_hook_capture(&first).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.phase(), ActivityPhase::Idle);
    let batch = current(w.storage.connection().unwrap(), &w.agent)
        .unwrap()
        .unwrap();
    w.now += 10;
    let sent_at = w.now;
    let db = w.storage.connection().unwrap();
    db.execute("INSERT INTO request_attempts(attempt_id,request_id,route_kind,recipient_identity_id,originator_kind,originator_identity_id,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,settled_at_ms,expires_at_ms,retention_expires_at_ms,message_text,message_bytes)
        VALUES('unbound-input','unbound-input','inbox',?1,'explicit',?1,0,'sent',0,0,?2,?2,?3,?3,'a new question',14)", params![w.agent,sent_at as i64,(sent_at+86_400_000) as i64]).unwrap();
    db.execute(
        "INSERT INTO request_delivery_policies(request_id,urgent,kind,automatic,mode)
        VALUES('unbound-input',0,'fyi',0,'passthrough')",
        [],
    )
    .unwrap();
    super::super::requests::passthrough::start_capture(db, "unbound-input", sent_at, None).unwrap();
    // Reproduce the owner report: no new prompt/batch was admitted, but the
    // same verified provider process supplies a different ordinary final.
    let second = w.event("the next turn's distinct final", false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&second).unwrap();
    let queued = queue.next().unwrap().unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&queued).unwrap(),
        CaptureResult::Committed,
        "a different final is not a duplicate of the previous turn"
    );
    queue.acknowledge(&queued).unwrap();
    assert!(queue.next().unwrap().is_none());
    let page = w
        .storage
        .recent_responses(&w.agent, 10, None, w.now + 10)
        .unwrap();
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["text"], "the next turn's distinct final");
    assert_eq!(items[1]["text"], "the previous turn's final");
    assert_ne!(items[0]["responseId"], items[1]["responseId"]);
    let finals: i64 = w
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM capture_entries WHERE kind='final'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        finals, 2,
        "each retained answer is visible in the chat timeline"
    );
    let recorded_batches: Vec<String> = w
        .storage
        .connection()
        .unwrap()
        .prepare("SELECT batch_id FROM capture_entries WHERE kind='final' ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(recorded_batches, [batch.clone(), batch.clone()]);
    let timeline = w
        .storage
        .conversation_timeline(&w.agent, 10, None, w.now + 10)
        .unwrap();
    let timeline_finals: Vec<_> = timeline["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "final")
        .collect();
    assert_eq!(timeline_finals.len(), 2);
    assert!(
        timeline_finals
            .iter()
            .all(|entry| entry["inputBatchId"] == batch)
    );
    assert_eq!(timeline["responses"].as_array().unwrap().len(), 2);
    // A final cannot guess which unadmitted prompt it answers. The request
    // instead exposes the existing bounded no-capture outcome, with no sleep
    // or invented association to the older batch's final.
    use tmt_core::request::{RequestRepository, delivery, passthrough};
    let observed_at = sent_at + passthrough::BIND_WINDOW_MS + 1;
    let view = w
        .storage
        .with_request_observation(|records| {
            let capture = records.passthrough_capture("unbound-input")?.unwrap();
            assert_eq!(capture.state, passthrough::CaptureState::Pending);
            assert!(capture.bound_session.is_none());
            assert!(capture.answer.is_none());
            let attempt = records.find_request("unbound-input")?.unwrap();
            let facts = records.delivery_facts(&attempt, observed_at)?;
            Ok::<_, super::super::StorageError>(delivery::delivery_view(&facts, observed_at))
        })
        .unwrap();
    assert_eq!(view.state, delivery::DeliveryState::Unconfirmed);
    assert_eq!(view.reason, delivery::DeliveryReason::NoTurnStarted);
    assert!(view.reason_text.contains("no answer captured"));
}

#[test]
fn ordinary_stops_dedupe_in_the_batch_but_distinct_continuations_keep_their_finals() {
    let mut w = World::new();
    let first = w.event("first", false);
    let duplicate = w.event("first", false);
    let continued = w.event("continued", true);
    let continued2 = w.event("continued again", true);
    assert_eq!(
        w.storage.ingest_hook_capture(&first).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(
        w.storage.ingest_hook_capture(&duplicate).unwrap(),
        CaptureResult::Duplicate
    );
    assert_eq!(
        w.storage.ingest_hook_capture(&continued).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(
        w.storage.ingest_hook_capture(&continued2).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.final_count(), 3);
}
#[test]
fn a_receipt_failure_rolls_back_the_final_and_activity_then_a_later_flush_can_ingest() {
    let mut w = World::new();
    let event = w.event("atomic", false);
    w.storage.connection().unwrap().execute_batch("CREATE TEMP TRIGGER reject_receipt BEFORE INSERT ON hook_capture_receipts BEGIN SELECT RAISE(ABORT,'receipt failure'); END;").unwrap();
    assert!(w.storage.ingest_hook_capture(&event).is_err());
    assert_eq!(w.final_count(), 0);
    assert_eq!(w.phase(), ActivityPhase::Working);
    w.storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_receipt")
        .unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.final_count(), 1);
}
#[test]
fn retired_identity_and_missing_batch_leave_durable_failure_receipts() {
    let mut w = World::new();
    let event = w.event("missing batch", false);
    w.storage
        .connection()
        .unwrap()
        .execute("DELETE FROM capture_batches", [])
        .unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::MissingBatch
    );
    assert_eq!(w.final_count(), 0);
    let outcome: String = w
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT outcome FROM hook_capture_receipts WHERE event_id=?",
            [event.id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outcome, "missing_batch");
    let mut retired = World::new();
    let event = retired.event("retired", false);
    retired
        .storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identities SET retired_at_ms=?",
            [retired.now as i64],
        )
        .unwrap();
    assert_eq!(
        retired.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::RetiredIdentity
    );
    assert_eq!(retired.final_count(), 0);
}
#[test]
fn parallel_first_appends_use_private_wal_and_one_singleton_lock() {
    let w = World::new();
    let event = serde_json::to_string(&w.event("parallel", false)).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let jobs = (0..4)
        .map(|_| {
            let path = w.path.clone();
            let value = event.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                let mut event: CaptureEvent = serde_json::from_str(&value).unwrap();
                let mut value = serde_json::to_value(&event).unwrap();
                value["id"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
                event = serde_json::from_value(value).unwrap();
                b.wait();
                let queue = HookQueue::open(&path, Duration::from_secs(2)).unwrap();
                queue.enqueue(&event).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for j in jobs {
        j.join().unwrap();
    }
    let mut queue = HookQueue::open(&w.path, Duration::from_secs(1)).unwrap();
    let count: i64 = queue
        .connection
        .query_row("SELECT COUNT(*) FROM hook_writes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 4);
    let mut guard = Some(queue.claim().unwrap());
    assert_eq!(queue.claim().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    assert!(!queue.finish_if_empty(&mut guard).unwrap());
    assert!(guard.is_some());
    queue
        .connection
        .execute("DELETE FROM hook_writes", [])
        .unwrap();
    assert!(queue.finish_if_empty(&mut guard).unwrap());
    assert!(guard.is_none());
    assert!(queue.claim().is_ok());
    for suffix in ["", "-wal", "-shm"] {
        let mut p = HookQueue::path(&w.path).as_os_str().to_os_string();
        p.push(suffix);
        let m = std::fs::metadata(p).unwrap();
        assert_eq!(m.mode() & 0o777, 0o600);
    }
}
#[test]
fn unsafe_queue_paths_and_malformed_or_oversized_entries_are_refused() {
    let w = World::new();
    let target = w.path.with_file_name("foreign");
    std::fs::write(&target, "untouched").unwrap();
    std::fs::remove_file(HookQueue::path(&w.path)).unwrap();
    std::os::unix::fs::symlink(&target, HookQueue::path(&w.path)).unwrap();
    assert!(HookQueue::open(&w.path, Duration::ZERO).is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched");
    let w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue
        .connection
        .execute(
            "INSERT INTO hook_writes(event_id,kind,payload,enqueued_ms) VALUES('bad','capture','{}',0)",
            [],
        )
        .unwrap();
    assert!(queue.next().unwrap().is_none());
    let failures: i64 = queue
        .connection
        .query_row(
            "SELECT count(*) FROM invalid_hook_writes WHERE event_id='bad'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        failures, 1,
        "the malformed event leaves a durable failure receipt"
    );
    let mut wire = serde_json::to_value(w.event("bounded", false)).unwrap();
    wire["follow_up"]["transcript"] = serde_json::json!("x".repeat(ENTRY_LIMIT));
    let large: CaptureEvent = serde_json::from_value(wire).unwrap();
    assert!(queue.enqueue(&large).is_err());
    let count: i64 = queue
        .connection
        .query_row("SELECT count(*) FROM hook_writes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "oversized data was not appended");
}

#[test]
fn the_empty_handoff_keeps_producers_out_until_the_singleton_lock_is_released() {
    let w = World::new();
    let event = w.event("after empty", false);
    let mut queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let producer = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let mut guard = Some(queue.claim().unwrap());
    assert!(
        queue
            .finish_if_empty_with(&mut guard, || {
                let error = producer.enqueue(&event).unwrap_err();
                assert_eq!(
                    error.code,
                    StorageErrorCode::Busy,
                    "producer cannot cross the empty-check/unlock gap"
                );
                assert_eq!(
                    producer.claim().unwrap_err().kind(),
                    io::ErrorKind::WouldBlock
                );
            })
            .unwrap()
    );
    producer.enqueue(&event).unwrap();
    assert!(
        producer.claim().is_ok(),
        "an append after the handoff can start its flusher"
    );
    assert!(producer.next().unwrap().is_some());
}
#[test]
fn truncation_provenance_survives_durable_enqueue_and_ingestion() {
    let mut w = World::new();
    let original = format!(
        "{}🦀tail",
        "x".repeat(tmt_core::responses::MAX_TEXT_BYTES - 2)
    );
    let event = w.event(&original, false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&event).unwrap();
    w.storage
        .ingest_hook_capture(&queue.next().unwrap().unwrap())
        .unwrap();
    let row: (String, i64, i64) = w
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT text,truncated,original_bytes FROM recent_responses",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, "x".repeat(tmt_core::responses::MAX_TEXT_BYTES - 2));
    assert_eq!(row.1, 1);
    assert_eq!(row.2, original.len() as i64);
}
#[test]
fn a_cross_identity_batch_reference_never_captures_into_another_identity() {
    let mut w = World::new();
    let event = w.event("must stay with its owner", false);
    let other = tmt_core::identity::create_or_resolve(&mut w.storage, "Other", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    let mut wire = serde_json::to_value(event).unwrap();
    wire["capture"]["identity"] = serde_json::json!(other);
    let event: CaptureEvent = serde_json::from_value(wire).unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::MissingBatch
    );
    assert_eq!(w.final_count(), 0);
    assert_eq!(w.phase(), ActivityPhase::Working);
}

#[test]
fn future_and_foreign_queue_schemas_refuse_without_rewriting_their_history() {
    for version in [0, 1, 2, 3] {
        let directory = TestDirectory::new();
        let main = directory.path.join("uninitialized-main.db");
        let path = HookQueue::path(&main);
        create_database(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute_batch(&format!("CREATE TABLE foreign_data(value TEXT); INSERT INTO foreign_data VALUES('preserve'); PRAGMA user_version={version};")).unwrap();
        assert!(HookQueue::open(&main, Duration::ZERO).is_err());
        let kept: String = db
            .query_row("SELECT value FROM foreign_data", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, "preserve");
        let kept: u32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, version);
    }
}

#[test]
fn hooks_do_not_create_a_missing_queue_and_core_provisions_before_publishing_schema() {
    let directory = TestDirectory::new();
    let main = directory.path.join("core.db");
    assert!(HookQueue::open(&main, Duration::ZERO).is_err());
    assert!(!HookQueue::path(&main).exists());
    let storage = Storage::open(&main).unwrap();
    assert_eq!(storage.health().unwrap().schema_version, 84);
    let queue = HookQueue::open(&main, Duration::ZERO).unwrap();
    let mode: String = queue
        .connection
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}
#[test]
fn no_admitted_start_queues_only_its_terminal_metadata_not_the_unadmitted_answer() {
    let mut w = World::new();
    w.storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE identity_session_preferences SET driver_state=NULL,driver_state_version=NULL",
            [],
        )
        .unwrap();
    let event = w.event("unadmitted answer must be dropped", false);
    assert!(
        !serde_json::to_string(&event)
            .unwrap()
            .contains("unadmitted answer")
    );
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::NoAdmittedStart
    );
    assert_eq!(w.final_count(), 0);
}

#[test]
fn a_queue_initialization_failure_cannot_publish_the_main_receipt_migration() {
    let directory = TestDirectory::new();
    let main = directory.path.join("core.db");
    let mut predecessor = Connection::open(&main).unwrap();
    predecessor
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    super::super::migrations::seed_capture_queue_predecessor(&mut predecessor);
    drop(predecessor);
    let queue = HookQueue::path(&main);
    create_database(&queue).unwrap();
    let foreign = Connection::open(&queue).unwrap();
    foreign
        .execute_batch(
            "CREATE TABLE foreign_data(value TEXT); INSERT INTO foreign_data VALUES('keep');",
        )
        .unwrap();
    assert!(Storage::open(&main).is_err());
    let db = Connection::open(&main).unwrap();
    let version: i64 = db
        .query_row("SELECT max(version) FROM _migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 83);
    let receipt: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='hook_capture_receipts')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!receipt);
    let text: String = foreign
        .query_row("SELECT value FROM foreign_data", [], |r| r.get(0))
        .unwrap();
    assert_eq!(text, "keep");
    let journal: String = foreign
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "delete");
}
#[test]
fn parallel_core_initializers_publish_one_compatible_queue_before_first_appends() {
    let directory = TestDirectory::new();
    let main = directory.path.join("core.db");
    // Keep the existing main WAL initialization outside this queue regression.
    // The concurrent subject is the first 082 -> 083 queue provisioning.
    let mut predecessor = Connection::open(&main).unwrap();
    predecessor
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    super::super::migrations::seed_capture_queue_predecessor(&mut predecessor);
    drop(predecessor);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let jobs = (0..4)
        .map(|_| {
            let path = main.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                let mut storage = Storage::open(&path).unwrap();
                let queue = HookQueue::open(&path, Duration::ZERO).unwrap();
                assert!(queue.next().unwrap().is_none());
                storage.close().unwrap();
            })
        })
        .collect::<Vec<_>>();
    for job in jobs {
        job.join().unwrap();
    }
    let queue = HookQueue::open(&main, Duration::ZERO).unwrap();
    let tables: i64 = queue
        .connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='hook_writes'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 1);
}

#[test]
fn a_later_supplied_final_is_not_lost_after_a_textless_stop() {
    for historical in [false, true] {
        let mut w = World::new();
        let first = w.event("none retained", false);
        let second = w.event("the later supplied final", false);
        let mut wire = serde_json::to_value(first).unwrap();
        wire["capture"]["response"] = serde_json::Value::Null;
        let first: CaptureEvent = serde_json::from_value(wire).unwrap();
        if historical {
            w.new_prompt();
        }
        assert_eq!(
            w.storage.ingest_hook_capture(&first).unwrap(),
            CaptureResult::Committed
        );
        assert_eq!(
            w.storage.ingest_hook_capture(&second).unwrap(),
            CaptureResult::Committed
        );
        assert_eq!(w.final_count(), 1);
    }
}
#[test]
fn delayed_stop_follow_up_is_fenced_out_of_a_new_input_batch() {
    let mut w = World::new();
    let event = w.event("older final", false);
    assert!(event.is_current_batch(&w.path).unwrap());
    w.new_prompt();
    assert!(!event.is_current_batch(&w.path).unwrap());
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.phase(), ActivityPhase::Working);
    assert!(!event.is_current_batch(&w.path).unwrap());
}

#[test]
fn a_diagnostic_only_stop_cannot_restore_a_warning_after_a_new_prompt() {
    use crate::runtime::hook_protocol::HookLaunch;
    use tmt_core::binding::session::{
        HarnessId,
        launch::{
            SessionLearningDetail, SessionLearningDiagnostic, SessionLearningReason,
            SessionLearningStep,
        },
    };

    for had_batch in [false, true] {
        let mut w = World::new();
        if !had_batch {
            super::super::capture_batches::close(w.storage.connection().unwrap(), &w.agent, w.now)
                .unwrap();
        }
        let binding = BindingRows(w.storage.connection().unwrap())
            .entry_by_id(&w.agent)
            .unwrap()
            .unwrap()
            .binding
            .unwrap();
        let driver = HarnessId::new("claude").unwrap();
        assert!(
            w.storage
                .with_binding_transaction(|records| {
                    records.begin_launch_observation(&binding, &w.process, &driver, w.now)
                })
                .unwrap()
        );
        let launch = HookLaunch {
            identity_id: w.agent.clone(),
            binding_id: binding.id.clone(),
            owner_pid: w.process.pid(),
            owner_start: w.process.start_identity().into(),
        };
        // Diagnostic admission reads a WAL snapshot while the main writer is held.
        let locked_main = Connection::open(&w.path).unwrap();
        locked_main.execute_batch("BEGIN IMMEDIATE").unwrap();
        let event = CaptureEvent::diagnostic(
            &w.path,
            CaptureFollowUp {
                learning: Some(CaptureLearning {
                    launch,
                    provider: "claude".into(),
                    step: None,
                    detail: Some("stopped:turn:Stop:not_current".into()),
                    failure: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        locked_main.execute_batch("ROLLBACK").unwrap();
        let owner = w.process.clone();
        let binding_id = binding.id.clone();
        let apply_warning =
            |records: &mut dyn BindingRecords<Error = super::super::StorageError>| {
                let previous = records.launch_observation(&binding_id, &owner)?.unwrap();
                let mut next = previous.clone();
                let mut warning = SessionLearningDiagnostic::new(
                    SessionLearningStep::Input,
                    SessionLearningReason::NotAdmitted,
                );
                warning.detail = SessionLearningDetail::parse("stopped:turn:Stop:not_current");
                next.diagnostic = Some(warning);
                records.set_launch_observation(&previous, &next)
            };
        assert!(
            w.storage
                .update_hook_capture_learning(&event, apply_warning)
                .unwrap(),
            "the same launch and input batch may record its Stop diagnostic"
        );
        let mut records = BindingRows(w.storage.connection().unwrap());
        let previous = records
            .launch_observation(&binding_id, &owner)
            .unwrap()
            .unwrap();
        let mut cleared = previous.clone();
        cleared.diagnostic = None;
        assert!(records.set_launch_observation(&previous, &cleared).unwrap());

        let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
        queue.enqueue(&event).unwrap();
        w.new_prompt();
        let event = queue.next().unwrap().unwrap();
        assert_eq!(
            w.storage.ingest_hook_capture(&event).unwrap(),
            CaptureResult::NotApplicable
        );
        let applied = w
            .storage
            .update_hook_capture_learning(&event, apply_warning)
            .unwrap();
        assert!(
            !applied,
            "an old Stop diagnostic cannot alter the newer turn"
        );
        let diagnostic = BindingRows(w.storage.connection().unwrap())
            .launch_observation(&binding.id, &w.process)
            .unwrap()
            .unwrap()
            .diagnostic;
        assert!(
            diagnostic.is_none(),
            "the newer prompt stays free of the old warning"
        );
        assert_eq!(w.phase(), ActivityPhase::Working);
    }
}

#[test]
fn malformed_pending_queue_work_cannot_veto_an_explicit_core_writer() {
    let mut w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.connection.execute("INSERT INTO hook_writes(event_id,kind,payload,enqueued_ms) VALUES('bad','capture','{}',0)",[]).unwrap();
    let control = tmt_core::identity::create_or_resolve(
        &mut w.storage,
        "Explicit writer still commits",
        Lifetime::Saved,
    )
    .unwrap()
    .identity;
    let persisted = Storage::context_by_identity(&w.path, &control.id, w.now)
        .unwrap()
        .unwrap();
    assert_eq!(
        persisted.entry.identity.name,
        "Explicit writer still commits"
    );
    assert!(queue.next().unwrap().is_none());
    assert_eq!(
        queue
            .connection
            .query_row(
                "SELECT count(*) FROM invalid_hook_writes WHERE event_id='bad'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(w.final_count(), 0);
    assert_eq!(w.phase(), ActivityPhase::Working);
}

#[test]
fn digest_drain_quarantines_bad_events_and_ignores_other_identity_backlog() {
    let mut w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.connection.execute("INSERT INTO hook_writes(event_id,identity_id,kind,payload,enqueued_ms) VALUES('bad',?,'capture','{}',0)",[&w.agent]).unwrap();
    for _ in 0..65 {
        let mut wire = serde_json::to_value(w.event("other", true)).unwrap();
        wire["id"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
        wire["capture"]["identity"] = serde_json::json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let event: CaptureEvent = serde_json::from_value(wire).unwrap();
        queue.enqueue(&event).unwrap();
    }
    let own = w.event("own final", false);
    queue.enqueue(&own).unwrap();
    w.storage
        .ingest_pending_hook_captures_for_identity(&w.agent)
        .unwrap();
    assert_eq!(w.final_count(), 1);
    assert_eq!(
        queue
            .connection
            .query_row(
                "SELECT count(*) FROM invalid_hook_writes WHERE event_id='bad'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(queue.connection.query_row("SELECT count(*) FROM hook_writes WHERE identity_id='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' AND committed=0",[],|r|r.get::<_,i64>(0)).unwrap(),65);
    assert!(
        queue
            .next_uncommitted_for_identity(&w.agent)
            .unwrap()
            .is_none()
    );
}

#[test]
fn digest_drain_uses_the_identity_index_for_unrelated_backlogs() {
    let w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let mut statement = queue
        .connection
        .prepare(&format!("EXPLAIN QUERY PLAN {READ_IDENTITY}"))
        .unwrap();
    let plan = statement
        .query_map(params![true, &w.agent], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|line| line.contains("hook_writes_identity_pending")),
        "{plan:?}"
    );
    assert!(
        !plan.iter().any(|line| line.contains("TEMP B-TREE")),
        "{plan:?}"
    );
}

#[test]
fn flusher_skips_invalid_row_and_processes_the_next_capture() {
    let mut w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.connection.execute("INSERT INTO hook_writes(event_id,kind,payload,enqueued_ms) VALUES('bad','capture','{}',0)",[]).unwrap();
    let event = w.event("kept after malformed row", false);
    queue.enqueue(&event).unwrap();
    let next = queue.next().unwrap().unwrap();
    assert_eq!(next.id(), event.id());
    w.storage.ingest_hook_capture(&next).unwrap();
    queue.acknowledge(&next).unwrap();
    assert!(queue.next().unwrap().is_none());
    assert_eq!(w.final_count(), 1);
    assert!(
        queue
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM invalid_hook_writes WHERE event_id='bad')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
}

#[test]
fn non_utf8_queue_payload_is_quarantined_without_blocking_the_next_event() {
    let w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.connection.execute("INSERT INTO hook_writes(event_id,kind,payload,enqueued_ms) VALUES('non-utf8','capture',CAST(x'80' AS TEXT),0)",[]).unwrap();
    let event = w.event("after invalid text", false);
    queue.enqueue(&event).unwrap();
    assert_eq!(queue.next().unwrap().unwrap().id(), event.id());
    assert!(
        queue
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM invalid_hook_writes WHERE event_id='non-utf8')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
}

#[test]
fn digest_drain_refuses_when_its_own_bounded_prefix_leaves_work() {
    let mut w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    for n in 0..65 {
        queue
            .enqueue(&w.event(&format!("final {n}"), true))
            .unwrap();
    }
    let error = w
        .storage
        .ingest_pending_hook_captures_for_identity(&w.agent)
        .unwrap_err();
    assert_eq!(error.code, StorageErrorCode::Busy);
    assert!(
        queue
            .next_uncommitted_for_identity(&w.agent)
            .unwrap()
            .is_some()
    );
}

#[test]
fn housekeeping_keeps_queued_replay_receipts_and_prunes_old_unqueued_ones() {
    let mut w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let event = w.event("retained final", false);
    queue.enqueue(&event).unwrap();
    w.storage.ingest_hook_capture(&event).unwrap();
    let db = w.storage.connection().unwrap();
    db.execute(
        "UPDATE hook_capture_receipts SET committed_ms=1 WHERE event_id=?",
        [event.id()],
    )
    .unwrap();
    db.execute("INSERT INTO hook_capture_receipts(event_id,outcome,committed_ms) VALUES('old-unrelated','duplicate',1)",[]).unwrap();
    db.execute("INSERT INTO hook_capture_receipts(event_id,outcome,committed_ms) VALUES('recent','duplicate',?)",[w.now as i64]).unwrap();
    let now = w.now;
    tmt_core::request::RequestService::new(&mut w.storage, || now)
        .cleanup()
        .unwrap();
    let db = w.storage.connection().unwrap();
    let ids = |id: &str| {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM hook_capture_receipts WHERE event_id=?)",
            [id],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    };
    assert!(ids(event.id()), "a queued event keeps its replay receipt");
    assert!(!ids("old-unrelated"));
    assert!(ids("recent"));
    w.storage.ingest_hook_capture(&event).unwrap();
    assert_eq!(
        w.final_count(),
        1,
        "replay of the still-queued event is a no-op"
    );
    queue.acknowledge(&event).unwrap();
    tmt_core::request::RequestService::new(&mut w.storage, || now)
        .cleanup()
        .unwrap();
    let db = w.storage.connection().unwrap();
    assert!(
        !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM hook_capture_receipts WHERE event_id=?)",
            [event.id()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[test]
fn bounded_retention_advances_past_many_protected_receipts() {
    let w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let db = w.storage.connection().unwrap();
    for n in 0..101 {
        let id = format!("pending-{n:03}");
        db.execute("INSERT INTO hook_capture_receipts(event_id,outcome,committed_ms) VALUES(?,'committed',1)",[&id]).unwrap();
        queue.connection.execute("INSERT INTO hook_writes(event_id,kind,payload,enqueued_ms) VALUES(?,'capture','{}',0)",[&id]).unwrap();
    }
    db.execute("INSERT INTO hook_capture_receipts(event_id,outcome,committed_ms) VALUES('z-unrelated','committed',1)",[]).unwrap();
    for _ in 0..2 {
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate).unwrap();
        let held = HookQueue::begin_retention(&tx, w.now, 100)
            .unwrap()
            .unwrap();
        tx.commit().unwrap();
        held.finish_retention().unwrap();
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM hook_capture_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        101
    );
    assert!(
        !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM hook_capture_receipts WHERE event_id='z-unrelated')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[test]
fn receipt_retention_waits_for_the_full_thirty_day_window() {
    let w = World::new();
    let db = w.storage.connection().unwrap();
    db.execute("INSERT INTO hook_capture_receipts(event_id,outcome,committed_ms) VALUES('at-epoch','committed',0)",[]).unwrap();
    const WINDOW: u64 = 30 * 24 * 60 * 60 * 1000;
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate).unwrap();
    assert!(
        HookQueue::begin_retention(&tx, WINDOW - 1, 100)
            .unwrap()
            .is_none()
    );
    tx.commit().unwrap();
    assert!(
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM hook_capture_receipts WHERE event_id='at-epoch')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate).unwrap();
    let held = HookQueue::begin_retention(&tx, WINDOW, 100)
        .unwrap()
        .unwrap();
    tx.commit().unwrap();
    held.finish_retention().unwrap();
    assert!(
        !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM hook_capture_receipts WHERE event_id='at-epoch')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[test]
fn provisional_retention_keeps_active_and_queued_batches() {
    let w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let db = w.storage.connection().unwrap();
    let active = current(db, &w.agent).unwrap().unwrap();
    let closed = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let queued = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let expired = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    for id in [closed, queued] {
        db.execute("INSERT INTO capture_batches(id,identity_id,binding_id,driver,session_id,runtime_pid,runtime_start,provider_turn,created_ms,closed_ms) SELECT ?1,identity_id,binding_id,driver,session_id,runtime_pid,runtime_start,provider_turn,created_ms,?2 FROM capture_batches WHERE id=?3",params![id,w.now as i64,active]).unwrap();
    }
    for (key, id) in [
        ("active", active.as_str()),
        ("closed", closed),
        ("queued", queued),
        ("expired", expired),
    ] {
        queue
            .connection
            .execute(
                "INSERT INTO provisional_batches(turn_key,batch_id,created_ms) VALUES(?,?,1)",
                params![key, id],
            )
            .unwrap();
    }
    queue.connection.execute("INSERT INTO hook_writes(event_id,provisional_batch,kind,payload,enqueued_ms) VALUES('pending',?,'capture','{}',0)",[queued]).unwrap();
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate).unwrap();
    let held = HookQueue::begin_retention(&tx, w.now, 100)
        .unwrap()
        .unwrap();
    tx.commit().unwrap();
    held.finish_retention().unwrap();
    let exists = |key: &str| {
        queue
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM provisional_batches WHERE turn_key=?)",
                [key],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
    };
    assert!(exists("active"));
    assert!(exists("queued"));
    assert!(!exists("closed"));
    assert!(
        !exists("expired"),
        "a pruned batch must not leak its reservation"
    );
}

#[test]
fn ordinary_core_initialization_repairs_a_missing_queue_without_touching_current_data() {
    let w = World::new();
    let cursor = w.storage.change_cursor().unwrap();
    std::fs::remove_file(HookQueue::path(&w.path)).unwrap();
    assert!(HookQueue::open(&w.path, Duration::ZERO).is_err());
    assert!(!HookQueue::path(&w.path).exists());
    let reopened = Storage::open(&w.path).unwrap();
    assert_eq!(reopened.change_cursor().unwrap(), cursor);
    assert!(
        HookQueue::open(&w.path, Duration::ZERO)
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    assert_eq!(w.phase(), ActivityPhase::Working);
}
#[test]
fn a_trusted_core_root_alias_uses_the_same_private_queue() {
    let w = World::new();
    let alias = w.path.parent().unwrap().join("core-root-alias");
    std::os::unix::fs::symlink(w.path.parent().unwrap(), &alias).unwrap();
    let selected = alias.join(w.path.file_name().unwrap());
    let queue = HookQueue::open(&selected, Duration::ZERO).unwrap();
    let event = w.event("through the selected alias", false);
    queue.enqueue(&event).unwrap();
    let original = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    assert_eq!(original.next().unwrap().unwrap().id(), event.id());
}

#[test]
fn oversized_optional_telemetry_cannot_discard_a_supplied_final() {
    let mut w = World::new();
    let event = w.event_with_follow_up(
        "the supplied final wins",
        false,
        CaptureFollowUp {
            usage: true,
            transcript: Some("x".repeat(ENTRY_LIMIT).into()),
            learning: None,
        },
    );
    assert!(!event.follow_up.usage);
    assert!(event.follow_up.transcript.is_none());
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&event).unwrap();
    w.storage
        .ingest_hook_capture(&queue.next().unwrap().unwrap())
        .unwrap();
    let text: String = w
        .storage
        .connection()
        .unwrap()
        .query_row("SELECT text FROM recent_responses", [], |r| r.get(0))
        .unwrap();
    assert_eq!(text, "the supplied final wins");
}

#[test]
fn truncated_finals_stay_in_the_recorded_batch_when_equality_is_unprovable() {
    for delayed in [false, true] {
        let mut w = World::new();
        let prefix = "x".repeat(tmt_core::responses::MAX_TEXT_BYTES);
        let first_text = format!("{prefix}A");
        let second_text = format!("{prefix}B");
        let first = w.event(&first_text, false);
        let second = w.event(&second_text, false);
        let retry = w.event(&second_text, false);
        let recorded = current(w.storage.connection().unwrap(), &w.agent).unwrap();
        if delayed {
            w.new_prompt();
        }
        let preferences = BindingRows(w.storage.connection().unwrap())
            .session_preferences(&w.agent)
            .unwrap();
        let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
        for event in [&first, &second, &retry] {
            queue.enqueue(event).unwrap();
        }
        for expected in [
            CaptureResult::Committed,
            CaptureResult::Committed,
            CaptureResult::Committed,
        ] {
            let decoded = queue.next().unwrap().unwrap();
            assert_eq!(w.storage.ingest_hook_capture(&decoded).unwrap(), expected);
            queue.acknowledge(&decoded).unwrap();
        }
        assert!(queue.next().unwrap().is_none());
        // Receipt replay of either event cannot append a second row.
        assert_eq!(
            w.storage.ingest_hook_capture(&second).unwrap(),
            CaptureResult::Committed
        );
        assert_eq!(
            w.final_count(),
            3,
            "a different event ID cannot prove a truncated retry is identical"
        );
        let db = w.storage.connection().unwrap();
        let placed: i64 = db
            .query_row(
                "SELECT count(*) FROM capture_entries WHERE batch_id=? AND kind='final'",
                [&recorded],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(placed, 3);
        if delayed {
            assert_eq!(
                BindingRows(db).session_preferences(&w.agent).unwrap(),
                preferences,
                "old finals cannot settle newer activity"
            );
        }
    }
}

#[test]
fn truncated_equality_is_unknown_but_complete_text_dedupes_in_the_same_batch() {
    for truncated in [false, true] {
        let mut w = World::new();
        let text = if truncated {
            "x".repeat(tmt_core::responses::MAX_TEXT_BYTES + 1)
        } else {
            "complete final".into()
        };
        let first = w.event(&text, false);
        assert_eq!(
            w.storage.ingest_hook_capture(&first).unwrap(),
            CaptureResult::Committed
        );
        let repeated = w.event(&text, false);
        assert_eq!(
            w.storage.ingest_hook_capture(&repeated).unwrap(),
            if truncated {
                CaptureResult::Committed
            } else {
                CaptureResult::Duplicate
            }
        );
        assert_eq!(w.final_count(), if truncated { 2 } else { 1 });
    }
}

#[test]
fn identical_finals_in_different_admitted_batches_are_both_retained() {
    let mut w = World::new();
    let first = w.event("the same answer in two real turns", false);
    assert_eq!(
        w.storage.ingest_hook_capture(&first).unwrap(),
        CaptureResult::Committed
    );
    let previous = current(w.storage.connection().unwrap(), &w.agent).unwrap();
    w.new_prompt();
    let second = w.event("the same answer in two real turns", false);
    assert_eq!(
        w.storage.ingest_hook_capture(&second).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(w.final_count(), 2);
    assert_ne!(
        current(w.storage.connection().unwrap(), &w.agent).unwrap(),
        previous
    );
}

#[test]
fn new_queued_finals_round_trip_through_the_shipped_strict_payload_shape() {
    // Frozen KeptFinal wire shape from main4b1b27805, before #5534. A running old flusher
    // rejects unknown fields and quarantines the row, so additions are unsafe.
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(deny_unknown_fields)]
    struct ShippedKeptFinal {
        text: String,
        original_bytes: usize,
    }
    let w = World::new();
    let text = "x".repeat(tmt_core::responses::MAX_TEXT_BYTES + 1);
    let event = w.event(&text, false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&event).unwrap();
    let payload: String = queue
        .connection
        .query_row("SELECT payload FROM hook_writes", [], |row| row.get(0))
        .unwrap();
    let wire: serde_json::Value = serde_json::from_str(&payload).unwrap();
    let response = wire["capture"]["response"].clone();
    let old: ShippedKeptFinal = serde_json::from_value(response.clone())
        .expect("a running old flusher must decode a new row");
    assert_eq!(old.original_bytes, text.len());
    assert_eq!(old.text, "x".repeat(tmt_core::responses::MAX_TEXT_BYTES));
    assert_eq!(serde_json::to_value(old).unwrap(), response);
    let mut incompatible = response;
    incompatible["digest"] = serde_json::json!("a".repeat(64));
    assert!(serde_json::from_value::<ShippedKeptFinal>(incompatible).is_err());
    assert_eq!(queue.next().unwrap().unwrap().id(), event.id());
}
