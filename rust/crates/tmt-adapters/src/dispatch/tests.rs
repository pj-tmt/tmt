use super::*;

#[test]
fn receipt_lookup_admits_only_one_bounded_operation_id() {
    let id = "11111111-1111-4111-8111-111111111111";
    assert_eq!(
        decode_dispatch_lookup(format!("{{\"operationId\":\"{id}\"}}").as_bytes()).as_deref(),
        Some(id)
    );
    for input in ["{}", "{\"operationId\":\"bad\"}", "{\"operationId\":null}"] {
        assert!(decode_dispatch_lookup(input.as_bytes()).is_none());
    }
    assert!(
        decode_dispatch_lookup(
            format!("{{\"operationId\":\"{id}\",\"message\":\"new send\"}}").as_bytes()
        )
        .is_none()
    );
    assert!(decode_dispatch_lookup(&vec![b' '; 257]).is_none());
}
use serde_json::json;

#[test]
fn advisory_wake_does_not_change_the_stored_receipt_envelope() {
    use tmt_core::dispatch::{Acceptance, DispatchItem, DispatchReceipt};
    let receipt = DispatchReceipt {
        operation_id: "11111111-1111-4111-8111-111111111111".into(),
        created_at_ms: 1,
        items: vec![DispatchItem {
            recipient_id: "22222222-2222-4222-8222-222222222222".into(),
            request_id: "req_33333333-3333-4333-8333-333333333333".into(),
            acceptance: Acceptance::Queued,
        }],
    };
    let original = encode_receipt(&receipt);
    let shown: serde_json::Value = serde_json::from_slice(&encode_receipt_with_wake(
        &receipt,
        Some(WakeState::Claimed),
    ))
    .unwrap();
    assert_eq!(
        shown["wake"],
        json!({"status":"unknown","paneAttempted":null,"agentProcessed":null})
    );
    assert_eq!(decode_receipt(&original), Some(receipt.clone()));
    assert!(decode_receipt(&encode_receipt_with_wake(&receipt, Some(WakeState::Sent))).is_none());
    assert_eq!(encode_receipt(&receipt), original);
}

fn valid() -> serde_json::Value {
    json!({"operationId":"11111111-1111-4111-8111-111111111111","recipientIds":["22222222-2222-4222-8222-222222222222"],"message":"Review this."})
}

#[test]
fn dispatch_admission_rejects_unknown_fields_invalid_ids_and_empty_audiences() {
    assert!(decode_input(&serde_json::to_vec(&valid()).unwrap()).is_some());
    for (key, value) in [
        ("recipientIds", json!([])),
        ("recipientIds", json!(["alice"])),
        ("operationId", json!("00000000-0000-0000-0000-000000000000")),
        ("message", json!("  \n")),
        ("command", json!("rm -rf /")),
        (
            "originator",
            json!({"kind":"explicit","identityId":"22222222-2222-4222-8222-222222222222"}),
        ),
        ("kind", json!("broadcast")),
        ("kind", json!(null)),
    ] {
        let mut value_under_test = valid();
        value_under_test[key] = value;
        assert!(
            decode_input(&serde_json::to_vec(&value_under_test).unwrap()).is_none(),
            "{key}"
        );
    }
    let duplicate = br#"{"operationId":"11111111-1111-4111-8111-111111111111","recipientIds":[],"recipientIds":["22222222-2222-4222-8222-222222222222"],"message":"hello"}"#;
    assert!(decode_input(duplicate).is_none());
}

#[test]
fn trusted_sender_is_part_of_intent_but_not_owner_json_authority() {
    use tmt_core::request::Originator;
    let owner = decode_input(&serde_json::to_vec(&valid()).unwrap()).unwrap();
    assert_eq!(owner.originator, Originator::Unknown);
    let mut cli = owner.clone();
    cli.originator = Originator::Explicit("22222222-2222-4222-8222-222222222222".into());
    assert_ne!(intent_digest(&owner), intent_digest(&cli));
    let mut verified = cli.clone();
    verified.originator = Originator::Verified("22222222-2222-4222-8222-222222222222".into());
    assert_ne!(intent_digest(&cli), intent_digest(&verified));
    cli.originator = Originator::Explicit("Alice".into());
    assert!(cli.normalize().is_none());
}

