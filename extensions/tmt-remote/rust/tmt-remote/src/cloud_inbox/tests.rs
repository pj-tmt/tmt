use crate::{
    state::Layout,
    store::{Grant, Store, uuid_v4},
};
use ed25519_dalek::SigningKey;
const PEER: &str = "33333333-3333-4333-8333-333333333333";
const ID: &str = "44444444-4444-4444-8444-444444444444";

#[test]
fn pre_cloud_store_migrates_without_changing_grants_then_recovers_frozen_bytes() {
    let root = std::env::temp_dir().join(format!("tmt-cloud-inbox-{}", uuid_v4().unwrap()));
    let layout = Layout::open(&root).unwrap();
    let lease = layout.serve_lock().unwrap();
    let mut store = Store::open(&lease).unwrap();
    store
        .insert_grant(&Grant {
            client_id: PEER.into(),
            public_key: SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Native peer".into(),
            agents: "all".into(),
            scopes: vec!["talk".into()],
            mode: "hold".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        })
        .unwrap();
    // Exact previous schema history, with its existing identity/grant kept in place.
    let machine = store.machine().unwrap();
    let grants = store.grants().unwrap();
    store.connection.execute_batch("DROP TABLE cloud_transport; DELETE FROM _migrations WHERE version=10 AND name='cloud_transport';").unwrap();
    drop(store);
    let mut store = Store::open(&lease).unwrap();
    assert_eq!(store.machine().unwrap(), machine);
    assert_eq!(store.grants().unwrap(), grants);
    assert_eq!(
        store
            .cloud_begin(PEER, ID, b"original ciphertext", 1000)
            .unwrap(),
        super::Replay::Fresh
    );
    drop(store);
    let mut store = Store::open(&lease).unwrap();
    assert_eq!(
        store
            .cloud_begin(PEER, ID, b"original ciphertext", 1000)
            .unwrap(),
        super::Replay::Pending
    );
    assert_eq!(
        store
            .cloud_begin(PEER, ID, b"different ciphertext", 1000)
            .unwrap_err()
            .code,
        "REMOTE_CLOUD_CONFLICT"
    );
    store
        .cloud_freeze(PEER, ID, b"exact encrypted response")
        .unwrap();
    assert!(
        store
            .cloud_freeze(PEER, ID, b"new ciphertext under reused context")
            .is_err()
    );
    assert!(store.cloud_claim_publication(PEER, ID).unwrap());
    assert!(
        !store.cloud_claim_publication(PEER, ID).unwrap(),
        "an ambiguous publication must be observed, never repeated"
    );
    store.cloud_complete(&[PEER.into()], 2000).unwrap();
    store.cloud_complete(&[PEER.into()], 1000).unwrap();
    drop(store);
    let store = Store::open(&lease).unwrap();
    assert_eq!(store.cloud_checkpoint(PEER).unwrap().created_at_ms(), 2000);
    let mut store = store;
    assert_eq!(
        store
            .cloud_begin(PEER, ID, b"original ciphertext", 1000)
            .unwrap(),
        super::Replay::Frozen(b"exact encrypted response".to_vec())
    );
    assert!(!store.cloud_claim_publication(PEER, ID).unwrap());
    drop(store);
    drop(lease);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn entropy_failure_refuses_before_signing_and_each_seal_has_a_fresh_seed() {
    use crate::cloud_hpke::{Descriptor, Pin, Sender};
    use ed25519_dalek::Signer;
    let f: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/cloud-hpke-v1.json")).unwrap();
    let auth = SigningKey::from_bytes(&[7; 32]);
    let sender = SigningKey::from_bytes(&[8; 32]);
    let now = 1_800_000_000_000;
    let desc = Descriptor::verify(
        f["descriptor"].as_str().unwrap().as_bytes(),
        Pin {
            machine_id: "11111111-1111-4111-8111-111111111111",
            authentication_key: &auth.verifying_key().to_bytes(),
        },
        now,
        None,
    )
    .unwrap();
    let public = sender.verifying_key().to_bytes();
    let peer = Sender {
        client_id: PEER,
        authentication_key: &public,
    };
    let exact = f["request"].as_str().unwrap().as_bytes();
    let failed = super::seal_with_entropy(
        exact,
        &desc,
        now,
        peer,
        |_| panic!("entropy failure must precede signing"),
        |_| Err(()),
    );
    assert_eq!(failed.err().unwrap().code, "REMOTE_ENTROPY");
    let a = super::seal_with_entropy(
        exact,
        &desc,
        now,
        peer,
        |b| sender.sign(b).to_bytes(),
        |seed| {
            seed.fill(42);
            Ok(())
        },
    )
    .unwrap()
    .0;
    let b = super::seal_with_entropy(
        exact,
        &desc,
        now,
        peer,
        |b| sender.sign(b).to_bytes(),
        |seed| {
            seed.fill(43);
            Ok(())
        },
    )
    .unwrap()
    .0;
    assert_ne!(a, b, "independent seed per seal, no process-global RNG");
}

#[test]
fn provider_document_uses_output_create_time_and_exact_peer_namespace() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    let root = "projects/demo-cloud/databases/(default)/documents";
    let machine = "11111111-1111-4111-8111-111111111111";
    let row = json!({"name":format!("{root}/m/{machine}/peers/{PEER}/inbox/{ID}"),"createTime":"2026-10-23T04:00:00.123456789Z","fields":{"peerId":{"stringValue":PEER},"carrier":{"bytesValue":STANDARD.encode(b"ciphertext")},"createdAt":{"timestampValue":"2026-10-23T04:00:00.123Z"}}});
    let document = super::document::Document::read(&row, root, machine, "inbox").unwrap();
    assert_eq!(document.created_ms(), 1_792_728_000_123);
    assert_eq!(document.bytes(), b"ciphertext");
    for field in ["name", "createTime", "fields"] {
        let mut bad = row.clone();
        bad.as_object_mut().unwrap().remove(field);
        assert!(super::document::Document::read(&bad, root, machine, "inbox").is_err());
    }
    let mut missing_peer = row.clone();
    let fields = missing_peer["fields"].as_object_mut().unwrap();
    fields.remove("peerId");
    fields.insert("unexpected".into(), json!({"stringValue": PEER}));
    assert!(super::document::Document::read(&missing_peer, root, machine, "inbox").is_err());
    let mut bad = row.clone();
    bad["fields"]["createdAt"]["timestampValue"] = json!("2026-10-23T04:00:05Z");
    assert!(super::document::Document::read(&bad, root, machine, "inbox").is_err());
    assert!(super::document::Document::read(&row, root, machine, "responses").is_err());
    for time in [
        "2026-02-30T00:00:00Z",
        "2026-10-23T24:00:00Z",
        "2026-10-23T04:00:00+00:00",
    ] {
        assert!(super::document::timestamp(time).is_err());
    }
}

