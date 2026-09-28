use super::super::test_fixture::HttpFixture as Fixture;
use super::super::tests::{parse_wire, response_value, test_receipt};
use super::*;
use serde_json::{Value, json};
use tmt_core::{
    identity::{Lifetime, create_or_resolve},
    request::{Originator, RequestService},
};

fn request(body: Value) -> Request {
    Request {
        method: "POST".into(),
        path: PATH.into(),
        headers: vec![
            ("Authorization".into(), "Bearer browser".into()),
            ("Origin".into(), "http://127.0.0.1:1234".into()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: serde_json::to_vec(&body).unwrap(),
    }
}
fn input(recipient: &str) -> Value {
    json!({"operationId":"11111111-1111-4111-8111-111111111111","recipientIds":[recipient],"message":"Please review the snapshot."})
}

#[test]
fn direct_room_http_enqueues_one_member_and_reports_membership_rejection_without_a_receipt() {
    use tmt_core::room::{MembershipChange, RoomRepository, RoomWrite};
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let alice = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    let bob = create_or_resolve(&mut storage, "Bob", Lifetime::Saved)
        .unwrap()
        .identity;
    let room_id = "33333333-3333-4333-8333-333333333333";
    storage
        .save_meeting_room(
            room_id,
            RoomWrite {
                expected_revision: 0,
                name: "Design".into(),
                member_ids: vec![alice.id.clone(), bob.id.clone()],
            },
        )
        .unwrap();
    storage.close().unwrap();
    let mut body = input(&alice.id);
    body["room"] = json!({"kind":"direct","roomId":room_id});
    let response = fixture.call(request(body.clone()));
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let receipt = response_value(&response);
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let mut service = RequestService::new(&mut storage, wall_time_ms);
    assert_eq!(
        service
            .list_incoming(&alice.id, Some(room_id), None, None)
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(
        service
            .list_incoming(&bob.id, Some(room_id), None, None)
            .unwrap()
            .items
            .is_empty()
    );
    storage
        .change_meeting_membership(room_id, &alice.id, MembershipChange::Leave)
        .unwrap();
    storage.close().unwrap();
    let replay = response_value(&fixture.call(request(body.clone())));
    assert_eq!(replay["items"], receipt["items"]);
    assert_eq!(replay["operationId"], receipt["operationId"]);
    assert_eq!(replay["createdAtMs"], receipt["createdAtMs"]);
    assert!(replay.get("wake").is_none());
    let rejected_id = "44444444-4444-4444-8444-444444444444";
    body["operationId"] = json!(rejected_id);
    let rejected = fixture.call(request(body));
    assert!(rejected.starts_with("HTTP/1.1 409"), "{rejected}");
    assert_eq!(
        response_value(&rejected),
        json!({"error":"ROOM_RECIPIENT_NOT_MEMBER"})
    );
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    assert_eq!(storage.dispatch_receipt(rejected_id).unwrap(), None);
    storage.close().unwrap();
}

#[test]
fn one_member_roster_and_announcement_stay_inbox_only() {
    use tmt_core::room::{RoomRepository, RoomWrite};
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let alice = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity;
    let room_id = "33333333-3333-4333-8333-333333333333";
    storage
        .save_meeting_room(
            room_id,
            RoomWrite {
                expected_revision: 0,
                name: "Design".into(),
                member_ids: vec![alice.id.clone()],
            },
        )
        .unwrap();
    storage.close().unwrap();
    let mut roster = input(&alice.id);
    roster["room"] = json!({"kind":"roster","roomId":room_id,"revision":1});
    let accepted = response_value(&fixture.call(request(roster)));
    assert_eq!(accepted["items"][0]["acceptance"], "queued");
    assert!(accepted.get("wake").is_none());
    let mut announcement = input(&alice.id);
    announcement["operationId"] = json!("44444444-4444-4444-8444-444444444444");
    announcement["kind"] = json!("announcement");
    let accepted = response_value(&fixture.call(request(announcement)));
    assert_eq!(accepted["items"][0]["acceptance"], "queued");
    assert!(accepted.get("wake").is_none());
}

#[test]
fn admission_rejects_missing_auth_origin_and_unknown_fields_before_storage() {
    let fixture = Fixture::new();
    let valid = input("22222222-2222-4222-8222-222222222222");
    let mut no_auth = request(valid.clone());
    no_auth.headers.retain(|(key, _)| key != "Authorization");
    assert!(fixture.call(no_auth).starts_with("HTTP/1.1 401"));
    let mut no_origin = request(valid.clone());
    no_origin.headers.retain(|(key, _)| key != "Origin");
    assert!(fixture.call(no_origin).starts_with("HTTP/1.1 403"));
    let mut invalid = valid;
    invalid["actor"] = json!("Alice");
    assert!(fixture.call(request(invalid)).starts_with("HTTP/1.1 400"));
    assert!(!fixture.paths.database.exists());
    // Same valid authority can reach the service; negative cases are not a dead route.
    let response = fixture.call(request(input("22222222-2222-4222-8222-222222222222")));
    assert!(response.starts_with("HTTP/1.1 200"));
    assert_eq!(
        response_value(&response)["items"][0]["acceptance"],
        "recipientUnavailable"
    );
}

#[test]
fn owner_dispatch_and_retry_use_the_existing_inbox_without_an_identity_or_pane() {
    let fixture = Fixture::new();
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let receiver = create_or_resolve(&mut storage, "Reviewer", Lifetime::Saved)
        .unwrap()
        .identity;
    storage.close().unwrap();
    let body = input(&receiver.id);
    let response = fixture.call(request(body.clone()));
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let receipt = response_value(&response);
    assert_eq!(receipt["items"][0]["acceptance"], "queued");
    assert_eq!(
        receipt["wake"],
        json!({"status":"unavailable","paneAttempted":false,"agentProcessed":null})
    );
    let replay = response_value(&fixture.call(request(body)));
    assert_eq!(replay["items"], receipt["items"]);
    assert_eq!(replay["operationId"], receipt["operationId"]);
    assert_eq!(replay["createdAtMs"], receipt["createdAtMs"]);
    assert!(replay.get("wake").is_none());
    let mut changed = input(&receiver.id);
    changed["message"] = json!("Different request");
    assert!(fixture.call(request(changed)).starts_with("HTTP/1.1 409"));
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let mut service = RequestService::new(&mut storage, wall_time_ms);
    let incoming = service
        .list_incoming(&receiver.id, None, None, None)
        .unwrap();
    assert_eq!(incoming.items.len(), 1);
    let context = service
        .get_context(receipt["items"][0]["requestId"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(context.attempt.originator, Originator::Unknown);
    assert!(matches!(
        context.attempt.route,
        tmt_core::request::RequestRoute::Inbox { .. }
    ));
    assert!(
        matches!(context.prompt,tmt_core::request::RequestPrompt::Retained(prompt) if prompt.message == "Please review the snapshot.")
    );
    storage.close().unwrap();
}

#[test]
fn large_dispatch_budget_applies_only_to_the_exact_post_route() {
    let wire = |method: &str, path: &str, length: usize| {
        let mut bytes = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: {length}\r\n\r\n"
        )
        .into_bytes();
        bytes.resize(bytes.len() + length, b' ');
        bytes
    };
    let limit = tmt_adapters::dispatch::INPUT_LIMIT;
    assert!(parse_wire(&wire("POST", PATH, limit)).is_ok());
    assert!(parse_wire(&wire("POST", PATH, limit + 1)).is_err());
    for (method, path) in [
        ("GET", PATH),
        ("POST", "/api/v1/local/dispatch/extra"),
        ("POST", "/api/v1/local/profiles"),
    ] {
        assert!(parse_wire(&wire(method, path, super::super::BODY_LIMIT + 1)).is_err());
    }
}

#[test]
fn oversized_http_body_returns_shared_framing_error_before_dispatch_storage() {
    use std::{
        collections::HashMap,
        io::{Read, Write},
        net::{Ipv4Addr, TcpListener},
        sync::{Arc, Mutex, atomic::AtomicBool},
    };
    let fixture = Fixture::new();
    let exchange = |wire: Vec<u8>| {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(super::super::RESPONSE_DEADLINE))
                .unwrap();
            stream
                .set_write_timeout(Some(super::super::REQUEST_DEADLINE))
                .unwrap();
            stream.write_all(&wire).unwrap();
            let mut bytes = String::new();
            stream.read_to_string(&mut bytes).unwrap();
            bytes
        });
        let (mut stream, _) = listener.accept().unwrap();
        super::super::serve_connection(
            &mut stream,
            &fixture.paths,
            &test_receipt(),
            &AtomicBool::new(false),
            &Arc::new(Mutex::new(HashMap::new())),
        )
        .unwrap();
        drop(stream);
        client.join().unwrap()
    };
    let over_limit = tmt_adapters::dispatch::INPUT_LIMIT + 1;
    // Header rejection does not need to buffer a body the endpoint cannot admit.
    let wire = format!(
        "POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nAuthorization: Bearer browser\r\nOrigin: http://127.0.0.1:1234\r\nContent-Type: application/json\r\nContent-Length: {over_limit}\r\n\r\n"
    )
    .into_bytes();
    let rejected = exchange(wire);
    assert!(rejected.starts_with("HTTP/1.1 400"));
    assert_eq!(response_value(&rejected), json!({"error":"BAD_REQUEST"}));
    assert!(!fixture.paths.database.exists());
    let body = serde_json::to_vec(&input("22222222-2222-4222-8222-222222222222")).unwrap();
    let mut valid = format!("POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nAuthorization: Bearer browser\r\nOrigin: http://127.0.0.1:1234\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",body.len()).into_bytes();
    valid.extend(body);
    let accepted = exchange(valid);
    assert!(accepted.starts_with("HTTP/1.1 200"));
    assert_eq!(
        response_value(&accepted)["items"][0]["acceptance"],
        "recipientUnavailable"
    );
}
