//! Cloud transport bounds are derived from the existing Firestore document owner.
use tmt_remote::{cloud_inbox, firestore_limits, limits};

#[test]
fn cloud_envelope_cap_is_the_largest_whose_carrier_fits_one_document() {
    let n = limits::CLOUD_ENVELOPE_BYTES;
    let bound =
        |n: usize| 4 * (n + 16).div_ceil(3) + 4 * 65_usize.div_ceil(3) + limits::METADATA_BYTES;
    assert!(bound(n) <= firestore_limits::DOCUMENT_PAYLOAD_BYTES as usize);
    assert!(bound(n + 1) > firestore_limits::DOCUMENT_PAYLOAD_BYTES as usize);
    assert!(cloud_inbox::check_envelope_size(n).is_ok());
    assert_eq!(
        cloud_inbox::check_envelope_size(n + 1).unwrap_err().code,
        "REMOTE_CLOUD_MESSAGE_TOO_LARGE"
    );
}

#[test]
fn checkpoint_keeps_bounded_overlap_and_never_moves_backwards() {
    let mut checkpoint = cloud_inbox::Checkpoint::new(200_000).unwrap();
    assert_eq!(
        checkpoint.query_from_ms(),
        200_000 - limits::CLOUD_CHECKPOINT_OVERLAP_MS
    );
    checkpoint.completed(190_000).unwrap();
    assert_eq!(checkpoint.created_at_ms(), 200_000);
    checkpoint.completed(210_000).unwrap();
    assert_eq!(checkpoint.created_at_ms(), 210_000);
    assert_eq!(cloud_inbox::Checkpoint::new(1).unwrap().query_from_ms(), 0);
    assert!(cloud_inbox::Checkpoint::new(u64::MAX).is_err());
}

#[test]
fn offline_delivery_uses_server_create_time_and_expires_at_journal_retention() {
    let created = 1_800_000_000_000;
    let later = created + 20 * tmt_remote::journal::DAY;
    assert!(cloud_inbox::check_freshness(created, created, later).is_ok());
    assert!(cloud_inbox::check_freshness(created + 60_000, created, later).is_ok());
    assert_eq!(
        cloud_inbox::check_freshness(created + 60_001, created, later)
            .unwrap_err()
            .code,
        "REMOTE_CLOUD_TIMESTAMP_INVALID"
    );
    assert_eq!(
        cloud_inbox::check_freshness(created, created, created + tmt_remote::journal::RECOVERY)
            .unwrap_err()
            .code,
        "REMOTE_CLOUD_MESSAGE_EXPIRED"
    );
}

#[test]
fn cloud_polling_never_inherits_idle_jitter_above_sixty_seconds() {
    let mut cadence = tmt_remote::native_sync::PollCadence::default();
    assert_eq!(
        tmt_remote::cloud_inbox::poll_delay(&cadence, 0, |max| max),
        60_000
    );
    cadence.activity(1000);
    assert_eq!(
        tmt_remote::cloud_inbox::poll_delay(&cadence, 1000, |_| 0),
        5000
    );
    for _ in 0..30 {
        cadence.failed();
    }
    assert_eq!(
        tmt_remote::cloud_inbox::poll_delay(&cadence, 86_400_000, |max| max),
        60_000
    );
    assert_eq!(3 * 86_400_000u64 / 60_000, 4320);
}

#[test]
fn local_reply_capability_is_strictly_admitted_without_entering_the_dispatch() {
    use serde_json::json;
    use tmt_remote::cloud_command::Request;
    let request_id = "44444444-4444-4444-8444-444444444444";
    let receipt = "v2_PRIVATE_REPLY_CANARY";
    let value = json!({"version":1,"remote":"team","createdAtMs":1800000000000_u64,
        "requestId":request_id,"from":{"id":"33333333-3333-4333-8333-333333333333","name":"sender"},
        "target":{"name":"target"},"message":"hello",
        "reply":{"requestId":format!("req_{request_id}"),"receipt":receipt}});
    let request = Request::read(value.to_string().as_bytes()).unwrap();
    assert_eq!(request.reply.as_ref().unwrap().receipt, receipt);
    assert!(request.dispatch().get("reply").is_none());
    assert!(!request.dispatch().to_string().contains(receipt));
    for reply in [
        json!({"requestId":"req_other","receipt":receipt}),
        json!({"requestId":format!("req_{request_id}"),"receipt":""}),
        json!({"requestId":format!("req_{request_id}"),"receipt":"v2_PRIVATE_REPLY_CANARY\n"}),
        json!({"requestId":format!("req_{request_id}"),"receipt":false}),
        json!({"requestId":format!("req_{request_id}"),"receipt":receipt,"extra":true}),
        json!([]),
    ] {
        let mut invalid = value.clone();
        invalid["reply"] = reply;
        let error = Request::read(invalid.to_string().as_bytes()).err().unwrap();
        assert_eq!(error.code, "REMOTE_INPUT_INVALID");
        assert!(!error.json_value().to_string().contains(receipt));
    }
    let mut oversized = value;
    oversized["reply"]["receipt"] = json!("x".repeat(limits::CLOUD_COMMAND_BYTES));
    assert_eq!(
        Request::read(oversized.to_string().as_bytes())
            .err()
            .unwrap()
            .code,
        "REMOTE_CLOUD_MESSAGE_TOO_LARGE"
    );
}

#[test]
fn public_message_schema_is_bounded_strict_and_freezes_exact_core_request_bytes() {
    use tmt_remote::cloud_command::Request;
    let value = serde_json::json!({"version":1,"remote":null,"createdAtMs":1800000000000_u64,"requestId":"44444444-4444-4444-8444-444444444444","from":{"id":"33333333-3333-4333-8333-333333333333","name":"sender"},"target":{"id":"66666666-6666-4666-8666-666666666666","name":"target"},"message":"hello"});
    let exact = serde_json::to_vec(&value).unwrap();
    let request = Request::read(&exact).unwrap();
    assert_eq!(request.dispatch()["input"]["expectedName"], "target");
    let padded = [b" ".as_slice(), exact.as_slice()].concat();
    assert_ne!(
        request.dispatch()["requestDigest"],
        Request::read(&padded).unwrap().dispatch()["requestDigest"],
        "same ID with different command bytes must conflict"
    );
    let mut bad = value.clone();
    bad.as_object_mut().unwrap().remove("remote");
    assert!(Request::read(&serde_json::to_vec(&bad).unwrap()).is_err());
    let mut bad = value.clone();
    bad["version"] = serde_json::json!(2);
    assert_eq!(
        Request::read(&serde_json::to_vec(&bad).unwrap())
            .err()
            .unwrap()
            .code,
        "REMOTE_CLOUD_UPGRADE_REQUIRED"
    );
    let mut bad = value.clone();
    bad["message"] = serde_json::json!("x".repeat(limits::CLOUD_MESSAGE_BYTES + 1));
    assert_eq!(
        Request::read(&serde_json::to_vec(&bad).unwrap())
            .err()
            .unwrap()
            .code,
        "REMOTE_CLOUD_MESSAGE_TOO_LARGE"
    );
    let mut bad = value;
    bad["role"] = serde_json::json!("owner");
    assert!(Request::read(&serde_json::to_vec(&bad).unwrap()).is_err());
}
