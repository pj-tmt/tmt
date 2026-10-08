//! Real neutral-carrier tests; test CallbackOwner ports are NOT Colab authority
//! or the final Remote-backend routed acceptance proof.
use super::*;
use crate::attachments::CommittedObjectVerifier;
use std::sync::{Barrier, atomic::AtomicUsize};
use tmt_extension_objects::{
    AdmitInput, Budgets, Bytes32, Chunk, ConfigInput, Decision, Disclosure, Expect, Offer,
    Operation, Policy, ReadInput, ResultFrame, Success, accept, initiate,
};
const HOST: &str = "127.0.0.1:51234";
const MOUNT: &str = "/r/fixture/x/colab/";
const GENERATION: &str = "11111111-1111-4111-8111-111111111111";
fn limit() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
fn pair() -> (ObjectChannel, Bus) {
    let (remote, extension) = UnixStream::pair().unwrap();
    let interrupt = extension.try_clone().unwrap();
    let acceptor = thread::spawn(move || {
        let link = accept(
            extension,
            &Expect {
                host: HOST.into(),
                mount: MOUNT.into(),
            },
            &Budgets::contract(),
            limit(),
        )
        .unwrap();
        ObjectChannel::start(link, interrupt).unwrap()
    });
    let link = initiate(
        remote,
        &Offer {
            host: HOST.into(),
            mount: MOUNT.into(),
            generation: Uuid4::parse(GENERATION).unwrap(),
        },
        limit(),
    )
    .unwrap();
    (
        acceptor.join().unwrap(),
        Bus::start(link, Budgets::contract(), Caps::contract(), None).unwrap(),
    )
}
fn receive(bus: &Bus) -> Request {
    let Frame::Request(request) = bus.recv(Some(limit())).unwrap() else {
        panic!("request")
    };
    request
}
fn admit(
    bus: &Bus,
    req: &Request,
    callback: u64,
    boundary: Checkpoint,
    disclosure: Option<Disclosure>,
) -> Admission {
    let input = match &req.call {
        Call::Config(input) => AdmitInput::Config(input.clone()),
        Call::Read(input) => AdmitInput::Read(input.clone()),
        _ => panic!("test input"),
    };
    bus.send(&Frame::Admit(Admit {
        generation: req.generation,
        callback_id: Counter::new(callback).unwrap(),
        request_id: req.request_id,
        boundary,
        context: Context::LocalExtension,
        operation: Operation { input, disclosure },
    }))
    .unwrap();
    let Frame::Admission(answer) = bus.recv(Some(limit())).unwrap() else {
        panic!("admission")
    };
    answer
}
fn finish(bus: &Bus, req: Request, outcome: Outcome) {
    bus.send(&Frame::Result(ResultFrame {
        generation: req.generation,
        request_id: req.request_id,
        method: req.call.method(),
        transfer_id: None,
        outcome,
    }))
    .unwrap();
}
fn config() -> Call {
    Call::Config(ConfigInput {
        namespace: Bytes32::from_bytes([1; 32]),
        policy: Policy::new(vec![]).unwrap(),
    })
}
struct Current(AtomicBool, AtomicUsize);
impl CallbackOwner for Current {
    fn decide(&self, _: &Admit, _: Instant) -> Decision {
        self.1.fetch_add(1, Ordering::SeqCst);
        if self.0.load(Ordering::SeqCst) {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}
#[test]
fn each_boundary_rechecks_the_owner_and_retained_clients_die_with_the_generation() {
    let (channel, remote) = pair();
    let client = channel.client();
    let current = Arc::new(Current(AtomicBool::new(true), AtomicUsize::new(0)));
    let owner: Arc<dyn CallbackOwner> = current.clone();
    let caller = client.clone();
    let request =
        thread::spawn(move || caller.request(Origin::LocalExtension, config(), owner, limit()));
    let received = receive(&remote);
    assert_eq!(
        admit(&remote, &received, 1, Checkpoint::Acquire, None).decision,
        Decision::Allow
    );
    current.0.store(false, Ordering::SeqCst);
    assert_eq!(
        admit(
            &remote,
            &received,
            2,
            Checkpoint::Disclose,
            Some(Disclosure::Config {
                projection: tmt_extension_objects::Projection::Local
            })
        )
        .decision,
        Decision::Deny
    );
    finish(
        &remote,
        received,
        Outcome::Failure(tmt_extension_objects::ErrorCode::Denied),
    );
    assert!(matches!(
        request.join().unwrap().unwrap(),
        Outcome::Failure(tmt_extension_objects::ErrorCode::Denied)
    ));
    assert_eq!(current.1.load(Ordering::SeqCst), 2);
    assert!(channel.active());
    drop(channel);
    assert!(!client.standing(Origin::LocalExtension));
    assert!(
        client
            .request(Origin::LocalExtension, config(), current, limit())
            .is_err()
    );
    assert!(remote.recv(Some(limit())).is_err());
}
struct Held {
    entered: mpsc::SyncSender<()>,
    release: Arc<Barrier>,
}
impl CallbackOwner for Held {
    fn decide(&self, _: &Admit, _: Instant) -> Decision {
        self.entered.send(()).unwrap();
        self.release.wait();
        Decision::Allow
    }
}
#[test]
fn callback_can_wait_while_the_dispatcher_delivers_another_request_result() {
    let (channel, remote) = pair();
    let (entered, arrived) = mpsc::sync_channel(1);
    let release = Arc::new(Barrier::new(2));
    let held = Arc::new(Held {
        entered,
        release: release.clone(),
    });
    let caller = channel.client();
    let first =
        thread::spawn(move || caller.request(Origin::LocalExtension, config(), held, limit()));
    let one = receive(&remote);
    remote
        .send(&Frame::Admit(Admit {
            generation: one.generation,
            callback_id: Counter::new(1).unwrap(),
            request_id: one.request_id,
            boundary: Checkpoint::Acquire,
            context: Context::LocalExtension,
            operation: Operation {
                input: AdmitInput::Config(match &one.call {
                    Call::Config(v) => v.clone(),
                    _ => unreachable!(),
                }),
                disclosure: None,
            },
        }))
        .unwrap();
    arrived.recv_timeout(Duration::from_secs(2)).unwrap();
    let caller = channel.client();
    let current = Arc::new(Current(AtomicBool::new(true), AtomicUsize::new(0)));
    let second =
        thread::spawn(move || caller.request(Origin::LocalExtension, config(), current, limit()));
    let two = receive(&remote);
    finish(
        &remote,
        two,
        Outcome::Failure(tmt_extension_objects::ErrorCode::Unavailable),
    );
    assert!(matches!(
        second.join().unwrap().unwrap(),
        Outcome::Failure(tmt_extension_objects::ErrorCode::Unavailable)
    ));
    release.wait();
    assert!(matches!(
        remote.recv(Some(limit())).unwrap(),
        Frame::Admission(_)
    ));
    finish(
        &remote,
        one,
        Outcome::Failure(tmt_extension_objects::ErrorCode::Unavailable),
    );
    assert!(first.join().unwrap().is_ok());
}
#[test]
fn complete_committed_reader_checks_every_range_and_the_final_raw_digest() {
    for corrupt in [false, true] {
        let (channel, remote) = pair();
        let body = vec![37; 2 * tmt_extension_objects::limits::CHUNK_BYTES + 3];
        let expected = tmt_colab_model::crypto::digest(&body);
        let source = body.clone();
        let responder = thread::spawn(move || {
            for part in source.chunks(tmt_extension_objects::limits::CHUNK_BYTES) {
                let req = receive(&remote);
                let Call::Read(ReadInput {
                    offset,
                    count,
                    payload_sha256,
                    payload_bytes,
                    ..
                }) = &req.call
                else {
                    panic!("read")
                };
                assert_eq!(*count as usize, part.len());
                assert_eq!(*payload_bytes as usize, source.len());
                assert_eq!(payload_sha256.as_bytes(), &expected);
                assert_eq!(
                    admit(
                        &remote,
                        &req,
                        req.request_id.get() * 2 - 1,
                        Checkpoint::Acquire,
                        None
                    )
                    .decision,
                    Decision::Allow
                );
                assert_eq!(
                    admit(
                        &remote,
                        &req,
                        req.request_id.get() * 2,
                        Checkpoint::Disclose,
                        Some(Disclosure::Bytes {
                            offset: *offset,
                            length: *count
                        })
                    )
                    .decision,
                    Decision::Allow
                );
                let mut bytes = part.to_vec();
                if corrupt {
                    bytes[0] ^= 1;
                }
                let success = Success::Read {
                    offset: *offset,
                    total_bytes: source.len() as u64,
                    bytes: Chunk::new(bytes).unwrap(),
                };
                finish(&remote, req, Outcome::Success(success));
            }
            remote
        });
        let reader = CommittedReader {
            client: channel.client(),
            origin: Origin::LocalExtension,
            owner: Arc::new(Current(AtomicBool::new(true), AtomicUsize::new(0))),
            namespace: [1; 32],
            key: [2; 32],
            policy: Policy::new(vec![]).unwrap(),
            digest: expected,
            bytes: body.len() as u64,
        };
        let result = reader.read_committed(&[1; 32], &[2; 32], limit());
        if corrupt {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), body);
        }
        responder.join().unwrap();
        assert!(reader.read_committed(&[9; 32], &[2; 32], limit()).is_err());
    }
}
#[test]
fn mounted_work_without_an_established_notice_and_spent_requests_refuse() {
    let (channel, remote) = pair();
    let client = channel.client();
    let owner = Arc::new(Current(AtomicBool::new(true), AtomicUsize::new(0)));
    assert!(
        client
            .request(
                Origin::Mounted(Uuid4::parse(GENERATION).unwrap()),
                config(),
                owner.clone(),
                limit()
            )
            .is_err()
    );
    assert!(
        client
            .request(
                Origin::LocalExtension,
                config(),
                owner.clone(),
                Instant::now()
            )
            .is_err()
    );
    let caller = client.clone();
    let waiting =
        thread::spawn(move || caller.request(Origin::LocalExtension, config(), owner, limit()));
    let _request = receive(&remote);
    drop(remote);
    assert!(waiting.join().unwrap().is_err());
    assert!(!channel.active());
}