#[test]
fn announcement_kind_is_explicit_and_request_default_preserves_old_replay() {
    let mut wire = valid();
    let original = decode_input(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_eq!(original.kind, RequestKind::Request);
    wire["kind"] = json!("request");
    let explicit = decode_input(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_eq!(intent_digest(&original), intent_digest(&explicit));
    wire["kind"] = json!("announcement");
    let announcement = decode_input(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_eq!(announcement.kind, RequestKind::Announcement);
    assert_ne!(intent_digest(&original), intent_digest(&announcement));
    let duplicate = serde_json::to_string(&wire).unwrap().replacen(
        "\"kind\":\"announcement\"",
        "\"kind\":\"announcement\",\"kind\":\"request\"",
        1,
    );
    assert!(decode_input(duplicate.as_bytes()).is_none());
}

#[test]
fn dispatch_preserves_exact_text_and_normalizes_only_recipient_selection() {
    let mut wire = valid();
    wire["message"] = json!("  hello\r\n!\0");
    wire["recipientIds"] = json!([
        "33333333-3333-4333-8333-333333333333",
        "22222222-2222-4222-8222-222222222222",
        "33333333-3333-4333-8333-333333333333"
    ]);
    let input = decode_input(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_eq!(input.message, "  hello\r\n!\0");
    assert_eq!(input.recipient_ids.len(), 2);
    assert!(input.recipient_ids[0] < input.recipient_ids[1]);
    let mut another = input.clone();
    another.recipient_ids.reverse();
    assert_eq!(
        intent_digest(&input),
        intent_digest(&another.normalize().unwrap())
    );
}

#[test]
fn room_modes_are_strict_and_scope_changes_the_intent_digest() {
    let input = decode_input(&serde_json::to_vec(&valid()).unwrap()).unwrap();
    let previous = br#"{"operationId":"11111111-1111-4111-8111-111111111111","recipientIds":["22222222-2222-4222-8222-222222222222"],"message":"Review this."}"#;
    assert_eq!(
        intent_digest(&input),
        tmt_core::content_digest::framed_sha256(b"tmt:office:dispatch:v1\0", previous)
    );
    let mut wire = valid();
    wire["room"] =
        json!({"kind":"roster","roomId":"33333333-3333-4333-8333-333333333333","revision":1});
    let with_room = decode_input(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_ne!(intent_digest(&input), intent_digest(&with_room));
    wire["room"] = json!({"kind":"direct","roomId":"33333333-3333-4333-8333-333333333333"});
    let direct = decode_input(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_ne!(intent_digest(&input), intent_digest(&direct));
    assert_ne!(intent_digest(&with_room), intent_digest(&direct));
    wire["recipientIds"] = json!([
        "22222222-2222-4222-8222-222222222222",
        "44444444-4444-4444-8444-444444444444"
    ]);
    assert!(decode_input(&serde_json::to_vec(&wire).unwrap()).is_none());
    wire["recipientIds"] = valid()["recipientIds"].clone();
    for room in [
        json!({"kind":"roster","roomId":"Design","revision":1}),
        json!({"kind":"roster","roomId":"33333333-3333-4333-8333-333333333333","revision":0}),
        json!({"kind":"roster","roomId":"33333333-3333-4333-8333-333333333333","revision":1,"all":true}),
        json!({"roomId":"33333333-3333-4333-8333-333333333333","revision":1}),
        json!({"kind":"direct","roomId":"33333333-3333-4333-8333-333333333333","revision":1}),
        json!({"kind":"roster","roomId":"33333333-3333-4333-8333-333333333333"}),
        json!({"kind":"other","roomId":"33333333-3333-4333-8333-333333333333"}),
    ] {
        wire["room"] = room;
        assert!(decode_input(&serde_json::to_vec(&wire).unwrap()).is_none());
    }
    wire["room"] = json!(null);
    assert_eq!(
        intent_digest(&decode_input(&serde_json::to_vec(&wire).unwrap()).unwrap()),
        intent_digest(&input)
    );
}

#[test]
fn full_exact_text_and_recipient_limits_fit_their_json_envelopes() {
    let mut wire = valid();
    wire["message"] = json!("\0".repeat(MAX_EXCHANGE_TEXT_BYTES));
    wire["recipientIds"] = json!(
        (0..MAX_RECIPIENTS)
            .map(|n| format!("22222222-2222-4222-8222-{n:012x}"))
            .collect::<Vec<_>>()
    );
    let encoded = serde_json::to_vec(&wire).unwrap();
    assert!(encoded.len() <= INPUT_LIMIT);
    let input = decode_input(&encoded).unwrap();
    let receipt = DispatchReceipt {
        operation_id: input.operation_id,
        created_at_ms: 1,
        items: input
            .recipient_ids
            .into_iter()
            .map(|recipient_id| DispatchItem {
                request_id: format!("req_{recipient_id}"),
                recipient_id,
                acceptance: Acceptance::RecipientUnavailable,
            })
            .collect(),
    };
    assert_eq!(decode_receipt(&encode_receipt(&receipt)), Some(receipt));
    wire["message"] = json!("\0".repeat(MAX_EXCHANGE_TEXT_BYTES + 1));
    assert!(decode_input(&serde_json::to_vec(&wire).unwrap()).is_none());
    wire["message"] = json!("valid text");
    wire["recipientIds"]
        .as_array_mut()
        .unwrap()
        .push(json!("33333333-3333-4333-8333-333333333333"));
    assert!(decode_input(&serde_json::to_vec(&wire).unwrap()).is_none());
}
