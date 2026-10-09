//! Actual registered owner, native target and real neutral Bus. Remote backend
//! effects are represented by explicit fixture replies, not claimed routed proof.
use super::*;
use crate::{
    readers::test_support::{Fixture, PAGE},
    registration::OwnerAdmission,
};
use ed25519_dalek::{Signer, SigningKey};
use tmt_colab_model::{
    attachment::{Descriptor, Source},
    crypto, framing, object, values,
};
const DEVICE: &str = "33333333-3333-4333-8333-333333333333";
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
struct OwnerObjectFixture {
    state: Fixture,
    channel: ObjectChannel,
    remote: Bus,
    peer: Arc<admission::PeerIdentity>,
    jobs: PeerObjects,
    original: binding::FrozenUpload,
    raw: Vec<u8>,
    callback: u64,
    request: u64,
}
impl OwnerObjectFixture {
    fn new() -> Self {
        let mut state = Fixture::new();
        with_object_source(&mut state);
        let signer = SigningKey::from_bytes(&[10; 32]);
        let remote_key = SigningKey::from_bytes(&[9; 32]);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let forwarded = serde_json::json!({"deviceId":DEVICE,"kind":"browser","origin":"http://127.0.0.1:1","name":"Browser","publicKey":values::encode_binary(remote_key.verifying_key().as_bytes()),"owner":true,"grantRevision":1}).to_string();
        let certificate = |purpose: &str, key: &[u8; 32]| {
            let input = framing::frame(&[
                b"tmt-ext-cert-v1",
                b"colab",
                purpose.as_bytes(),
                key,
                now.to_string().as_bytes(),
            ])
            .unwrap();
            serde_json::json!({"publicKey":values::encode_binary(key),"issuedAtMs":now,"signature":values::encode_binary(&remote_key.sign(&input).to_bytes())})
        };
        let encryption = tmt_colab_model::wrap::RecipientKey::from_seed(&[11; 32])
            .unwrap()
            .public_key();
        let register = serde_json::json!({"deviceId":DEVICE,"sign":certificate("sign", signer.verifying_key().as_bytes()),"enc":certificate("enc", &encryption)});
        state
            .service
            .lock()
            .unwrap()
            .register(
                Some(&forwarded),
                &serde_json::to_vec(&register).unwrap(),
                now,
            )
            .unwrap();
        let (channel, remote) = pair();
        let origin = Uuid4::parse("22222222-2222-4222-8222-222222222222").unwrap();
        notice(&channel, &remote, origin, OriginPhase::Established);
        let peer = Arc::new(admission::PeerIdentity {
            client: channel.client(),
            origin: Origin::Mounted(origin),
            principal: DEVICE.into(),
            forwarded: Some(forwarded),
            reader: false,
            closed: Arc::new(AtomicBool::new(false)),
            admission: OwnerAdmission(state.service.clone()),
        });
        assert!(peer.capture_scope(&state.scope(), true).is_ok());
        let revision = state
            .store
            .owner_head(&state.key.space_id, &state.key.owner_public())
            .unwrap()
            .unwrap()
            .revision
            .to_string();
        let asset = object::seal(
            &object::Context {
                space: state.key.space_id.clone(),
                page: PAGE.into(),
                epoch: "1".into(),
                kind: "asset".into(),
                namespace: "own".into(),
                author_device: DEVICE.into(),
                membership_revision: revision.clone(),
                stream_seq: "0".into(),
                prev_hash: [0; 32],
            },
            &[11; 32],
            &signer,
            b"original bytes",
        )
        .unwrap();
        let raw = asset.to_json().unwrap();
        let descriptor = Descriptor {
            version: 1,
            attachment_id: GENERATION.into(),
            space: state.key.space_id.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            namespace: "own".into(),
            object_id: object::Header::decode(asset.header()).unwrap().object_id,
            author_device: DEVICE.into(),
            membership_revision: revision,
            source: Source::Message {
                writer_id: DEVICE.into(),
                message_id: GENERATION.into(),
                message_revision: "1".into(),
            },
            envelope_hash: hex(&asset.hash().unwrap()),
            signature: values::encode_binary(asset.signature()),
            payload_sha256: hex(&crypto::digest(&raw)),
            payload_bytes: raw.len().to_string(),
            plaintext_bytes: "14".into(),
            filename: "private label.txt".into(),
            media_type: "text/plain".into(),
        };
        let base =
            crate::attachments::captured_base(&state.store, &state.key, &descriptor).unwrap();
        let original =
            binding::FrozenUpload::new(descriptor, base, GENERATION, raw.clone()).unwrap();
        let jobs = PeerObjects::new(peer.clone()).unwrap();
        Self {
            state,
            channel,
            remote,
            peer,
            jobs,
            original,
            raw,
            callback: 0,
            request: 0,
        }
    }
    fn submit(&mut self, request: ObjectRequest) {
        self.request += 1;
        let id = format!("44444444-4444-4444-8444-{:012}", self.request);
        self.jobs
            .submit(self.state.scope(), id, request, limit())
            .unwrap();
    }
    fn reply(&self) -> peer::ObjectReply {
        self.jobs
            .replies
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    }
    fn admit(
        &mut self,
        request: &Request,
        boundary: Checkpoint,
        disclosure: Option<Disclosure>,
    ) -> Decision {
        self.admit_context(request, boundary, disclosure, 1)
    }
    fn admit_context(
        &mut self,
        request: &Request,
        boundary: Checkpoint,
        disclosure: Option<Disclosure>,
        grant_revision: u64,
    ) -> Decision {
        self.callback += 1;
        let input = match &request.call {
            Call::Read(input) => AdmitInput::Read(input.clone()),
            _ => self.original.admission_input(&request.call).unwrap(),
        };
        self.remote
            .send(&Frame::Admit(Admit {
                generation: request.generation,
                callback_id: Counter::new(self.callback).unwrap(),
                request_id: request.request_id,
                boundary,
                context: Context::OwnerSession {
                    origin_id: match self.peer.origin {
                        Origin::Mounted(id) => id,
                        _ => unreachable!(),
                    },
                    device_id: Uuid4::parse(DEVICE).unwrap(),
                    grant_revision,
                },
                operation: Operation { input, disclosure },
            }))
            .unwrap();
        let Frame::Admission(answer) = self.remote.recv(Some(limit())).unwrap() else {
            panic!("admission");
        };
        answer.decision
    }
    fn finish(&self, request: Request, outcome: Outcome) {
        let transfer = match &request.call {
            Call::Begin(input) => Some(input.transfer_id),
            Call::Status(input) => Some(input.transfer_id),
            Call::Part(input) => Some(input.transfer_id),
            Call::Commit(input) | Call::Discard(input) => Some(input.transfer_id),
            _ => None,
        };
        self.remote
            .send(&Frame::Result(ResultFrame {
                generation: request.generation,
                request_id: request.request_id,
                method: request.call.method(),
                transfer_id: transfer,
                outcome,
            }))
            .unwrap();
    }
}
impl Drop for OwnerObjectFixture {
    fn drop(&mut self) {
        self.jobs.close();
    }
}

