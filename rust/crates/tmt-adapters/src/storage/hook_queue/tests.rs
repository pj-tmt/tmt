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
    assert!(
        queued.recorded_batch().is_none(),
        "a provisional locator is not a recorded batch"
    );
    let lifecycle = crate::drivers::claude::ClaudeLifecycle;
    let payload = br#"{"hook_event_name":"Stop","session_id":"fixture-session","last_assistant_message":"direct answer"}"#;
    let turn = lifecycle.decode_turn(payload).unwrap();
    let activity = lifecycle.decode_activity(payload).unwrap();
    let expected = Storage::context_by_identity(&w.path, &w.agent, w.now)
        .unwrap()
        .unwrap();
    assert_eq!(
        w.storage
            .capture_turn_end(
                queued.id(),
                queued.recorded_batch(),
                TurnCapture {
                    expected: &expected,
                    process: &w.process,
                    activity: &activity,
                    response: Some(ResponseCapture {
                        text: turn.final_text.unwrap()
                    }),
                    interrupted: false,
                    now_ms: w.now + 1,
                    deadline: Instant::now() + Duration::from_secs(2),
                }
            )
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
    assert!(
        w.visible_failures().is_empty(),
        "a missing recorded batch never becomes unbatched"
    );
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
    assert_eq!(storage.health().unwrap().schema_version, 85);
    let queue = HookQueue::open(&main, Duration::ZERO).unwrap();
    let mode: String = queue
        .connection
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}
impl World {
    fn refuse_end(&self, outcome: CaptureResult) {
        let mut rows = BindingRows(self.storage.connection().unwrap());
        let mut preferences = rows.session_preferences(&self.agent).unwrap();
        preferences.remembered.as_mut().unwrap().state =
            if outcome == CaptureResult::NoAdmittedStart {
                None
            } else {
                crate::runtime::driver_state::after_activity(
                    None,
                    &ProviderSessionId::new("fixture-session").unwrap(),
                    &self.process,
                    &Event {
                        kind: EventKind::TurnStarted,
                        turn: None,
                    },
                    None,
                    self.now + 50,
                )
            };
        rows.set_session_preferences(&self.agent, &preferences)
            .unwrap();
    }
    fn direct_end(&mut self, event: &CaptureEvent, text: &str) -> super::super::TurnCaptureOutcome {
        let expected = Storage::context_by_identity(&self.path, &self.agent, self.now)
            .unwrap()
            .unwrap();
        self.direct_end_with_snapshot(event, text, &expected)
    }
    fn direct_end_with_snapshot(
        &mut self,
        event: &CaptureEvent,
        text: &str,
        expected: &super::super::IdentityContextSnapshot,
    ) -> super::super::TurnCaptureOutcome {
        let lifecycle = crate::drivers::claude::ClaudeLifecycle;
        let payload = serde_json::json!({"hook_event_name":"Stop","session_id":"fixture-session","last_assistant_message":text}).to_string();
        let turn = lifecycle.decode_turn(payload.as_bytes()).unwrap();
        let activity = lifecycle.decode_activity(payload.as_bytes()).unwrap();
        self.storage
            .capture_turn_end(
                event.id(),
                event.recorded_batch(),
                TurnCapture {
                    expected,
                    process: &self.process,
                    activity: &activity,
                    response: Some(ResponseCapture {
                        text: turn.final_text.unwrap(),
                    }),
                    interrupted: false,
                    now_ms: self.now + 1,
                    deadline: Instant::now() + Duration::from_secs(2),
                },
            )
            .unwrap()
    }
    fn visible_failures(&mut self) -> Vec<serde_json::Value> {
        let page = self
            .storage
            .conversation_timeline(&self.agent, 50, None, self.now + 100)
            .unwrap();
        page["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["kind"] == "failure")
            .cloned()
            .collect()
    }
}

#[test]
fn queued_and_direct_refusals_keep_a_visible_failure_in_the_recorded_batch() {
    for queued in [false, true] {
        for refused in [CaptureResult::NoAdmittedStart, CaptureResult::NotApplicable] {
            let mut w = World::new();
            w.refuse_end(refused);
            let event = w.event("verified text could not be retained", false);
            assert_eq!(event.admission(), refused);
            assert!(
                serde_json::to_string(&event)
                    .unwrap()
                    .contains("verified text")
            );
            let batch = event.recorded_batch().unwrap().to_owned();
            let before = BindingRows(w.storage.connection().unwrap())
                .session_preferences(&w.agent)
                .unwrap();
            if queued {
                assert_eq!(w.storage.ingest_hook_capture(&event).unwrap(), refused);
            } else {
                assert_eq!(
                    CaptureResult::from(
                        w.direct_end(&event, "verified text could not be retained")
                    ),
                    refused
                );
            }
            assert_eq!(w.final_count(), 0);
            let failures = w.visible_failures();
            assert_eq!(
                failures.len(),
                1,
                "a verified refused final must not disappear"
            );
            assert_eq!(failures[0]["inputBatchId"], batch);
            assert_eq!(failures[0]["failure"]["code"], "worker_error");
            assert_eq!(
                BindingRows(w.storage.connection().unwrap())
                    .session_preferences(&w.agent)
                    .unwrap(),
                before
            );
            assert_eq!(
                current(w.storage.connection().unwrap(), &w.agent)
                    .unwrap()
                    .as_deref(),
                Some(batch.as_str())
            );
        }
    }
}

#[test]
fn two_refused_events_in_one_batch_are_visible_but_an_event_replay_is_not() {
    for queued in [false, true] {
        let mut w = World::new();
        w.refuse_end(CaptureResult::NoAdmittedStart);
        let first = w.event("first missed final", false);
        let second = w.event("second missed final", false);
        for (event, text) in [
            (&first, "first missed final"),
            (&first, "first missed final"),
            (&second, "second missed final"),
        ] {
            if queued {
                w.storage.ingest_hook_capture(event).unwrap();
            } else {
                w.direct_end(event, text);
            }
        }
        let failures = w.visible_failures();
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0]["inputBatchId"], failures[1]["inputBatchId"]);
        assert_ne!(failures[0]["entryId"], failures[1]["entryId"]);
    }
}

