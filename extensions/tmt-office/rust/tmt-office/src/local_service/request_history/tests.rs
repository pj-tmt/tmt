use super::super::{
    test_fixture::HttpFixture,
    tests::{parse_wire, response_value},
};
use super::*;
use serde_json::{Value, json};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{ResponseProof, SubmitResponse, correlation},
};

const OPERATION: &str = "11111111-1111-4111-8111-111111111111";
const ID: &str = "22222222-2222-4222-8222-222222222222";

fn request(path: &str, body: Value) -> Request {
    Request {
        method: "POST".into(),
        path: path.into(),
        headers: vec![
            ("Authorization".into(), "Bearer browser".into()),
            ("Origin".into(), "http://127.0.0.1:1234".into()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: serde_json::to_vec(&body).unwrap(),
    }
}

#[test]
fn owner_history_admission_precedes_storage_and_rejects_unbounded_or_ambiguous_inputs() {
    let fixture = HttpFixture::new();
    for (path, body) in [
        (LIST, json!({"recipientId":ID})),
        (SHOW, json!({"requestId":format!("req_{ID}")})),
        (RECEIPT, json!({"operationId":OPERATION})),
    ] {
        for header in ["Authorization", "Origin", "Content-Type"] {
            let mut denied = request(path, body.clone());
            denied.headers.retain(|(key, _)| key != header);
            let reply = fixture.call(denied);
            let status = if header == "Authorization" {
                "401"
            } else {
                "403"
            };
            assert!(reply.starts_with(&format!("HTTP/1.1 {status}")), "{reply}");
        }
        let mut unknown = body.clone();
        unknown["actor"] = json!("forged");
        assert!(
            fixture
                .call(request(path, unknown))
                .starts_with("HTTP/1.1 400")
        );
        let mut get = request(path, body);
        get.method = "GET".into();
        assert!(fixture.call(get).starts_with("HTTP/1.1 405"));
    }
    assert!(
        fixture
            .call(request(LIST, json!({})))
            .starts_with("HTTP/1.1 400")
    );
    assert!(
        fixture
            .call(request(LIST, json!({"recipientId":ID,"limit":51})))
            .starts_with("HTTP/1.1 400")
    );
    assert!(!fixture.paths.database.exists());
    let positive = fixture.call(request(LIST, json!({"recipientId":ID})));
    assert!(positive.starts_with("HTTP/1.1 200"), "{positive}");
    assert_eq!(
        response_value(&positive),
        json!({"items":[],"nextBefore":null})
    );
    let unknown = fixture.call(request(SHOW, json!({"requestId":format!("req_{ID}")})));
    assert_eq!(
        response_value(&unknown),
        json!({"error":"REQUEST_NOT_FOUND"})
    );
    assert!(unknown.starts_with("HTTP/1.1 404"));
}

#[test]
fn browser_dispatch_native_reply_and_reopened_history_share_canonical_requests() {
    let fixture = HttpFixture::new();
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let recipient = create_or_resolve(&mut storage, "Reviewer", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    storage.close().unwrap();
    let accepted = fixture.call(request(
        super::super::dispatch::PATH,
        json!({"operationId":OPERATION,"recipientIds":[recipient],"message":"  Please review\n"}),
    ));
    assert!(accepted.starts_with("HTTP/1.1 200"), "{accepted}");
    let receipt = response_value(&accepted);
    let request_id = receipt["items"][0]["requestId"].as_str().unwrap();
    let pending = fixture.call(request(SHOW, json!({"requestId":request_id})));
    assert_eq!(response_value(&pending)["final"]["status"], "not_submitted");
    assert_eq!(response_value(&pending)["recipientAcknowledged"], false);
    let recovered = fixture.call(request(RECEIPT, json!({"operationId":OPERATION})));
    assert_eq!(
        receipt["wake"],
        json!({"status":"unavailable","paneAttempted":false,"agentProcessed":null})
    );
    let mut immutable_receipt = receipt.clone();
    immutable_receipt.as_object_mut().unwrap().remove("wake");
    assert_eq!(response_value(&recovered), immutable_receipt);

    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let mut service = RequestService::new(&mut storage, wall_time_ms);
    let inbox = service.list_incoming(&recipient, None, None, None).unwrap();
    assert_eq!(inbox.items.len(), 1);
    let context = service.get_context(request_id).unwrap().unwrap();
    service
        .acknowledge_incoming_request(&recipient, request_id, inbox.items[0].exchange.revision)
        .unwrap();
    service
        .submit_response(SubmitResponse {
            request_id: request_id.into(),
            proof: ResponseProof::Compact(correlation::response_token(
                request_id,
                &context.attempt.attempt_id,
                &context.attempt.route,
            )),
            body: "  Reviewed\nLooks good.\n".into(),
        })
        .unwrap();
    storage.close().unwrap();

    // Each HTTP call opens canonical SQLite anew; no browser/process-local cache supplies the reply.
    let history = fixture.call(request(LIST, json!({"recipientId":recipient})));
    assert!(history.starts_with("HTTP/1.1 200"), "{history}");
    let page = response_value(&history);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["requestId"], request_id);
    assert_eq!(page["items"][0]["recipientAcknowledged"], true);
    assert_eq!(page["items"][0]["final"]["status"], "retained");
    assert!(page["items"][0]["final"].get("response").is_none());
    let detail = fixture.call(request(SHOW, json!({"requestId":request_id})));
    assert!(detail.starts_with("HTTP/1.1 200"), "{detail}");
    let detail = response_value(&detail);
    assert_eq!(detail["prompt"]["message"], "  Please review\n");
    assert_eq!(detail["final"]["response"], "  Reviewed\nLooks good.\n");
    for secret in ["proof", "attemptId", "nonce", "socketPath", "paneId"] {
        assert!(detail.get(secret).is_none());
    }
}

#[test]
fn history_routes_have_small_exact_body_budgets() {
    let limit = tmt_adapters::request_history::HISTORY_INPUT_LIMIT;
    for path in [LIST, SHOW, RECEIPT] {
        let wire = |length| {
            let mut bytes = format!(
                "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: {length}\r\n\r\n"
            )
            .into_bytes();
            bytes.resize(bytes.len() + length, b' ');
            bytes
        };
        assert!(parse_wire(&wire(limit)).is_ok());
        assert!(parse_wire(&wire(limit + 1)).is_err());
    }
    assert!(!handles("/api/v1/local/requests/list/extra"));
    assert!(!handles("/api/v1/local/dispatch"));
}
