//! Uploads over the actual bus and retained backend; no mock mutation algorithm.
use super::observe::{allowed, payload_snapshot, spec, status_input, transfer};
use super::*;
use crate::objects::{BeginSpec, Milestone, TransferState};
use std::sync::atomic::AtomicU64;
use tmt_extension_objects::{PartInput, Retained, State};

pub(super) fn begin(spec: &BeginSpec) -> Call {
    Call::Begin(BeginInput {
        transfer_id: transfer(),
        namespace: Bytes32::from_bytes(spec.key.namespace.0),
        opaque_key: Bytes32::from_bytes(spec.key.object.0),
        policy: Policy::new(spec.binding.clone()).unwrap(),
        payload_sha256: Sha256Hex::from_bytes(spec.payload_sha256),
        payload_bytes: spec.payload_bytes,
    })
}
pub(super) fn part(bytes: &[u8], index: u32) -> Call {
    Call::Part(PartInput {
        transfer_id: transfer(),
        index,
        bytes: Chunk::new(bytes.to_vec()).unwrap(),
    })
}
pub(super) fn commit() -> Call {
    Call::Commit(TransferInput {
        transfer_id: transfer(),
    })
}
pub(super) fn discard() -> Call {
    Call::Discard(TransferInput {
        transfer_id: transfer(),
    })
}
fn retained(spec: &BeginSpec) -> Retained {
    Retained {
        namespace: Bytes32::from_bytes(spec.key.namespace.0),
        opaque_key: Bytes32::from_bytes(spec.key.object.0),
        policy: Policy::new(spec.binding.clone()).unwrap(),
        payload_sha256: Sha256Hex::from_bytes(spec.payload_sha256),
        payload_bytes: spec.payload_bytes,
    }
}
fn admitted(peer: &Peer, id: u64, call: Call, spec: &BeginSpec) -> Outcome {
    request(peer, id, Origin::LocalExtension, call.clone());
    for boundary in [
        Checkpoint::Acquire,
        Checkpoint::Effect,
        Checkpoint::Disclose,
    ] {
        let admit = callback(peer);
        assert_eq!(
            (admit.boundary, admit.context, admit.request_id),
            (boundary, Context::LocalExtension, counter(id))
        );
        let expected = match &call {
            Call::Begin(x) => AdmitInput::Begin(x.clone()),
            Call::Part(x) => AdmitInput::Part(tmt_extension_objects::PartAdmit {
                transfer_id: transfer(),
                index: x.index,
                length: u32::try_from(x.bytes.as_bytes().len()).unwrap(),
                retained: retained(spec),
            }),
            Call::Commit(_) => AdmitInput::Commit(tmt_extension_objects::TransferAdmit {
                transfer_id: transfer(),
                retained: retained(spec),
            }),
            Call::Discard(_) => AdmitInput::Discard(tmt_extension_objects::TransferAdmit {
                transfer_id: transfer(),
                retained: retained(spec),
            }),
            _ => panic!("upload"),
        };
        assert_eq!(
            admit.operation.input, expected,
            "stored retained input, never part bytes"
        );
        assert_eq!(
            admit.operation.disclosure.is_some(),
            boundary == Checkpoint::Disclose
        );
        decide(peer, admit.callback_id.get(), id, Decision::Allow);
    }
    let answer = result(peer);
    assert_eq!(
        (answer.request_id, answer.method, answer.transfer_id),
        (counter(id), call.method(), Some(transfer()))
    );
    answer.outcome
}
fn authorize_effect(peer: &Peer, id: u64, call: Call) {
    request(peer, id, Origin::LocalExtension, call);
    for boundary in [Checkpoint::Acquire, Checkpoint::Effect] {
        let admit = callback(peer);
        assert_eq!(admit.boundary, boundary);
        decide(peer, admit.callback_id.get(), id, Decision::Allow);
    }
}
fn pending() -> Outcome {
    Outcome::Success(Success::Pending {
        next_index: 0,
        received: 0,
        expires_at_ms: None,
    })
}

