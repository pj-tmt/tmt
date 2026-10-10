use super::*;
use tmt_core::request::{AttemptStatus, Originator, RequestKind, StoredPrompt};

const ID: &str = "11111111-1111-4111-8111-111111111111";
const REQUEST: &str = "req_22222222-2222-4222-8222-222222222222";

#[test]
fn history_admission_is_scoped_strict_and_bounded() {
    for scope in [
        json!({"recipientId":ID}),
        json!({"roomId":ID}),
        json!({"recipientId":ID,"roomId":ID}),
    ] {
        let query = decode_history_query(&serde_json::to_vec(&scope).unwrap()).unwrap();
        assert_eq!(query.limit, 20);
        assert_eq!(query.before, None);
    }
    let input =
        json!({"recipientId":ID,"limit":50,"before":{"preparedAtMs":1,"requestId":REQUEST}});
    assert_eq!(
        decode_history_query(&serde_json::to_vec(&input).unwrap())
            .unwrap()
            .before
            .unwrap()
            .request_id(),
        REQUEST
    );
    for invalid in [
        json!({}),
        json!({"recipientId":"Alice"}),
        json!({"roomId":""}),
        json!({"recipientId":ID,"limit":0}),
        json!({"recipientId":ID,"limit":51}),
        json!({"recipientId":ID,"unknown":true}),
        json!({"recipientId":ID,"before":{"preparedAtMs":0,"requestId":REQUEST}}),
        json!({"recipientId":ID,"before":{"preparedAtMs":9007199254740992_u64,"requestId":REQUEST}}),
        json!({"recipientId":ID,"before":{"preparedAtMs":1,"requestId":"bad"}}),
    ] {
        assert!(
            decode_history_query(&serde_json::to_vec(&invalid).unwrap()).is_none(),
            "{invalid}"
        );
    }
    assert!(
        decode_history_query(
            format!("{{\"recipientId\":\"{ID}\",\"recipientId\":\"{ID}\"}}").as_bytes()
        )
        .is_none()
    );
    assert_eq!(
        decode_history_request(&serde_json::to_vec(&json!({"requestId":REQUEST})).unwrap())
            .as_deref(),
        Some(REQUEST)
    );
    assert!(
        decode_history_request(
            &serde_json::to_vec(&json!({"requestId":REQUEST,"proof":"forged"})).unwrap()
        )
        .is_none()
    );
    assert!(decode_history_request(&vec![b' '; HISTORY_INPUT_LIMIT + 1]).is_none());
}

#[test]
fn history_wire_contains_only_public_metadata_and_exact_detail_text() {
    let item = HistoryItem {
        delivery_policy: tmt_core::request::digest::DeliveryPolicy::default(),
        request_id: REQUEST.into(),
        room_id: None,
        recipient_identity_id: Some(ID.into()),
        originator: Originator::Unknown,
        kind: RequestKind::Request,
        prepared_at_ms: 1000,
        delivery: AttemptStatus::Queued,
        recipient_acknowledged: Some(false),
        final_state: FinalState::Retained {
            content: (),
            submitted_at_ms: 2000,
            body_bytes: 8,
            expires_at_ms: 3000,
        },
    };
    let page = HistoryPage {
        items: vec![HistorySummary {
            item,
            preview: Some("Question".into()),
            response_preview: None,
        }],
        next_before: None,
    };
    let value: Value = serde_json::from_slice(&encode_history_page(&page, false)).unwrap();
    assert_eq!(
        value,
        json!({"items":[{
        "requestId":REQUEST,"roomId":null,"recipientId":ID,"sender":{"kind":"unknown","identityId":null},
        "kind":"request","urgent":false,"digestKind":"fyi","preparedAtMs":1000,"delivery":"queued","recipientAcknowledged":false,
        "preview":"Question","final":{"status":"retained","submittedAtMs":2000,"bodyBytes":8,"expiresAtMs":3000}
    }],"nextBefore":null})
    );
    let detail = HistoryDetail {
        item: HistoryItem {
            delivery_policy: tmt_core::request::digest::DeliveryPolicy::default(),
            request_id: REQUEST.into(),
            room_id: None,
            recipient_identity_id: Some(ID.into()),
            originator: Originator::Explicit(ID.into()),
            kind: RequestKind::Request,
            prepared_at_ms: 1000,
            delivery: AttemptStatus::Sent,
            recipient_acknowledged: None,
            final_state: FinalState::Retained {
                content: "  answer\n".into(),
                submitted_at_ms: 2000,
                body_bytes: 9,
                expires_at_ms: 3000,
            },
        },
        prompt: RequestPrompt::Retained(StoredPrompt {
            message: "Question".into(),
            message_bytes: 8,
            expires_at_ms: 3000,
        }),
    };
    let value: Value = serde_json::from_slice(&encode_history_detail(&detail)).unwrap();
    assert_eq!(
        value["prompt"],
        json!({"status":"retained","message":"Question","messageBytes":8,"expiresAtMs":3000})
    );
    assert_eq!(value["final"]["response"], "  answer\n");
    assert_eq!(value["recipientAcknowledged"], Value::Null);
    assert_eq!(value.as_object().unwrap().len(), 12);
    assert_eq!(value["urgent"], false);
    assert_eq!(value["digestKind"], "fyi");
    for private in ["proof", "attemptId", "nonce", "socketPath", "paneId"] {
        assert!(value.get(private).is_none());
    }
}

