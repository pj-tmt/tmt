//! Namespace management through the actual channel and ledger.
use super::observe::{seed, spec};
use super::*;
use tmt_extension_objects::{NamespaceInput, UsageScope};

fn remove(tag: u8) -> Call {
    Call::RemoveNamespace(NamespaceInput {
        namespace: Bytes32::from_bytes([tag; 32]),
    })
}
fn allowed(peer: &Peer, id: u64, call: Call) -> Outcome {
    request(peer, id, Origin::LocalExtension, call.clone());
    let checkpoints: &[Checkpoint] = if matches!(call, Call::RemoveNamespace(_)) {
        &[
            Checkpoint::Acquire,
            Checkpoint::Effect,
            Checkpoint::Disclose,
        ]
    } else {
        &[Checkpoint::Acquire, Checkpoint::Disclose]
    };
    for boundary in checkpoints {
        let admit = callback(peer);
        assert_eq!(
            (admit.boundary, admit.context),
            (*boundary, Context::LocalExtension)
        );
        assert_eq!(
            admit.operation.disclosure.is_some(),
            *boundary == Checkpoint::Disclose
        );
        decide(peer, admit.callback_id.get(), id, Decision::Allow);
    }
    let answer = result(peer);
    assert_eq!(
        (answer.request_id, answer.method, answer.transfer_id),
        (counter(id), call.method(), None)
    );
    answer.outcome
}
fn usage(peer: &Peer, id: u64, scope: UsageScope) -> tmt_extension_objects::Usage {
    match allowed(peer, id, Call::Usage(scope)) {
        Outcome::Success(Success::Usage(value)) => value,
        other => panic!("expected usage, got {other:?}"),
    }
}
#[test]
fn namespace_removal_is_idempotent_and_retains_originals_in_usage() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    seed(&service, &original, b"hello", true);
    let before = usage(
        &peer,
        1,
        UsageScope::Namespace(Bytes32::from_bytes([7; 32])),
    );
    assert_eq!(
        (
            before.charged_bytes,
            before.entries,
            before.retained_identities
        ),
        (
            crate::objects::payload_charge(5)
                + crate::limits::OBJECT_RECORD_BYTES
                + crate::limits::OBJECT_FENCE_BYTES,
            1,
            1
        )
    );
    for (id, tag) in [(2, 7), (3, 7), (4, 8)] {
        assert_eq!(
            allowed(&peer, id, remove(tag)),
            Outcome::Success(Success::Removed)
        );
    }
    let after = usage(
        &peer,
        5,
        UsageScope::Namespace(Bytes32::from_bytes([7; 32])),
    );
    assert_eq!(
        (
            after.charged_bytes,
            after.entries,
            after.active_uploads,
            after.retained_identities
        ),
        (
            crate::limits::OBJECT_RECORD_BYTES + crate::limits::OBJECT_FENCE_BYTES,
            0,
            0,
            1
        )
    );
    assert_eq!(
        service.handle("alpha").unwrap().begin(&original, &io()),
        Ok(crate::objects::BeginResult::Terminal(
            crate::objects::TransferState::Unavailable
        )),
        "deleted original only returns its retained terminal state"
    );
    assert!(
        service
            .handle("alpha")
            .unwrap()
            .read(original.key, 0, 5, &io())
            .is_err()
    );
}
#[test]
fn extension_usage_keeps_the_existing_namespace_sum_and_tree_charge() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    assert_eq!(
        usage(&peer, 1, UsageScope::Extension).charged_bytes,
        0,
        "no namespace means no tree charge"
    );
    let first = spec(Context::LocalExtension, 7, b"hello");
    let mut second = spec(Context::LocalExtension, 8, b"world!");
    second.intent = crate::objects::IntentId([8; 32]);
    seed(&service, &first, b"hello", true);
    seed(&service, &second, b"world!", false);
    let a = usage(
        &peer,
        2,
        UsageScope::Namespace(Bytes32::from_bytes([7; 32])),
    );
    let b = usage(
        &peer,
        3,
        UsageScope::Namespace(Bytes32::from_bytes([8; 32])),
    );
    let total = usage(&peer, 4, UsageScope::Extension);
    assert_eq!(
        total,
        tmt_extension_objects::Usage {
            charged_bytes: a.charged_bytes
                + b.charged_bytes
                + crate::limits::OBJECT_TREE_BASE_BYTES,
            entries: a.entries + b.entries,
            active_uploads: a.active_uploads + b.active_uploads,
            retained_identities: a.retained_identities + b.retained_identities
        }
    );
}
#[test]
fn mounted_namespace_management_is_denied_without_callbacks_or_changes() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    seed(&service, &original, b"hello", true);
    let backend = service.handle("alpha").unwrap();
    let before = backend.usage(None, &io()).unwrap();
    for (id, call) in [(1, remove(7)), (2, Call::Usage(UsageScope::Extension))] {
        request(
            &peer,
            id,
            Origin::Mounted(Uuid4::parse("d9f6456b-2951-4c1e-a130-cbac198e3a50").unwrap()),
            call,
        );
        assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Denied));
        assert_eq!(backend.usage(None, &io()).unwrap(), before);
        assert_eq!(
            backend.read(original.key, 0, 5, &io()).unwrap().bytes,
            b"hello"
        );
    }
}