#[test]
fn upload_begin_parts_commit_repeats_and_discard_keep_one_original() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let bytes = vec![0xa5; 40_000];
    let original = spec(Context::LocalExtension, 7, &bytes);
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    let stored = service
        .handle("alpha")
        .unwrap()
        .status(original.intent, &io())
        .unwrap()
        .original
        .unwrap();
    assert_eq!(admitted(&peer, 2, begin(&original), &original), pending());
    for (id, index) in [(3, 0), (4, 0), (5, 1)] {
        let start = index as usize * 32_768;
        let chunk = &bytes[start..bytes.len().min(start + 32_768)];
        let next = if index == 0 { 1 } else { 2 };
        assert_eq!(
            admitted(&peer, id, part(chunk, index), &original),
            Outcome::Success(Success::Progress {
                next_index: next,
                received: if index == 0 { 32_768 } else { 40_000 }
            })
        );
    }
    let expected = Outcome::Success(Success::Committed {
        opaque_key: Bytes32::from_bytes(original.key.object.0),
        payload_sha256: Sha256Hex::from_bytes(original.payload_sha256),
        payload_bytes: 40_000,
    });
    for (id, call) in [(6, commit()), (7, commit()), (8, begin(&original))] {
        assert_eq!(admitted(&peer, id, call, &original), expected);
    }
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .read(original.key, 32_768, 32_768, &io())
            .unwrap()
            .bytes,
        bytes[32_768..]
    );
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .status(original.intent, &io())
            .unwrap()
            .original
            .unwrap(),
        stored
    );
    // Reopening settles once and returns the same original receipt.
    drop(service);
    peer.buses.lock().unwrap().clear();
    let service = env.service(&ALPHA, bounds()).unwrap();
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    assert_eq!(admitted(&peer, 1, begin(&original), &original), expected);
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .status(original.intent, &io())
            .unwrap()
            .original
            .unwrap(),
        stored
    );
}

#[test]
fn upload_discard_retains_terminal_identity_and_releases_only_payload_charge() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    let before = service.handle("alpha").unwrap().usage(None, &io()).unwrap();
    assert_eq!(
        admitted(&peer, 2, discard(), &original),
        Outcome::Success(Success::State(State::Discarded))
    );
    let after = service.handle("alpha").unwrap().usage(None, &io()).unwrap();
    assert_eq!(
        (
            after.entries,
            after.active_uploads,
            after.retained_identities
        ),
        (0, 0, 1)
    );
    assert!(after.charged_bytes < before.charged_bytes);
    assert_eq!(
        admitted(&peer, 3, begin(&original), &original),
        Outcome::Success(Success::State(State::Discarded))
    );
    assert_eq!(
        admitted(&peer, 4, discard(), &original),
        Outcome::Success(Success::State(State::Discarded))
    );
}

#[test]
fn upload_acquire_and_effect_refusals_allocate_nothing_and_channel_lives() {
    for boundary in [Checkpoint::Acquire, Checkpoint::Effect] {
        for decision in [Decision::Deny, Decision::Unavailable] {
            let env = Env::new();
            let (service, peer) = running(&env, bounds());
            let original = spec(Context::LocalExtension, 7, b"hello");
            let ledger = fs::read(env.ledger()).unwrap();
            let files = payload_snapshot(&env.directory("alpha"));
            request(&peer, 1, Origin::LocalExtension, begin(&original));
            let acquire = callback(&peer);
            if boundary == Checkpoint::Effect {
                decide(&peer, acquire.callback_id.get(), 1, Decision::Allow);
                let effect = callback(&peer);
                assert_eq!(effect.boundary, Checkpoint::Effect);
                decide(&peer, effect.callback_id.get(), 1, decision);
            } else {
                decide(&peer, acquire.callback_id.get(), 1, decision);
            }
            assert_eq!(
                result(&peer).outcome,
                Outcome::Failure(if decision == Decision::Deny {
                    ErrorCode::Denied
                } else {
                    ErrorCode::Unavailable
                })
            );
            assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
            assert_eq!(payload_snapshot(&env.directory("alpha")), files);
            assert_eq!(admitted(&peer, 2, begin(&original), &original), pending());
            assert_eq!(service.ended("alpha"), None);
        }
    }
}

#[test]
fn upload_disclose_refusal_is_unknown_and_keeps_charge_and_successor() {
    for decision in [Decision::Deny, Decision::Unavailable] {
        let env = Env::new();
        let (service, peer) = running(&env, bounds());
        let original = spec(Context::LocalExtension, 7, b"hello");
        authorize_effect(&peer, 1, begin(&original));
        let disclose = callback(&peer);
        assert_eq!(disclose.boundary, Checkpoint::Disclose);
        let before = service.handle("alpha").unwrap().usage(None, &io()).unwrap();
        decide(&peer, disclose.callback_id.get(), 1, decision);
        assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Unknown));
        assert_eq!(
            service.handle("alpha").unwrap().usage(None, &io()).unwrap(),
            before
        );
        assert_eq!(
            (
                before.entries,
                before.active_uploads,
                before.retained_identities
            ),
            (1, 1, 1)
        );
        assert!(matches!(
            allowed(&peer, 2, status_input(&original)).1,
            Outcome::Success(Success::Pending {
                expires_at_ms: Some(_),
                ..
            })
        ));
        assert_eq!(service.ended("alpha"), None);
    }
}

