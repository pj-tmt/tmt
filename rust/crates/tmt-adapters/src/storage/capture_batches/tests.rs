//! Real private SQLite state for ordered inputs/finals and frozen ack revisions.
use super::super::{ResponseCapture, Storage, TurnCapture};
use super::*;
use crate::{runtime::lifecycle::RuntimeLifecycle, test_support::TestDirectory};
use std::time::{Duration, Instant};
use tmt_core::{
    binding::session::{
        ProviderSessionId,
        activity::{Event, EventKind},
    },
    endpoint::ProcessIncarnation,
    identity::Lifetime,
};

struct World {
    _directory: TestDirectory,
    path: std::path::PathBuf,
    storage: Storage,
    agent: String,
    person: String,
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
        let person = tmt_core::identity::create_or_resolve(&mut storage, "Ben", Lifetime::Saved)
            .unwrap()
            .identity
            .id;
        let now = crate::request_runtime::wall_time_ms();
        storage.connection().unwrap().execute("INSERT INTO bindings(id,identity_id,transport,pane_id,server_id,socket_path,server_pid,server_start_time,pane_pid,bound_at,last_verified_at,runtime_state,runtime_pid,runtime_start_identity,observed_provider_session_id)
            VALUES('binding',?1,'tmux','%1','server','/private/fake.sock',10,'server-start',11,'t','t','running',12,'provider-start','fixture-session')",[&agent]).unwrap();
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
            person,
            process,
            now,
        }
    }
    fn input(&self, index: usize) -> String {
        let request = format!("req_{}", tmt_core::operation::new_operation_id());
        self.storage.connection().unwrap().execute("INSERT INTO request_attempts(attempt_id,request_id,route_kind,recipient_identity_id,originator_kind,originator_identity_id,wait_active,status,inject_preamble,cadence_reserved,prepared_at_ms,expires_at_ms,retention_expires_at_ms,message_text,message_bytes,message_expires_at_ms)
            VALUES(?1,?1,'inbox',?2,'explicit',?3,0,'sent',0,0,?4,?5,?5,?6,?7,?5)",params![request,self.agent,self.person,self.now as i64,(self.now+60_000) as i64,format!("input {index}"),format!("input {index}").len() as i64]).unwrap();
        self.storage.connection().unwrap().execute("INSERT INTO request_passthrough(request_id,delivered_at_ms,state,bound_session,bound_at_ms,folded) VALUES(?1,?2,'bound','fixture-session',?2,1)",params![request,self.now as i64]).unwrap();
        let batch = current(self.storage.connection().unwrap(), &self.agent)
            .unwrap()
            .unwrap();
        input(
            self.storage.connection().unwrap(),
            &self.agent,
            &batch,
            &request,
            "incoming",
            "delivered",
            self.now,
        )
        .unwrap();
        request
    }
    fn end(&mut self, text: &str, continued: bool) {
        let lifecycle = crate::drivers::claude::ClaudeLifecycle;
        let payload = serde_json::json!({"hook_event_name":"Stop","session_id":"fixture-session","stop_hook_active":continued,"last_assistant_message":text});
        let payload = payload.to_string();
        let turn = lifecycle.decode_turn(payload.as_bytes()).unwrap();
        let activity = lifecycle.decode_activity(payload.as_bytes()).unwrap();
        let expected = Storage::context_by_identity(&self.path, &self.agent, self.now)
            .unwrap()
            .unwrap();
        self.now += 1;
        assert!(
            self.storage
                .capture_turn_end(TurnCapture {
                    expected: &expected,
                    process: &self.process,
                    activity: &activity,
                    interrupted: false,
                    response: Some(ResponseCapture {
                        text: turn.final_text.unwrap()
                    }),
                    now_ms: self.now,
                    deadline: Instant::now() + Duration::from_secs(2)
                })
                .unwrap()
        );
    }
    fn timeline(&mut self, limit: u64, before: Option<u64>) -> serde_json::Value {
        self.storage
            .conversation_timeline(&self.agent, limit, before, self.now + 100)
            .unwrap()
    }
}