#[test]
fn actual_owner_upload_keeps_unknown_original_and_verifies_committed_raw_ranges() {
    let mut fixture = OwnerObjectFixture::new();
    let original = fixture.original.descriptor().clone();
    let base = fixture.original.base().to_owned();
    fixture.submit(ObjectRequest::Begin {
        transfer_id: GENERATION.into(),
        descriptor: original.clone(),
        base: base.clone(),
    });
    let begin = receive(&fixture.remote);
    let Call::Begin(input) = &begin.call else {
        panic!("begin");
    };
    let input = input.clone();
    let policy = std::str::from_utf8(input.policy.as_bytes()).unwrap();
    assert!(
        !policy.contains("private label")
            && !policy.contains("filename")
            && !policy.contains("mediaType")
    );
    assert_eq!(
        fixture.admit(&begin, Checkpoint::Acquire, None),
        Decision::Allow
    );
    assert_eq!(
        fixture.admit(&begin, Checkpoint::Effect, None),
        Decision::Allow
    );
    assert_eq!(
        fixture.admit(
            &begin,
            Checkpoint::Disclose,
            Some(Disclosure::Status {
                next_index: 0,
                received: 0,
                expires_at_ms: None
            })
        ),
        Decision::Allow
    );
    fixture.finish(
        begin,
        Outcome::Success(Success::Pending {
            next_index: 0,
            received: 0,
            expires_at_ms: None,
        }),
    );
    assert_eq!(fixture.reply().value["ok"]["result"], "pending");
    fixture.submit(ObjectRequest::Part {
        transfer_id: GENERATION.into(),
        index: 0,
        bytes: values::encode_binary(&fixture.raw),
    });
    let part = receive(&fixture.remote);
    assert_eq!(
        fixture.admit(&part, Checkpoint::Acquire, None),
        Decision::Allow
    );
    assert_eq!(
        fixture.admit(&part, Checkpoint::Effect, None),
        Decision::Allow
    );
    let received = fixture.raw.len() as u64;
    assert_eq!(
        fixture.admit(
            &part,
            Checkpoint::Disclose,
            Some(Disclosure::Progress {
                next_index: 1,
                received
            })
        ),
        Decision::Allow
    );
    fixture.finish(
        part,
        Outcome::Success(Success::Progress {
            next_index: 1,
            received,
        }),
    );
    assert_eq!(fixture.reply().value["ok"]["result"], "progress");
    fixture.submit(ObjectRequest::Commit {
        transfer_id: GENERATION.into(),
    });
    let commit = receive(&fixture.remote);
    assert_eq!(
        fixture.admit(&commit, Checkpoint::Acquire, None),
        Decision::Allow
    );
    assert_eq!(
        fixture.admit(&commit, Checkpoint::Effect, None),
        Decision::Allow
    );
    fixture.finish(
        commit,
        Outcome::Failure(tmt_extension_objects::ErrorCode::Unknown),
    );
    assert_eq!(fixture.reply().value["error"]["code"], "unknown");
    fixture.submit(ObjectRequest::Status {
        transfer_id: GENERATION.into(),
        descriptor: original.clone(),
        base: base.clone(),
    });
    let status = receive(&fixture.remote);
    assert!(
        matches!(&status.call, Call::Status(status) if status.transfer_id == input.transfer_id)
    );
    assert_eq!(
        fixture.admit(&status, Checkpoint::Acquire, None),
        Decision::Allow
    );
    assert_eq!(
        fixture.admit(
            &status,
            Checkpoint::Disclose,
            Some(Disclosure::Receipt {
                opaque_key: input.opaque_key,
                payload_sha256: input.payload_sha256,
                payload_bytes: input.payload_bytes
            })
        ),
        Decision::Allow
    );
    fixture.finish(
        status,
        Outcome::Success(Success::Committed {
            opaque_key: input.opaque_key,
            payload_sha256: input.payload_sha256,
            payload_bytes: input.payload_bytes,
        }),
    );
    assert_eq!(fixture.reply().value["ok"]["result"], "committed");
    fixture.submit(ObjectRequest::Verify {
        transfer_id: GENERATION.into(),
        descriptor: original,
        base,
        offset: 0,
        count: fixture.raw.len() as u32,
    });
    let read = receive(&fixture.remote);
    assert_eq!(
        fixture.admit(&read, Checkpoint::Acquire, None),
        Decision::Allow
    );
    assert_eq!(
        fixture.admit(
            &read,
            Checkpoint::Disclose,
            Some(Disclosure::Bytes {
                offset: 0,
                length: fixture.raw.len() as u32
            })
        ),
        Decision::Allow
    );
    fixture.finish(
        read,
        Outcome::Success(Success::Read {
            offset: 0,
            total_bytes: received,
            bytes: Chunk::new(fixture.raw.clone()).unwrap(),
        }),
    );
    let reply = fixture.reply();
    assert_eq!(
        values::binary(reply.value["ok"]["bytes"].as_str().unwrap(), 32_768).unwrap(),
        fixture.raw
    );
    assert!(reply.fence.unwrap().current().is_ok());
    fixture.jobs.close();
    assert!(
        fixture.channel.active(),
        "peer shutdown must not end the healthy carrier"
    );
}