#[test]
fn a_refused_exact_text_replay_of_a_retained_final_has_no_failure() {
    for queued in [false, true] {
        let mut w = World::new();
        let first = w.event("already visible", false);
        w.storage.ingest_hook_capture(&first).unwrap();
        w.refuse_end(CaptureResult::NoAdmittedStart);
        let replay = w.event("already visible", false);
        if queued {
            w.storage.ingest_hook_capture(&replay).unwrap();
        } else {
            w.direct_end(&replay, "already visible");
        }
        assert_eq!(w.final_count(), 1);
        assert!(w.visible_failures().is_empty());
    }
}

#[test]
fn a_refused_capture_cannot_write_through_a_reassigned_binding() {
    for queued in [false, true] {
        let mut w = World::new();
        w.refuse_end(CaptureResult::NoAdmittedStart);
        let event = w.event("belongs to the former binding", false);
        let expected = Storage::context_by_identity(&w.path, &w.agent, w.now)
            .unwrap()
            .unwrap();
        let other = tmt_core::identity::create_or_resolve(&mut w.storage, "Other", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        w.storage
            .connection()
            .unwrap()
            .execute("UPDATE bindings SET identity_id=?", [&other])
            .unwrap();
        if queued {
            w.storage.ingest_hook_capture(&event).unwrap();
        } else {
            w.direct_end_with_snapshot(&event, "belongs to the former binding", &expected);
        }
        assert!(w.visible_failures().is_empty());
        let other_page = w
            .storage
            .conversation_timeline(&other, 50, None, w.now + 100)
            .unwrap();
        assert!(other_page["entries"].as_array().unwrap().is_empty());
    }
}

#[test]
fn delayed_refusal_only_adds_a_failure_to_its_old_batch() {
    for queued in [false, true] {
        let mut w = World::new();
        w.refuse_end(CaptureResult::NoAdmittedStart);
        let event = w.event("old verified text without a start", false);
        let old = event.recorded_batch().unwrap().to_owned();
        let expected = Storage::context_by_identity(&w.path, &w.agent, w.now)
            .unwrap()
            .unwrap();
        let newer = w.new_prompt();
        let db = w.storage.connection().unwrap();
        db.execute("INSERT INTO request_attempts(attempt_id,request_id,route_kind,recipient_identity_id,originator_kind,originator_identity_id,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms)
            VALUES('new-input','new-input','inbox',?1,'explicit',?1,0,'sent',0,0,?2,?3,?3)",params![w.agent,w.now as i64,(w.now+60_000) as i64]).unwrap();
        db.execute("INSERT INTO request_passthrough(request_id,delivered_at_ms,state,bound_session,bound_at_ms,folded) VALUES('new-input',?1,'bound','fixture-session',?1,1)",[w.now as i64]).unwrap();
        let before = BindingRows(db).session_preferences(&w.agent).unwrap();
        if queued {
            w.storage.ingest_hook_capture(&event).unwrap();
        } else {
            w.direct_end_with_snapshot(&event, "old verified text without a start", &expected);
        }
        let failures = w.visible_failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["inputBatchId"], old);
        assert_eq!(w.final_count(), 0);
        let db = w.storage.connection().unwrap();
        assert_eq!(
            current(db, &w.agent).unwrap().as_deref(),
            Some(newer.as_str())
        );
        assert_eq!(
            BindingRows(db).session_preferences(&w.agent).unwrap(),
            before
        );
        let state: String = db
            .query_row(
                "SELECT state FROM request_passthrough WHERE request_id='new-input'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "bound");
    }
}

#[test]
fn a_refused_event_and_its_failure_share_the_receipt_transaction() {
    let mut w = World::new();
    w.refuse_end(CaptureResult::NoAdmittedStart);
    let event = w.event("atomic refused final", false);
    w.storage.connection().unwrap().execute_batch("CREATE TEMP TRIGGER reject_refused_receipt BEFORE INSERT ON hook_capture_receipts BEGIN SELECT RAISE(ABORT,'receipt failure'); END;").unwrap();
    assert!(w.storage.ingest_hook_capture(&event).is_err());
    assert!(w.visible_failures().is_empty());
    w.storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_refused_receipt")
        .unwrap();
    w.storage.ingest_hook_capture(&event).unwrap();
    assert_eq!(w.visible_failures().len(), 1);
}

#[test]
fn shipped_style_commit_returns_before_using_text_of_a_refused_admission() {
    // Frozen decisive branch from the shipped pre-T-2a commit: response is
    // consumed only after this return. Keeping Some(response) changes content,
    // not the old decoder's shape or its permission to store an answer.
    fn shipped_commit(admitted: CaptureResult, use_response: impl FnOnce()) -> CaptureResult {
        if admitted != CaptureResult::Committed {
            return admitted;
        }
        use_response();
        CaptureResult::Committed
    }
    let mut used = false;
    assert_eq!(
        shipped_commit(CaptureResult::Committed, || used = true),
        CaptureResult::Committed
    );
    assert!(
        used,
        "the shipped-style body is exercised for admitted ends"
    );
    for refused in [CaptureResult::NoAdmittedStart, CaptureResult::NotApplicable] {
        let w = World::new();
        w.refuse_end(refused);
        let event = w.event("must not become an admitted old-build answer", false);
        let wire = serde_json::to_value(&event).unwrap();
        assert!(wire["capture"]["response"].is_object());
        let old_shape: CaptureEvent = serde_json::from_value(wire).unwrap();
        assert_eq!(
            shipped_commit(old_shape.admission(), || panic!(
                "old commit used refused text"
            )),
            refused
        );
        assert_eq!(w.final_count(), 0);
    }
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
    // The concurrent subject is the first 083 -> 084 queue provisioning.
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
                let mut storage = Storage::open(&path)?;
                // A sibling may still own transient WAL admission. This
                // observation has no wait budget; Core initialization does.
                let observed = HookQueue::open(&path, Duration::ZERO).map(|queue| {
                    let version =
                        queue
                            .connection
                            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0));
                    (version, queue.next().map(|event| event.is_none()))
                });
                let closed = storage.close();
                Ok::<_, StorageError>((observed, closed))
            })
        })
        .collect::<Vec<_>>();
    // Join every initializer and close every connection before asserting, so
    // even a failed initializer cannot outlive the private directory.
    let outcomes = jobs.into_iter().map(|job| job.join()).collect::<Vec<_>>();
    for outcome in outcomes {
        let (observed, closed) = outcome.unwrap().expect("Core initializer must succeed");
        closed.unwrap();
        match observed {
            Ok((version, empty)) => {
                assert_eq!(version.unwrap(), QUEUE_SCHEMA_VERSION);
                assert!(empty.unwrap());
            }
            Err(error) => {
                assert_eq!(error.code, StorageErrorCode::Busy, "{error:?}");
                assert!(
                    error.message.starts_with("Read hook queue schema"),
                    "{error:?}"
                );
                assert!(error.retryable);
            }
        }
    }
    // No retry or settle wait: one quiescent open must see the complete queue.
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
    let version: u32 = queue
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, QUEUE_SCHEMA_VERSION);
    assert!(queue.next().unwrap().is_none());
    let db = Connection::open(&main).unwrap();
    let receipt: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='hook_capture_receipts')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(receipt);
    let version: i64 = db
        .query_row("SELECT max(version) FROM _migrations", [], |row| row.get(0))
        .unwrap();
    assert!(version >= 84);
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