#[test]
fn results_admission_uses_exclusive_originator_scope_and_submission_cursor() {
    let decode = |value: Value| decode_history_query(&serde_json::to_vec(&value).unwrap());
    let query = decode(json!({"originatorId":ID,"view":"results"})).unwrap();
    assert_eq!(query.limit, 8);
    assert_eq!(query.scope, HistoryScope::OriginatorResults(ID.into()));
    let query = decode(json!({"originatorId":ID,"view":"results","limit":50,
        "before":{"submittedAtMs":7,"requestId":REQUEST}}))
    .unwrap();
    assert_eq!(
        query.before,
        Some(HistoryCursor::Submitted {
            submitted_at_ms: 7,
            request_id: REQUEST.into()
        })
    );
    for invalid in [
        json!({"originatorId":ID}),
        json!({"view":"results"}),
        json!({"originatorId":"name","view":"results"}),
        json!({"originatorId":ID,"view":"history"}),
        json!({"originatorId":ID,"view":"results","roomId":ID}),
        json!({"originatorId":ID,"view":"results","recipientId":ID}),
        json!({"originatorId":ID,"view":"results","limit":0}),
        json!({"originatorId":ID,"view":"results","limit":51}),
        json!({"originatorId":ID,"view":"results","before":{"preparedAtMs":1,"requestId":REQUEST}}),
        json!({"originatorId":ID,"view":"results","before":{"submittedAtMs":0,"requestId":REQUEST}}),
        json!({"originatorId":ID,"view":"results","before":{"submittedAtMs":9007199254740992_u64,"requestId":REQUEST}}),
        json!({"originatorId":ID,"view":"results","before":{"submittedAtMs":1,"preparedAtMs":1,"requestId":REQUEST}}),
        json!({"recipientId":ID,"before":{"submittedAtMs":1,"requestId":REQUEST}}),
    ] {
        assert!(decode(invalid.clone()).is_none(), "{invalid}");
    }
    assert!(decode_history_query(&vec![b' '; HISTORY_INPUT_LIMIT + 1]).is_none());
}

#[test]
fn results_wire_adds_only_preview_metadata_and_submission_cursor() {
    let page = HistoryPage {
        items: vec![HistorySummary {
            item: HistoryItem {
                delivery_policy: tmt_core::request::digest::DeliveryPolicy::default(),
                request_id: REQUEST.into(),
                room_id: None,
                recipient_identity_id: Some(ID.into()),
                originator: Originator::Explicit(ID.into()),
                kind: RequestKind::Request,
                prepared_at_ms: 1,
                delivery: AttemptStatus::Queued,
                recipient_acknowledged: Some(false),
                final_state: FinalState::Expired {
                    submitted_at_ms: 2,
                    expires_at_ms: 3,
                },
            },
            preview: None,
            response_preview: None,
        }],
        next_before: Some(HistoryCursor::Submitted {
            submitted_at_ms: 2,
            request_id: REQUEST.into(),
        }),
    };
    let value: Value = serde_json::from_slice(&encode_history_page(&page, true)).unwrap();
    assert_eq!(
        value["nextBefore"],
        json!({"submittedAtMs":2,"requestId":REQUEST})
    );
    assert_eq!(value["items"][0]["responsePreview"], Value::Null);
    assert_eq!(value["items"][0]["previewTruncated"], false);
    assert_eq!(value["items"][0]["kind"], "request");
    assert_eq!(
        value["items"][0]["final"],
        json!({"status":"expired","submittedAtMs":2,"expiresAtMs":3})
    );
    assert_eq!(value["items"][0].as_object().unwrap().len(), 14);
}