#[test]
fn encrypted_offline_delivery_uses_common_hold_and_survives_new_door_window() {
    use super::{document::Document, ingress::Receiver};
    use crate::{
        canonical::{self, Envelope},
        cloud_hpke::Sender,
        cloud_keys::CloudKeys,
        core::CoreClient,
        message::MessageService,
        operations::Operations,
        session::DoorSessions,
        state::MachineKey,
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ed25519_dalek::Signer;
    use serde_json::json;
    use std::sync::{Arc, Mutex, atomic::AtomicBool};
    let machine = "11111111-1111-4111-8111-111111111111";
    let root = std::env::temp_dir().join(format!("tmt-cloud-ingress-{}", uuid_v4().unwrap()));
    let layout = Layout::open(&root).unwrap();
    let lease = layout.serve_lock().unwrap();
    let mut store = Store::open(&lease).unwrap();
    let sender = SigningKey::from_bytes(&[8; 32]);
    let now = crate::pairing::now_ms().unwrap();
    let grant = Grant {
        client_id: PEER.into(),
        public_key: sender.verifying_key().to_bytes(),
        kind: "cli".into(),
        origin: "cli".into(),
        name: "Peer".into(),
        agents: "all".into(),
        scopes: vec!["talk".into()],
        mode: "hold".into(),
        issued_at_ms: now,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    };
    store.insert_grant(&grant).unwrap();
    let store = Arc::new(Mutex::new(store));
    let keys = Arc::new(CloudKeys::new(
        layout.clone(),
        machine.into(),
        MachineKey::open(&layout).unwrap(),
    ));
    keys.maintain(now).unwrap();
    let descriptor = keys.advertised(now).unwrap();
    let window = "22222222-2222-4222-8222-222222222222";
    let session = "55555555-5555-4555-8555-555555555555";
    let payload=json!({"version":1,"operation":"dispatch.create","originator":"anonymous","createdAtMs":now,"input":{"operationId":ID,"recipientIds":["66666666-6666-4666-8666-666666666666"],"message":"offline request","kind":"request","expectedName":"recipient"}}).to_string();
    let envelope = Envelope {
        kind: "request",
        id: ID,
        correlation_id: None,
        machine_id: machine,
        window_id: window,
        client_id: PEER,
        session_id: session,
        sequence: "1",
        timestamp_ms: now,
        origin: "cli",
        operation: "dispatch.create",
        payload: payload.as_bytes(),
    };
    let exact=json!({"version":1,"profile":"local-v1","kind":"request","id":ID,"correlationId":null,"machineId":machine,"windowId":window,"clientId":PEER,"sessionId":session,"sequence":"1","timestampMs":now,"origin":"cli","operation":"dispatch.create","payload":canonical::base64url(payload.as_bytes()),"signature":canonical::base64url(&sender.sign(&canonical::envelope(&envelope).unwrap()).to_bytes())}).to_string();
    let (carrier, opener) = super::seal_native(
        exact.as_bytes(),
        &descriptor,
        now,
        Sender {
            client_id: PEER,
            authentication_key: &grant.public_key,
        },
        |b| sender.sign(b).to_bytes(),
    )
    .unwrap();
    let provider_root = "projects/demo-cloud/databases/(default)/documents";
    let stamp = format!(
        "{}T{:02}:{:02}:{:02}.{:03}Z",
        crate::calendar::utc_date(now),
        now / 3_600_000 % 24,
        now / 60_000 % 60,
        now / 1000 % 60,
        now % 1000
    );
    let value = json!({"name":format!("{provider_root}/m/{machine}/peers/{PEER}/inbox/{ID}"),"createTime":stamp,"fields":{"peerId":{"stringValue":PEER},"carrier":{"bytesValue":STANDARD.encode(&carrier)},"createdAt":{"timestampValue":stamp}}});
    let document = Document::read(&value, provider_root, machine, "inbox").unwrap();
    let fresh = |fresh_id: &str, created: u64| {
        let mut fresh_payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        fresh_payload["input"]["operationId"] = json!(fresh_id);
        fresh_payload["createdAtMs"] = json!(created);
        let fresh_payload = fresh_payload.to_string();
        let fresh_envelope = Envelope {
            id: fresh_id,
            payload: fresh_payload.as_bytes(),
            ..envelope
        };
        let mut fresh_exact: serde_json::Value = serde_json::from_str(&exact).unwrap();
        fresh_exact["id"] = json!(fresh_id);
        fresh_exact["payload"] = json!(canonical::base64url(fresh_payload.as_bytes()));
        fresh_exact["signature"] = json!(canonical::base64url(
            &sender
                .sign(&canonical::envelope(&fresh_envelope).unwrap())
                .to_bytes()
        ));
        let (fresh_carrier, fresh_opener) = super::seal_native(
            fresh_exact.to_string().as_bytes(),
            &descriptor,
            now,
            Sender {
                client_id: PEER,
                authentication_key: &grant.public_key,
            },
            |bytes| sender.sign(bytes).to_bytes(),
        )
        .unwrap();
        let mut fresh_doc = value.clone();
        fresh_doc["name"] = json!(format!(
            "{provider_root}/m/{machine}/peers/{PEER}/inbox/{fresh_id}"
        ));
        fresh_doc["fields"]["carrier"] = json!({"bytesValue":STANDARD.encode(fresh_carrier)});
        (
            Document::read(&fresh_doc, provider_root, machine, "inbox").unwrap(),
            fresh_opener,
        )
    };
    let build = |window: &str| {
        let sessions = Arc::new(DoorSessions::new(
            machine.into(),
            window.into(),
            "http://127.0.0.1:1".into(),
            "/r/test/x/".into(),
            MachineKey::open(&layout).unwrap(),
            Arc::clone(&store),
            crate::session::IDLE,
        ));
        // Hold mode invokes no Core command. Any accidental effect hits an absent executable.
        let ops = Arc::new(Operations::new(
            CoreClient::at(root.join("absent-core")).unwrap(),
            Arc::new(AtomicBool::new(false)),
            crate::limits::CLOUD_ENVELOPE_BYTES,
        ));
        let service = Arc::new(
            MessageService::new(Arc::clone(&sessions), crate::limits::CLOUD_ENVELOPE_BYTES)
                .with_operations(ops),
        );
        Receiver::new(sessions, Arc::clone(&keys), service)
    };
    let first = build(window);
    let response = first
        .receive(&document, now + 3 * 86_400_000)
        .unwrap()
        .unwrap();
    let opened = opener
        .open(&response.bytes, crate::limits::CLOUD_ENVELOPE_BYTES)
        .unwrap();
    let opened =
        crate::wire::SignedMessage::decode_any(&opened, crate::limits::CLOUD_ENVELOPE_BYTES)
            .unwrap();
    assert_eq!(opened.input["state"], "held");
    assert_eq!(opened.envelope().window_id, window);
    drop(first);
    let restarted = build("77777777-7777-4777-8777-777777777777");
    assert_eq!(
        restarted
            .receive(&document, now + 3 * 86_400_000)
            .unwrap()
            .unwrap()
            .bytes,
        response.bytes,
        "reopened driver returns exact frozen ciphertext, never reseals"
    );
    // The provider and envelope timestamps are fresh, but Core's original request is not.
    let expired_id = "99999999-9999-4999-8999-999999999999";
    let (expired_doc, expired_opener) = fresh(expired_id, now - crate::journal::RECOVERY);
    let expired = restarted.receive(&expired_doc, now).unwrap().unwrap();
    let plain = expired_opener
        .open(&expired.bytes, crate::limits::CLOUD_ENVELOPE_BYTES)
        .unwrap();
    let signed =
        crate::wire::SignedMessage::decode_any(&plain, crate::limits::CLOUD_ENVELOPE_BYTES)
            .unwrap();
    assert_eq!(
        signed.input["error"]["code"],
        "REMOTE_CLOUD_MESSAGE_EXPIRED"
    );
    assert!(
        store
            .lock()
            .unwrap()
            .owned(&grant, expired_id, now)
            .is_err(),
        "expired immutable Core request is never adopted"
    );
    let mut guard = store.lock().unwrap();
    assert_eq!(guard.owned(&grant, ID, now).unwrap().phase, "held");
    guard.revoke(PEER).unwrap();
    drop(guard);
    assert_eq!(
        response.claim_publication().unwrap_err().code,
        "REMOTE_CLOSED",
        "revocation between receipt and HTTP prevents response publication"
    );
    assert!(
        restarted
            .receive(&document, now + 3 * 86_400_000)
            .unwrap()
            .is_none(),
        "an already-frozen accepted response must not be resealed under a revoked context"
    );
    // A fresh, valid encrypted carrier can receive a signed refusal after revoke. This
    // consumes only its own response context; it never changes a previously frozen reply.
    let fresh_id = "88888888-8888-4888-8888-888888888888";
    let (fresh_doc, fresh_opener) = fresh(fresh_id, now);
    let refused = restarted
        .receive(&fresh_doc, now + 3 * 86_400_000)
        .unwrap()
        .unwrap();
    let plain = fresh_opener
        .open(&refused.bytes, crate::limits::CLOUD_ENVELOPE_BYTES)
        .unwrap();
    let signed =
        crate::wire::SignedMessage::decode_any(&plain, crate::limits::CLOUD_ENVELOPE_BYTES)
            .unwrap();
    assert_eq!(signed.input["error"]["code"], "REMOTE_CLOSED");
    assert!(
        refused.claim_publication().unwrap(),
        "signed refusal publication grants no operation access"
    );
    assert_eq!(store.lock().unwrap().connection.query_row("SELECT grant_revision FROM audit WHERE envelope_id=?1 AND decision='refused' ORDER BY position DESC LIMIT 1",[fresh_id],|r|r.get::<_,i64>(0)).unwrap(),2,"known peer refusal keeps its observed authority revision");
    drop(refused);
    drop(restarted);
    drop(response);
    drop(keys);
    drop(store);
    drop(lease);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sender_recovery_cap_prunes_only_after_retention_and_skew_and_keeps_original_expiry() {
    let root = std::env::temp_dir().join(format!("tmt-cloud-retention-{}", uuid_v4().unwrap()));
    let layout = Layout::open(&root).unwrap();
    let lease = layout.serve_lock().unwrap();
    let mut store = Store::open(&lease).unwrap();
    store
        .insert_grant(&Grant {
            client_id: PEER.into(),
            public_key: [8; 32],
            kind: "cli".into(),
            origin: "cli".into(),
            name: "peer".into(),
            agents: "all".into(),
            scopes: vec!["talk".into()],
            mode: "direct".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        })
        .unwrap();
    let created = 1_800_000_000_000;
    for _ in 0..crate::journal::OPERATIONS {
        store
            .cloud_freeze_outgoing(
                PEER,
                &uuid_v4().unwrap(),
                b"intent",
                b"frozen carrier",
                created,
            )
            .unwrap();
    }
    assert_eq!(
        store
            .cloud_freeze_outgoing(PEER, &uuid_v4().unwrap(), b"intent", b"carrier", created)
            .err()
            .unwrap()
            .code,
        "REMOTE_CLOUD_CAPACITY"
    );
    let boundary =
        created + crate::journal::RECOVERY + crate::session::CLOCK_SKEW.as_millis() as u64;
    store.cloud_prune_outgoing(boundary).unwrap();
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM cloud_transport WHERE kind='outbox'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        crate::journal::OPERATIONS
    );
    store.cloud_prune_outgoing(boundary + 1).unwrap();
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM cloud_transport WHERE kind='outbox'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        super::check_request_age(created, boundary + 1)
            .unwrap_err()
            .code,
        "REMOTE_CLOUD_MESSAGE_EXPIRED"
    );
    // The original time is immutable even when bytes and ID are otherwise unchanged.
    let id = uuid_v4().unwrap();
    store
        .cloud_freeze_outgoing(PEER, &id, b"fresh intent", b"fresh carrier", boundary)
        .unwrap();
    assert_eq!(
        store
            .cloud_outgoing(PEER, &id, b"fresh intent", boundary + 1)
            .err()
            .unwrap()
            .code,
        "REMOTE_CLOUD_CONFLICT"
    );
    assert!(super::check_request_age(boundary, boundary + 1).is_ok());
    drop(store);
    drop(lease);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_peer_refusal_is_audit_metadata_without_a_grant_or_operation() {
    let root = std::env::temp_dir().join(format!("tmt-cloud-unknown-{}", uuid_v4().unwrap()));
    let layout = Layout::open(&root).unwrap();
    let lease = layout.serve_lock().unwrap();
    let mut store = Store::open(&lease).unwrap();
    assert_eq!(
        store
            .cloud_begin(PEER, ID, b"undecryptable", 1_000)
            .unwrap(),
        super::Replay::Fresh
    );
    store
        .cloud_refuse(PEER, ID, "REMOTE_CLOSED", None, 2_000)
        .unwrap();
    store
        .cloud_refuse(PEER, ID, "REMOTE_CLOSED", None, 2_001)
        .unwrap();
    assert_eq!(
        store
            .cloud_begin(PEER, ID, b"undecryptable", 1_000)
            .unwrap(),
        super::Replay::Refused(None)
    );
    assert_eq!(store.connection.query_row("SELECT COUNT(*) FROM audit WHERE envelope_id=?1 AND decision='refused' AND grant_revision=0",[ID],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert!(store.grant(PEER).unwrap().is_none());
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(store);
    drop(lease);
    std::fs::remove_dir_all(root).unwrap();
}