#[test]
fn a_rejected_append_is_unpublished_only_after_exact_queue_and_receipt_readback() {
    let w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.connection.execute_batch("CREATE TRIGGER reject_publication BEFORE INSERT ON hook_writes BEGIN SELECT RAISE(ABORT, 'fixture rejection'); END;").unwrap();
    let event = w.event("never queued", false);
    let outcome = queue.enqueue_observed(&event, Instant::now() + Duration::from_secs(2));
    assert!(
        matches!(outcome, CapturePublication::Unpublished(_)),
        "{outcome:?}"
    );
    assert!(queue.next().unwrap().is_none());
    assert_eq!(w.final_count(), 0);
}

#[test]
fn uncertain_append_readback_distinguishes_queued_ingested_and_absent_events() {
    let mut w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let event = w.event("accepted once", false);
    queue.enqueue(&event).unwrap();
    let uncertain = || {
        Err(StorageError::new(
            StorageErrorCode::Unknown,
            "fixture commit response unavailable",
        ))
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    assert!(matches!(
        queue.observe_enqueue_result(&event, uncertain(), deadline),
        CapturePublication::Queued
    ));
    // Another event's row cannot establish publication for this exact ID.
    let absent = w.event("not this event", false);
    assert!(matches!(
        queue.observe_enqueue_result(&absent, uncertain(), deadline),
        CapturePublication::Unpublished(_)
    ));
    assert_eq!(
        w.storage.ingest_hook_capture(&event).unwrap(),
        CaptureResult::Committed
    );
    queue.acknowledge(&event).unwrap();
    assert!(queue.next().unwrap().is_none());
    assert!(matches!(
        queue.observe_enqueue_result(&event, uncertain(), deadline),
        CapturePublication::Recorded(CaptureResult::Committed)
    ));
    assert_eq!(w.final_count(), 1);
}

#[test]
fn unavailable_or_expired_publication_readback_never_claims_absence_or_replays() {
    for expired in [false, true] {
        let w = World::new();
        let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
        let event = w.event("possibly accepted", false);
        queue.enqueue(&event).unwrap();
        let deadline = if expired {
            Instant::now() - Duration::from_secs(1)
        } else {
            queue
                .connection
                .execute_batch("ALTER TABLE hook_writes RENAME TO retained_hook_writes")
                .unwrap();
            Instant::now() + Duration::from_secs(2)
        };
        let outcome = queue.observe_enqueue_result(
            &event,
            Err(StorageError::new(
                StorageErrorCode::Unknown,
                "fixture commit response unavailable",
            )),
            deadline,
        );
        assert!(
            matches!(outcome, CapturePublication::Unknown { .. }),
            "{outcome:?}"
        );
        let table = if expired {
            "hook_writes"
        } else {
            "retained_hook_writes"
        };
        assert_eq!(
            queue
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(w.final_count(), 0);
        assert_eq!(
            w.storage
                .connection()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM hook_capture_receipts", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
    }
}

#[test]
fn an_expired_publication_budget_never_attempts_an_append() {
    let w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let event = w.event("too late", false);
    let outcome = queue.enqueue_observed(&event, Instant::now() - Duration::from_secs(1));
    assert!(matches!(outcome, CapturePublication::Unpublished(_)));
    assert!(queue.next().unwrap().is_none());
}

#[test]
fn confirmed_unpublished_events_have_distinct_event_fenced_failures_in_the_recorded_batch() {
    let mut w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.connection.execute_batch("CREATE TRIGGER reject_publication BEFORE INSERT ON hook_writes BEGIN SELECT RAISE(ABORT, 'fixture rejection'); END;").unwrap();
    let first = w.event("first never published", false);
    let second = w.event("second never published", false);
    let batch = first.recorded_batch().unwrap().to_owned();
    let before = BindingRows(w.storage.connection().unwrap())
        .session_preferences(&w.agent)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let outcome = queue.enqueue_observed(&first, deadline);
    outcome.record_failure(&w.path, &first, deadline).unwrap();
    outcome.record_failure(&w.path, &first, deadline).unwrap();
    CapturePublication::Unpublished(StorageError::new(
        StorageErrorCode::Busy,
        "fixture queue budget",
    ))
    .record_failure(&w.path, &second, deadline)
    .unwrap();
    let failures = w.visible_failures();
    assert_eq!(
        failures.len(),
        2,
        "different unpublished events must not collapse by batch"
    );
    assert!(
        failures
            .iter()
            .all(|failure| failure["inputBatchId"] == batch)
    );
    assert_ne!(failures[0]["entryId"], failures[1]["entryId"]);
    assert_eq!(failures[0]["failure"]["code"], "storage_error");
    assert_eq!(failures[1]["failure"]["code"], "budget_timeout");
    assert_eq!(w.final_count(), 0);
    assert!(queue.next().unwrap().is_none());
    assert_eq!(
        BindingRows(w.storage.connection().unwrap())
            .session_preferences(&w.agent)
            .unwrap(),
        before
    );
}

#[test]
fn accepted_and_unknown_publications_never_consume_a_failure_receipt() {
    let mut w = World::new();
    let event = w.event("still recoverable", false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&event).unwrap();
    for outcome in [
        CapturePublication::Queued,
        CapturePublication::Recorded(CaptureResult::Committed),
        CapturePublication::Unknown {
            publication: StorageError::new(
                StorageErrorCode::Unknown,
                "fixture commit response unavailable",
            ),
            readback: StorageError::new(StorageErrorCode::Busy, "fixture readback unavailable"),
        },
    ] {
        outcome
            .record_failure(&w.path, &event, Instant::now() + Duration::from_secs(2))
            .unwrap();
    }
    assert!(w.visible_failures().is_empty());
    assert_eq!(
        w.storage
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM hook_capture_receipts", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    let queued = queue.next().unwrap().unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&queued).unwrap(),
        CaptureResult::Committed
    );
    queue.acknowledge(&queued).unwrap();
    assert_eq!(w.final_count(), 1);
    assert!(w.visible_failures().is_empty());
}

#[test]
fn unpublished_failure_never_moves_to_a_newer_batch_or_another_binding_identity() {
    for rebound in [false, true] {
        let mut w = World::new();
        let event = w.event("belongs to the recorded input", false);
        let old = event.recorded_batch().unwrap().to_owned();
        let other = if rebound {
            let other =
                tmt_core::identity::create_or_resolve(&mut w.storage, "Other", Lifetime::Saved)
                    .unwrap()
                    .identity
                    .id;
            w.storage
                .connection()
                .unwrap()
                .execute("UPDATE bindings SET identity_id=?", [&other])
                .unwrap();
            Some(other)
        } else {
            w.new_prompt();
            None
        };
        let before = BindingRows(w.storage.connection().unwrap())
            .session_preferences(&w.agent)
            .unwrap();
        let current_batch = current(w.storage.connection().unwrap(), &w.agent).unwrap();
        CapturePublication::Unpublished(StorageError::new(
            StorageErrorCode::Permission,
            "fixture publication refusal",
        ))
        .record_failure(&w.path, &event, Instant::now() + Duration::from_secs(2))
        .unwrap();
        let failures = w.visible_failures();
        if let Some(other) = other {
            assert!(failures.is_empty());
            let page = w
                .storage
                .conversation_timeline(&other, 50, None, w.now + 100)
                .unwrap();
            assert!(page["entries"].as_array().unwrap().is_empty());
        } else {
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0]["inputBatchId"], old);
        }
        assert_eq!(
            current(w.storage.connection().unwrap(), &w.agent).unwrap(),
            current_batch
        );
        assert_eq!(
            BindingRows(w.storage.connection().unwrap())
                .session_preferences(&w.agent)
                .unwrap(),
            before
        );
    }
}

#[test]
fn unpublished_diagnostics_require_text_and_a_remaining_budget() {
    for case in ["usage_only", "interrupted", "expired"] {
        let mut w = World::new();
        let event = w.event("not a diagnostic authority by itself", false);
        let event = if case == "usage_only" {
            event.usage_only()
        } else if case == "interrupted" {
            let mut wire = serde_json::to_value(event).unwrap();
            wire["capture"]["interrupted"] = serde_json::json!(true);
            serde_json::from_value(wire).unwrap()
        } else {
            event
        };
        let deadline = if case == "expired" {
            Instant::now() - Duration::from_secs(1)
        } else {
            Instant::now() + Duration::from_secs(2)
        };
        let result = CapturePublication::Unpublished(StorageError::new(
            StorageErrorCode::Busy,
            "fixture budget",
        ))
        .record_failure(&w.path, &event, deadline);
        if case == "expired" {
            assert_eq!(result.unwrap_err().code, StorageErrorCode::Busy);
        } else {
            result.unwrap();
        }
        assert!(w.visible_failures().is_empty(), "{case}");
        assert_eq!(
            w.storage
                .connection()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM hook_capture_receipts", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
    }
}

#[test]
fn unreadable_main_receipt_evidence_never_proves_publication_absent() {
    let w = World::new();
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let event = w.event("receipt proof unavailable", false);
    w.storage
        .connection()
        .unwrap()
        .execute("DROP TABLE hook_capture_receipts", [])
        .unwrap();
    let outcome = queue.observe_enqueue_result(
        &event,
        Err(StorageError::new(
            StorageErrorCode::Unknown,
            "fixture commit response unavailable",
        )),
        Instant::now() + Duration::from_secs(2),
    );
    assert!(
        matches!(outcome, CapturePublication::Unknown { .. }),
        "{outcome:?}"
    );
    assert!(queue.next().unwrap().is_none());
    assert_eq!(w.final_count(), 0);
}

#[test]
fn a_failed_flusher_start_keeps_the_accepted_event_and_bounded_startup_diagnostics() {
    let mut w = World::new();
    let event = w.event("accepted text must remain recoverable", false);
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    queue.enqueue(&event).unwrap();
    let directory = w.path.parent().unwrap();
    let error = super::worker::start_logged(
        directory,
        &directory.join("missing-core-executable"),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    let logs: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("hook-flush-")
        })
        .collect();
    assert_eq!(logs.len(), 1);
    let diagnostic = std::fs::read_to_string(&logs[0]).unwrap();
    assert!(
        diagnostic.contains("hook_flush_start=unconfirmed stage=spawn queue_publication=accepted")
    );
    assert!(diagnostic.len() <= 160);
    assert!(!diagnostic.contains("accepted text"));
    assert!(!diagnostic.contains(directory.to_str().unwrap()));
    let pending = queue.next().unwrap().unwrap();
    assert_eq!(pending.id(), event.id());
    assert_eq!(w.final_count(), 0);
    assert!(w.visible_failures().is_empty());
    assert_eq!(
        w.storage.ingest_hook_capture(&pending).unwrap(),
        CaptureResult::Committed
    );
    queue.acknowledge(&pending).unwrap();
    assert_eq!(
        w.storage.ingest_hook_capture(&pending).unwrap(),
        CaptureResult::Committed
    );
    assert_eq!(
        w.final_count(),
        1,
        "the existing receipt owns recovery exactly once"
    );
    assert!(queue.next().unwrap().is_none());
    assert!(w.visible_failures().is_empty());
}

#[test]
fn unbatched_refused_ends_have_standalone_failures_on_queued_and_direct_paths() {
    for queued in [false, true] {
        for refused in [CaptureResult::NoAdmittedStart, CaptureResult::NotApplicable] {
            let mut w = World::new();
            w.storage
                .connection()
                .unwrap()
                .execute("DELETE FROM capture_batches", [])
                .unwrap();
            w.refuse_end(refused);
            let event = w.event("unbatched verified final", false);
            assert_eq!(event.admission(), refused);
            assert!(serde_json::to_value(&event).unwrap()["capture"]["batch"].is_null());
            let before = BindingRows(w.storage.connection().unwrap())
                .session_preferences(&w.agent)
                .unwrap();
            for _ in 0..2 {
                let outcome = if queued {
                    w.storage.ingest_hook_capture(&event).unwrap()
                } else {
                    CaptureResult::from(w.direct_end(&event, "unbatched verified final"))
                };
                assert_eq!(outcome, refused);
            }
            let failures = w.visible_failures();
            assert_eq!(
                failures.len(),
                1,
                "a verified unbatched refusal must be visible exactly once"
            );
            assert!(failures[0].get("inputBatchId").is_none());
            assert!(failures[0].get("requestId").is_none());
            assert_eq!(failures[0]["failure"]["code"], "worker_error");
            assert_eq!(w.final_count(), 0);
            let db = w.storage.connection().unwrap();
            assert_eq!(current(db, &w.agent).unwrap(), None);
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM capture_batches", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                BindingRows(db).session_preferences(&w.agent).unwrap(),
                before
            );
            let receipt: (Option<String>, String) = db
                .query_row(
                    "SELECT batch_id,outcome FROM hook_capture_receipts WHERE event_id=?",
                    [event.id()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(receipt, (None, refused.token().into()));
        }
    }
}

#[test]
fn distinct_unbatched_misses_keep_separate_entries_even_for_the_same_text() {
    for queued in [false, true] {
        let mut w = World::new();
        w.storage
            .connection()
            .unwrap()
            .execute("DELETE FROM capture_batches", [])
            .unwrap();
        w.refuse_end(CaptureResult::NoAdmittedStart);
        let first = w.event("same unassociated text", false);
        let second = w.event("same unassociated text", false);
        for event in [&first, &first, &second] {
            if queued {
                w.storage.ingest_hook_capture(event).unwrap();
            } else {
                w.direct_end(event, "same unassociated text");
            }
        }
        let failures = w.visible_failures();
        assert_eq!(failures.len(), 2);
        assert_ne!(failures[0]["entryId"], failures[1]["entryId"]);
        assert!(
            failures.iter().all(
                |entry| entry.get("inputBatchId").is_none() && entry.get("requestId").is_none()
            )
        );
        assert_eq!(w.final_count(), 0);
    }
}

#[test]
fn an_unbatched_failure_does_not_borrow_a_newer_prompt_or_change_its_state() {
    for queued in [false, true] {
        let mut w = World::new();
        w.storage
            .connection()
            .unwrap()
            .execute("DELETE FROM capture_batches", [])
            .unwrap();
        w.refuse_end(CaptureResult::NoAdmittedStart);
        let event = w.event("older unbatched miss", false);
        let expected = Storage::context_by_identity(&w.path, &w.agent, w.now)
            .unwrap()
            .unwrap();
        let newer = w.new_prompt();
        let db = w.storage.connection().unwrap();
        db.execute("INSERT INTO request_attempts(attempt_id,request_id,route_kind,recipient_identity_id,originator_kind,originator_identity_id,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms) VALUES('new-input','new-input','inbox',?1,'explicit',?1,0,'sent',0,0,?2,?3,?3)",params![w.agent,w.now as i64,(w.now+60_000) as i64]).unwrap();
        db.execute("INSERT INTO request_passthrough(request_id,delivered_at_ms,state,bound_session,bound_at_ms,folded) VALUES('new-input',?1,'bound','fixture-session',?1,1)",[w.now as i64]).unwrap();
        let before = BindingRows(db).session_preferences(&w.agent).unwrap();
        if queued {
            w.storage.ingest_hook_capture(&event).unwrap();
        } else {
            w.direct_end_with_snapshot(&event, "older unbatched miss", &expected);
        }
        let failures = w.visible_failures();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].get("inputBatchId").is_none());
        assert!(failures[0].get("requestId").is_none());
        let db = w.storage.connection().unwrap();
        assert_eq!(
            current(db, &w.agent).unwrap().as_deref(),
            Some(newer.as_str())
        );
        assert_eq!(
            BindingRows(db).session_preferences(&w.agent).unwrap(),
            before
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM capture_batches", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT state FROM request_passthrough WHERE request_id='new-input'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "bound"
        );
        assert_eq!(w.final_count(), 0);
    }
}

