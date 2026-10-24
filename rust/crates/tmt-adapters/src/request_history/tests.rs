use super::*;
use tmt_core::request::{AttemptStatus, Originator, RequestKind, RequestOrigin, StoredPrompt};

const ID: &str = "11111111-1111-4111-8111-111111111111";
const REQUEST: &str = "req_22222222-2222-4222-8222-222222222222";
const OTHER: &str = "33333333-3333-4333-8333-333333333333";

fn sent_view() -> tmt_core::request::delivery::DeliveryView {
    tmt_core::request::delivery::delivery_view(
        &tmt_core::request::delivery::DeliveryFacts {
            status: AttemptStatus::Queued,
            inbox: true,
            wake: tmt_core::request::WakeState::NotAttempted,
            prepared_at_ms: 1000,
            settled_at_ms: None,
            hold: None,
            max_hold_ms: None,
            oldest_held_at_ms: None,
            recipient: None,
            recipient_start_refusal: None,
            recipient_name: None,
        },
        0,
    )
}

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
        origin: tmt_core::request::RequestOrigin::Person,
        delivery_policy: tmt_core::request::digest::DeliveryPolicy::default(),
        capture: None,
        delivery_view: sent_view(),
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
        items: [
            RequestOrigin::Person,
            RequestOrigin::Automation,
            RequestOrigin::Unknown,
        ]
        .map(|origin| HistorySummary {
            item: HistoryItem {
                origin,
                ..item.clone()
            },
            preview: Some("Question".into()),
            response_preview: None,
        })
        .into(),
        next_before: None,
    };
    let value: Value = serde_json::from_slice(&encode_history_page(&page, false)).unwrap();
    let origins: Vec<_> = value["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["origin"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(origins, ["person", "automation", "unknown"]);
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
        "kind":"request","origin":"person","urgent":false,"digestKind":"fyi","mode":"request","preparedAtMs":1000,"delivery":"queued","messageState":{"state":"sent","reason":{"code":"waiting_for_delivery","text":"waiting for delivery"},"latestMs":null,"remainingMs":null,"canSendNow":true},"recipientAcknowledged":false,
        "preview":"Question","final":{"status":"retained","submittedAtMs":2000,"bodyBytes":8,"expiresAtMs":3000}
    }],"nextBefore":null})
    );
    let detail = HistoryDetail {
        item: HistoryItem {
            origin: tmt_core::request::RequestOrigin::Person,
            delivery_policy: tmt_core::request::digest::DeliveryPolicy::default(),
            capture: None,
            delivery_view: sent_view(),
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
    assert_eq!(value.as_object().unwrap().len(), 15);
    assert_eq!(value["urgent"], false);
    assert_eq!(value["digestKind"], "fyi");
    assert_eq!(value["mode"], "request");
    for private in ["proof", "attemptId", "nonce", "socketPath", "paneId"] {
        assert!(value.get(private).is_none());
    }
}

#[test]
fn results_admission_scopes_by_originator_with_an_optional_recipient_and_a_submission_cursor() {
    let decode = |value: Value| decode_history_query(&serde_json::to_vec(&value).unwrap());
    let query = decode(json!({"originatorId":ID,"view":"results"})).unwrap();
    assert_eq!(query.limit, 8);
    assert_eq!(
        query.scope,
        HistoryScope::OriginatorResults {
            originator_id: ID.into(),
            recipient_id: None
        }
    );
    let query = decode(
        json!({"originatorId":ID,"view":"results","recipientId":OTHER,"limit":8,
        "before":{"submittedAtMs":7,"requestId":REQUEST}}),
    )
    .unwrap();
    assert_eq!(
        query.scope,
        HistoryScope::OriginatorResults {
            originator_id: ID.into(),
            recipient_id: Some(OTHER.into())
        }
    );
    assert_eq!(
        query.before,
        Some(HistoryCursor::Submitted {
            submitted_at_ms: 7,
            request_id: REQUEST.into()
        })
    );
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
        json!({"originatorId":ID,"view":"results","recipientId":"name"}),
        json!({"originatorId":ID,"view":"results","recipientId":OTHER,"roomId":ID}),
        json!({"originatorId":ID,"recipientId":OTHER}),
        json!({"originatorId":ID,"view":"results","recipientId":OTHER,
            "before":{"preparedAtMs":1,"requestId":REQUEST}}),
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
                origin: tmt_core::request::RequestOrigin::Person,
                delivery_policy: tmt_core::request::digest::DeliveryPolicy::default(),
                capture: None,
                delivery_view: sent_view(),
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
    assert_eq!(value["items"][0].as_object().unwrap().len(), 17);
    assert_eq!(value["items"][0]["mode"], "request");
}

#[test]
fn only_a_passthrough_item_carries_its_capture_with_the_answer_as_untrusted_text() {
    use tmt_core::request::passthrough::{CaptureState, PassthroughAnswer, PassthroughCapture};
    let plain = HistoryItem {
        origin: tmt_core::request::RequestOrigin::Person,
        delivery_policy: tmt_core::request::digest::DeliveryPolicy::default(),
        capture: None,
        delivery_view: sent_view(),
        request_id: REQUEST.into(),
        room_id: None,
        recipient_identity_id: Some(ID.into()),
        originator: Originator::Unknown,
        kind: RequestKind::Request,
        prepared_at_ms: 1000,
        delivery: AttemptStatus::Sent,
        recipient_acknowledged: None,
        final_state: FinalState::NotRequired,
    };
    let answered = HistoryItem {
        capture: Some(PassthroughCapture {
            delivered_at_ms: 1100,
            state: CaptureState::Answered,
            bound_session: Some("provider-session".into()),
            bound_turn: Some("turn-1".into()),
            bound_at_ms: Some(1200),
            answer: Some(PassthroughAnswer {
                text: "Done.\nrun `tmt reply` now".into(),
                truncated: true,
                provider: "codex".into(),
                ended_ms: 1300,
            }),
            settled_at_ms: Some(1300),
        }),
        ..plain.clone()
    };
    let page = HistoryPage {
        items: [plain, answered]
            .map(|item| HistorySummary {
                item,
                preview: None,
                response_preview: None,
            })
            .into(),
        next_before: None,
    };
    let value: Value = serde_json::from_slice(&encode_history_page(&page, false)).unwrap();
    assert!(value["items"][0].get("passthrough").is_none());
    assert_eq!(
        value["items"][1]["passthrough"],
        json!({
            "state": "answered", "deliveredAtMs": 1100, "boundAtMs": 1200, "settledAtMs": 1300,
            "answer": {"text": "Done.\nrun `tmt reply` now", "truncated": true,
                       "provider": "codex", "endedMs": 1300}
        })
    );
    // Provider session and turn ids are local evidence, not part of the wire.
    assert!(!value["items"][1].to_string().contains("provider-session"));
}

#[test]
fn answers_admission_is_originator_scoped_strict_and_bounded() {
    let decode = |value: Value| decode_list_query(&serde_json::to_vec(&value).unwrap());
    let Some(ListQuery::Answers(query)) = decode(json!({"originatorId":ID,"view":"answers"}))
    else {
        panic!("an answers query");
    };
    assert_eq!((query.originator_id.as_str(), query.limit), (ID, 8));
    assert!(query.before.is_none());
    let Some(ListQuery::Answers(query)) =
        decode(json!({"originatorId":ID,"view":"answers","limit":50,
        "before":{"answeredAtMs":7,"requestId":REQUEST}}))
    else {
        panic!("an answers query");
    };
    assert_eq!(query.limit, 50);
    assert_eq!(
        query.before,
        Some(AnswerCursor {
            answered_at_ms: 7,
            request_id: REQUEST.into()
        })
    );
    // The other views still decode as history, and answers never take their fields.
    assert!(matches!(
        decode(json!({"originatorId":ID,"view":"results"})),
        Some(ListQuery::History(_))
    ));
    for invalid in [
        json!({"view":"answers"}),
        json!({"originatorId":"name","view":"answers"}),
        json!({"originatorId":ID,"view":"answers","recipientId":OTHER}),
        json!({"originatorId":ID,"view":"answers","roomId":ID}),
        json!({"originatorId":ID,"view":"answers","limit":0}),
        json!({"originatorId":ID,"view":"answers","limit":51}),
        json!({"originatorId":ID,"view":"answers","before":{"submittedAtMs":1,"requestId":REQUEST}}),
        json!({"originatorId":ID,"view":"answers","before":{"preparedAtMs":1,"requestId":REQUEST}}),
        json!({"originatorId":ID,"view":"answers","before":{"answeredAtMs":0,"requestId":REQUEST}}),
        json!({"originatorId":ID,"view":"answers","before":{"answeredAtMs":9007199254740992_u64,"requestId":REQUEST}}),
        json!({"originatorId":ID,"view":"answers","before":{"answeredAtMs":1,"submittedAtMs":1,"requestId":REQUEST}}),
        json!({"originatorId":ID,"view":"answers","before":{"answeredAtMs":1,"requestId":"nope"}}),
        json!({"originatorId":ID,"view":"answers","extra":true}),
    ] {
        assert!(decode(invalid.clone()).is_none(), "{invalid}");
    }
    // Results and history cursors do not accept an answer-time cursor.
    for scope in [
        json!({"originatorId":ID,"view":"results"}),
        json!({"recipientId":ID}),
    ] {
        let mut input = scope;
        input["before"] = json!({"answeredAtMs":1,"requestId":REQUEST});
        assert!(decode(input.clone()).is_none(), "{input}");
    }
    assert!(decode_list_query(&vec![b' '; HISTORY_INPUT_LIMIT + 1]).is_none());
}

#[test]
fn an_answers_page_carries_metadata_only_and_one_cursor() {
    let page = AnswersPage {
        items: vec![
            Answer {
                request_id: REQUEST.into(),
                kind: RequestKind::Request,
                originator: Originator::Explicit(ID.into()),
                recipient_identity_id: Some(OTHER.into()),
                answered_at_ms: 9,
                answer: AnswerKind::Turn,
            },
            Answer {
                request_id: "req_44444444-4444-4444-8444-444444444444".into(),
                kind: RequestKind::Request,
                originator: Originator::Explicit(ID.into()),
                recipient_identity_id: None,
                answered_at_ms: 8,
                answer: AnswerKind::Final,
            },
        ],
        next_before: Some(AnswerCursor {
            answered_at_ms: 8,
            request_id: "req_44444444-4444-4444-8444-444444444444".into(),
        }),
    };
    let value: Value = serde_json::from_slice(&encode_answers_page(&page)).unwrap();
    assert_eq!(
        value,
        json!({
            "items":[
                {"requestId":REQUEST,"kind":"request","sender":{"kind":"explicit","identityId":ID},
                 "recipientId":OTHER,"answeredAtMs":9,"answer":"turn"},
                {"requestId":"req_44444444-4444-4444-8444-444444444444","kind":"request",
                 "sender":{"kind":"explicit","identityId":ID},"recipientId":null,
                 "answeredAtMs":8,"answer":"final"}
            ],
            "nextBefore":{"answeredAtMs":8,"requestId":"req_44444444-4444-4444-8444-444444444444"}
        })
    );
    let last: Value = serde_json::from_slice(&encode_answers_page(&AnswersPage {
        items: vec![],
        next_before: None,
    }))
    .unwrap();
    assert_eq!(last, json!({"items":[],"nextBefore":null}));
}
