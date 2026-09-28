use super::super::test_fixture::HttpFixture as Fixture;
use super::super::tests::{parse_wire, response_value, test_receipt};
use super::*;
use serde_json::{Value, json};
use tmt_adapters::office_whiteboard::document::SAVE_INPUT_LIMIT;

mod snapshots;

const OPERATION: &str = "11111111-1111-4111-8111-111111111111";
const NEXT_OPERATION: &str = "22222222-2222-4222-8222-222222222222";

fn scene() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../../../../../contracts/office/whiteboard-scene-v1.json"
    ))
    .unwrap()
}
fn request(method: &str, id: &str, body: Value) -> Request {
    Request {
        method: method.into(),
        path: format!("/api/v1/local/whiteboards/{id}"),
        headers: vec![
            ("Authorization".into(), "Bearer browser".into()),
            ("Origin".into(), "http://127.0.0.1:1234".into()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: if method == "GET" {
            Vec::new()
        } else {
            serde_json::to_vec(&body).unwrap()
        },
    }
}
fn save(revision: u64, operation: &str) -> Request {
    request(
        "PUT",
        "lobby",
        json!({"expectedRevision":revision,"operationId":operation,"scene":scene()}),
    )
}

#[test]
fn browser_whiteboard_save_reopen_replay_and_conflicts_preserve_the_document() {
    let fixture = Fixture::new();
    let absent = fixture.call(request("GET", "lobby", Value::Null));
    assert!(absent.starts_with("HTTP/1.1 200"));
    assert_eq!(response_value(&absent)["revision"], 0);
    let connection = rusqlite::Connection::open(&fixture.paths.database).unwrap();
    let worlds: i64 = connection
        .query_row("SELECT count(*) FROM office_local_worlds", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(worlds, 0);
    connection.close().unwrap();

    let first = fixture.call(save(0, OPERATION));
    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    let original = response_value(&first);
    assert_eq!(original["revision"], 1);
    assert_eq!(original["changed"], true);
    let shown = fixture.call(request("GET", "lobby", Value::Null));
    assert_eq!(response_value(&shown)["scene"], scene());
    assert_eq!(
        response_value(&shown)["updatedAtMs"],
        original["updatedAtMs"]
    );
    let no_op = fixture.call(save(1, NEXT_OPERATION));
    assert_eq!(response_value(&no_op)["changed"], false);
    assert_eq!(response_value(&no_op)["revision"], 1);

    let mut edited = scene();
    edited["background"] = json!("#eeeeee");
    let changed = fixture.call(request("PUT", "lobby", json!({"expectedRevision":1,"operationId":"33333333-3333-4333-8333-333333333333","scene":edited})));
    assert_eq!(response_value(&changed)["revision"], 2);
    assert_eq!(response_value(&fixture.call(save(0, OPERATION))), original);
    for (operation, code) in [
        (OPERATION, "WHITEBOARD_IDEMPOTENCY_CONFLICT"),
        (
            "44444444-4444-4444-8444-444444444444",
            "WHITEBOARD_REVISION_CONFLICT",
        ),
    ] {
        let conflict = fixture.call(save(1, operation));
        assert!(conflict.starts_with("HTTP/1.1 409"));
        assert_eq!(response_value(&conflict)["error"], code);
    }
    let latest = response_value(&fixture.call(request("GET", "lobby", Value::Null)));
    assert_eq!(latest["revision"], 2);
    assert_eq!(latest["scene"], edited);
}

#[test]
fn invalid_authority_path_method_and_payload_cannot_open_storage() {
    let fixture = Fixture::new();
    for (header, value, expected) in [
        ("Authorization", "Bearer wrong", 401),
        ("Origin", "http://127.0.0.1:4321", 403),
        ("Content-Type", "text/plain", 403),
    ] {
        let mut input = save(0, OPERATION);
        input
            .headers
            .iter_mut()
            .find(|(name, _)| name == header)
            .unwrap()
            .1 = value.into();
        assert!(
            fixture
                .call(input)
                .starts_with(&format!("HTTP/1.1 {expected}"))
        );
        assert!(!fixture.paths.database.exists());
    }
    for id in ["../secret", "lobby/extra", "lobby%2fextra", "", "not-an-id"] {
        assert!(
            fixture
                .call(request("GET", id, Value::Null))
                .starts_with("HTTP/1.1 404")
        );
    }
    assert!(
        fixture
            .call(request("DELETE", "lobby", Value::Null))
            .starts_with("HTTP/1.1 405")
    );
    for body in [
        json!({"expectedRevision":0,"operationId":OPERATION,"scene":scene(),"actor":"alice"}),
        json!({"expectedRevision":0,"operationId":OPERATION,"scene":{"formatVersion":1}}),
    ] {
        assert!(
            fixture
                .call(request("PUT", "lobby", body))
                .starts_with("HTTP/1.1 400")
        );
    }
    let mut duplicate = save(0, OPERATION);
    duplicate.body = String::from_utf8(duplicate.body)
        .unwrap()
        .replacen(
            "\"formatVersion\":1",
            "\"formatVersion\":1,\"formatVersion\":1",
            1,
        )
        .into_bytes();
    assert!(fixture.call(duplicate).starts_with("HTTP/1.1 400"));
    assert!(!fixture.paths.database.exists());
    // Positive control: identical credentials and otherwise valid request creates a document.
    assert!(fixture.call(save(0, OPERATION)).starts_with("HTTP/1.1 200"));
    assert!(fixture.paths.database.exists());
    let blank = fixture.call(request("GET", NEXT_OPERATION, Value::Null));
    assert!(blank.starts_with("HTTP/1.1 200"));
    let document = response_value(&blank);
    assert_eq!(document["id"], NEXT_OPERATION);
    assert_eq!(document["revision"], 0);
    assert_eq!(document["updatedAtMs"], 0);
    assert_eq!(document["scene"]["elements"], json!([]));
}

#[test]
fn only_exact_whiteboard_put_admits_the_scene_envelope_budget() {
    let wire = |method: &str, path: &str, length: usize| {
        let mut bytes = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: {length}\r\n\r\n"
        )
        .into_bytes();
        bytes.resize(bytes.len() + length, b' ');
        bytes
    };
    assert!(
        parse_wire(&wire(
            "PUT",
            "/api/v1/local/whiteboards/lobby",
            SAVE_INPUT_LIMIT
        ))
        .is_ok()
    );
    assert!(
        parse_wire(&wire(
            "PUT",
            "/api/v1/local/whiteboards/lobby",
            SAVE_INPUT_LIMIT + 1
        ))
        .is_err()
    );
    for (method, path) in [
        ("POST", "/api/v1/local/whiteboards/lobby"),
        ("PUT", "/api/v1/local/whiteboards/lobby/extra"),
        ("PUT", "/api/v1/local/whiteboards/not-an-id"),
        ("PUT", "/api/v1/local/profiles"),
    ] {
        assert!(parse_wire(&wire(method, path, super::super::BODY_LIMIT + 1)).is_err());
    }
}
