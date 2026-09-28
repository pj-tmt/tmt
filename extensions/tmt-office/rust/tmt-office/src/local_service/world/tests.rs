use super::super::test_fixture::HttpFixture;
use super::super::tests::{parse_wire, response_value};
use super::*;
use serde_json::Value;

fn request(method: &str, body: Value) -> Request {
    Request {
        method: method.into(),
        path: PATH.into(),
        headers: vec![
            ("Authorization".into(), "Bearer browser".into()),
            ("Origin".into(), "http://127.0.0.1:1234".into()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: if method == "GET" {
            vec![]
        } else {
            serde_json::to_vec(&body).unwrap()
        },
    }
}

fn save(snapshot: &Value) -> Value {
    json!({"expectedRevision":snapshot["revision"], "legacyBasis":snapshot["legacyBasis"], "layout":snapshot["layout"]})
}

fn source() -> Value {
    let maps: Value = serde_json::from_slice(include_bytes!(
        "../../../../../../../contracts/office/map-v1-vectors.json"
    ))
    .unwrap();
    json!({"expectedRevision":0,"legacyBasis":"a".repeat(64),"layout":{"version":1,"map":maps["lobby"],"objects":[]}})
}

#[test]
fn world_read_save_reopen_and_stale_writes_share_one_revision_and_resource_store() {
    let fixture = HttpFixture::new();
    let observed = fixture.call(request("GET", Value::Null));
    assert!(observed.starts_with("HTTP/1.1 200"), "{observed}");
    let preview = response_value(&observed);
    assert_eq!(preview["revision"], 0);
    assert!(preview["worldId"].is_null());
    let observer = rusqlite::Connection::open(&fixture.paths.database).unwrap();
    assert_eq!(
        observer
            .query_row("SELECT count(*) FROM office_local_worlds", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let first = fixture.call(request("PUT", save(&preview)));
    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    let saved = response_value(&first);
    assert_eq!(saved["revision"], 1);
    assert_eq!(saved["changed"], true);
    assert_eq!(saved["layout"], preview["layout"]);
    assert!(saved["legacyBasis"].is_null());
    // A fresh request opens a fresh Storage handle, proving no process-local layout cache.
    let reopened = response_value(&fixture.call(request("GET", Value::Null)));
    assert_eq!(reopened["layout"], saved["layout"]);
    assert_eq!(reopened["worldId"], saved["worldId"]);
    let no_op = response_value(&fixture.call(request("PUT", save(&reopened))));
    assert_eq!(no_op["revision"], 1);
    assert_eq!(no_op["changed"], false);
    assert_eq!(no_op["updatedAtMs"], saved["updatedAtMs"]);
    let stale = fixture.call(request("PUT", save(&preview)));
    assert!(stale.starts_with("HTTP/1.1 409"));
    assert_eq!(response_value(&stale)["error"], "WORLD_REVISION_CONFLICT");
    let (revision, resources): (i64, i64) = observer
        .query_row(
            "SELECT layout_revision, (SELECT count(*) FROM office_whiteboards) +
         (SELECT count(*) FROM office_board_entries) + (SELECT count(*) FROM request_attempts)
         FROM office_local_worlds",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((revision, resources), (1, 0));
}

#[test]
fn concurrent_world_candidates_have_one_winner_and_no_partial_merge() {
    let fixture = HttpFixture::new();
    let preview = response_value(&fixture.call(request("GET", Value::Null)));
    let first = response_value(&fixture.call(request("PUT", save(&preview))));
    assert_eq!(first["layout"]["map"]["version"], 8);
    let candidates = ["Design studio", "Operations studio"].map(|name| {
        let mut candidate = save(&first);
        candidate["layout"]["map"]["modules"][0]["area"]["name"] = json!(name);
        candidate
    });
    let barrier = std::sync::Barrier::new(2);
    let responses = std::thread::scope(|scope| {
        let tasks: Vec<_> = candidates
            .iter()
            .map(|candidate| {
                scope.spawn(|| {
                    barrier.wait();
                    fixture.call(request("PUT", candidate.clone()))
                })
            })
            .collect();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    let winner = responses
        .iter()
        .position(|response| response.starts_with("HTTP/1.1 200"))
        .unwrap();
    let loser = 1 - winner;
    assert!(responses[loser].starts_with("HTTP/1.1 409"));
    assert_eq!(
        response_value(&responses[loser])["error"],
        "WORLD_REVISION_CONFLICT"
    );
    let current = response_value(&fixture.call(request("GET", Value::Null)));
    assert_eq!(current["revision"], 2);
    assert_eq!(current["layout"], candidates[winner]["layout"]);
    assert_eq!(current["worldId"], first["worldId"]);
}

#[test]
fn removed_block_routes_cannot_read_or_write_and_world_reads_require_authority() {
    let fixture = HttpFixture::new();
    let mut anonymous = request("GET", Value::Null);
    anonymous.headers.clear();
    assert!(fixture.call(anonymous).starts_with("HTTP/1.1 401"));
    assert!(!fixture.paths.database.exists());
    for path in [
        "/api/v1/local/blocks",
        "/api/v1/local/blocks/lobby",
        "/api/v1/local/identities/10000000-0000-4000-8000-000000000001/block",
    ] {
        for method in ["GET", "PUT"] {
            let mut input = request(method, source());
            input.path = path.into();
            let response = fixture.call(input);
            assert!(response.starts_with("HTTP/1.1 404"), "{path}: {response}");
            assert_eq!(response_value(&response)["error"], "NOT_FOUND");
            assert!(!fixture.paths.database.exists());
        }
    }
}

#[test]
fn lost_floor_reports_affected_placement_without_partially_saving() {
    let fixture = HttpFixture::new();
    let preview = response_value(&fixture.call(request("GET", Value::Null)));
    let saved = response_value(&fixture.call(request("PUT", save(&preview))));
    let mut invalid = save(&saved);
    let id = invalid["layout"]["objects"][0]["id"].clone();
    invalid["layout"]["objects"][0]["placement"]["x"] = json!(-100);
    let rejected = fixture.call(request("PUT", invalid));
    assert!(rejected.starts_with("HTTP/1.1 400"));
    assert_eq!(
        response_value(&rejected)["issues"],
        json!([{"objectId":id,"reason":"outsideFloor"}])
    );
    let current = response_value(&fixture.call(request("GET", Value::Null)));
    assert_eq!(current["layout"], saved["layout"]);
    assert_eq!(current["revision"], 1);
}

#[test]
fn authority_and_raw_nested_admission_precede_storage() {
    let fixture = HttpFixture::new();
    for (header, value, status) in [
        ("Authorization", "Bearer wrong", 401),
        ("Origin", "http://127.0.0.1:4321", 403),
        ("Content-Type", "text/plain", 403),
    ] {
        let mut input = request("PUT", source());
        input
            .headers
            .iter_mut()
            .find(|(name, _)| name == header)
            .unwrap()
            .1 = value.into();
        assert!(
            fixture
                .call(input)
                .starts_with(&format!("HTTP/1.1 {status}"))
        );
        assert!(!fixture.paths.database.exists());
    }
    let mut unknown = request("PUT", source());
    unknown.path.push_str("/extra");
    assert!(fixture.call(unknown).starts_with("HTTP/1.1 404"));
    assert!(
        fixture
            .call(request("DELETE", Value::Null))
            .starts_with("HTTP/1.1 405")
    );
    let mut duplicate = request("PUT", source());
    duplicate.body = String::from_utf8(duplicate.body)
        .unwrap()
        .replace("\"version\":1", "\"version\":1,\"version\":1")
        .into_bytes();
    assert!(fixture.call(duplicate).starts_with("HTTP/1.1 400"));
    let mut spoofed = source();
    spoofed["actor"] = json!("owner");
    assert!(
        fixture
            .call(request("PUT", spoofed))
            .starts_with("HTTP/1.1 400")
    );
    assert!(!fixture.paths.database.exists());
    let preview = response_value(&fixture.call(request("GET", Value::Null)));
    assert!(
        fixture
            .call(request("PUT", save(&preview)))
            .starts_with("HTTP/1.1 200")
    );
}

#[test]
fn world_body_budget_applies_only_to_its_exact_write_route() {
    let wire = |method: &str, path: &str, length: usize| {
        let mut bytes = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: {length}\r\n\r\n"
        )
        .into_bytes();
        bytes.resize(bytes.len() + length, b' ');
        bytes
    };
    assert!(parse_wire(&wire("PUT", PATH, WORLD_ENVELOPE_LIMIT)).is_ok());
    for (method, path, length) in [
        ("PUT", PATH, WORLD_ENVELOPE_LIMIT + 1),
        ("POST", PATH, WORLD_ENVELOPE_LIMIT),
        ("PUT", "/api/v1/local/world/extra", WORLD_ENVELOPE_LIMIT),
    ] {
        assert!(parse_wire(&wire(method, path, length)).is_err());
    }
}
