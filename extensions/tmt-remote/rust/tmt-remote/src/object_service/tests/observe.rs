//! Real admitted observations, with originals seeded through the delivered backend.
use super::*;
use crate::objects::Milestone;
use crate::objects::{BackendError, BeginSpec, BlobKey, NamespaceId, OpaqueKey, TransferState};
use sha2::{Digest as _, Sha256};
use tmt_extension_objects::State as WireState;

pub(super) fn transfer() -> Uuid4 {
    Uuid4::parse("5c1f0a3e-9d7b-4c2a-8f61-3b0e7d5a9c24").unwrap()
}
pub(super) fn spec(context: Context, tag: u8, bytes: &[u8]) -> BeginSpec {
    BeginSpec {
        intent: super::super::original::original_id(
            &ExtensionId::new("alpha").unwrap(),
            context,
            transfer(),
        ),
        key: BlobKey {
            namespace: NamespaceId([tag; 32]),
            object: OpaqueKey([9; 32]),
        },
        payload_sha256: Sha256::digest(bytes).into(),
        payload_bytes: bytes.len() as u64,
        binding: vec![1, 2, 3],
    }
}
pub(super) fn status_input(spec: &BeginSpec) -> Call {
    Call::Status(StatusInput {
        transfer_id: transfer(),
        namespace: Bytes32::from_bytes(spec.key.namespace.0),
        policy: Policy::new(spec.binding.clone()).unwrap(),
    })
}
pub(super) fn read_input(spec: &BeginSpec, offset: u64, count: u32) -> Call {
    Call::Read(ReadInput {
        namespace: Bytes32::from_bytes(spec.key.namespace.0),
        opaque_key: Bytes32::from_bytes(spec.key.object.0),
        policy: Policy::new(vec![99]).unwrap(),
        payload_sha256: Sha256Hex::from_bytes(spec.payload_sha256),
        payload_bytes: spec.payload_bytes,
        offset,
        count,
    })
}
pub(super) fn seed(service: &ObjectService<'_>, spec: &BeginSpec, bytes: &[u8], committed: bool) {
    let backend = service.handle("alpha").unwrap();
    backend.begin(spec, &io()).unwrap();
    for (index, chunk) in bytes.chunks(32_768).enumerate() {
        backend
            .append(spec.intent, u32::try_from(index).unwrap(), chunk, &io())
            .unwrap();
    }
    if committed {
        backend.commit(spec.intent, &io()).unwrap();
    }
}
pub(super) fn allowed(peer: &Peer, id: u64, call: Call) -> (Disclosure, Outcome) {
    request(peer, id, Origin::LocalExtension, call.clone());
    let acquire = callback(peer);
    assert_eq!(
        (acquire.boundary, acquire.context),
        (Checkpoint::Acquire, Context::LocalExtension)
    );
    assert_eq!(acquire.operation.disclosure, None);
    assert_eq!(
        acquire.operation.input,
        match call {
            Call::Status(x) => AdmitInput::Status(x),
            Call::Read(x) => AdmitInput::Read(x),
            _ => panic!("observation"),
        }
    );
    decide(peer, acquire.callback_id.get(), id, Decision::Allow);
    let disclose = callback(peer);
    assert_eq!(disclose.boundary, Checkpoint::Disclose);
    let disclosure = disclose.operation.disclosure.unwrap();
    decide(peer, disclose.callback_id.get(), id, Decision::Allow);
    (disclosure, result(peer).outcome)
}
fn view(service: &ObjectService<'_>) -> std::sync::Weak<crate::objects::LocalObjectReader> {
    service.locked().slots[0].active.as_ref().unwrap().view()
}

pub(super) fn payload_snapshot(directory: &Path) -> Vec<(String, Vec<u8>)> {
    tree(directory)
        .into_iter()
        .filter_map(|name| {
            let path = directory.join(&name);
            path.is_file().then(|| (name, fs::read(path).unwrap()))
        })
        .collect()
}

