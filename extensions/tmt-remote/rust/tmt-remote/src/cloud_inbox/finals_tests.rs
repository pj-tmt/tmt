use super::*;
use crate::{
    cloud_hpke,
    cloud_keys::CloudKeys,
    message::MessageService,
    operations::Operations,
    session::DoorSessions,
    state::{Layout, MachineKey, Serving},
    store::{Grant, uuid_v4},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const MACHINE: &str = "11111111-1111-4111-8111-111111111111";
const PEER: &str = "33333333-3333-4333-8333-333333333333";
const OPERATION: &str = "44444444-4444-4444-8444-444444444444";
const AGENT: &str = "66666666-6666-4666-8666-666666666666";

#[derive(Default)]
struct Core {
    finals: Mutex<BTreeMap<String, String>>,
}
impl CloudFinalDelivery for Core {
    fn deliver_final(
        &self,
        reply: &crate::cloud_command::Reply,
        response: &str,
        _: &AtomicBool,
    ) -> Result<(), RemoteError> {
        assert_eq!(reply.receipt, "private-local-receipt");
        let operation = reply.request_id.strip_prefix("req_").unwrap();
        let mut finals = self.finals.lock().unwrap();
        if let Some(old) = finals.get(operation) {
            if old != response {
                return Err(RemoteError::new("RESPONSE_CONFLICT", "Conflicting final."));
            }
        } else {
            finals.insert(operation.into(), response.into());
        }
        Ok(())
    }
}
struct World {
    layout: Layout,
    lease: Option<Serving>,
    store: Option<Arc<Mutex<Store>>>,
    core: Arc<Core>,
    now: u64,
}
impl World {
    fn new() -> Self {
        Self::with_origin(
            OPERATION,
            ReplyOrigin {
                machine: PEER.into(),
                remote: "paired".into(),
                sender_id: AGENT.into(),
                reply: Some(crate::cloud_command::Reply {
                    request_id: format!("req_{OPERATION}"),
                    receipt: "private-local-receipt".into(),
                }),
            },
        )
    }
    fn with_origin(operation: &str, origin: ReplyOrigin) -> Self {
        let root = std::env::temp_dir().join(format!("tmt-cloud-final-{}", uuid_v4().unwrap()));
        let layout = Layout::open(&root).unwrap();
        let lease = layout.serve_lock().unwrap();
        let mut store = Store::open(&lease).unwrap();
        let now = now_ms().unwrap();
        store
            .insert_grant(&Grant {
                client_id: PEER.into(),
                public_key: SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes(),
                kind: "cli".into(),
                origin: "cli".into(),
                name: "Peer".into(),
                agents: "all".into(),
                scopes: vec!["results.read".into()],
                mode: "hold".into(),
                issued_at_ms: now,
                expires_at_ms: None,
                revision: 1,
                disabled: false,
            })
            .unwrap();
        store
            .cloud_freeze_outgoing(
                MACHINE,
                operation,
                b"original prepared intent",
                b"frozen ciphertext",
                now,
            )
            .unwrap();
        store
            .cloud_bind_outgoing(MACHINE, operation, &origin)
            .unwrap();
        assert!(store.cloud_claim_outgoing(MACHINE, operation).unwrap());
        Self {
            layout,
            lease: Some(lease),
            store: Some(Arc::new(Mutex::new(store))),
            core: Arc::new(Core::default()),
            now,
        }
    }
    fn store(&self) -> Arc<Mutex<Store>> {
        self.store.as_ref().unwrap().clone()
    }
    fn receiver(&self) -> (super::super::Receiver, Arc<CloudKeys>) {
        self.receiver_with_core(self.core.clone())
    }
    fn receiver_with_core(
        &self,
        core: Arc<dyn CloudFinalDelivery>,
    ) -> (super::super::Receiver, Arc<CloudKeys>) {
        let key = MachineKey::open(&self.layout).unwrap();
        let sessions = Arc::new(DoorSessions::new(
            MACHINE.into(),
            uuid_v4().unwrap(),
            "http://127.0.0.1:1".into(),
            "/r/final/x/".into(),
            key.clone(),
            self.store(),
            crate::session::IDLE,
        ));
        let keys = Arc::new(CloudKeys::new(self.layout.clone(), MACHINE.into(), key));
        keys.maintain(self.now).unwrap();
        let operations = Operations::new(
            crate::core::CoreClient::at("/no-core-process".into()).unwrap(),
            Arc::new(AtomicBool::new(false)),
            crate::limits::CLOUD_ENVELOPE_BYTES,
        )
        .with_cloud_final(core);
        let messages = Arc::new(
            MessageService::new(sessions.clone(), crate::limits::CLOUD_ENVELOPE_BYTES)
                .with_operations(Arc::new(operations)),
        );
        (
            super::super::Receiver::new(sessions, keys.clone(), messages),
            keys,
        )
    }
    fn document(
        &self,
        keys: &CloudKeys,
        operation: &str,
        body: &str,
    ) -> super::super::document::Document {
        let final_value = Final {
            version: 1,
            operation_id: operation.into(),
            request_id: format!("req_{AGENT}"),
            response: body.into(),
        };
        let payload = serde_json::to_vec(&final_value).unwrap();
        let id = id(operation).unwrap();
        let session = uuid_v4().unwrap();
        let window = uuid_v4().unwrap();
        let envelope = canonical::Envelope {
            kind: "request",
            id: &id,
            correlation_id: None,
            machine_id: MACHINE,
            window_id: &window,
            client_id: PEER,
            session_id: &session,
            sequence: "1",
            timestamp_ms: self.now,
            origin: "cli",
            operation: "final",
            payload: &payload,
        };
        let signer = SigningKey::from_bytes(&[8; 32]);
        let signature = canonical::base64url(
            &signer
                .sign(&canonical::envelope(&envelope).unwrap())
                .to_bytes(),
        );
        let exact = json!({"version":1,"profile":"local-v1","kind":"request","id":id,"correlationId":null,
            "machineId":MACHINE,"windowId":window,"clientId":PEER,"sessionId":session,"sequence":"1",
            "timestampMs":self.now,"origin":"cli","operation":"final","payload":canonical::base64url(&payload),"signature":signature}).to_string();
        let descriptor = keys.advertised(self.now).unwrap();
        let (carrier, _) = super::super::seal_native(
            exact.as_bytes(),
            &descriptor,
            self.now,
            cloud_hpke::Sender {
                client_id: PEER,
                authentication_key: &signer.verifying_key().to_bytes(),
            },
            |bytes| signer.sign(bytes).to_bytes(),
        )
        .unwrap();
        let root = "projects/demo-final/databases/(default)/documents";
        let stamp = super::super::stamp(self.now);
        let row = json!({"name":format!("{root}/m/{MACHINE}/inbox/{id}"),"createTime":stamp,
            "fields":{"peer":{"stringValue":PEER},"carrier":{"bytesValue":STANDARD.encode(carrier)},"createdAt":{"timestampValue":stamp}}});
        super::super::document::Document::read(&row, root, MACHINE, "inbox").unwrap()
    }
    fn reopen(&mut self) {
        drop(self.store.take());
        self.store = Some(Arc::new(Mutex::new(
            Store::open(self.lease.as_ref().unwrap()).unwrap(),
        )));
    }
}
impl Drop for World {
    fn drop(&mut self) {
        drop(self.store.take());
        drop(self.lease.take());
        std::fs::remove_dir_all(self.layout.directory.parent().unwrap()).unwrap();
    }
}

#[test]
#[ignore = "Selected by cloud-final-core-acceptance.mjs with an isolated actual Core request"]
fn pushed_final_submits_a_real_core_request_receipt_once() {
    let path = std::env::var_os("TMT_REMOTE_FINAL_CORE_PROOF")
        .expect("explicit isolated Core proof required");
    let input = std::fs::read(path).unwrap();
    assert!(input.len() <= crate::limits::CLOUD_COMMAND_BYTES);
    let proof: serde_json::Value = serde_json::from_slice(&input).unwrap();
    let request =
        crate::cloud_command::Request::read(&serde_json::to_vec(&proof["request"]).unwrap())
            .unwrap();
    let body = proof["response"].as_str().unwrap();
    let core =
        Arc::new(crate::core::CoreClient::at(proof["core"].as_str().unwrap().into()).unwrap());
    let origin = ReplyOrigin {
        machine: PEER.into(),
        remote: request.remote.unwrap(),
        sender_id: request.from.id,
        reply: request.reply,
    };
    assert!(origin.reply.is_some());
    let mut world = World::with_origin(&request.request_id, origin);
    let (receiver, keys) = world.receiver_with_core(core.clone());
    let document = world.document(&keys, &request.request_id, body);
    let first = receiver.receive(&document, world.now).unwrap().unwrap();
    let repeated = receiver.receive(&document, world.now).unwrap().unwrap();
    assert_eq!(first.bytes, repeated.bytes);
    drop(first);
    drop(repeated);
    drop(receiver);
    drop(keys);
    world.reopen();
    let (receiver, _) = world.receiver_with_core(core);
    receiver.receive(&document, world.now).unwrap().unwrap();
    let calls = std::fs::read_to_string(proof["calls"].as_str().unwrap()).unwrap();
    assert_eq!(calls.lines().filter(|line| *line == "reply").count(), 1);
}

#[test]
fn pushed_final_preserves_exact_bytes_and_deduplicates_after_reopen() {
    for body in ["", " \r\nBOM:\u{feff} NUL:\0 Unicode: π\n "] {
        let mut world = World::new();
        let (receiver, keys) = world.receiver();
        let document = world.document(&keys, OPERATION, body);
        let first = receiver.receive(&document, world.now).unwrap().unwrap();
        assert!(!first.response_required());
        assert_eq!(
            world.core.finals.lock().unwrap().get(OPERATION).unwrap(),
            body
        );
        assert_eq!(world.core.finals.lock().unwrap().len(), 1);
        assert_eq!(
            receiver
                .receive(&document, world.now)
                .unwrap()
                .unwrap()
                .bytes,
            first.bytes
        );
        drop(first);
        drop(receiver);
        drop(keys);
        world.reopen();
        let (receiver, _) = world.receiver();
        assert!(
            !receiver
                .receive(&document, world.now)
                .unwrap()
                .unwrap()
                .response_required()
        );
        assert_eq!(world.core.finals.lock().unwrap().len(), 1);
    }
}

#[test]
fn final_refuses_foreign_operation_peer_allowlist_and_revocation_before_core() {
    for refusal in ["operation", "peer", "allowlist", "revoked", "unclaimed"] {
        let world = World::new();
        let (receiver, keys) = world.receiver();
        let operation = if refusal == "operation" {
            AGENT
        } else {
            OPERATION
        };
        let document = world.document(&keys, operation, "must never be accepted");
        let store = world.store();
        let mut store = store.lock().unwrap();
        match refusal {
            "peer" => {
                store.connection.execute("UPDATE cloud_transport SET binding=json_set(binding,'$.machine',?1) WHERE kind='outbox'", [AGENT]).unwrap();
            }
            "allowlist" => {
                store
                    .connection
                    .execute("UPDATE grants SET agents=?1", [format!("[\"{PEER}\"]")])
                    .unwrap();
            }
            "revoked" => {
                store.revoke(PEER).unwrap();
            }
            "unclaimed" => {
                store
                    .connection
                    .execute(
                        "UPDATE cloud_transport SET attempted=0 WHERE kind='outbox'",
                        [],
                    )
                    .unwrap();
            }
            _ => {}
        }
        drop(store);
        let _ = receiver.receive(&document, world.now).unwrap();
        assert!(world.core.finals.lock().unwrap().is_empty(), "{refusal}");
    }
}

#[test]
fn oversized_final_is_a_durable_refusal_and_does_not_block_another_final() {
    let mut world = World::new();
    let (receiver, keys) = world.receiver();
    let oversized = world.document(
        &keys,
        AGENT,
        &"x".repeat(crate::limits::CLOUD_MESSAGE_BYTES + 1),
    );
    let refusal = receiver.receive(&oversized, world.now).unwrap().unwrap();
    assert!(!refusal.response_required());
    let code: String = world
        .store()
        .lock()
        .unwrap()
        .connection
        .query_row(
            "SELECT refusal FROM cloud_transport WHERE peer_id=?1 AND kind='inbox' AND id=?2",
            params![PEER, id(AGENT).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(code, "REMOTE_CLOUD_MESSAGE_TOO_LARGE");
    assert!(world.core.finals.lock().unwrap().is_empty());
    let valid = world.document(&keys, OPERATION, "still deliverable");
    receiver.receive(&valid, world.now).unwrap().unwrap();
    assert_eq!(
        world.core.finals.lock().unwrap().get(OPERATION).unwrap(),
        "still deliverable"
    );
    drop(refusal);
    drop(receiver);
    drop(keys);
    world.reopen();
    let (receiver, _) = world.receiver();
    receiver.receive(&oversized, world.now).unwrap().unwrap();
    assert_eq!(world.core.finals.lock().unwrap().len(), 1);
}

#[test]
fn final_id_is_deterministic_and_old_binding_cannot_change_destination() {
    // Independently calculated with Python hashlib + big-endian LP framing.
    assert_eq!(
        id(OPERATION).unwrap(),
        "1ffaa631-fe17-47d7-a542-13804801f787"
    );
    assert_ne!(id(OPERATION).unwrap(), id(AGENT).unwrap());
    assert!(canonical::is_core_id(&id(OPERATION).unwrap()));
    let world = World::new();
    let error = world
        .store()
        .lock()
        .unwrap()
        .cloud_bind_outgoing(
            MACHINE,
            OPERATION,
            &ReplyOrigin {
                machine: AGENT.into(),
                remote: "paired".into(),
                sender_id: AGENT.into(),
                reply: None,
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "REMOTE_INTENT_CONFLICT");
    let oversized = Final {
        version: 1,
        operation_id: OPERATION.into(),
        request_id: format!("req_{AGENT}"),
        response: "x".repeat(crate::limits::CLOUD_MESSAGE_BYTES + 1),
    };
    assert_eq!(
        oversized.validate().unwrap_err().code,
        "REMOTE_CLOUD_MESSAGE_TOO_LARGE"
    );
}

#[test]
fn local_reply_receipt_is_admitted_but_never_part_of_the_dispatch_payload() {
    let input = json!({"version":1,"remote":"paired","requestId":OPERATION,"createdAtMs":1000,
        "from":{"id":AGENT,"name":"sender"},"target":{"name":"target"},"message":"hello",
        "reply":{"requestId":format!("req_{OPERATION}"),"receipt":"PRIVATE_RECEIPT_CANARY"}});
    let request = crate::cloud_command::Request::read(input.to_string().as_bytes()).unwrap();
    assert_eq!(
        request.reply.as_ref().unwrap().receipt,
        "PRIVATE_RECEIPT_CANARY"
    );
    let dispatch = request.dispatch();
    assert!(dispatch.get("reply").is_none());
    assert!(!dispatch.to_string().contains("PRIVATE_RECEIPT_CANARY"));
    let mut wrong = input;
    wrong["reply"]["requestId"] = json!(format!("req_{AGENT}"));
    assert!(crate::cloud_command::Request::read(wrong.to_string().as_bytes()).is_err());
}

#[test]
fn delayed_unclaimed_final_refreshes_context_but_only_the_claimed_bytes_can_publish() {
    let world = World::new();
    let store = world.store();
    let mut store = store.lock().unwrap();
    store
        .cloud_begin(PEER, OPERATION, b"original incoming ciphertext", world.now)
        .unwrap();
    // The two UUID domains may coincide. Keep the recipient reference before the
    // trailing Core request reference rather than removing equal values.
    store.connection.execute(
        "INSERT INTO operations(id,client_id,operation,digest,frozen,phase,receipt,references_json,updated_ms) VALUES(?1,?2,'dispatch.create',?3,NULL,'accepted',?4,?5,?6)",
        params![OPERATION, PEER, &[0u8;32][..], json!({"requestId":format!("req_{AGENT}")}).to_string(),
            json!([AGENT,AGENT]).to_string(), world.now as i64],
    ).unwrap();
    let grant = store.grant(PEER).unwrap().unwrap();
    let final_id = id(OPERATION).unwrap();
    let initial = world.now.saturating_sub(120_000);
    store
        .cloud_prepare_final(
            MACHINE,
            &final_id,
            b"exact final",
            b"old unused HPKE context",
            initial,
        )
        .unwrap();
    let fresh = store
        .cloud_prepare_final(
            MACHINE,
            &final_id,
            b"exact final",
            b"fresh HPKE context",
            world.now,
        )
        .unwrap();
    assert_eq!(fresh.created_ms, world.now);
    assert_eq!(fresh.carrier, b"fresh HPKE context");
    let request = format!("req_{AGENT}");
    assert!(
        !store
            .cloud_claim_final(
                &grant,
                MACHINE,
                &final_id,
                OPERATION,
                &request,
                b"old unused HPKE context"
            )
            .unwrap()
    );
    assert!(
        store
            .cloud_claim_final(
                &grant,
                MACHINE,
                &final_id,
                OPERATION,
                &request,
                b"fresh HPKE context"
            )
            .unwrap()
    );
    let after_claim = store
        .cloud_prepare_final(
            MACHINE,
            &final_id,
            b"exact final",
            b"must not replace claimed bytes",
            world.now + 120_000,
        )
        .unwrap();
    assert!(after_claim.attempted);
    assert_eq!(after_claim.carrier, b"fresh HPKE context");
    assert_eq!(after_claim.created_ms, world.now);
    assert!(
        store
            .cloud_prepare_final(
                MACHINE,
                &final_id,
                b"different final",
                b"different context",
                world.now
            )
            .is_err()
    );
    store
        .connection
        .execute(
            "UPDATE operations SET updated_ms=?1 WHERE id=?2",
            params![
                world.now.saturating_sub(crate::journal::RECOVERY) as i64,
                OPERATION
            ],
        )
        .unwrap();
    assert!(store.cloud_accepted_dispatches(PEER).unwrap().is_empty());
}