#[test]
fn unbatched_failures_require_the_same_active_binding_session_and_process() {
    for queued in [false, true] {
        for change in [
            "binding", "session", "pid", "start", "ended", "retired", "provider", "detached",
        ] {
            let mut w = World::new();
            w.storage
                .connection()
                .unwrap()
                .execute("DELETE FROM capture_batches", [])
                .unwrap();
            w.refuse_end(CaptureResult::NoAdmittedStart);
            let event = w.event("verified before replacement", false);
            let expected = Storage::context_by_identity(&w.path, &w.agent, w.now)
                .unwrap()
                .unwrap();
            let db = w.storage.connection().unwrap();
            let sql = match change {
                "binding" => "UPDATE bindings SET id='22222222-2222-4222-8222-222222222222'",
                "session" => {
                    "UPDATE bindings SET observed_provider_session_id='replacement-session'"
                }
                "pid" => "UPDATE bindings SET runtime_pid=13",
                "start" => "UPDATE bindings SET runtime_start_identity='replacement-start'",
                "ended" => "UPDATE bindings SET runtime_state='ended'",
                "retired" => "UPDATE identities SET retired_at_ms=1",
                "provider" => "UPDATE identity_session_preferences SET remembered_harness='codex'",
                "detached" => "DELETE FROM bindings",
                _ => unreachable!(),
            };
            db.execute(sql, []).unwrap();
            let before = BindingRows(db).entry_by_id(&w.agent).unwrap();
            if queued {
                w.storage.ingest_hook_capture(&event).unwrap();
            } else {
                w.direct_end_with_snapshot(&event, "verified before replacement", &expected);
            }
            assert!(w.visible_failures().is_empty(), "{queued} {change}");
            assert_eq!(w.final_count(), 0);
            let db = w.storage.connection().unwrap();
            assert_eq!(BindingRows(db).entry_by_id(&w.agent).unwrap(), before);
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM capture_batches", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}

#[test]
fn an_unbatched_failure_and_its_receipt_roll_back_together() {
    let mut w = World::new();
    w.storage
        .connection()
        .unwrap()
        .execute("DELETE FROM capture_batches", [])
        .unwrap();
    w.refuse_end(CaptureResult::NoAdmittedStart);
    let event = w.event("atomic unbatched miss", false);
    w.storage.connection().unwrap().execute_batch("CREATE TEMP TRIGGER reject_unbatched_receipt BEFORE INSERT ON hook_capture_receipts BEGIN SELECT RAISE(ABORT,'receipt failure'); END;").unwrap();
    assert!(w.storage.ingest_hook_capture(&event).is_err());
    assert!(w.visible_failures().is_empty());
    assert_eq!(
        w.storage
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM hook_capture_receipts", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    w.storage
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_unbatched_receipt")
        .unwrap();
    w.storage.ingest_hook_capture(&event).unwrap();
    assert_eq!(w.visible_failures().len(), 1);
}

#[test]
fn confirmed_unpublished_unbatched_text_uses_the_existing_reason_and_event_fence() {
    for provisional in [false, true] {
        for code in [StorageErrorCode::Busy, StorageErrorCode::Unknown] {
            let mut w = World::new();
            w.storage
                .connection()
                .unwrap()
                .execute("DELETE FROM capture_batches", [])
                .unwrap();
            if !provisional {
                w.refuse_end(CaptureResult::NoAdmittedStart);
            }
            let event = w.event("never published", false);
            assert!(event.recorded_batch().is_none());
            let before = BindingRows(w.storage.connection().unwrap())
                .session_preferences(&w.agent)
                .unwrap();
            let publication =
                CapturePublication::Unpublished(StorageError::new(code, "fixture rejection"));
            for _ in 0..2 {
                publication
                    .record_failure(&w.path, &event, Instant::now() + Duration::from_secs(2))
                    .unwrap();
            }
            let failures = w.visible_failures();
            assert_eq!(failures.len(), 1);
            assert!(failures[0].get("inputBatchId").is_none());
            assert_eq!(
                failures[0]["failure"]["code"],
                if code == StorageErrorCode::Busy {
                    "budget_timeout"
                } else {
                    "storage_error"
                }
            );
            let db = w.storage.connection().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT batch_id FROM hook_capture_receipts WHERE event_id=?",
                    [event.id()],
                    |row| row.get::<_, Option<String>>(0)
                )
                .unwrap(),
                None
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM capture_batches", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                BindingRows(db).session_preferences(&w.agent).unwrap(),
                before
            );
            assert_eq!(w.final_count(), 0);
        }
    }
}

#[test]
fn unbatched_text_never_turns_accepted_unknown_or_expired_publication_into_a_failure() {
    for case in [
        "queued",
        "unknown",
        "expired",
        "usage_only",
        "interrupted",
        "replacement",
    ] {
        let mut w = World::new();
        w.storage
            .connection()
            .unwrap()
            .execute("DELETE FROM capture_batches", [])
            .unwrap();
        w.refuse_end(CaptureResult::NoAdmittedStart);
        let mut event = w.event("not enough to claim nonpublication", false);
        if case == "usage_only" {
            event = event.usage_only();
        }
        if case == "interrupted" {
            let mut wire = serde_json::to_value(&event).unwrap();
            wire["capture"]["interrupted"] = serde_json::json!(true);
            event = serde_json::from_value(wire).unwrap();
        }
        if case == "replacement" {
            w.storage
                .connection()
                .unwrap()
                .execute("UPDATE bindings SET runtime_start_identity='replaced'", [])
                .unwrap();
        }
        let publication = match case {
            "queued" => CapturePublication::Queued,
            "unknown" => CapturePublication::Unknown {
                publication: StorageError::new(StorageErrorCode::Unknown, "uncertain"),
                readback: StorageError::new(StorageErrorCode::Busy, "unavailable"),
            },
            _ => CapturePublication::Unpublished(StorageError::new(
                StorageErrorCode::Busy,
                "fixture rejection",
            )),
        };
        let deadline = if case == "expired" {
            Instant::now() - Duration::from_secs(1)
        } else {
            Instant::now() + Duration::from_secs(2)
        };
        let result = publication.record_failure(&w.path, &event, deadline);
        if case == "expired" {
            assert_eq!(result.unwrap_err().code, StorageErrorCode::Busy);
        } else {
            result.unwrap();
        }
        assert!(w.visible_failures().is_empty(), "{case}");
        assert_eq!(w.final_count(), 0);
    }
}

#[test]
fn a_zero_wait_schema_read_reports_busy_until_wal_admission_is_released() {
    let directory = TestDirectory::new();
    let main = directory.path.join("core.db");
    Storage::open(&main).unwrap().close().unwrap();
    let holder = Connection::open(HookQueue::path(&main)).unwrap();
    holder
        .pragma_update(None, "locking_mode", "EXCLUSIVE")
        .unwrap();
    // Successful BEGIN is the readiness signal: the second real connection
    // must attempt admission while this private WAL lock is still held.
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let mode: String = holder
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    let observed = HookQueue::open(&main, Duration::ZERO).map(drop);
    holder.execute_batch("ROLLBACK").unwrap();
    holder.close().unwrap();
    // Release the owned lock before every assertion and prove one fresh
    // zero-wait open sees the fully provisioned, unchanged empty queue.
    let queue = HookQueue::open(&main, Duration::ZERO).unwrap();
    let version: u32 = queue
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    assert_eq!(version, QUEUE_SCHEMA_VERSION);
    assert!(queue.next().unwrap().is_none());
    let failure = observed
        .as_ref()
        .expect_err("the controlled WAL admission lock must be observed");
    assert_eq!(failure.code, StorageErrorCode::Busy);
    assert!(
        failure.message.starts_with("Read hook queue schema"),
        "{failure:?}"
    );
    assert!(failure.retryable);
}

#[test]
fn a_near_expired_enqueue_open_reports_unpublished_busy_without_appending() {
    let mut w = World::new();
    let event = w.event("no budget to publish this final", false);
    let holder = Connection::open(HookQueue::path(&w.path)).unwrap();
    holder
        .pragma_update(None, "locking_mode", "EXCLUSIVE")
        .unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    // Exercise the adapters boundary used by CLI hook_flush_command::enqueue:
    // queue open must share the remaining hook budget, with no wait floor.
    let deadline = Instant::now() + Duration::from_micros(1);
    let wait = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(500));
    let publication = match HookQueue::open(&w.path, wait) {
        Ok(queue) => queue.enqueue_observed(&event, deadline),
        Err(error) => CapturePublication::Unpublished(error),
    };
    holder.execute_batch("ROLLBACK").unwrap();
    holder.close().unwrap();
    let CapturePublication::Unpublished(error) = &publication else {
        panic!("expected confirmed non-publication: {publication:?}");
    };
    assert_eq!(error.code, StorageErrorCode::Busy);
    assert!(
        error.message.starts_with("Read hook queue schema"),
        "{error:?}"
    );
    let queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    assert!(queue.next().unwrap().is_none());
    assert_eq!(w.final_count(), 0);
    // Give the diagnostic its own allowance only to pin Busy's existing
    // caption. Production shares the original deadline and may have none left.
    publication
        .record_failure(&w.path, &event, Instant::now() + Duration::from_secs(2))
        .unwrap();
    let failures = w.visible_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["failure"]["code"], "budget_timeout");
    assert!(queue.next().unwrap().is_none());
    assert_eq!(w.final_count(), 0);
}