#[test]
fn altered_owner_target_and_spent_pre_call_have_no_pending_carrier_call() {
    let mut fixture = OwnerObjectFixture::new();
    let mut descriptor = fixture.original.descriptor().clone();
    descriptor.author_device = GENERATION.into();
    fixture.submit(ObjectRequest::Begin {
        transfer_id: GENERATION.into(),
        descriptor,
        base: fixture.original.base().into(),
    });
    let denied = fixture.reply();
    assert_eq!(denied.value["error"]["code"], "denied");
    assert!(denied.fence.is_none());
    assert!(locked(&fixture.channel.shared.state).pending.is_empty());
    fixture
        .jobs
        .submit(
            fixture.state.scope(),
            "55555555-5555-4555-8555-555555555555".into(),
            ObjectRequest::Config {},
            Instant::now(),
        )
        .unwrap();
    assert_eq!(fixture.reply().value["error"]["code"], "unavailable");
    assert!(locked(&fixture.channel.shared.state).pending.is_empty());
    assert!(fixture.channel.active());
}

#[test]
fn actual_revoke_after_effect_suppresses_disclosure_and_preserves_unknown() {
    let mut fixture = OwnerObjectFixture::new();
    fixture.submit(ObjectRequest::Begin {
        transfer_id: GENERATION.into(),
        descriptor: fixture.original.descriptor().clone(),
        base: fixture.original.base().into(),
    });
    let begin = receive(&fixture.remote);
    assert_eq!(
        fixture.admit(&begin, Checkpoint::Acquire, None),
        Decision::Allow
    );
    assert_eq!(
        fixture.admit(&begin, Checkpoint::Effect, None),
        Decision::Allow
    );
    assert!(
        fixture
            .state
            .service
            .lock()
            .unwrap()
            .revoke(DEVICE, 2)
            .unwrap()
    );
    assert_eq!(
        fixture.admit(
            &begin,
            Checkpoint::Disclose,
            Some(Disclosure::Status {
                next_index: 0,
                received: 0,
                expires_at_ms: None
            })
        ),
        Decision::Deny
    );
    fixture.finish(
        begin,
        Outcome::Failure(tmt_extension_objects::ErrorCode::Unknown),
    );
    let reply = fixture.reply();
    assert_eq!(reply.value["error"]["code"], "unknown");
    assert!(reply.fence.unwrap().current().is_err());
    fixture.submit(ObjectRequest::Status {
        transfer_id: GENERATION.into(),
        descriptor: fixture.original.descriptor().clone(),
        base: fixture.original.base().into(),
    });
    assert_eq!(fixture.reply().value["error"]["code"], "denied");
    assert!(locked(&fixture.channel.shared.state).pending.is_empty());
    assert!(fixture.channel.active());
}

#[test]
fn callback_cannot_borrow_an_altered_remote_grant_binding() {
    let mut fixture = OwnerObjectFixture::new();
    fixture.submit(ObjectRequest::Begin {
        transfer_id: GENERATION.into(),
        descriptor: fixture.original.descriptor().clone(),
        base: fixture.original.base().into(),
    });
    let begin = receive(&fixture.remote);
    assert_eq!(
        fixture.admit_context(&begin, Checkpoint::Acquire, None, 2),
        Decision::Deny
    );
    fixture.finish(
        begin,
        Outcome::Failure(tmt_extension_objects::ErrorCode::Denied),
    );
    assert_eq!(fixture.reply().value["error"]["code"], "denied");
    assert!(locked(&fixture.channel.shared.state).pending.is_empty());
    assert!(fixture.channel.active());
}
