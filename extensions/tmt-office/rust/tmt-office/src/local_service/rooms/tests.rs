use super::super::{test_fixture::HttpFixture, tests::response_value};
use super::*;
use serde_json::json;
use tmt_core::identity::{Lifetime, create_or_resolve};

const ROOM: &str = "11111111-1111-4111-8111-111111111111";

#[test]
fn retirement_requires_write_authority_and_revision_then_replays_without_recreating_room() {
    let fixture = HttpFixture::new();
    let path = format!("{PATH}/{ROOM}/retire");
    let input = json!({"expectedRevision":1});
    assert_eq!(
        input_limit("POST", &path),
        Some(tmt_adapters::room::INPUT_LIMIT)
    );
    let mut forbidden = request("POST", path.clone(), input.clone());
    forbidden.headers.retain(|(key, _)| key != "Origin");
    assert!(fixture.call(forbidden).starts_with("HTTP/1.1 403"));
    assert!(
        fixture
            .call(request(
                "POST",
                path.clone(),
                json!({"expectedRevision":1,"force":true})
            ))
            .starts_with("HTTP/1.1 400")
    );
    assert!(!fixture.paths.database.exists());
    let target = format!("{PATH}/{ROOM}");
    assert!(
        fixture
            .call(request(
                "PUT",
                target.clone(),
                json!({"expectedRevision":0,"name":"Design","memberIds":[]})
            ))
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        fixture
            .call(request("POST", path.clone(), json!({"expectedRevision":2})))
            .starts_with("HTTP/1.1 409")
    );
    let retired = fixture.call(request("POST", path.clone(), input.clone()));
    assert!(retired.starts_with("HTTP/1.1 200"));
    assert_eq!(
        response_value(&retired),
        json!({"id":ROOM,"name":"Design","revision":2,"retired":true,"memberIds":[]})
    );
    assert_eq!(
        response_value(&fixture.call(request("POST", path, input))),
        response_value(&retired)
    );
    assert_eq!(
        response_value(&fixture.call(request("GET", PATH.into(), json!(null)))),
        json!([])
    );
    let recreate = fixture.call(request(
        "PUT",
        target,
        json!({"expectedRevision":0,"name":"Replacement","memberIds":[]}),
    ));
    assert!(recreate.starts_with("HTTP/1.1 409"));
    assert_eq!(response_value(&recreate), json!({"error":"ROOM_RETIRED"}));
}
fn request(method: &str, path: String, body: serde_json::Value) -> Request {
    Request {
        method: method.into(),
        path,
        headers: vec![
            ("Authorization".into(), "Bearer browser".into()),
            ("Origin".into(), "http://127.0.0.1:1234".into()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: serde_json::to_vec(&body).unwrap(),
    }
}

#[test]
fn room_admission_preserves_authority_and_rejects_unsupported_fields_before_storage() {
    let fixture = HttpFixture::new();
    let body = json!({"expectedRevision":0,"name":"Design","memberIds":[]});
    let target = format!("{PATH}/{ROOM}");
    let mut no_auth = request("PUT", target.clone(), body.clone());
    no_auth.headers.retain(|(name, _)| name != "Authorization");
    assert!(fixture.call(no_auth).starts_with("HTTP/1.1 401"));
    let mut no_origin = request("PUT", target.clone(), body.clone());
    no_origin.headers.retain(|(name, _)| name != "Origin");
    assert!(fixture.call(no_origin).starts_with("HTTP/1.1 403"));
    let mut invalid = body.clone();
    invalid["members"] = json!("everyone");
    assert!(
        fixture
            .call(request("PUT", target.clone(), invalid))
            .starts_with("HTTP/1.1 400")
    );
    assert!(
        fixture
            .call(request("PUT", format!("{target}/extra"), body.clone()))
            .starts_with("HTTP/1.1 404")
    );
    assert!(!fixture.paths.database.exists());
    assert!(
        fixture
            .call(request("PUT", target.clone(), body.clone()))
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        fixture
            .call(request("PUT", target, body))
            .starts_with("HTTP/1.1 409")
    );
    let listed = fixture.call(request("GET", PATH.into(), json!(null)));
    assert_eq!(
        response_value(&listed),
        json!([{"id":ROOM,"name":"Design","revision":1,"retired":false,"memberIds":[]}])
    );
}

#[test]
fn room_dispatch_checks_current_membership_under_the_shared_owner_route() {
    let fixture = HttpFixture::new();
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let alice = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
        .unwrap()
        .identity
        .id;
    storage.close().unwrap();
    let target = format!("{PATH}/{ROOM}");
    let saved = fixture.call(request(
        "PUT",
        target.clone(),
        json!({"expectedRevision":0,"name":"Design","memberIds":[alice]}),
    ));
    assert!(saved.starts_with("HTTP/1.1 200"));
    let input = json!({"operationId":"22222222-2222-4222-8222-222222222222","recipientIds":[alice],"message":"Review","room":{"kind":"roster","roomId":ROOM,"revision":1}});
    assert!(
        fixture
            .call(request(
                "PUT",
                target,
                json!({"expectedRevision":1,"name":"Design","memberIds":[]})
            ))
            .starts_with("HTTP/1.1 200")
    );
    let rejected = fixture.call(request("POST", super::super::dispatch::PATH.into(), input));
    assert!(rejected.starts_with("HTTP/1.1 409"));
    assert_eq!(
        response_value(&rejected),
        json!({"error":"ROOM_ROSTER_CHANGED"})
    );
}