fn assert_expired_recorded_text_does_not_suppress_a_new_event(queued: bool) {
    let mut w = World::new();
    let text = "a complete final whose original retention has ended";
    let capture = |w: &mut World, event: &CaptureEvent| {
        if queued {
            w.storage.ingest_hook_capture(event).unwrap()
        } else {
            CaptureResult::from(w.direct_end(event, text))
        }
    };
    let original = w.event(text, false);
    assert_eq!(capture(&mut w, &original), CaptureResult::Committed);
    let original_key: i64 = w
        .storage
        .connection()
        .unwrap()
        .query_row("SELECT sequence FROM recent_responses", [], |row| {
            row.get(0)
        })
        .unwrap();
    let live_replay = w.event(text, false);
    assert_eq!(capture(&mut w, &live_replay), CaptureResult::Duplicate);
    assert_eq!(w.final_count(), 1);
    assert!(w.visible_failures().is_empty());

    // Leave the expired body physically present. No sleep or retention sweep
    // may hide the predicate this regression is meant to exercise.
    w.storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE recent_responses SET ended_ms=1, expires_ms=2 WHERE sequence=?",
            [original_key],
        )
        .unwrap();
    assert_eq!(w.final_count(), 1);
    assert_eq!(capture(&mut w, &original), CaptureResult::Committed);
    assert_eq!(
        w.final_count(),
        1,
        "event receipt replay remains a no-op after expiry"
    );
    let fresh = w.event(text, false);
    assert_eq!(
        capture(&mut w, &fresh),
        CaptureResult::Committed,
        "an expired-but-unpruned final cannot suppress a different event",
    );
    let retained_key: i64 = w
        .storage
        .connection()
        .unwrap()
        .query_row("SELECT sequence FROM recent_responses", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_ne!(retained_key, original_key);
    assert_eq!(w.final_count(), 1, "the new append prunes the expired body");
    assert!(w.visible_failures().is_empty());
    let page = w
        .storage
        .conversation_timeline(&w.agent, 50, None, w.now + 100)
        .unwrap();
    let finals: Vec<_> = page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "final")
        .collect();
    assert_eq!(finals.len(), 2);
    assert_eq!(
        finals[0]["responseId"],
        tmt_core::responses::response_id(u64::try_from(original_key).unwrap())
    );
    assert_eq!(finals[0]["availability"], "unavailable");
    assert_eq!(
        finals[1]["responseId"],
        tmt_core::responses::response_id(u64::try_from(retained_key).unwrap())
    );
    assert_eq!(finals[1]["availability"], "retained");
    assert_eq!(finals[0]["inputBatchId"], finals[1]["inputBatchId"]);
    assert_eq!(capture(&mut w, &fresh), CaptureResult::Committed);
    assert_eq!(capture(&mut w, &original), CaptureResult::Committed);
    assert_eq!(w.final_count(), 1);
    assert!(w.visible_failures().is_empty());
}

