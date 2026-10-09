//! Real neutral-carrier tests; test CallbackOwner ports are NOT Colab authority
//! or the final Remote-backend routed acceptance proof.
#[path = "tests/attach.rs"]
mod attach;
#[path = "tests/upload.rs"]
mod upload;
use super::*;
use crate::attachments::CommittedObjectVerifier;
use std::sync::{Barrier, atomic::AtomicUsize};
use std::time::Duration;
use tmt_extension_objects::Checkpoint;
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
        let metadata =
            FrozenUpload::metadata(descriptor.clone(), base.clone(), GENERATION).unwrap();
        assert_eq!(metadata.begin(), frozen.begin());
        assert_eq!(metadata.status(), frozen.status());
        assert_eq!(metadata.commit(), frozen.commit());
        assert_eq!(metadata.discard(), frozen.discard());
        assert!(metadata.part(0).is_err());
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
        let streamed = metadata.streamed_part(0, bytes.clone()).unwrap();
        assert_eq!(streamed, Call::Part(part.clone()));
        assert_eq!(
            metadata.admission_input(&streamed).unwrap(),
            frozen.admission_input(&streamed).unwrap()
        );
        assert!(metadata.streamed_part(0, Vec::new()).is_err());
        assert!(metadata.streamed_part(0, vec![0; 32_769]).is_err());
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
        assert!(metadata.committed(&receipt));
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

/// Exercise the routed-head owner using actual duplex sockets. Discovery is a
/// separate fixture port; no offered value can select its expected binding.
fn owned_generation(
    owner: Arc<ChannelOwner>,
    host: &str,
    mount: &str,
    generation: &str,
) -> std::result::Result<Bus, Fault> {
    use std::io::Read;
    let (remote, mut extension) = UnixStream::pair().unwrap();
    let acceptor = thread::spawn(move || {
        extension
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            extension.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
            assert!(head.len() <= 8192);
        }
        owner.accept(extension, &head, &[], limit())
    });
    let result = initiate(
        remote,
        &Offer {
            host: host.into(),
            mount: mount.into(),
            generation: Uuid4::parse(generation).unwrap(),
        },
        limit(),
    );
    let accepted = acceptor.join().unwrap();
    match result {
        Ok(link) => {
            accepted.unwrap();
            Bus::start(link, Budgets::contract(), Caps::contract(), None)
        }
        Err(fault) => {
            assert!(accepted.is_err());
            Err(fault)
        }
    }
}
#[test]
fn routed_owner_refreshes_discovery_and_joins_replaced_generations() {
    let discoveries = Arc::new(AtomicUsize::new(0));
    let observed = discoveries.clone();
    let binding = Arc::new(Mutex::new((HOST.to_owned(), MOUNT.to_owned())));
    let actual = binding.clone();
    let owner = Arc::new(ChannelOwner::new(Arc::new(move || {
        observed.fetch_add(1, Ordering::SeqCst);
        Some(locked(&actual).clone())
    })));
    let first = owned_generation(owner.clone(), HOST, MOUNT, GENERATION).unwrap();
    let retained = owner.client().unwrap();
    assert!(retained.standing(Origin::LocalExtension));
    *locked(&binding) = ("127.0.0.1:51235".into(), "/r/new/x/colab/".into());
    // The old offered binding cannot override live discovery. Refusal must not
    // close the healthy generation that already exists.
    assert!(owned_generation(owner.clone(), HOST, MOUNT, GENERATION).is_err());
    assert!(retained.standing(Origin::LocalExtension));
    let second = owned_generation(
        owner.clone(),
        "127.0.0.1:51235",
        "/r/new/x/colab/",
        "22222222-2222-4222-8222-222222222222",
    )
    .unwrap();
    assert!(!retained.standing(Origin::LocalExtension));
    assert!(first.recv(Some(limit())).is_err());
    let successor = owner.client().unwrap();
    assert_ne!(retained.generation(), successor.generation());
    assert_eq!(discoveries.load(Ordering::SeqCst), 3);
    owner.close();
    assert!(!successor.standing(Origin::LocalExtension));
    assert!(second.recv(Some(limit())).is_err());
    assert!(owner.client().is_none());
}