#[test]
fn fifty_one_folded_inputs_keep_the_answer_for_all_and_bound_only_the_painted_ack_keys() {
    let mut world = World::new();
    for index in 0..51 {
        world.input(index);
    }
    world.end("one answer", false);
    let count: i64 = world
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM request_passthrough WHERE state='answered'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 51);
    let cursor = world.storage.change_cursor().unwrap();
    let page = world.timeline(50, None);
    let last = page["entries"].as_array().unwrap().last().unwrap();
    assert_eq!(last["kind"], "final");
    assert_eq!(last["acknowledgements"].as_array().unwrap().len(), 50);
    assert_eq!(last["acknowledgementsTruncated"], true);
    assert_eq!(page["responses"][0]["text"], "one answer");
    let before = page["nextBefore"]
        .as_str()
        .unwrap()
        .strip_prefix("ce_")
        .unwrap()
        .parse()
        .unwrap();
    let older = world.timeline(50, Some(before));
    assert_eq!(older["entries"].as_array().unwrap().len(), 2);
    assert_eq!(
        world.storage.change_cursor().unwrap(),
        cursor,
        "paged reads do not write or acknowledge"
    );
    let key = &last["acknowledgements"][0];
    let request = key["requestId"].as_str().unwrap();
    let original = key["revision"].as_u64().unwrap();
    world
        .storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE request_attempts SET attention_revision=?2 WHERE request_id=?1",
            params![request, (original + 100) as i64],
        )
        .unwrap();
    acknowledgeable(world.storage.connection().unwrap(), request, original + 100).unwrap();
    let after = world.timeline(50, None);
    let final_entry = after["entries"].as_array().unwrap().last().unwrap();
    assert_eq!(
        final_entry["acknowledgements"], last["acknowledgements"],
        "a later request revision never replaces the painted body's revision"
    );
}

#[test]
fn equal_text_finals_keep_separate_ids_and_explicit_order() {
    let mut world = World::new();
    world.input(0);
    world.end("same text", false);
    world.end("same text", true);
    let page = world.timeline(50, None);
    assert_eq!(page["responses"].as_array().unwrap().len(), 2);
    assert_ne!(
        page["responses"][0]["responseId"],
        page["responses"][1]["responseId"]
    );
    assert_eq!(
        page["entries"][1]["inputBatchId"],
        page["entries"][2]["inputBatchId"]
    );
    assert_eq!(
        page["entries"][2]["acknowledgements"],
        serde_json::json!([])
    );
}