#[test]
fn frozen_upload_status_and_policy_bytes_match_the_shared_independent_corpus() {
    use super::binding::{FrozenUpload, read_policy};
    use tmt_colab_model::{
        attachment::{AttachmentSelector, Descriptor},
        values,
    };
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../contracts/vectors/attachment-v1.json"
    ))
    .unwrap();
    for answer in corpus["channelPolicies"].as_array().unwrap() {
        let name = answer["name"].as_str().unwrap();
        let case = corpus["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == format!("{name}-asset"))
            .unwrap();
        let descriptor = Descriptor::from_json(case["input"].as_str().unwrap().as_bytes()).unwrap();
        let bytes = values::binary(
            case["payload"].as_str().unwrap(),
            tmt_colab_model::attachment::PAYLOAD_BYTES,
        )
        .unwrap();
        let base = format!("v1:{}", "ab".repeat(32));
        let frozen =
            FrozenUpload::new(descriptor.clone(), base.clone(), GENERATION, bytes.clone()).unwrap();
        assert_eq!(frozen.descriptor(), &descriptor);
        assert_eq!(frozen.base(), base);
        let Call::Begin(begin) = frozen.begin() else {
            unreachable!()
        };
        let Call::Status(status) = frozen.status() else {
            unreachable!()
        };
        assert_eq!(begin.transfer_id, status.transfer_id);
        assert_eq!(begin.namespace, status.namespace);
        assert_eq!(begin.policy, status.policy);
        assert_eq!(
            begin.policy.as_bytes(),
            answer["upload"].as_str().unwrap().as_bytes()
        );
        for private in ["filename", "mediaType", "authorDevice"] {
            assert!(
                !std::str::from_utf8(begin.policy.as_bytes())
                    .unwrap()
                    .contains(private)
            );
        }
        let reference = AttachmentSelector::from_json(
            serde_json::to_string(&answer["reference"])
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            read_policy(
                &descriptor,
                answer["peerEpoch"].as_str().unwrap(),
                &reference
            )
            .unwrap()
            .as_bytes(),
            answer["read"].as_str().unwrap().as_bytes()
        );
        let Call::Part(part) = frozen.part(0).unwrap() else {
            unreachable!()
        };
        assert_eq!(part.bytes.as_bytes(), bytes);
        assert!(frozen.part(1).is_err());
        for call in [frozen.commit(), frozen.discard()] {
            let id = match call {
                Call::Commit(v) | Call::Discard(v) => v.transfer_id,
                _ => unreachable!(),
            };
            assert_eq!(id, begin.transfer_id);
        }
        let mut bad = bytes.clone();
        bad[0] ^= 1;
        assert!(FrozenUpload::new(descriptor.clone(), base.clone(), GENERATION, bad).is_err());
        assert!(FrozenUpload::new(descriptor.clone(), "v1:bad".into(), GENERATION, bytes).is_err());
        assert!(!frozen.committed(&Outcome::Success(Success::Pending {
            next_index: 1,
            received: begin.payload_bytes,
            expires_at_ms: None
        })));
        let receipt = Outcome::Success(Success::Committed {
            opaque_key: begin.opaque_key,
            payload_sha256: begin.payload_sha256,
            payload_bytes: begin.payload_bytes,
        });
        assert!(frozen.committed(&receipt));
        assert!(!frozen.committed(&Outcome::Success(Success::Committed {
            opaque_key: Bytes32::from_bytes([0; 32]),
            payload_sha256: begin.payload_sha256,
            payload_bytes: begin.payload_bytes
        })));
        let (channel, _remote) = pair();
        assert!(
            frozen
                .verifier(
                    channel.client(),
                    Origin::LocalExtension,
                    Arc::new(Current(AtomicBool::new(true), AtomicUsize::new(0)))
                )
                .is_err()
        );
    }
}