#[test]
fn original_observation_pending_committed_and_terminal_is_read_only() {
    for mode in 0..7 {
        let env = Env::new();
        let now = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let tick = Arc::clone(&now);
        let service = ObjectService::open(
            &env.serving,
            &ALPHA,
            Quotas::contract(),
            Arc::new(move || tick.load(Ordering::Acquire)),
            bounds(),
            Origins::default(),
            &io(),
        )
        .unwrap()
        .unwrap();
        let peer = Peer::start(&env, "alpha", Answer::Expect);
        service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
        let original = spec(Context::LocalExtension, 7, b"hello");
        if mode != 6 {
            seed(&service, &original, b"hello", mode == 1 || mode == 4);
        }
        let backend = service.handle("alpha").unwrap();
        match mode {
            2 => now.store(u64::MAX, Ordering::Release),
            3 => backend.discard(original.intent, &io()).unwrap(),
            4 => backend
                .remove_namespace(original.key.namespace, &io())
                .unwrap(),
            5 => {
                service
                    .storage
                    .observe(Arc::new(|point| point != Milestone::CommitAdopted));
                assert_eq!(
                    backend.commit(original.intent, &io()),
                    Err(BackendError::Unavailable)
                );
            }
            _ => {}
        }
        let before = fs::read(env.ledger()).unwrap();
        let files = payload_snapshot(&env.directory("alpha"));
        let observed = backend.status(original.intent, &io()).unwrap();
        let (disclosure, outcome) = allowed(&peer, 1, status_input(&original));
        match observed.state {
            TransferState::Pending(progress) => {
                let expires_at_ms = Some(observed.original.unwrap().expires_at_ms);
                assert_eq!(
                    disclosure,
                    Disclosure::Status {
                        next_index: progress.next_index,
                        received: progress.received,
                        expires_at_ms
                    }
                );
                assert_eq!(
                    outcome,
                    Outcome::Success(Success::Pending {
                        next_index: progress.next_index,
                        received: progress.received,
                        expires_at_ms
                    })
                );
            }
            TransferState::Committed(receipt) => assert_eq!(
                outcome,
                Outcome::Success(Success::Committed {
                    opaque_key: Bytes32::from_bytes(receipt.key.object.0),
                    payload_sha256: Sha256Hex::from_bytes(receipt.payload_sha256),
                    payload_bytes: receipt.payload_bytes
                })
            ),
            state => {
                let expected = match state {
                    TransferState::Expired => WireState::Expired,
                    TransferState::Discarded => WireState::Discarded,
                    TransferState::Unavailable => WireState::Unavailable,
                    TransferState::Unknown => WireState::Unknown,
                    TransferState::NotObserved => WireState::NotObserved,
                    _ => unreachable!(),
                };
                assert_eq!(disclosure, Disclosure::Terminal { state: expected });
                assert_eq!(outcome, Outcome::Success(Success::State(expected)));
            }
        }
        assert_eq!(
            fs::read(env.ledger()).unwrap(),
            before,
            "status changes no ledger bytes, mode {mode}"
        );
        assert_eq!(payload_snapshot(&env.directory("alpha")), files);
    }
}

#[test]
fn original_observation_denies_changed_namespace_or_frozen_policy() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    seed(&service, &original, b"hello", true);
    for id in 1..=2 {
        let Call::Status(mut input) = status_input(&original) else {
            unreachable!()
        };
        if id == 1 {
            input.namespace = Bytes32::from_bytes([8; 32]);
        } else {
            input.policy = Policy::new(vec![4]).unwrap();
        }
        request(&peer, id, Origin::LocalExtension, Call::Status(input));
        let acquire = callback(&peer);
        decide(&peer, acquire.callback_id.get(), id, Decision::Allow);
        assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Denied));
    }
}

#[test]
fn admitted_raw_reads_are_bounded_and_check_digest_length_and_presence() {
    for bytes in [vec![], b"hello".to_vec(), vec![0xa5; 40_000]] {
        let env = Env::new();
        let (service, peer) = running(&env, bounds());
        let original = spec(Context::LocalExtension, 7, &bytes);
        seed(&service, &original, &bytes, true);
        for (id, offset, count) in [(1, 0, 32_768), (2, if bytes.is_empty() { 0 } else { 1 }, 3)] {
            let (disclosure, outcome) = allowed(&peer, id, read_input(&original, offset, count));
            let start = usize::try_from(offset).unwrap();
            let end = bytes.len().min(start + count as usize);
            assert_eq!(
                disclosure,
                Disclosure::Bytes {
                    offset,
                    length: u32::try_from(end - start).unwrap()
                }
            );
            assert_eq!(
                outcome,
                Outcome::Success(Success::Read {
                    offset,
                    total_bytes: bytes.len() as u64,
                    bytes: Chunk::new(bytes[start..end].to_vec()).unwrap()
                })
            );
        }
        for id in 3..=5 {
            let Call::Read(mut input) = read_input(&original, 0, 5) else {
                unreachable!()
            };
            let expected = match id {
                3 => {
                    input.payload_sha256 = Sha256Hex::from_bytes([0; 32]);
                    ErrorCode::Denied
                }
                4 => {
                    input.payload_bytes += 1;
                    ErrorCode::Denied
                }
                5 => {
                    input.opaque_key = Bytes32::from_bytes([6; 32]);
                    ErrorCode::NotFound
                }
                _ => {
                    input.offset = input.payload_bytes + 1;
                    ErrorCode::Invalid
                }
            };
            request(&peer, id, Origin::LocalExtension, Call::Read(input));
            let acquire = callback(&peer);
            decide(&peer, acquire.callback_id.get(), id, Decision::Allow);
            assert_eq!(result(&peer).outcome, Outcome::Failure(expected));
        }
    }
}