#[test]
fn expired_text_in_the_recorded_batch_cannot_suppress_a_queued_event() {
    assert_expired_recorded_text_does_not_suppress_a_new_event(true);
}

#[test]
fn expired_text_in_the_recorded_batch_cannot_suppress_a_direct_event() {
    assert_expired_recorded_text_does_not_suppress_a_new_event(false);
}

#[test]
fn exact_text_replay_requires_a_body_live_at_the_capture_read_clock() {
    let mut w = World::new();
    let text = "a retained body with an explicit availability boundary";
    let event = w.event(text, false);
    w.storage.ingest_hook_capture(&event).unwrap();
    let db = w.storage.connection().unwrap();
    db.execute("UPDATE recent_responses SET ended_ms=1, expires_ms=2", [])
        .unwrap();
    let batch = current(db, &w.agent).unwrap();
    let response = ResponseCapture {
        text: tmt_core::responses::FinalText::new(text).unwrap(),
    };
    assert!(
        super::super::responses::same_final_in_batch(db, batch.as_deref(), &response, 1).unwrap()
    );
    for read_clock in [2, 3] {
        assert!(
            !super::super::responses::same_final_in_batch(
                db,
                batch.as_deref(),
                &response,
                read_clock
            )
            .unwrap()
        );
    }
    assert_eq!(
        w.final_count(),
        1,
        "availability checks do not prune the body"
    );
}

