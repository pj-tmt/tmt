use super::*;
use crate::orchestration::{decode_orch_query, orchestration_read};
use serde_json::{Value, json};
fn read(f: &Fixture, query: Value) -> Value {
    let query = decode_orch_query(&serde_json::to_vec(&query).unwrap()).unwrap();
    serde_json::from_slice(&orchestration_read(&f.storage, query, NOW).unwrap()).unwrap()
}
#[test]
fn public_jobs_page_is_complete_scoped_and_bounded_while_show_keeps_exact_text() {
    let mut f = Fixture::new();
    let text = format!("{}END-PRIVATE-BODY", "雪".repeat(300));
    let unscoped = f
        .storage
        .create_orchestration_job(f.definition(None, &text))
        .unwrap();
    let scoped = f
        .storage
        .create_orchestration_job(f.definition(Some(ROOM), "Squad job"))
        .unwrap();
    let revision = f
        .storage
        .orchestration_snapshot(None)
        .unwrap()
        .snapshot
        .revision;
    let first = read(&f, json!({"view":"jobs","limit":1}));
    assert_eq!(first["jobs"][0]["jobId"], unscoped.id);
    assert!(first["jobs"][0]["target"]["squadRoomId"].is_null());
    assert_eq!(first["jobs"][0]["target"]["identityId"], f.owner);
    assert_eq!(
        first["jobs"][0]["messagePreview"].as_str().unwrap().len(),
        255
    );
    assert!(first["jobs"][0].get("definition").is_none());
    assert!(!first.to_string().contains("END-PRIVATE-BODY"));
    assert!(first["jobs"][0]["lastRun"].is_null());
    assert!(first["jobs"][0]["nextRunAtMs"].as_i64().unwrap() > NOW as i64);
    let next = read(
        &f,
        json!({"view":"jobs","limit":1,"cursor":first["nextCursor"]}),
    );
    assert_eq!(next["jobs"][0]["jobId"], scoped.id);
    assert!(next["nextCursor"].is_null());
    let room = read(&f, json!({"view":"jobs","squadRoomId":ROOM,"limit":1}));
    assert_eq!(room["jobs"][0]["jobId"], scoped.id);
    assert!(room["nextCursor"].is_null());
    assert_eq!(room["historyCoverage"], "recorded_only");
    let show = read(&f, json!({"view":"job","jobId":unscoped.id}));
    assert_eq!(show["job"]["definition"]["message"], text);
    assert_eq!(show["job"]["definition"]["ownerIdentityId"], f.owner);
    let changed_scope = decode_orch_query(
        &serde_json::to_vec(
            &json!({"view":"jobs","squadRoomId":ROOM,"cursor":first["nextCursor"]}),
        )
        .unwrap(),
    )
    .unwrap();
    let error = orchestration_read(&f.storage, changed_scope, NOW).unwrap_err();
    assert_eq!(error.code(), "ORCH_INPUT_INVALID");
    assert_eq!(error.exit(), 1);
    assert_eq!(
        f.storage
            .orchestration_snapshot(None)
            .unwrap()
            .snapshot
            .revision,
        revision
    );
    assert_eq!(count(&f, "orchestration_decisions"), 0);
}
#[test]
fn last_run_uses_canonical_final_state_without_read_mutations_or_response_bodies() {
    use tmt_core::request::{
        RequestRoute, RequestService, ResponseProof, SubmitResponse, correlation,
    };
    let mut f = Fixture::new();
    let job = f
        .storage
        .create_orchestration_job(f.definition(None, "Run this job"))
        .unwrap();
    let JobRunResult::Accepted { receipt, .. } = f
        .storage
        .run_orchestration_job(run_input(&job, 10_000), 7, || NOW)
        .unwrap()
    else {
        panic!("accept job")
    };
    f.storage
        .connection()
        .unwrap()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    let before = read(&f, json!({"view":"jobs","jobId":job.id}));
    assert_eq!(before["jobs"][0]["lastRun"]["acceptance"], "queued");
    assert_eq!(
        before["jobs"][0]["lastRun"]["request"]["final"]["status"],
        "not_submitted"
    );
    assert!(before["jobs"][0]["lastRun"]["resultReference"].is_null());
    f.storage
        .connection()
        .unwrap()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    let request_id = &receipt.items[0].request_id;
    let attempt: String = f
        .storage
        .connection()
        .unwrap()
        .query_row(
            "SELECT attempt_id FROM request_attempts WHERE request_id=?",
            [request_id],
            |row| row.get(0),
        )
        .unwrap();
    let route = RequestRoute::Inbox {
        recipient_identity_id: f.owner.clone(),
    };
    RequestService::new(&mut f.storage, || NOW)
        .submit_response(SubmitResponse {
            request_id: request_id.clone(),
            proof: ResponseProof::Compact(correlation::response_token(
                request_id, &attempt, &route,
            )),
            body: "FINAL-BODY-MUST-NOT-BE-IN-JOBS".into(),
        })
        .unwrap();
    let cursor: i64 = f
        .storage
        .connection()
        .unwrap()
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    f.storage
        .connection()
        .unwrap()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    let after = read(&f, json!({"view":"jobs","jobId":job.id}));
    assert_eq!(after["jobs"][0]["lastRun"]["acceptance"], "queued");
    assert_eq!(
        after["jobs"][0]["lastRun"]["request"]["final"]["status"],
        "retained"
    );
    assert_eq!(
        after["jobs"][0]["lastRun"]["resultReference"]["requestId"],
        *request_id
    );
    assert!(!after.to_string().contains("FINAL-BODY-MUST-NOT-BE-IN-JOBS"));
    let after_cursor: i64 = f
        .storage
        .connection()
        .unwrap()
        .query_row("SELECT value FROM change_cursor WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(cursor, after_cursor);
    f.storage
        .connection()
        .unwrap()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
}
#[test]
fn selected_job_policy_does_not_expand_the_whole_workspace_identity_inventory() {
    let mut f = Fixture::new();
    let job = f
        .storage
        .create_orchestration_job(f.definition(None, "Selected owner"))
        .unwrap();
    for index in 0..260 {
        create_or_resolve(&mut f.storage, &format!("noise-{index}"), Lifetime::Saved).unwrap();
    }
    let selected = read(&f, json!({"view":"job","jobId":job.id}));
    assert_eq!(selected["job"]["target"]["identityId"], f.owner);
    assert_eq!(selected["job"]["routine"]["value"], "run");
    assert_eq!(selected["job"]["definition"]["message"], "Selected owner");
}
#[test]
fn strict_jobs_queries_and_missing_job_keep_bounded_error_codes() {
    let f = Fixture::new();
    for query in [
        json!({"view":"jobs","limit":0}),
        json!({"view":"jobs","limit":129}),
        json!({"view":"jobs","ownerIdentityId":"Member"}),
        json!({"view":"jobs","extra":true}),
        json!({"view":"job","jobId":"j1"}),
    ] {
        assert!(decode_orch_query(&serde_json::to_vec(&query).unwrap()).is_err());
    }
    let query = decode_orch_query(
        &serde_json::to_vec(&json!({"view":"job","jobId":"99999999-9999-4999-8999-999999999999"}))
            .unwrap(),
    )
    .unwrap();
    let error = orchestration_read(&f.storage, query, NOW).unwrap_err();
    assert_eq!(error.code(), "ORCH_JOB_NOT_FOUND");
    assert_eq!(error.exit(), 3);
    assert_eq!(count(&f, "orchestration_jobs"), 0);
}