#[test]
fn observations_never_cache_an_allow_or_disclose_after_a_refusal() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    seed(&service, &original, b"hello", true);
    for (id, at_disclose, decision) in [
        (1, false, Decision::Deny),
        (2, false, Decision::Unavailable),
        (3, true, Decision::Deny),
        (4, true, Decision::Unavailable),
    ] {
        request(
            &peer,
            id,
            Origin::LocalExtension,
            read_input(&original, 0, 5),
        );
        let acquire = callback(&peer);
        let refused = if at_disclose {
            decide(&peer, acquire.callback_id.get(), id, Decision::Allow);
            callback(&peer)
        } else {
            acquire
        };
        decide(&peer, refused.callback_id.get(), id, decision);
        assert_eq!(
            result(&peer).outcome,
            Outcome::Failure(if decision == Decision::Deny {
                ErrorCode::Denied
            } else {
                ErrorCode::Unavailable
            })
        );
    }
    assert!(matches!(
        allowed(&peer, 5, read_input(&original, 0, 5)).1,
        Outcome::Success(Success::Read { .. })
    ));
}

#[test]
fn observation_shutdown_cancels_an_in_flight_read_and_releases_every_view() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    seed(&service, &original, b"hello", true);
    let weak = view(&service);
    let hub = service.locked().slots[0].active.as_ref().unwrap().hub();
    let (seen, entered) = mpsc::channel();
    service.storage.observe(Arc::new(move |point| {
        if point == Milestone::ReadBytes {
            let _ = seen.send(());
            let limit = Instant::now() + Duration::from_secs(5);
            while hub
                .upgrade()
                .is_some_and(|hub| !hub.stop.load(Ordering::Acquire))
                && Instant::now() < limit
            {
                thread::yield_now();
            }
        }
        true
    }));
    request(
        &peer,
        1,
        Origin::LocalExtension,
        read_input(&original, 0, 5),
    );
    let acquire = callback(&peer);
    decide(&peer, acquire.callback_id.get(), 1, Decision::Allow);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    service.shutdown();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(weak.upgrade().is_none());
    with_bus(&peer, |bus| {
        assert!(
            bus.recv(Some(Instant::now() + Duration::from_secs(5)))
                .is_err()
        )
    });
}

#[test]
fn observation_drop_releases_the_view_before_a_second_serving_lease() {
    let mut env = Env::new();
    let (service, peer) = running(&env, bounds());
    let weak = view(&service);
    drop(service);
    assert!(weak.upgrade().is_none());
    drop(peer);
    // Move the original lease out without moving out of a Drop type. The replacement
    // is unrelated; the second lock is taken on the exact original directory.
    let replacement = Layout::open(&env.root.join("replacement"))
        .unwrap()
        .serve_lock()
        .unwrap();
    let original = std::mem::replace(&mut env.serving, replacement);
    drop(original);
    let second = Layout::open(&env.root).unwrap().serve_lock().unwrap();
    drop(second);
}

#[test]
fn observation_metadata_change_during_disclose_suppresses_the_snapshot() {
    let env = Env::new();
    let now = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let tick = Arc::clone(&now);
    let service = ObjectService::open(
        &env.serving,
        &ALPHA,
        Quotas::contract(),
        Arc::new(move || tick.load(Ordering::Acquire)),
        bounds(),
        Origins::default(),
        &io(),
    )
    .unwrap()
    .unwrap();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    let original = spec(Context::LocalExtension, 7, b"hello");
    seed(&service, &original, b"hello", false);
    request(&peer, 1, Origin::LocalExtension, status_input(&original));
    let acquire = callback(&peer);
    decide(&peer, acquire.callback_id.get(), 1, Decision::Allow);
    let disclose = callback(&peer);
    now.store(u64::MAX, Ordering::Release);
    decide(&peer, disclose.callback_id.get(), 1, Decision::Allow);
    assert_eq!(
        result(&peer).outcome,
        Outcome::Failure(ErrorCode::Unavailable)
    );
    // A separate publication disappears while its read disclosure is admitted.
    let original = BeginSpec {
        intent: crate::objects::IntentId([5; 32]),
        ..spec(Context::LocalExtension, 8, b"world")
    };
    now.store(1, Ordering::Release);
    seed(&service, &original, b"world", true);
    request(
        &peer,
        2,
        Origin::LocalExtension,
        read_input(&original, 0, 5),
    );
    let acquire = callback(&peer);
    decide(&peer, acquire.callback_id.get(), 2, Decision::Allow);
    let disclose = callback(&peer);
    service
        .handle("alpha")
        .unwrap()
        .remove_namespace(original.key.namespace, &io())
        .unwrap();
    decide(&peer, disclose.callback_id.get(), 2, Decision::Allow);
    assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::NotFound));
}

