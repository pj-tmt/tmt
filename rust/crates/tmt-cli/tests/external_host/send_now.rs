//! `tmt send-now` takes one message out of the Digest hold and types it now (#4378): only the
//! originator or a person may ask, only the hold is skipped, and a message that reached the
//! agent is never typed twice.
use super::*;

/// A delivering driver with a saved `worker` in pane f1 whose Digest holds messages from
/// other agents, an agent `lead`, a bystander `eve` and a person `ben` (not yet marked).
fn world(name: &str) -> (Fixture, String) {
    register();
    let fixture = Fixture::delivering(name);
    fixture.approve();
    let named = fixture.tmt_in(Some("f1"), &["name", "worker", "--save", "--json"]);
    assert!(named.status.success(), "{named:?}");
    for other in ["lead", "eve", "ben"] {
        let created = fixture.tmt(&["identity", "create", other, "--json"]);
        assert!(created.status.success(), "{created:?}");
    }
    let id = worker(&fixture.json(&["ls", "--json"]))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let now = tmt_adapters::request_runtime::wall_time_ms() as i64;
    rusqlite::Connection::open(fixture.state().join("tmt.db"))
        .unwrap()
        .execute(
            "INSERT INTO focus_policies(identity_id,revision,until_ms,owner_identity_id,setter_identity_id,max_hold_ms)
             VALUES(?1,1,?2,?1,?1,300000)",
            rusqlite::params![id, now + 3_600_000],
        )
        .unwrap();
    (fixture, id)
}

fn held_request(fixture: &Fixture, message: &str) -> String {
    fs::write(fixture.root.join("driver/agent"), "ok").unwrap();
    let sent = fixture.tmt(&[
        "talk",
        "worker",
        message,
        "--identity",
        "lead",
        "--detach",
        "--json",
    ]);
    assert!(sent.status.success(), "{sent:?}");
    let report: Value = serde_json::from_slice(&sent.stdout).unwrap();
    assert_eq!(
        report["digest"], true,
        "held by the member's Digest: {report:#}"
    );
    assert!(
        fixture.requests("prompted").is_empty(),
        "a held message types nothing"
    );
    report["requestId"].as_str().unwrap().to_owned()
}

fn send_now(fixture: &Fixture, request: &str, identity: &str) -> (Option<i32>, Value) {
    let out = fixture.tmt(&["send-now", request, "--identity", identity, "--json"]);
    (
        out.status.code(),
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
    )
}

#[test]
fn the_originator_sends_a_held_message_now_and_it_is_never_typed_twice() {
    let (fixture, _) = world("send-now-originator");
    let request = held_request(&fixture, "Review this patch");

    let (code, sent) = send_now(&fixture, &request, "lead");
    assert_eq!(
        (code, sent["status"].as_str()),
        (Some(0), Some("sent")),
        "{sent:#}"
    );
    let prompted = fixture.requests("prompted");
    assert_eq!(prompted.len(), 1, "{prompted:?}");
    let text = prompted[0]["text"].as_str().unwrap();
    assert!(text.contains("Review this patch"), "{text}");
    assert!(
        text.contains(&format!("tmt reply {request} --receipt ")),
        "typed as a request, with its own reply command: {text}"
    );

    // Asking again reports where it is and types nothing.
    for _ in 0..2 {
        let (code, again) = send_now(&fixture, &request, "lead");
        assert_eq!(
            (code, again["status"].as_str(), again["state"].as_str()),
            (Some(0), Some("already"), Some("delivered")),
            "{again:#}"
        );
    }
    assert_eq!(fixture.requests("prompted").len(), 1);

    // The sender's history says the same, and the request is still answerable.
    let shown = fixture.tmt(&["x", "show", &request, "--identity", "lead", "--json"]);
    assert!(shown.status.success(), "{shown:?}");
}

fn held_references(fixture: &Fixture, request: &str) -> i64 {
    rusqlite::Connection::open(fixture.state().join("tmt.db"))
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM focus_items WHERE request_id = ?",
            [request],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn send_now_releases_only_that_message_and_the_rest_of_the_batch_stays_held() {
    let (fixture, _) = world("send-now-batch");
    let now = held_request(&fixture, "Do this first");
    let later = held_request(&fixture, "Do this later");
    assert_eq!(
        (
            held_references(&fixture, &now),
            held_references(&fixture, &later)
        ),
        (1, 1)
    );

    let (code, sent) = send_now(&fixture, &now, "lead");
    assert_eq!(
        (code, sent["status"].as_str()),
        (Some(0), Some("sent")),
        "{sent:#}"
    );
    assert_eq!(
        (
            held_references(&fixture, &now),
            held_references(&fixture, &later)
        ),
        (0, 1),
        "the sent message left the hold; the other is untouched"
    );
    let prompted = fixture.requests("prompted");
    assert_eq!(prompted.len(), 1, "{prompted:?}");
    assert!(
        prompted[0]["text"]
            .as_str()
            .unwrap()
            .contains("Do this first")
    );
}

#[test]
fn only_the_originator_or_a_person_may_send_now_and_a_refusal_types_nothing() {
    let (fixture, _) = world("send-now-allowed");
    let request = held_request(&fixture, "Run the migration");

    // A bystander, and the person before anyone marked them, may not.
    for who in ["eve", "ben"] {
        let (code, refused) = send_now(&fixture, &request, who);
        assert_eq!(code, Some(1), "{who}: {refused:#}");
        assert_eq!(
            refused["error"]["code"], "SEND_NOW_NOT_ALLOWED",
            "{refused:#}"
        );
    }
    assert!(fixture.requests("prompted").is_empty());

    let marked = fixture.tmt(&[
        "identity",
        "meta",
        "set",
        "user",
        "true",
        "--identity",
        "ben",
    ]);
    assert!(marked.status.success(), "{marked:?}");
    let (code, sent) = send_now(&fixture, &request, "ben");
    assert_eq!(
        (code, sent["status"].as_str()),
        (Some(0), Some("sent")),
        "{sent:#}"
    );
    assert_eq!(fixture.requests("prompted").len(), 1);
}

#[test]
fn a_recipient_waiting_on_its_user_gets_nothing_typed_and_the_message_stays_sendable() {
    let (fixture, _) = world("send-now-approval");
    let request = held_request(&fixture, "Deploy to staging");
    fs::write(fixture.root.join("driver/agent"), "blocked").unwrap();

    let (code, blocked) = send_now(&fixture, &request, "lead");
    assert_ne!(code, Some(0), "{blocked:#}");
    assert!(
        fixture.requests("prompted").is_empty(),
        "nothing typed past a prompt"
    );

    // Once the agent continues, the same command sends it, and the hold stays released.
    fs::write(fixture.root.join("driver/agent"), "ok").unwrap();
    let (code, sent) = send_now(&fixture, &request, "lead");
    assert_eq!(
        (code, sent["status"].as_str()),
        (Some(0), Some("sent")),
        "{sent:#}"
    );
    assert_eq!(fixture.requests("prompted").len(), 1);
}