struct ObjectMountFixture {
    root: std::path::PathBuf,
    path: std::path::PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<crate::Result<()>>>,
}
impl ObjectMountFixture {
    fn start() -> Self {
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::path::PathBuf::from("/tmp").join(format!(
            "co-{}",
            nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        let layout = crate::keyring::Layout::open(&root).unwrap();
        let mount =
            crate::socket::MountSocket::bind(&layout, "fixture", crate::socket::Tunnels::PRODUCT)
                .unwrap()
                .with_object_discovery(|| Some((HOST.into(), MOUNT.into())));
        let path = mount.path.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::spawn(move || mount.run(&stopping));
        Self {
            root,
            path,
            stop,
            worker: Some(worker),
        }
    }
    fn remote(&self, host: &str, generation: &str) -> std::result::Result<Bus, Fault> {
        let stream = UnixStream::connect(&self.path).unwrap();
        let link = initiate(
            stream,
            &Offer {
                host: host.into(),
                mount: MOUNT.into(),
                generation: Uuid4::parse(generation).unwrap(),
            },
            limit(),
        )?;
        Bus::start(link, Budgets::contract(), Caps::contract(), None)
    }
    fn finish(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap().unwrap();
        assert!(!self.path.exists(), "mount leaked its socket");
    }
}
impl Drop for ObjectMountFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                assert!(
                    result.is_ok_and(|result| result.is_ok()),
                    "mount cleanup failed"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn same_mount_acceptor_routes_strict_object_heads_and_closes_the_generation_on_stop() {
    let mut mount = ObjectMountFixture::start();
    assert!(mount.remote("127.0.0.1:51235", GENERATION).is_err());
    let first = mount.remote(HOST, GENERATION).unwrap();
    let second = mount
        .remote(HOST, "22222222-2222-4222-8222-222222222222")
        .unwrap();
    assert!(first.recv(Some(limit())).is_err());
    mount.finish();
    assert!(second.recv(Some(limit())).is_err());
}
#[test]
fn stop_during_live_discovery_cannot_install_a_successor() {
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = Mutex::new(gate);
    let owner = Arc::new(ChannelOwner::new(Arc::new(move || {
        entered.send(()).unwrap();
        locked(&gate).recv_timeout(Duration::from_secs(3)).unwrap();
        Some((HOST.into(), MOUNT.into()))
    })));
    let running = owner.clone();
    let attempt = thread::spawn(move || owned_generation(running, HOST, MOUNT, GENERATION));
    observed.recv_timeout(Duration::from_secs(3)).unwrap();
    owner.close();
    release.send(()).unwrap();
    assert!(attempt.join().unwrap().is_err());
    assert!(owner.client().is_none());
}

#[test]
fn peer_jobs_are_finite_correlated_and_close_joins_a_pending_origin_wait() {
    use crate::{
        readers::test_support::Fixture,
        registration::OwnerAdmission,
        sync::{Code, SyncScope},
    };
    let fixture = Fixture::new();
    let (channel, remote) = pair();
    let client = channel.client();
    let peer = Arc::new(admission::PeerIdentity {
        client: client.clone(),
        origin: Origin::Mounted(Uuid4::parse(GENERATION).unwrap()),
        principal: GENERATION.into(),
        forwarded: None,
        reader: true,
        closed: Arc::new(AtomicBool::new(false)),
        admission: OwnerAdmission(fixture.service.clone()),
    });
    let mut jobs = PeerObjects::new(peer.clone()).unwrap();
    let scope = SyncScope {
        space: fixture.key.space_id.clone(),
        page: crate::readers::test_support::PAGE.into(),
        epoch: "1".into(),
    };
    for i in 1..=8 {
        jobs.submit(
            scope.clone(),
            format!("00000000-0000-4000-8000-{i:012}"),
            ObjectRequest::Config {},
            limit(),
        )
        .unwrap();
    }
    assert_eq!(
        jobs.submit(
            scope.clone(),
            "00000000-0000-4000-8000-000000000001".into(),
            ObjectRequest::Config {},
            limit()
        ),
        Err(Code::Invalid)
    );
    assert_eq!(
        jobs.submit(
            scope,
            "00000000-0000-4000-8000-000000000009".into(),
            ObjectRequest::Config {},
            limit()
        ),
        Err(Code::Capacity)
    );
    jobs.close();
    assert!(peer.closed.load(Ordering::Acquire));
    assert!(
        channel.active(),
        "closing a peer that issued no bus call must preserve the channel"
    );
    assert!(
        locked(&client.0.state).pending.is_empty(),
        "no request crosses an unestablished origin"
    );
    drop(jobs);
    drop(channel);
    assert!(remote.recv(Some(limit())).is_err());
}

#[test]
fn object_rpc_grammar_rejects_duplicate_fields_and_browser_authority_claims() {
    use crate::sync::wire::Frame;
    let prefix = format!(
        r#"{{"version":1,"type":"object","space":"s","page":"{GENERATION}","epoch":"1","requestId":"{GENERATION}","request":"#
    );
    for request in [
        r#"{"method":"config","namespace":"forged"}"#,
        r#"{"method":"config","principal":"owner"}"#,
        r#"{"method":"config","method":"config"}"#,
        r#"{"method":"read","selector":{},"offset":0,"count":32768,"committed":true}"#,
    ] {
        assert!(serde_json::from_str::<Frame>(&format!("{prefix}{request}}}")).is_err());
    }
    assert!(serde_json::from_str::<Frame>(&format!("{prefix}{{\"method\":\"config\"}}}}")).is_ok());
}

fn with_object_source(fixture: &mut crate::readers::test_support::Fixture) {
    use crate::{
        keyring::{Keyring, Layout},
        registration::{OwnerAdmission, Registration},
        store::Store,
        sync::Server,
    };
    let layout = Layout::existing(&fixture.root).unwrap().unwrap();
    let root = fixture.root.clone();
    let clock = fixture.clock.clone();
    let program = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tmt-colab");
    fixture.service = Arc::new(Mutex::new(
        Registration::with_decoder_config(
            Store::open(&layout).unwrap(),
            Keyring::read(&layout).unwrap(),
            crate::readers::test_support::decoder_config(program.clone()),
        )
        .unwrap()
        .with_reader_clock(move || Ok(clock.load(Ordering::SeqCst)))
        .with_save_source(Arc::new(move || {
            crate::page::save::open_source(
                &root,
                crate::readers::test_support::decoder_config(program.clone()),
            )
        })),
    ));
    fixture.server = Server::new(
        Store::open(&layout).unwrap(),
        OwnerAdmission(fixture.service.clone()),
    );
}

#[test]
fn detached_history_pages_ciphertext_under_current_reader_entitlement_and_cancels_on_change() {
    use crate::{
        readers::test_support::{Fixture, PAGE},
        registration::OwnerAdmission,
        transitions::{OwnerAction, ShareMode},
    };
    let mut fixture = Fixture::new();
    with_object_source(&mut fixture);
    fixture.share(ShareMode::Link);
    fixture.add_link();
    fixture.apply(OwnerAction::EpochAdvance { page: PAGE });
    let ticket = fixture.session(true);
    let principal = fixture.upgrade(&ticket).unwrap();
    let scope = fixture.scope();
    let (channel, remote) = pair();
    let origin = Uuid4::parse("22222222-2222-4222-8222-222222222222").unwrap();
    notice(&channel, &remote, origin, OriginPhase::Established);
    let peer = Arc::new(admission::PeerIdentity {
        client: channel.client(),
        origin: Origin::Mounted(origin),
        principal,
        forwarded: None,
        reader: true,
        closed: Arc::new(AtomicBool::new(false)),
        admission: OwnerAdmission(fixture.service.clone()),
    });
    let before = crate::page::revision(&fixture.store, &fixture.key, PAGE).unwrap();
    let mut history =
        history::History::new(peer.clone(), scope.clone(), GENERATION.into(), "1", limit())
            .unwrap();
    let mut finished = false;
    let mut frames = Vec::new();
    for _ in 0..64 {
        let (value, done) = history.next().unwrap();
        assert_eq!(value["ok"]["frame"]["epoch"], "1");
        assert_eq!(value["ok"]["more"], !done);
        frames.push(value);
        if done {
            finished = true;
            break;
        }
    }
    assert!(
        finished,
        "historical paging did not finish within fixture cap"
    );
    assert!(
        frames
            .iter()
            .any(|frame| frame["ok"]["frame"]["type"] == "catchup")
    );
    assert_eq!(
        crate::page::revision(&fixture.store, &fixture.key, PAGE).unwrap(),
        before
    );
    let mut held =
        history::History::new(peer.clone(), scope.clone(), GENERATION.into(), "1", limit())
            .unwrap();
    fixture.share(ShareMode::Private);
    assert!(
        held.next().is_err(),
        "changed sharing must suppress old ciphertext"
    );
    assert!(history::History::new(peer.clone(), scope, GENERATION.into(), "1", limit()).is_err());
    drop(history);
    drop(held);
    drop(peer);
    drop(channel);
    assert!(remote.recv(Some(limit())).is_err());
}

#[test]
fn history_cannot_borrow_root_keys_for_a_current_only_reader() {
    use crate::{
        readers::test_support::{Fixture, PAGE},
        registration::OwnerAdmission,
        transitions::{HistoryMode, OwnerAction, ShareMode},
    };
    let mut fixture = Fixture::new();
    with_object_source(&mut fixture);
    fixture.share(ShareMode::Link);
    fixture.apply(OwnerAction::History {
        page: PAGE,
        mode: HistoryMode::Current,
    });
    fixture.apply(OwnerAction::EpochAdvance { page: PAGE });
    fixture.add_link();
    let ticket = fixture.session(true);
    let principal = fixture.upgrade(&ticket).unwrap();
    let (channel, remote) = pair();
    let origin = Uuid4::parse("22222222-2222-4222-8222-222222222222").unwrap();
    notice(&channel, &remote, origin, OriginPhase::Established);
    let peer = Arc::new(admission::PeerIdentity {
        client: channel.client(),
        origin: Origin::Mounted(origin),
        principal,
        forwarded: None,
        reader: true,
        closed: Arc::new(AtomicBool::new(false)),
        admission: OwnerAdmission(fixture.service.clone()),
    });
    assert!(peer.capture_scope(&fixture.scope(), false).is_ok());
    assert!(history::History::new(peer, fixture.scope(), GENERATION.into(), "1", limit()).is_err());
    drop(channel);
    assert!(remote.recv(Some(limit())).is_err());
}

#[test]
fn actual_reader_ticket_keeps_read_identity_under_an_owner_session_origin() {
    use crate::{
        readers::test_support::Fixture, registration::OwnerAdmission, transitions::ShareMode,
    };
    let mut fixture = Fixture::new();
    with_object_source(&mut fixture);
    fixture.share(ShareMode::Public);
    let ticket = fixture.session(false);
    let principal = fixture.upgrade(&ticket).unwrap();
    let scope = fixture.scope();
    let (channel, remote) = pair();
    let origin = Uuid4::parse("22222222-2222-4222-8222-222222222222").unwrap();
    notice(&channel, &remote, origin, OriginPhase::Established);
    // A paired cookie may accompany a reader ticket. The reader identity wins;
    // this context must not be registered or borrowed as write authority.
    let device = "33333333-3333-4333-8333-333333333333";
    let forwarded = serde_json::json!({"deviceId":device,"kind":"browser","origin":"http://127.0.0.1:1","name":"paired browser","publicKey":tmt_colab_model::values::encode_binary(&[9;32]),"owner":true,"grantRevision":1}).to_string();
    let peer = Arc::new(admission::PeerIdentity {
        client: channel.client(),
        origin: Origin::Mounted(origin),
        principal,
        forwarded: Some(forwarded),
        reader: true,
        closed: Arc::new(AtomicBool::new(false)),
        admission: OwnerAdmission(fixture.service.clone()),
    });
    assert!(peer.capture_scope(&scope, false).is_ok());
    assert!(peer.capture_scope(&scope, true).is_err());
    let mut jobs = PeerObjects::new(peer.clone()).unwrap();
    jobs.submit(scope, GENERATION.into(), ObjectRequest::Config {}, limit())
        .unwrap();
    let request = receive(&remote);
    let Call::Config(input) = &request.call else {
        panic!("config request");
    };
    for (index, boundary) in [Checkpoint::Acquire, Checkpoint::Disclose]
        .into_iter()
        .enumerate()
    {
        remote
            .send(&Frame::Admit(Admit {
                generation: request.generation,
                callback_id: Counter::new(index as u64 + 1).unwrap(),
                request_id: request.request_id,
                boundary,
                context: Context::OwnerSession {
                    origin_id: origin,
                    device_id: Uuid4::parse(device).unwrap(),
                    grant_revision: 1,
                },
                operation: Operation {
                    input: AdmitInput::Config(input.clone()),
                    disclosure: if boundary == Checkpoint::Disclose {
                        Some(Disclosure::Config {
                            projection: tmt_extension_objects::Projection::Browser,
                        })
                    } else {
                        None
                    },
                },
            }))
            .unwrap();
        let Frame::Admission(answer) = remote.recv(Some(limit())).unwrap() else {
            panic!("admission");
        };
        assert_eq!(answer.decision, Decision::Allow);
    }
    remote
        .send(&Frame::Result(ResultFrame {
            generation: request.generation,
            request_id: request.request_id,
            method: request.call.method(),
            transfer_id: None,
            outcome: Outcome::Success(Success::Config(tmt_extension_objects::Config {
                backend_id: "local-fs".into(),
                immutable_create: true,
                chunked_read: true,
                recover_by_original_id: true,
                limits: tmt_extension_objects::Limits::Browser(
                    tmt_extension_objects::TransferBounds {
                        payload_bytes: 12 * 1024 * 1024,
                        chunk_bytes: 32_768,
                    },
                ),
            })),
        }))
        .unwrap();
    // Blocking on the worker's bounded reply is a deterministic completion signal.
    let reply = jobs.replies.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(reply.value["ok"]["result"], "config");
    assert_eq!(reply.value["ok"]["projection"], "browser");
    assert_eq!(reply.value["ok"]["backend"]["id"], "local-fs");
    assert!(reply.fence.unwrap().current().is_ok());
    jobs.close();
    drop(channel);
    assert!(remote.recv(Some(limit())).is_err());
}