/// A reached boundary holds only the first request; its successor has a full budget.
fn hold_first(service: &ObjectService<'_>, boundary: Pause) {
    let first = Arc::new(AtomicBool::new(true));
    service.set_hook(Some(Arc::new(move |pause, deadline| {
        if pause == boundary && first.swap(false, Ordering::AcqRel) {
            while Instant::now() < deadline {
                thread::yield_now();
            }
        }
    })));
}

#[test]
fn observation_absolute_deadline_is_unavailable_without_data_and_channel_lives() {
    let env = Env::new();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    let service = env
        .service(
            &ALPHA,
            ServiceBounds {
                request: Duration::from_millis(400),
                ..bounds()
            },
        )
        .unwrap();
    hold_first(&service, Pause::BeforeResult);
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    let original = spec(Context::LocalExtension, 7, b"hello");
    seed(&service, &original, b"hello", true);
    let ledger = fs::read(env.ledger()).unwrap();
    let payload = payload_snapshot(&env.directory("alpha"));
    request(
        &peer,
        1,
        Origin::LocalExtension,
        read_input(&original, 0, 5),
    );
    let acquire = callback(&peer);
    decide(&peer, acquire.callback_id.get(), 1, Decision::Allow);
    let disclose = callback(&peer);
    decide(&peer, disclose.callback_id.get(), 1, Decision::Allow);
    let spent = result(&peer);
    assert_eq!(spent.request_id, counter(1));
    assert_eq!(spent.outcome, Outcome::Failure(ErrorCode::Unavailable));
    assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
    assert_eq!(payload_snapshot(&env.directory("alpha")), payload);
    let (_, next) = allowed(&peer, 2, read_input(&original, 0, 5));
    assert!(
        matches!(next, Outcome::Success(Success::Read { bytes, .. }) if bytes.as_bytes() == b"hello")
    );
    assert_eq!(service.ended("alpha"), None);
}

#[test]
fn observation_spent_mounted_denial_is_unavailable_and_channel_lives() {
    let env = Env::new();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    let service = env
        .service(
            &ALPHA,
            ServiceBounds {
                request: Duration::from_millis(400),
                ..bounds()
            },
        )
        .unwrap();
    hold_first(&service, Pause::BeforeRequest);
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    let before = fs::read(env.ledger()).unwrap();
    let payload = payload_snapshot(&env.directory("alpha"));
    let original = spec(Context::LocalExtension, 7, b"hello");
    request(
        &peer,
        1,
        Origin::Mounted(transfer()),
        status_input(&original),
    );
    // No callback or denied/data frame may renew this spent request budget.
    let spent = result(&peer);
    assert_eq!(spent.request_id, counter(1));
    assert_eq!(spent.outcome, Outcome::Failure(ErrorCode::Unavailable));
    assert_eq!(fs::read(env.ledger()).unwrap(), before);
    assert_eq!(payload_snapshot(&env.directory("alpha")), payload);
    let (_, next) = allowed(&peer, 2, status_input(&original));
    assert_eq!(
        next,
        Outcome::Success(Success::State(WireState::NotObserved))
    );
    assert_eq!(service.ended("alpha"), None);
}

#[test]
fn observation_callback_spent_before_write_is_unavailable_and_channel_lives() {
    let env = Env::new();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    let service = env
        .service(
            &ALPHA,
            ServiceBounds {
                request: Duration::from_millis(400),
                ..bounds()
            },
        )
        .unwrap();
    hold_first(&service, Pause::BeforeAdmitWrite);
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    let original = spec(Context::LocalExtension, 7, b"hello");
    let before = fs::read(env.ledger()).unwrap();
    let payload = payload_snapshot(&env.directory("alpha"));
    request(&peer, 1, Origin::LocalExtension, status_input(&original));
    // The result is the first frame: the pre-admission refusal sent no callback.
    let spent = result(&peer);
    assert_eq!(spent.request_id, counter(1));
    assert_eq!(spent.outcome, Outcome::Failure(ErrorCode::Unavailable));
    assert_eq!(fs::read(env.ledger()).unwrap(), before);
    assert_eq!(payload_snapshot(&env.directory("alpha")), payload);
    let (_, next) = allowed(&peer, 2, status_input(&original));
    assert_eq!(
        next,
        Outcome::Success(Success::State(WireState::NotObserved))
    );
    assert_eq!(service.ended("alpha"), None);
}