#[test]
fn unavailable_bodies_and_capture_failures_are_explicit_without_read_time_writes() {
    let mut world = World::new();
    let request = world.input(0);
    world
        .storage
        .connection()
        .unwrap()
        .execute(
            "UPDATE request_attempts SET message_text=NULL WHERE request_id=?",
            [request],
        )
        .unwrap();
    let expected = Storage::context_by_identity(&world.path, &world.agent, world.now)
        .unwrap()
        .unwrap();
    world
        .storage
        .record_capture_failure(
            &expected,
            super::super::CaptureFailure::Storage,
            world.now,
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
    let cursor = world.storage.change_cursor().unwrap();
    let page = world.timeline(50, None);
    assert_eq!(page["entries"][0]["availability"], "unavailable");
    assert!(page["entries"][0].get("text").is_none());
    assert_eq!(page["entries"][1]["kind"], "failure");
    assert_eq!(page["entries"][1]["failure"]["code"], "storage_error");
    assert!(page["entries"][0].get("acknowledgements").is_none());
    assert!(page["entries"][1].get("acknowledgements").is_none());
    assert_eq!(world.storage.change_cursor().unwrap(), cursor);
}

#[test]
fn pending_digest_inputs_keep_order_without_claiming_delivery() {
    let mut w = World::new();
    let request = w.input(0);
    let db = w.storage.connection().unwrap();
    db.execute("DELETE FROM capture_entries WHERE request_id=?", [&request])
        .unwrap();
    db.execute("INSERT INTO focus_checklists(id,identity_id,attempt_token,through_sequence,opportunity,state,created_at_ms) VALUES('pending',?1,'token',1,'turn_boundary','claimed',?2)",params![w.agent,w.now as i64]).unwrap();
    db.execute("INSERT INTO focus_items(identity_id,request_id,kind,source,created_at_ms,checklist_id) VALUES(?1,?2,'fyi','incoming',?3,'pending')",params![w.agent,request,w.now as i64]).unwrap();
    reserve_checklist(
        db,
        &w.agent,
        "pending",
        w.now,
        tmt_core::request::digest::DigestOpportunity::TurnBoundary,
    )
    .unwrap();
    assert_eq!(w.timeline(50, None)["entries"], serde_json::json!([]));
    w.end("fast final", false);
    assert_eq!(w.timeline(50, None)["entries"][0]["kind"], "final");
    settle_checklist(
        w.storage.connection().unwrap(),
        "pending",
        Some("uncertain"),
    )
    .unwrap();
    let page = w.timeline(50, None);
    assert_eq!(page["entries"][0]["kind"], "input");
    assert_eq!(page["entries"][0]["delivery"], "uncertain");
    assert_eq!(page["entries"][1]["kind"], "final");
}
#[test]
fn definitely_unsent_inputs_are_removed_instead_of_inferred_as_delivered() {
    let mut w = World::new();
    let request = w.input(0);
    let db = w.storage.connection().unwrap();
    db.execute(
        "UPDATE capture_entries SET delivery='pending',checklist_id='unsent' WHERE request_id=?",
        [request],
    )
    .unwrap();
    settle_checklist(db, "unsent", None).unwrap();
    assert_eq!(w.timeline(50, None)["entries"], serde_json::json!([]));
}
#[test]
fn expired_history_reports_its_coverage_without_a_read_time_write() {
    let mut w = World::new();
    w.input(0);
    w.end("retained", false);
    let later = w.now + tmt_core::responses::RETENTION_MS + 1;
    prune(w.storage.connection().unwrap(), &w.agent, later).unwrap();
    let cursor = w.storage.change_cursor().unwrap();
    let page = w
        .storage
        .conversation_timeline(&w.agent, 50, None, later)
        .unwrap();
    assert_eq!(page["entries"], serde_json::json!([]));
    assert_eq!(page["coverage"]["olderDropped"], true);
    assert_eq!(w.storage.change_cursor().unwrap(), cursor);
}
#[test]
fn stale_binding_and_expired_budget_cannot_capture_or_attach_failure() {
    for stale in [false, true] {
        let mut w = World::new();
        w.input(0);
        let expected = Storage::context_by_identity(&w.path, &w.agent, w.now)
            .unwrap()
            .unwrap();
        let lifecycle = crate::drivers::claude::ClaudeLifecycle;
        let payload=serde_json::json!({"hook_event_name":"Stop","session_id":"fixture-session","last_assistant_message":"refused"}).to_string();
        let turn = lifecycle.decode_turn(payload.as_bytes()).unwrap();
        let activity = lifecycle.decode_activity(payload.as_bytes()).unwrap();
        if stale {
            w.storage
                .connection()
                .unwrap()
                .execute(
                    "UPDATE bindings SET id='replacement' WHERE identity_id=?",
                    [&w.agent],
                )
                .unwrap();
        }
        let deadline = if stale {
            Instant::now() + Duration::from_secs(2)
        } else {
            Instant::now()
        };
        assert!(
            !w.storage
                .capture_turn_end(TurnCapture {
                    expected: &expected,
                    process: &w.process,
                    activity: &activity,
                    interrupted: false,
                    response: Some(ResponseCapture {
                        text: turn.final_text.unwrap()
                    }),
                    now_ms: w.now + 1,
                    deadline
                })
                .unwrap()
        );
        w.storage
            .record_capture_failure(
                &expected,
                super::super::CaptureFailure::Worker,
                w.now + 1,
                deadline,
            )
            .unwrap();
        let page = w.timeline(50, None);
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page["responses"], serde_json::json!([]));
    }
}