#[test]
fn upload_changed_frozen_begin_and_absent_scoped_original_never_mutate() {
    let env = Env::new();
    let (_service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    let ledger = fs::read(env.ledger()).unwrap();
    for (index, changed) in [
        BeginSpec {
            binding: vec![4],
            ..original.clone()
        },
        BeginSpec {
            key: crate::objects::BlobKey {
                namespace: crate::objects::NamespaceId([8; 32]),
                ..original.key
            },
            ..original.clone()
        },
        BeginSpec {
            payload_bytes: 6,
            ..original.clone()
        },
        BeginSpec {
            payload_sha256: [4; 32],
            ..original.clone()
        },
        BeginSpec {
            key: crate::objects::BlobKey {
                object: crate::objects::OpaqueKey([8; 32]),
                ..original.key
            },
            ..original.clone()
        },
    ]
    .into_iter()
    .enumerate()
    {
        request(
            &peer,
            index as u64 + 2,
            Origin::LocalExtension,
            begin(&changed),
        );
        assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Conflict));
    }
    assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
    let absent = Uuid4::parse("00000000-0000-4000-8000-000000000011").unwrap();
    for (id, call) in [
        (
            7,
            Call::Part(PartInput {
                transfer_id: absent,
                index: 0,
                bytes: Chunk::new(b"hello".to_vec()).unwrap(),
            }),
        ),
        (
            8,
            Call::Commit(TransferInput {
                transfer_id: absent,
            }),
        ),
        (
            9,
            Call::Discard(TransferInput {
                transfer_id: absent,
            }),
        ),
    ] {
        request(&peer, id, Origin::LocalExtension, call);
        assert_eq!(
            result(&peer).outcome,
            Outcome::Failure(ErrorCode::Unavailable)
        );
    }
    assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
}

#[test]
fn upload_backend_invalid_conflict_capacity_and_possible_effect_mapping() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    for (id, call, expected) in [
        (2, part(b"bad", 0), ErrorCode::Invalid),
        (3, part(b"hello", 1), ErrorCode::Conflict),
        (4, commit(), ErrorCode::Conflict),
    ] {
        authorize_effect(&peer, id, call);
        assert_eq!(result(&peer).outcome, Outcome::Failure(expected));
    }
    service
        .storage
        .observe(Arc::new(|point| point != Milestone::ChunkSynced));
    authorize_effect(&peer, 5, part(b"hello", 0));
    assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Unknown));
    assert!(matches!(
        service
            .handle("alpha")
            .unwrap()
            .status(original.intent, &io())
            .unwrap()
            .state,
        TransferState::Pending(crate::objects::Progress {
            next_index: 0,
            received: 0,
            ..
        })
    ));
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .active_uploads,
        1
    );
    let name = original
        .intent
        .0
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let staging = env.directory("alpha").join("objects/staging").join(name);
    assert_eq!(
        fs::read(&staging).unwrap(),
        b"hello",
        "synced but unacknowledged tail exists"
    );
    // Quota saturation maps the backend's exact class, not an invented no-effect attestation.
    drop(service);
    peer.buses.lock().unwrap().clear();
    let service = ObjectService::open(
        &env.serving,
        &ALPHA,
        Quotas {
            active_intents: 1,
            ..Quotas::contract()
        },
        system_clock(),
        bounds(),
        Origins::default(),
        &io(),
    )
    .unwrap()
    .unwrap();
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    assert!(
        fs::read(&staging).unwrap().is_empty(),
        "open settles the same original before readiness"
    );
    assert!(matches!(
        service
            .handle("alpha")
            .unwrap()
            .status(original.intent, &io())
            .unwrap()
            .state,
        TransferState::Pending(crate::objects::Progress {
            next_index: 0,
            received: 0,
            ..
        })
    ));
    let mut other = begin(&original);
    if let Call::Begin(x) = &mut other {
        x.transfer_id = Uuid4::parse("00000000-0000-4000-8000-000000000011").unwrap();
        x.opaque_key = Bytes32::from_bytes([10; 32]);
    }
    authorize_effect(&peer, 1, other);
    assert_eq!(
        result(&peer).outcome,
        Outcome::Failure(ErrorCode::Capacity(
            tmt_extension_objects::Limit::ActiveIntents
        ))
    );
}