fn notice(channel: &ObjectChannel, remote: &Bus, origin: Uuid4, phase: OriginPhase) {
    remote
        .send(&Frame::OriginState(tmt_extension_objects::OriginState {
            generation: remote.generation(),
            origin_id: origin,
            phase,
        }))
        .unwrap();
    let mut state = locked(&channel.shared.state);
    let until = limit();
    while state.origins.contains(&origin) != (phase == OriginPhase::Established) {
        let remaining = until.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "notice not delivered");
        state = channel
            .shared
            .changed
            .wait_timeout(state, remaining)
            .unwrap()
            .0;
    }
}
#[test]
fn closed_origin_cannot_disclose_in_flight_work_or_move_to_a_successor() {
    let (channel, remote) = pair();
    let origin = Uuid4::parse("22222222-2222-4222-8222-222222222222").unwrap();
    notice(&channel, &remote, origin, OriginPhase::Established);
    let caller = channel.client();
    let owner = Arc::new(Current(AtomicBool::new(true), AtomicUsize::new(0)));
    let waiting =
        thread::spawn(move || caller.request(Origin::Mounted(origin), config(), owner, limit()));
    let received = receive(&remote);
    notice(&channel, &remote, origin, OriginPhase::Closed);
    remote
        .send(&Frame::Admit(Admit {
            generation: received.generation,
            callback_id: Counter::new(1).unwrap(),
            request_id: received.request_id,
            boundary: Checkpoint::Acquire,
            context: Context::OwnerSession {
                origin_id: origin,
                device_id: Uuid4::parse(GENERATION).unwrap(),
                grant_revision: 1,
            },
            operation: Operation {
                input: AdmitInput::Config(match &received.call {
                    Call::Config(c) => c.clone(),
                    _ => unreachable!(),
                }),
                disclosure: None,
            },
        }))
        .unwrap();
    let Frame::Admission(answer) = remote.recv(Some(limit())).unwrap() else {
        panic!("admission")
    };
    assert_eq!(answer.decision, Decision::Deny);
    finish(
        &remote,
        received,
        Outcome::Failure(tmt_extension_objects::ErrorCode::Denied),
    );
    assert!(
        waiting.join().unwrap().is_err(),
        "closed origin exposes no result class"
    );
    let retained = channel.client();
    drop(channel);
    let (successor, _remote) = pair();
    assert!(!retained.standing(Origin::LocalExtension));
    assert!(!successor.client().standing(Origin::Mounted(origin)));
}