#[test]
fn expired_text_does_not_hide_a_refused_direct_or_queued_end() {
    for queued in [false, true] {
        let mut w = World::new();
        let text = "expired text is no longer a visible answer";
        let first = w.event(text, false);
        if queued {
            w.storage.ingest_hook_capture(&first).unwrap();
        } else {
            w.direct_end(&first, text);
        }
        w.storage
            .connection()
            .unwrap()
            .execute("UPDATE recent_responses SET ended_ms=1, expires_ms=2", [])
            .unwrap();
        w.refuse_end(CaptureResult::NoAdmittedStart);
        let refused = w.event(text, false);
        for _ in 0..2 {
            let outcome = if queued {
                w.storage.ingest_hook_capture(&refused).unwrap()
            } else {
                CaptureResult::from(w.direct_end(&refused, text))
            };
            assert_eq!(outcome, CaptureResult::NoAdmittedStart);
        }
        let failures = w.visible_failures();
        assert_eq!(
            failures.len(),
            1,
            "{queued}: the refused event stays event-fenced"
        );
        assert_eq!(failures[0]["failure"]["code"], "worker_error");
        assert_eq!(w.final_count(), 1, "a refused end adds no final");
    }
}

#[test]
fn the_flusher_opens_a_database_reached_through_a_linked_directory_but_never_a_linked_file() {
    let w = World::new();
    let real = w.path.parent().unwrap().to_path_buf();
    let links = TestDirectory::new();
    // A data root reached through a link, as macOS /var or a symlinked ~/.config is.
    let alias = links.path.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let mut flusher = Storage::open_hook_flusher(&alias.join("capture.db"))
        .expect("a linked directory resolves to the real one");
    flusher.close().unwrap();
    // The database file itself must not be a link.
    let linked_file = links.path.join("linked.db");
    std::os::unix::fs::symlink(&w.path, &linked_file).unwrap();
    assert!(Storage::open_hook_flusher(&linked_file).is_err());
    // A missing directory fails as before and creates nothing.
    let missing = links.path.join("missing/capture.db");
    assert!(Storage::open_hook_flusher(&missing).is_err());
    assert!(!links.path.join("missing").exists());
}

#[test]
fn housekeeping_through_a_linked_data_directory_still_prunes_old_receipts() {
    let w = World::new();
    let _queue = HookQueue::open(&w.path, Duration::ZERO).unwrap();
    let links = TestDirectory::new();
    let alias = links.path.join("alias");
    std::os::unix::fs::symlink(w.path.parent().unwrap(), &alias).unwrap();
    let mut storage = Storage::open(alias.join("capture.db")).unwrap();
    storage.connection().unwrap().execute("INSERT INTO hook_capture_receipts(event_id,outcome,committed_ms) VALUES('old-unrelated','duplicate',1)",[]).unwrap();
    let now = w.now;
    tmt_core::request::RequestService::new(&mut storage, || now)
        .cleanup()
        .unwrap();
    let exists: bool = storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM hook_capture_receipts WHERE event_id='old-unrelated')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!exists, "retention reached the queue through the link");
}