#[test]
fn upload_original_expiry_is_frozen_and_clips_effect_callback_without_renewal() {
    let env = Env::new();
    let now = Arc::new(AtomicU64::new(1));
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
    let armed = Arc::new(AtomicBool::new(false));
    let arm = Arc::clone(&armed);
    let expiry = Arc::new(AtomicU64::new(0));
    let frozen = Arc::clone(&expiry);
    let clock = Arc::clone(&now);
    let reached = Arc::new(AtomicBool::new(false));
    let marked = Arc::clone(&reached);
    service.set_hook(Some(Arc::new(move |pause, deadline| {
        if pause == Pause::BeforeAdmitWrite && arm.load(Ordering::Acquire) {
            assert!(deadline.saturating_duration_since(Instant::now()) <= Duration::from_secs(2),
                "callback uses the original's remaining second, not the fresh request/callback budget");
            marked.store(true, Ordering::Release);
            clock.store(frozen.load(Ordering::Acquire), Ordering::Release);
        }
    })));
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    let original = spec(Context::LocalExtension, 7, b"hello");
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    let stored = service
        .handle("alpha")
        .unwrap()
        .status(original.intent, &io())
        .unwrap()
        .original
        .unwrap();
    now.store(stored.expires_at_ms - 1_000, Ordering::Release);
    expiry.store(stored.expires_at_ms, Ordering::Release);
    armed.store(true, Ordering::Release);
    request(&peer, 2, Origin::LocalExtension, part(b"hello", 0));
    let acquire = callback(&peer);
    decide(&peer, acquire.callback_id.get(), 2, Decision::Allow);
    assert_eq!(
        result(&peer).outcome,
        Outcome::Failure(ErrorCode::Unavailable)
    );
    assert!(reached.load(Ordering::Acquire));
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .status(original.intent, &io())
            .unwrap()
            .original
            .unwrap(),
        stored
    );
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .active_uploads,
        1
    );
}

#[test]
fn upload_clock_expiry_during_effect_decision_refuses_without_payload() {
    let env = Env::new();
    let now = Arc::new(AtomicU64::new(1));
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
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    let stored = service
        .handle("alpha")
        .unwrap()
        .status(original.intent, &io())
        .unwrap()
        .original
        .unwrap();
    let ledger = fs::read(env.ledger()).unwrap();
    let files = payload_snapshot(&env.directory("alpha"));
    request(&peer, 2, Origin::LocalExtension, part(b"hello", 0));
    let acquire = callback(&peer);
    decide(&peer, acquire.callback_id.get(), 2, Decision::Allow);
    let effect = callback(&peer);
    now.store(stored.expires_at_ms, Ordering::Release);
    decide(&peer, effect.callback_id.get(), 2, Decision::Allow);
    assert_eq!(
        result(&peer).outcome,
        Outcome::Failure(ErrorCode::Unavailable)
    );
    assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
    assert_eq!(payload_snapshot(&env.directory("alpha")), files);
}

#[test]
fn upload_post_effect_deadline_and_result_write_spend_are_unknown_and_channel_lives() {
    for pause in [
        Pause::BetweenAdmissions,
        Pause::BeforeResult,
        Pause::BeforeResultWrite,
        Pause::BeforeAdmitWrite,
    ] {
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
        let count = Arc::new(AtomicUsize::new(0));
        service.set_hook(Some(Arc::new(move |point, deadline| {
            let selected = point == pause
                && (pause != Pause::BeforeAdmitWrite || count.fetch_add(1, Ordering::AcqRel) == 2);
            if selected && count.fetch_add(1_000, Ordering::AcqRel) < 1_000 {
                while Instant::now() < deadline {
                    thread::yield_now();
                }
            }
        })));
        service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
        let original = spec(Context::LocalExtension, 7, b"hello");
        authorize_effect(&peer, 1, begin(&original));
        if matches!(pause, Pause::BeforeResult | Pause::BeforeResultWrite) {
            let disclose = callback(&peer);
            decide(&peer, disclose.callback_id.get(), 1, Decision::Allow);
        }
        assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Unknown));
        assert_eq!(
            service
                .handle("alpha")
                .unwrap()
                .usage(None, &io())
                .unwrap()
                .active_uploads,
            1
        );
        assert!(matches!(
            allowed(&peer, 2, status_input(&original)).1,
            Outcome::Success(Success::Pending { .. })
        ));
        assert_eq!(service.ended("alpha"), None);
    }
}

#[test]
fn upload_post_effect_metadata_change_never_discloses_stale_success() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    authorize_effect(&peer, 1, begin(&original));
    let disclose = callback(&peer);
    service
        .handle("alpha")
        .unwrap()
        .remove_namespace(original.key.namespace, &io())
        .unwrap();
    decide(&peer, disclose.callback_id.get(), 1, Decision::Allow);
    assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Unknown));
    assert_eq!(
        allowed(&peer, 2, status_input(&original)).1,
        Outcome::Success(Success::State(State::Unavailable))
    );
}

#[test]
fn upload_callback_timeout_keeps_original_charged_and_ends_channel() {
    let env = Env::new();
    let (service, peer) = running(
        &env,
        ServiceBounds {
            callback: Duration::from_millis(100),
            ..bounds()
        },
    );
    let original = spec(Context::LocalExtension, 7, b"hello");
    authorize_effect(&peer, 1, begin(&original));
    assert_eq!(callback(&peer).boundary, Checkpoint::Disclose);
    assert!(peer.saw_close(0));
    assert_eq!(ended_soon(&service, "alpha"), ChannelEnd::CallbackTimeout);
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .active_uploads,
        1
    );
}