#[test]
fn interruption_keeps_earlier_answers_but_closes_the_batch_without_capturing_partial_text() {
    let mut w = World::new();
    w.input(0);
    w.end("kept", false);
    let old = w.timeline(50, None);
    let expected = Storage::context_by_identity(&w.path, &w.agent, w.now)
        .unwrap()
        .unwrap();
    // A continued canceled end has valid activity authority, but no final authority.
    let lifecycle = crate::drivers::claude::ClaudeLifecycle;
    let payload=serde_json::json!({"hook_event_name":"Stop","session_id":"fixture-session","stop_hook_active":true,"last_assistant_message":"partial"}).to_string();
    let turn = lifecycle.decode_turn(payload.as_bytes()).unwrap();
    let activity = lifecycle.decode_activity(payload.as_bytes()).unwrap();
    assert!(
        w.storage
            .capture_turn_end(TurnCapture {
                expected: &expected,
                process: &w.process,
                activity: &activity,
                interrupted: true,
                response: Some(ResponseCapture {
                    text: turn.final_text.unwrap()
                }),
                now_ms: w.now + 1,
                deadline: Instant::now() + Duration::from_secs(2)
            })
            .unwrap()
    );
    assert!(
        current(w.storage.connection().unwrap(), &w.agent)
            .unwrap()
            .is_none()
    );
    assert_eq!(w.timeline(50, None)["responses"], old["responses"]);
}

#[test]
fn a_new_session_key_and_an_ended_session_close_the_old_batch() {
    for ended in [false, true] {
        let w = World::new();
        let binding = BindingRows(w.storage.connection().unwrap())
            .entry_by_id(&w.agent)
            .unwrap()
            .unwrap()
            .binding
            .unwrap();
        let mut next = binding.session.clone();
        if ended {
            next.state = tmt_core::binding::session::RuntimeState::Ended;
        } else {
            next.key.as_mut().unwrap().provider_session =
                Some(ProviderSessionId::new("replacement").unwrap());
        }
        assert!(
            BindingRows(w.storage.connection().unwrap())
                .set_session_state(&binding.id, &binding.session, &next)
                .unwrap()
        );
        let active: i64 = w
            .storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM capture_batches WHERE identity_id=? AND closed_ms IS NULL",
                [&w.agent],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 0);
    }
}

#[test]
fn an_explicit_reply_and_equal_provider_final_remain_separate_and_freeze_the_reply_key() {
    use tmt_core::request::{FinalResponse, RequestRepository, RequestRoute};
    let mut w = World::new();
    let request = w.input(0);
    // An ordinary request is answered explicitly; the provider's own final is
    // a separate body even when the two texts happen to be equal.
    w.storage
        .connection()
        .unwrap()
        .execute(
            "DELETE FROM request_passthrough WHERE request_id=?",
            [&request],
        )
        .unwrap();
    w.end("same answer", false);
    let transaction = w.storage.connection_mut().unwrap().transaction().unwrap();
    let mut repository = super::super::requests::TransactionRequests(&transaction);
    repository
        .with_request_transaction::<_, StorageError>(|records| {
            records.create_response(&FinalResponse {
                request_id: request.clone(),
                attempt_id: request.clone(),
                route: RequestRoute::Inbox {
                    recipient_identity_id: w.agent.clone(),
                },
                body: "same answer".into(),
                body_bytes: 11,
                submitted_at_ms: w.now + 1,
                response_expires_at_ms: w.now + 60_000,
            })?;
            records.set_attention_revision(&request, 3)?;
            Ok(())
        })
        .unwrap();
    transaction.commit().unwrap();
    let page = w.timeline(50, None);
    assert_eq!(page["entries"][1]["kind"], "final");
    assert_eq!(page["entries"][2]["kind"], "reply");
    assert_eq!(page["responses"][0]["text"], "same answer");
    assert_eq!(page["entries"][2]["text"], "same answer");
    assert_eq!(
        page["entries"][1]["acknowledgements"],
        serde_json::json!([])
    );
    assert_eq!(
        page["entries"][2]["acknowledgements"],
        serde_json::json!([{"identityId":w.person,"requestId":request,"revision":3}])
    );
    acknowledgeable(w.storage.connection().unwrap(), &request, 4).unwrap();
    assert_eq!(
        w.timeline(50, None)["entries"][2]["acknowledgements"],
        page["entries"][2]["acknowledgements"]
    );
}