#[test]
fn namespace_management_admission_refuses_before_effect_and_preserves_payload() {
    for checkpoint in [Checkpoint::Acquire, Checkpoint::Effect] {
        let env = Env::new();
        let (service, peer) = running(&env, bounds());
        let original = spec(Context::LocalExtension, 7, b"hello");
        seed(&service, &original, b"hello", true);
        let before = service.handle("alpha").unwrap().usage(None, &io()).unwrap();
        request(&peer, 1, Origin::LocalExtension, remove(7));
        if checkpoint == Checkpoint::Effect {
            let admit = callback(&peer);
            assert_eq!(admit.boundary, Checkpoint::Acquire);
            decide(&peer, admit.callback_id.get(), 1, Decision::Allow);
        }
        let admit = callback(&peer);
        assert_eq!(admit.boundary, checkpoint);
        decide(&peer, admit.callback_id.get(), 1, Decision::Deny);
        assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Denied));
        let backend = service.handle("alpha").unwrap();
        assert_eq!(backend.usage(None, &io()).unwrap(), before);
        assert_eq!(
            backend.read(original.key, 0, 5, &io()).unwrap().bytes,
            b"hello"
        );
    }
}

#[test]
fn interrupted_removal_reports_unknown_and_keeps_cleanup_charge_until_confirmation() {
    let env = Env::new();
    let (service, peer) = running(&env, bounds());
    let original = spec(Context::LocalExtension, 7, b"hello");
    seed(&service, &original, b"hello", true);
    let before = service.handle("alpha").unwrap().usage(None, &io()).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    service.storage.observe(Arc::new(move |point| {
        if point == crate::objects::Milestone::RemovalMarked {
            seen.fetch_add(1, Ordering::SeqCst);
            false
        } else {
            true
        }
    }));
    request(&peer, 1, Origin::LocalExtension, remove(7));
    for boundary in [Checkpoint::Acquire, Checkpoint::Effect] {
        let admit = callback(&peer);
        assert_eq!(admit.boundary, boundary);
        decide(&peer, admit.callback_id.get(), 1, Decision::Allow);
    }
    assert_eq!(result(&peer).outcome, Outcome::Failure(ErrorCode::Unknown));
    assert_eq!(hits.load(Ordering::SeqCst), 1, "no automatic removal retry");
    let held = usage(&peer, 2, UsageScope::Extension);
    assert_eq!(held.charged_bytes, before.charged_bytes);
    assert_eq!(held.entries, before.entries);
    service.storage.observe(Arc::new(|_| true));
    assert_eq!(
        allowed(&peer, 3, remove(7)),
        Outcome::Success(Success::Removed)
    );
    let removed = usage(&peer, 4, UsageScope::Extension);
    assert_eq!(
        removed.charged_bytes,
        before.charged_bytes - crate::objects::payload_charge(5)
    );
    assert_eq!((removed.entries, removed.retained_identities), (0, 1));
    let mut fresh = original.clone();
    fresh.intent = crate::objects::IntentId([8; 32]);
    assert_eq!(
        service.handle("alpha").unwrap().begin(&fresh, &io()),
        Err(crate::objects::BackendError::Conflict)
    );
}

#[test]
fn a_new_namespace_tombstone_keeps_typed_capacity_refusal_without_changes() {
    let env = Env::new();
    let service = ObjectService::open(
        &env.serving,
        &ALPHA,
        Quotas {
            namespace_bytes: 0,
            ..Quotas::contract()
        },
        system_clock(),
        bounds(),
        Origins::default(),
        &io(),
    )
    .unwrap()
    .unwrap();
    let peer = Peer::start(&env, "alpha", Answer::Expect);
    service.activate(&env.mounts(&ALPHA), "alpha").unwrap();
    request(&peer, 1, Origin::LocalExtension, remove(7));
    for boundary in [Checkpoint::Acquire, Checkpoint::Effect] {
        let admit = callback(&peer);
        assert_eq!(admit.boundary, boundary);
        decide(&peer, admit.callback_id.get(), 1, Decision::Allow);
    }
    assert_eq!(
        result(&peer).outcome,
        Outcome::Failure(ErrorCode::Capacity(
            tmt_extension_objects::Limit::NamespaceBytes
        ))
    );
    assert_eq!(
        usage(&peer, 2, UsageScope::Extension),
        tmt_extension_objects::Usage {
            charged_bytes: 0,
            entries: 0,
            active_uploads: 0,
            retained_identities: 0
        }
    );
}