#[test]
fn upload_shutdown_cancels_mutation_joins_writer_and_reacquires_lease() {
    let mut env = Env::new();
    let (service, peer) = running(&env, bounds());
    let active = service.locked();
    let channel = active.slots[0].active.as_ref().unwrap();
    let writer = channel.writer_view();
    let reader = channel.view();
    let hub = channel.hub();
    drop(active);
    let bus = service.bus("alpha").unwrap();
    let (send, seen) = mpsc::channel();
    service.storage.observe(Arc::new(move |point| {
        if point == Milestone::Adopted {
            let _ = send.send(());
            let limit = Instant::now() + Duration::from_secs(5);
            while hub
                .upgrade()
                .is_some_and(|x| !x.stop.load(Ordering::Acquire))
                && Instant::now() < limit
            {
                thread::yield_now();
            }
        }
        true
    }));
    let original = spec(Context::LocalExtension, 7, b"hello");
    authorize_effect(&peer, 1, begin(&original));
    seen.recv_timeout(Duration::from_secs(5)).unwrap();
    service.shutdown();
    assert!(writer.upgrade().is_none());
    assert!(reader.upgrade().is_none());
    assert!(bus.upgrade().is_none());
    with_bus(&peer, |bus| {
        let frame = bus.recv(Some(Instant::now() + Duration::from_secs(5)));
        if let Ok(Frame::Result(answer)) = frame {
            assert_eq!(answer.outcome, Outcome::Failure(ErrorCode::Unknown));
            assert_eq!(
                bus.recv(Some(Instant::now() + Duration::from_secs(5))),
                Err(Fault::Closed)
            );
        } else {
            assert_eq!(frame, Err(Fault::Closed));
        }
    });
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .active_uploads,
        1
    );
    drop(service);
    drop(peer);
    let replacement = Layout::open(&env.root.join("replacement"))
        .unwrap()
        .serve_lock()
        .unwrap();
    drop(std::mem::replace(&mut env.serving, replacement));
    let second = Layout::open(&env.root).unwrap().serve_lock().unwrap();
    drop(second);
}

#[test]
fn upload_interrupted_commit_restarts_same_original_without_request_reconciliation() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    admitted(&peer, 2, part(b"hello", 0), &original);
    service
        .storage
        .observe(Arc::new(|point| point != Milestone::CommitAdopted));
    authorize_effect(&peer, 3, commit());
    assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Unknown));
    let observed = service
        .handle("alpha")
        .unwrap()
        .status(original.intent, &io())
        .unwrap();
    assert_eq!(observed.state, TransferState::Unknown);
    let charge = service.handle("alpha").unwrap().usage(None, &io()).unwrap();
    let ledger = fs::read(env.ledger()).unwrap();
    assert_eq!(
        allowed(&peer, 4, status_input(&original)).1,
        Outcome::Success(Success::State(State::Unknown))
    );
    assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
    drop(service);
    peer.buses.lock().unwrap().clear();
    let service = env.service(&ALPHA, bounds()).unwrap();
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    let settled = service
        .handle("alpha")
        .unwrap()
        .status(original.intent, &io())
        .unwrap();
    assert_eq!(settled.original, observed.original);
    assert!(matches!(settled.state, TransferState::Committed(_)));
    assert_eq!(
        admitted(&peer, 1, commit(), &original),
        Outcome::Success(Success::Committed {
            opaque_key: Bytes32::from_bytes(original.key.object.0),
            payload_sha256: Sha256Hex::from_bytes(original.payload_sha256),
            payload_bytes: 5
        })
    );
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .retained_identities,
        charge.retained_identities
    );
}

#[test]
fn upload_installed_extension_scopes_cannot_continue_each_other() {
    static PAIR: [Extension; 2] = [
        declared("alpha", ObjectDeclaration::Local),
        declared("beta", ObjectDeclaration::Local),
    ];
    let env = Env::new();
    let alpha = Peer::start(&env, "alpha", Answer::Expect);
    let beta = Peer::start(&env, "beta", Answer::Expect);
    let service = env.service(&PAIR, bounds()).unwrap();
    let mounts = env.mounts(&PAIR);
    service.activate(&mounts, "alpha").unwrap();
    service.activate(&mounts, "beta").unwrap();
    let original = spec(Context::LocalExtension, 7, b"hello");
    assert_eq!(admitted(&alpha, 1, begin(&original), &original), pending());
    request(&beta, 1, Origin::LocalExtension, part(b"hello", 0));
    assert_eq!(
        result(&beta).outcome,
        Outcome::Failure(ErrorCode::Unavailable)
    );
    assert_eq!(
        allowed(&beta, 2, status_input(&original)).1,
        Outcome::Success(Success::State(State::NotObserved))
    );
    assert_eq!(admitted(&beta, 3, begin(&original), &original), pending());
    let other = super::super::original::original_id(
        &ExtensionId::new("beta").unwrap(),
        Context::LocalExtension,
        transfer(),
    );
    assert_ne!(original.intent, other);
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .active_uploads,
        1
    );
    assert_eq!(
        service
            .handle("beta")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .active_uploads,
        1
    );
}

