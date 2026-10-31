//! Public conversation request bounds; no process, storage or user identity effects.
use super::request;
use serde_json::json;
const MEMBER: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const REQUEST: &str = "req_22222222-2222-4222-8222-222222222222";

#[test]
fn conversation_scope_is_fixed_and_positions_are_not_reconstructed() {
    let before = json!({"preparedAtMs":123,"requestId":REQUEST});
    let input = json!({"memberId":MEMBER,"before":before,"since":"opaque_Position-1"});
    let encoded = request(&input).unwrap();
    assert_eq!(
        encoded,
        json!({"version":1,"operation":"requests.list","input":{
            "view":"conversation","memberId":MEMBER,"limit":20,"before":before,"since":"opaque_Position-1"
        }})
    );
    assert_eq!(
        input,
        json!({"memberId":MEMBER,"before":before,"since":"opaque_Position-1"})
    );
    for since in [
        json!(null),
        json!(123),
        json!({"not":"a position"}),
        json!(""),
        json!("a.b"),
        json!("x".repeat(65)),
    ] {
        assert!(request(&json!({"memberId":MEMBER,"since":since})).is_err());
    }
}

#[test]
fn conversation_input_cannot_select_a_viewer_or_expand_the_scope() {
    for forbidden in [
        "viewerId",
        "originatorId",
        "identity",
        "roomId",
        "view",
        "recipientId",
    ] {
        let mut input = json!({"memberId":MEMBER});
        input[forbidden] = json!(MEMBER);
        assert!(request(&input).is_err(), "{forbidden}");
    }
    for input in [
        json!({}),
        json!({"memberId":"a display name"}),
        json!({"memberId":MEMBER.to_uppercase()}),
        json!({"memberId":MEMBER,"limit":0}),
        json!({"memberId":MEMBER,"limit":51}),
        json!({"memberId":MEMBER,"limit":1.5}),
        json!({"memberId":MEMBER,"limit":null}),
        json!({"memberId":MEMBER,"before":null}),
        json!({"memberId":MEMBER,"before":{"preparedAtMs":123,"requestId":REQUEST,"viewerId":MEMBER}}),
        json!({"memberId":MEMBER,"before":{"preparedAtMs":-1,"requestId":REQUEST}}),
        json!({"memberId":MEMBER,"before":{"preparedAtMs":9007199254740992u64,"requestId":REQUEST}}),
        json!({"memberId":MEMBER,"before":{"preparedAtMs":123,"requestId":MEMBER}}),
    ] {
        assert!(request(&input).is_err(), "{input}");
    }
    for limit in [1, 50] {
        assert_eq!(
            request(&json!({"memberId":MEMBER,"limit":limit})).unwrap()["input"]["limit"],
            limit
        );
    }
}

fn page() -> serde_json::Value {
    json!({"viewerId":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","member":"active","items":[{
        "requestId":REQUEST,"recipientId":MEMBER,"sender":{"kind":"verified","identityId":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"},
        "kind":"request","origin":"person","preparedAtMs":1,"delivery":"sent",
        "messageState":{"state":"answered","reason":{"code":"answered_by_reply","text":"Answered"},"latestMs":2,"remainingMs":null,"canSendNow":false},
        "final":{"status":"retained","submittedAtMs":2,"bodyBytes":7,"expiresAtMs":3},
        "preview":"question","responsePreview":"answer","previewTruncated":false,"direction":"toMember","changePosition":"older_Z", "responseId":"rsp_4"
    }],"nextBefore":null,"changePosition":"newer_A","changedSince":{"count":1,"more":false}})
}
#[test]
fn conversation_projection_keeps_only_the_viewer_pair_and_published_fields() {
    let query = request(&json!({"memberId":MEMBER,"since":"baseline"})).unwrap();
    let mut raw = page();
    raw["receipt"] = json!("private");
    raw["items"][0]["provider"] = json!("private");
    raw["items"][0]["sender"]["cwd"] = json!("private");
    raw["items"][0]["final"]["response"] = json!("full private final");
    let projected = super::response(&raw, &query).unwrap();
    assert!(!projected.to_string().contains("private"));
    assert_eq!(projected["viewerId"], raw["viewerId"]);
    assert_eq!(projected["changePosition"], "newer_A");
    assert_eq!(projected["items"][0]["changePosition"], "older_Z");
    assert_eq!(projected["items"][0]["responseId"], "rsp_4");
    assert_eq!(projected["changedSince"], json!({"count":1,"more":false}));
}
#[test]
fn malformed_or_cross_pair_conversations_are_unavailable_not_partial() {
    let query = request(&json!({"memberId":MEMBER,"since":"baseline"})).unwrap();
    for path in [
        "/viewerId",
        "/changePosition",
        "/items/0/requestId",
        "/items/0/recipientId",
        "/items/0/sender/identityId",
        "/items/0/direction",
        "/items/0/final/status",
    ] {
        let mut raw = page();
        *raw.pointer_mut(path).unwrap() = json!("invalid!");
        assert_eq!(
            super::response(&raw, &query).unwrap_err().code,
            "REMOTE_CORE_UNAVAILABLE",
            "{path}"
        );
    }
    for replacement in [
        json!({"count":51,"more":true}),
        json!({"count":1,"more":true}),
        json!({"count":0}),
        json!(null),
    ] {
        let mut raw = page();
        raw["changedSince"] = replacement;
        assert!(super::response(&raw, &query).is_err());
    }
    let mut raw = page();
    raw["items"][0]["responsePreview"] = json!("x".repeat(161));
    assert!(super::response(&raw, &query).is_err());
    let mut raw = page();
    raw["items"][0]["roomId"] = json!("private room");
    assert!(super::response(&raw, &query).is_err());
    let mut raw = page();
    raw["items"][0]["final"] = json!({"status":"expired","submittedAtMs":2,"expiresAtMs":3});
    assert!(super::response(&raw, &query).is_err());
    raw["items"][0]["responsePreview"] = json!(null);
    assert!(super::response(&raw, &query).is_ok());
    raw["unknownField"] = json!("x".repeat(65536));
    assert!(super::response(&raw, &query).is_err());
}
#[test]
fn unknown_baselines_and_missing_members_preserve_their_exact_states() {
    let query = request(&json!({"memberId":MEMBER,"since":"foreign"})).unwrap();
    for state in ["unknown", "retired"] {
        let mut raw = page();
        raw["member"] = json!(state);
        raw["items"] = json!([]);
        raw["changedSince"] = json!("unknown");
        let projected = super::response(&raw, &query).unwrap();
        assert_eq!(projected["member"], state);
        assert_eq!(projected["changedSince"], "unknown");
    }
}