#[test]
fn ordinary_transport_reserves_input_before_a_fast_final_and_discards_definite_failure() {
    for published in [false, true] {
        let mut w = World::new();
        let request = w.input(0);
        let db = w.storage.connection().unwrap();
        db.execute("DELETE FROM capture_entries WHERE request_id=?", [&request])
            .unwrap();
        delivered(db, &request, "pending", w.now).unwrap();
        assert_eq!(w.timeline(50, None)["entries"], serde_json::json!([]));
        w.end("fast reply", false);
        if published {
            delivered(
                w.storage.connection().unwrap(),
                &request,
                "delivered",
                w.now,
            )
            .unwrap();
        } else {
            discard_unpublished_input(w.storage.connection().unwrap(), &request).unwrap();
        }
        let page = w.timeline(50, None);
        if published {
            assert_eq!(page["entries"][0]["kind"], "input");
            assert_eq!(page["entries"][1]["kind"], "final");
        } else {
            assert_eq!(page["entries"].as_array().unwrap().len(), 1);
            assert_eq!(page["entries"][0]["kind"], "final");
        }
    }
}

#[test]
fn an_idle_checklist_joins_its_explicitly_confirmed_new_prompt_instead_of_the_previous_batch() {
    let mut w = World::new();
    w.end("previous answer", false);
    let request = w.input(0);
    let db = w.storage.connection().unwrap();
    db.execute("DELETE FROM capture_entries WHERE request_id=?", [&request])
        .unwrap();
    db.execute("INSERT INTO focus_checklists(id,identity_id,attempt_token,through_sequence,opportunity,state,created_at_ms) VALUES('idle',?1,'idle-token',1,'idle','claimed',?2)",params![w.agent,w.now as i64]).unwrap();
    db.execute("INSERT INTO focus_items(identity_id,request_id,kind,source,created_at_ms,checklist_id) VALUES(?1,?2,'fyi','incoming',?3,'idle')",params![w.agent,request,w.now as i64]).unwrap();
    reserve_checklist(
        db,
        &w.agent,
        "idle",
        w.now,
        tmt_core::request::digest::DigestOpportunity::Idle,
    )
    .unwrap();
    settle_checklist(db, "idle", Some("delivered")).unwrap();
    // Publication settlement owns both the checklist fact and these entry links.
    db.execute(
        "UPDATE focus_checklists SET state='delivered' WHERE id='idle'",
        [],
    )
    .unwrap();
    assert!(
        w.timeline(50, None)["entries"][1]
            .get("inputBatchId")
            .is_none()
    );
    let mut prefs = BindingRows(w.storage.connection().unwrap())
        .session_preferences(&w.agent)
        .unwrap();
    let remembered = prefs.remembered.as_mut().unwrap();
    remembered.state = crate::runtime::driver_state::after_activity(
        remembered.state.as_ref(),
        &remembered.provider_session,
        &w.process,
        &Event {
            kind: EventKind::TurnStarted,
            turn: None,
        },
        None,
        w.now + 1,
    );
    BindingRows(w.storage.connection().unwrap())
        .set_session_preferences(&w.agent, &prefs)
        .unwrap();
    let binding = BindingRows(w.storage.connection().unwrap())
        .entry_by_id(&w.agent)
        .unwrap()
        .unwrap()
        .binding
        .unwrap();
    begin(
        w.storage.connection().unwrap(),
        &binding,
        prefs.remembered.as_ref().unwrap(),
        None,
        w.now + 1,
    )
    .unwrap();
    // The canonical confirmation owner returns the exact newly confirmed IDs.
    let confirmed = super::super::requests::confirm_digest_by_prompt(
        w.storage.connection().unwrap(),
        &w.agent,
        &format!("delivered {request}"),
        w.now + 1,
    )
    .unwrap();
    assert_eq!(confirmed, 1);
    w.now += 2;
    w.end("next answer", false);
    let page = w.timeline(50, None);
    assert_eq!(page["entries"][0]["kind"], "final");
    assert_eq!(page["entries"][1]["kind"], "input");
    assert_eq!(page["entries"][2]["kind"], "final");
    assert_ne!(
        page["entries"][0]["inputBatchId"],
        page["entries"][1]["inputBatchId"]
    );
    assert_eq!(
        page["entries"][1]["inputBatchId"],
        page["entries"][2]["inputBatchId"]
    );
}