#[test]
fn upload_pre_effect_spend_has_zero_effects_and_keeps_channel() {
    for pause in [Pause::BeforeRequest, Pause::BeforeAdmitWrite] {
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
        let once = AtomicBool::new(true);
        service.set_hook(Some(Arc::new(move |point, deadline| {
            if point == pause && once.swap(false, Ordering::AcqRel) {
                while Instant::now() < deadline {
                    thread::yield_now();
                }
            }
        })));
        service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
        let original = spec(Context::LocalExtension, 7, b"hello");
        let ledger = fs::read(env.ledger()).unwrap();
        let files = payload_snapshot(&env.directory("alpha"));
        request(&peer, 1, Origin::LocalExtension, begin(&original));
        assert_eq!(
            result(&peer).outcome,
            Outcome::Failure(ErrorCode::Unavailable)
        );
        assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
        assert_eq!(payload_snapshot(&env.directory("alpha")), files);
        assert_eq!(admitted(&peer, 2, begin(&original), &original), pending());
        assert_eq!(service.ended("alpha"), None);
    }
}

#[test]
fn upload_each_backend_quota_keeps_its_exact_wire_class() {
    for (index, limit) in [
        tmt_extension_objects::Limit::NamespaceBytes,
        tmt_extension_objects::Limit::ExtensionBytes,
        tmt_extension_objects::Limit::InstallationBytes,
        tmt_extension_objects::Limit::NamespaceEntries,
        tmt_extension_objects::Limit::ExtensionEntries,
        tmt_extension_objects::Limit::InstallationEntries,
        tmt_extension_objects::Limit::ActiveIntents,
        tmt_extension_objects::Limit::RetainedExtension,
        tmt_extension_objects::Limit::RetainedInstallation,
    ]
    .into_iter()
    .enumerate()
    {
        let env = Env::new();
        let mut quotas = Quotas::contract();
        match index {
            0 => quotas.namespace_bytes = 0,
            1 => quotas.extension_bytes = 0,
            2 => quotas.installation_bytes = 0,
            3 => quotas.namespace_entries = 0,
            4 => quotas.extension_entries = 0,
            5 => quotas.installation_entries = 0,
            6 => quotas.active_intents = 0,
            7 => quotas.retained_extension = 0,
            8 => quotas.retained_installation = 0,
            _ => unreachable!(),
        }
        let service = ObjectService::open(
            &env.serving,
            &ALPHA,
            quotas,
            system_clock(),
            bounds(),
            Origins::default(),
            &io(),
        )
        .unwrap()
        .unwrap();
        let peer = Peer::start(&env, "alpha", Answer::Expect);
        service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
        let original = spec(Context::LocalExtension, 7, b"hello");
        authorize_effect(&peer, 1, begin(&original));
        assert_eq!(
            result(&peer).outcome,
            Outcome::Failure(ErrorCode::Capacity(limit)),
            "limit {limit:?}"
        );
        assert_eq!(
            service.handle("alpha").unwrap().usage(None, &io()).unwrap(),
            crate::objects::Usage::default()
        );
    }
}

#[test]
fn upload_drop_joins_both_views_after_a_write_before_lease_release() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    let active = service.locked();
    let channel = active.slots[0].active.as_ref().unwrap();
    let writer = channel.writer_view();
    let reader = channel.view();
    drop(active);
    let bus = service.bus("alpha").unwrap();
    drop(service);
    assert!(writer.upgrade().is_none());
    assert!(reader.upgrade().is_none());
    assert!(bus.upgrade().is_none());
    assert!(peer.saw_close(0));
}

#[test]
fn upload_eight_queued_requests_use_owned_workers_and_fresh_decisions() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    for id in 1..=8 {
        let Call::Begin(mut input) = begin(&original) else {
            unreachable!()
        };
        input.transfer_id =
            Uuid4::parse(&format!("5c1f0a3e-9d7b-4c2a-8f61-3b0e7d5a9c{id:02x}")).unwrap();
        input.opaque_key = Bytes32::from_bytes([id as u8; 32]);
        request(&peer, id, Origin::LocalExtension, Call::Begin(input));
    }
    let first = callback(&peer);
    let second = callback(&peer);
    assert_eq!(first.boundary, Checkpoint::Acquire);
    assert_eq!(second.boundary, Checkpoint::Acquire);
    assert_eq!(
        service.handle("alpha").unwrap().usage(None, &io()).unwrap(),
        crate::objects::Usage::default()
    );
    let mut phases = std::collections::BTreeMap::<u64, Vec<Checkpoint>>::new();
    for admit in [second, first] {
        phases
            .entry(admit.request_id.get())
            .or_default()
            .push(admit.boundary);
        decide(
            &peer,
            admit.callback_id.get(),
            admit.request_id.get(),
            Decision::Allow,
        );
    }
    let mut completed = Vec::new();
    while completed.len() < 8 {
        match heard(&peer) {
            Frame::Admit(admit) => {
                phases
                    .entry(admit.request_id.get())
                    .or_default()
                    .push(admit.boundary);
                decide(
                    &peer,
                    admit.callback_id.get(),
                    admit.request_id.get(),
                    Decision::Allow,
                );
            }
            Frame::Result(answer) => {
                assert_eq!(answer.outcome, pending());
                completed.push(answer.request_id.get());
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    completed.sort_unstable();
    assert_eq!(completed, (1..=8).collect::<Vec<_>>());
    assert!(phases.values().all(|x| *x
        == [
            Checkpoint::Acquire,
            Checkpoint::Effect,
            Checkpoint::Disclose
        ]));
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .usage(None, &io())
            .unwrap()
            .active_uploads,
        8
    );
    assert_eq!(service.ended("alpha"), None);
}

#[test]
fn upload_zero_payload_commits_without_parts_and_changed_part_conflicts() {
    for bytes in [b"".as_slice(), b"hello".as_slice()] {
        let env = Env::new();
        let (service, peer) = running(&env, bounds());
        let original = spec(Context::LocalExtension, 7, bytes);
        assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
        if !bytes.is_empty() {
            assert_eq!(
                admitted(&peer, 2, part(bytes, 0), &original),
                Outcome::Success(Success::Progress {
                    next_index: 1,
                    received: 5
                })
            );
            authorize_effect(&peer, 3, part(b"other", 0));
            assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Conflict));
        }
        assert!(
            matches!(admitted(&peer,4,commit(),&original),Outcome::Success(Success::Committed {payload_bytes,..}) if payload_bytes==bytes.len() as u64)
        );
        assert_eq!(
            service
                .handle("alpha")
                .unwrap()
                .read(original.key, 0, 32_768, &io())
                .unwrap()
                .bytes,
            bytes
        );
    }
}

#[test]
fn upload_real_mutation_losses_preserve_original_state_and_confirmed_charge() {
    for point in [
        Milestone::Adopted,
        Milestone::ChunkSynced,
        Milestone::ChunkAcked,
        Milestone::CommitAdopted,
        Milestone::Linked,
        Milestone::Receipted,
        Milestone::DiscardAdopted,
        Milestone::CloseUnlinked,
    ] {
        let env = Env::new();
        let (service, peer) = running(&env, bounds());
        let original = spec(Context::LocalExtension, 7, b"hello");
        let call = if point == Milestone::Adopted {
            begin(&original)
        } else {
            assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
            if matches!(
                point,
                Milestone::CommitAdopted | Milestone::Linked | Milestone::Receipted
            ) {
                admitted(&peer, 2, part(b"hello", 0), &original);
                commit()
            } else if matches!(point, Milestone::DiscardAdopted | Milestone::CloseUnlinked) {
                discard()
            } else {
                part(b"hello", 0)
            }
        };
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&hits);
        service.storage.observe(Arc::new(move |at| {
            if at == point {
                seen.fetch_add(1, Ordering::AcqRel);
                false
            } else {
                true
            }
        }));
        authorize_effect(&peer, 3, call);
        assert_eq!(
            result(&peer).outcome,
            Outcome::Failure(ErrorCode::Unknown),
            "{point:?}"
        );
        assert_eq!(
            hits.load(Ordering::Acquire),
            1,
            "real algorithm reached {point:?}"
        );
        let backend = service.handle("alpha").unwrap();
        let stored = backend.status(original.intent, &io()).unwrap();
        assert_eq!(stored.original.as_ref().unwrap().spec, original);
        let expected = match point {
            Milestone::Adopted | Milestone::ChunkSynced => {
                TransferState::Pending(crate::objects::Progress {
                    intent: original.intent,
                    next_index: 0,
                    received: 0,
                })
            }
            Milestone::ChunkAcked => TransferState::Pending(crate::objects::Progress {
                intent: original.intent,
                next_index: 1,
                received: 5,
            }),
            Milestone::Receipted => TransferState::Committed(crate::objects::Receipt {
                intent: original.intent,
                key: original.key,
                payload_sha256: original.payload_sha256,
                payload_bytes: 5,
                binding: original.binding.clone(),
            }),
            Milestone::DiscardAdopted | Milestone::CloseUnlinked => TransferState::Discarded,
            _ => TransferState::Unknown,
        };
        assert_eq!(stored.state, expected, "{point:?}");
        let charge = backend.usage(None, &io()).unwrap();
        assert_eq!(charge.retained_identities, 1);
        assert_eq!(
            charge.entries, 1,
            "no release before confirmed physical cleanup"
        );
        let ledger = fs::read(env.ledger()).unwrap();
        let files = payload_snapshot(&env.directory("alpha"));
        let (_, answer) = allowed(&peer, 4, status_input(&original));
        assert!(matches!(answer, Outcome::Success(_)));
        assert_eq!(
            hits.load(Ordering::Acquire),
            1,
            "status never retries mutation"
        );
        assert_eq!(backend.status(original.intent, &io()).unwrap(), stored);
        assert_eq!(backend.usage(None, &io()).unwrap(), charge);
        assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
        assert_eq!(payload_snapshot(&env.directory("alpha")), files);
        assert_eq!(service.ended("alpha"), None);
    }
}

#[test]
fn upload_transition_between_call_and_disclose_cannot_project_stale_progress() {
    let env = Env::new();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    let service = env.service(&ALPHA, bounds()).unwrap();
    let (signal, entered) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let gate = Mutex::new(gate);
    service.set_hook(Some(Arc::new(move |pause, _| {
        if pause == Pause::BetweenAdmissions {
            let _ = signal.send(());
            gate.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
    })));
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    let original = spec(Context::LocalExtension, 7, b"hello");
    authorize_effect(&peer, 1, begin(&original));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    service
        .handle("alpha")
        .unwrap()
        .discard(original.intent, &io())
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Unknown));
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .status(original.intent, &io())
            .unwrap()
            .state,
        TransferState::Discarded
    );
}

#[test]
fn upload_namespace_fence_during_effect_decision_refuses_before_invocation() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    request(&peer, 2, Origin::LocalExtension, part(b"hello", 0));
    let acquire = callback(&peer);
    decide(&peer, acquire.callback_id.get(), 2, Decision::Allow);
    let effect = callback(&peer);
    assert_eq!(effect.boundary, Checkpoint::Effect);
    service
        .handle("alpha")
        .unwrap()
        .remove_namespace(original.key.namespace, &io())
        .unwrap();
    let ledger = fs::read(env.ledger()).unwrap();
    let files = payload_snapshot(&env.directory("alpha"));
    decide(&peer, effect.callback_id.get(), 2, Decision::Allow);
    assert_eq!(
        result(&peer).outcome,
        Outcome::Failure(ErrorCode::Unavailable)
    );
    assert_eq!(fs::read(env.ledger()).unwrap(), ledger);
    assert_eq!(payload_snapshot(&env.directory("alpha")), files);
}

#[test]
fn upload_frozen_expiry_crossed_after_commit_is_unknown_not_late_receipt() {
    let env = Env::new();
    let now = Arc::new(AtomicU64::new(1));
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
    assert_eq!(admitted(&peer, 1, begin(&original), &original), pending());
    admitted(&peer, 2, part(b"hello", 0), &original);
    let expiry = service
        .handle("alpha")
        .unwrap()
        .status(original.intent, &io())
        .unwrap()
        .original
        .unwrap()
        .expires_at_ms;
    service.storage.observe(Arc::new(move |point| {
        if point == Milestone::Receipted {
            now.store(expiry, Ordering::Release);
        }
        true
    }));
    authorize_effect(&peer, 3, commit());
    assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Unknown));
    assert!(matches!(
        service
            .handle("alpha")
            .unwrap()
            .status(original.intent, &io())
            .unwrap()
            .state,
        TransferState::Committed(_)
    ));
    assert!(matches!(
        allowed(&peer, 4, status_input(&original)).1,
        Outcome::Success(Success::Committed {
            payload_bytes: 5,
            ..
        })
    ));
}

#[test]
fn scoped_service_shutdown_joins_bus_then_restart_reads_same_committed_original() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    admitted(&peer, 1, begin(&original), &original);
    admitted(&peer, 2, part(b"hello", 0), &original);
    let committed = admitted(&peer, 3, commit(), &original);
    assert!(matches!(
        committed,
        Outcome::Success(Success::Committed { .. })
    ));
    let receipt = service
        .handle("alpha")
        .unwrap()
        .status(original.intent, &io())
        .unwrap();
    let generation = service.active("alpha").unwrap();
    let bus = service.bus("alpha").unwrap();
    service.shutdown();
    assert!(service.active("alpha").is_none());
    assert!(
        bus.upgrade().is_none(),
        "shutdown must join every socket/view holder"
    );
    with_bus(&peer, |bus| {
        assert_eq!(
            bus.recv(Some(Instant::now() + Duration::from_secs(5))),
            Err(Fault::Closed)
        )
    });
    drop(service);
    peer.buses.lock().unwrap().clear();
    let service = env.service(&ALPHA, bounds()).unwrap();
    let next = service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    assert_ne!(generation, next);
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .status(original.intent, &io())
            .unwrap(),
        receipt
    );
    assert_eq!(allowed(&peer, 1, status_input(&original)).1, committed);
    assert_eq!(
        service
            .handle("alpha")
            .unwrap()
            .read(original.key, 0, 5, &io())
            .unwrap()
            .bytes,
        b"hello"
    );
    service.shutdown();
    drop(service);
}
