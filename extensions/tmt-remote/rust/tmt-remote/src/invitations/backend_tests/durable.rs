//! Both provider consumers use the actual M1 sink and existing receipt journal.
use super::durable_fixture::DurableFixture;
use super::*;
use crate::{
    members::Owner,
    pairing::now_ms,
    store::{DEFAULT_SCOPES, Grant},
};

fn committed<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    let f = DurableFixture::new();
    let backend = make(f.context.binding.clone());
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let owner = OwnerActor::local_interactive();
    let (coordinator, delivery, snapshot) = f.pending(backend.clone(), f.sink.clone(), &io);
    assert_eq!(
        f.counts(),
        (0, 0, 0, 0),
        "code and native possession created authority"
    );
    backend.lose_next_receipt();
    assert!(
        coordinator
            .confirm(
                &owner,
                &snapshot,
                true,
                &PairingPolicy::default(),
                now_ms().unwrap(),
                &io
            )
            .unwrap()
            .local_confirmation_known
    );
    assert!(!coordinator.retains_secret().unwrap());
    assert_eq!(f.counts(), (1, 1, 1, 1));
    let original = f.sink.original(&snapshot).unwrap().unwrap();
    let (grant, member) = {
        let store = f.store.lock().unwrap();
        (
            store.grant(&snapshot.candidate.machine).unwrap().unwrap(),
            store
                .member_for_client(&snapshot.candidate.machine)
                .unwrap()
                .unwrap(),
        )
    };
    assert_eq!(member.member_id, original.member);
    assert!(!member.sending_enabled);
    assert!(!grant.permits_scope("talk"));
    assert_eq!(grant.mode, "direct");
    assert_eq!(
        original.receipt,
        crate::pairing::receipt(&grant, &f.context.owner, &f.context.owner_key).unwrap()
    );
    delivery.with_code(|code| {
        let key = crypto::response_key(
            code,
            &enrollment(&snapshot.context, &snapshot.candidate).unwrap(),
        )
        .unwrap();
        crypto::verify_server_proof(&key, &original.receipt, &original.proof).unwrap();
        let saved: String = f
            .store
            .lock()
            .unwrap()
            .connection
            .query_row("SELECT outcome FROM management_receipts", [], |r| r.get(0))
            .unwrap();
        assert!(!saved.contains(&canonical::base64url(code)));
        assert!(!saved.contains(&canonical::base64url(&key)));
    });
    assert!(
        coordinator
            .confirm(
                &owner,
                &snapshot,
                true,
                &PairingPolicy::default(),
                now_ms().unwrap(),
                &io
            )
            .is_err()
    );
    drop(coordinator);
    f.reopen();
    let recovered = Coordinator::new(backend.clone(), f.sink.clone());
    recovered
        .recover(&owner, &snapshot, now_ms().unwrap(), &io)
        .unwrap();
    for _ in 0..2 {
        recovered.observe(now_ms().unwrap(), &io).unwrap();
    }
    assert_eq!(
        recovered.status(now_ms().unwrap()).unwrap().state,
        State::Confirmed
    );
    assert!(f.sink.original(&snapshot).unwrap().as_ref() == Some(&original));
    assert_eq!(
        backend.writes(Part::Receipt),
        1,
        "restart repeated receipt publication"
    );
    assert_eq!(f.counts(), (1, 1, 1, 1));
    assert!(matches!(
        f.sink.claim(&original, Part::Receipt).unwrap(),
        Claim::ObserveOnly
    ));
    // Mutable member policy cannot replace the admitted original receipt.
    f.store
        .lock()
        .unwrap()
        .set_member_sending(
            Owner::LocalOwner,
            &member.member_id,
            member.revision,
            true,
            now_ms().unwrap(),
        )
        .unwrap();
    assert!(f.sink.original(&snapshot).unwrap().as_ref() == Some(&original));
    assert!(matches!(
        f.sink.claim(&original, Part::Receipt).unwrap(),
        Claim::Stale
    ));
    assert!(!f.sink.acknowledge(&original, Part::Receipt).unwrap());
    let member = f
        .store
        .lock()
        .unwrap()
        .member_for_client(&grant.client_id)
        .unwrap()
        .unwrap();
    f.store
        .lock()
        .unwrap()
        .revoke_member(
            Owner::LocalOwner,
            &member.member_id,
            member.revision,
            now_ms().unwrap(),
        )
        .unwrap();
    assert!(matches!(
        f.sink.claim(&original, Part::Receipt).unwrap(),
        Claim::Stale
    ));
    assert_eq!(backend.writes(Part::Receipt), 1);
    assert_eq!(f.counts(), (1, 1, 1, 1));
}

fn rollback<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    let mut f = DurableFixture::new();
    let backend = make(f.context.binding.clone());
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (coordinator, _, snapshot) = f.pending(backend.clone(), f.sink.clone(), &io);
    f.store.lock().unwrap().connection.execute_batch("CREATE TRIGGER fail_invite_receipt BEFORE INSERT ON management_receipts BEGIN SELECT RAISE(ABORT,'fixture receipt failure'); END").unwrap();
    assert!(
        coordinator
            .confirm(
                &OwnerActor::local_interactive(),
                &snapshot,
                true,
                &PairingPolicy::default(),
                now_ms().unwrap(),
                &io
            )
            .is_err()
    );
    assert_eq!(
        f.counts(),
        (0, 0, 0, 0),
        "receipt failure left partial authority"
    );
    assert_eq!(
        coordinator.status(now_ms().unwrap()).unwrap().state,
        State::Refused,
        "verified transaction rollback was reported as an uncertain commit"
    );
    assert!(!coordinator.retains_secret().unwrap());
    assert_eq!(backend.writes(Part::Receipt), 0);
    drop(coordinator);
    f.reopen();
    assert_eq!(f.counts(), (0, 0, 0, 0));
    assert!(f.sink.original(&snapshot).unwrap().is_none());
    f.store
        .lock()
        .unwrap()
        .connection
        .execute_batch("DROP TRIGGER fail_invite_receipt")
        .unwrap();
    f.context.offer = crate::store::uuid_v4().unwrap();
    let backend = make(f.context.binding.clone());
    let (coordinator, _, snapshot) = f.pending(backend, f.sink.clone(), &io);
    coordinator
        .confirm(
            &OwnerActor::local_interactive(),
            &snapshot,
            true,
            &PairingPolicy::default(),
            now_ms().unwrap(),
            &io,
        )
        .unwrap();
    assert_eq!(f.counts(), (1, 1, 1, 1));
    f.reopen();
    assert!(f.sink.original(&snapshot).unwrap().is_some());
}

fn owner_race<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    let f = DurableFixture::new();
    let now = now_ms().unwrap();
    let browser = Grant {
        client_id: crate::store::uuid_v4().unwrap(),
        public_key: ed25519_dalek::SigningKey::from_bytes(&[24; 32])
            .verifying_key()
            .to_bytes(),
        kind: "browser".into(),
        origin: "http://127.0.0.1:49152".into(),
        name: "Owner".into(),
        agents: "all".into(),
        scopes: DEFAULT_SCOPES.map(str::to_owned).into(),
        mode: "direct".into(),
        issued_at_ms: now,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    };
    {
        let mut store = f.store.lock().unwrap();
        store.insert_grant(&browser).unwrap();
        store
            .designate(&browser.client_id, &browser.origin, now)
            .unwrap();
    }
    let browser = f
        .store
        .lock()
        .unwrap()
        .grant(&browser.client_id)
        .unwrap()
        .unwrap();
    let actor = OwnerActor::owner_browser(
        browser.client_id.clone(),
        browser.public_key,
        browser.revision,
    );
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let race = AdmissionRace {
        sink: f.sink.clone(),
        entered: entered.clone(),
        resume: resume.clone(),
    };
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let backend = make(f.context.binding.clone());
    let (coordinator, _, snapshot) = f.pending(backend.clone(), race, &io);
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            coordinator.confirm(
                &actor,
                &snapshot,
                true,
                &PairingPolicy::default(),
                now_ms().unwrap(),
                &io,
            )
        });
        entered.wait();
        f.store.lock().unwrap().undesignate().unwrap();
        resume.wait();
        assert!(
            worker.join().unwrap().is_err(),
            "writer reused stale owner admission"
        );
    });
    assert_eq!(f.counts(), (1, 1, 1, 0));
    assert!(
        f.store
            .lock()
            .unwrap()
            .grant(&snapshot.candidate.machine)
            .unwrap()
            .is_none()
    );
    assert_eq!(backend.writes(Part::Receipt), 0);
    assert!(!coordinator.retains_secret().unwrap());
}

fn competing_candidates<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    let f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    // Independent memory owners cannot bypass the same durable offer identity.
    let first_backend = make(f.context.binding.clone());
    let (first, _, first_snapshot) = f.pending(first_backend.clone(), f.sink.clone(), &io);
    let second_backend = make(f.context.binding.clone());
    let (_, mut candidate, _) = super::super::tests::fixture();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[33; 32]);
    candidate.machine = crate::store::uuid_v4().unwrap();
    candidate.public_key = signing.verifying_key().to_bytes();
    candidate.principal = b"second-authenticated-peer".to_vec();
    let (second, _, second_snapshot) = f.pending_candidate(
        second_backend.clone(),
        f.sink.clone(),
        candidate,
        signing,
        &io,
    );
    assert_eq!(first_snapshot.context.offer, second_snapshot.context.offer);
    assert!(first_snapshot.digest != second_snapshot.digest);
    let start = Barrier::new(3);
    let (one, two) = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            start.wait();
            first
                .confirm(
                    &OwnerActor::local_interactive(),
                    &first_snapshot,
                    true,
                    &PairingPolicy::default(),
                    now_ms().unwrap(),
                    &io,
                )
                .is_ok()
        });
        let two = scope.spawn(|| {
            start.wait();
            second
                .confirm(
                    &OwnerActor::local_interactive(),
                    &second_snapshot,
                    true,
                    &PairingPolicy::default(),
                    now_ms().unwrap(),
                    &io,
                )
                .is_ok()
        });
        start.wait();
        (one.join().unwrap(), two.join().unwrap())
    });
    assert_ne!(one, two, "one durable offer admitted two candidates");
    assert_eq!(f.counts(), (1, 1, 1, 1));
    assert_eq!(
        first_backend.writes(Part::Receipt) + second_backend.writes(Part::Receipt),
        1
    );
    f.reopen();
    let store = f.store.lock().unwrap();
    assert_eq!(
        store
            .grant(&first_snapshot.candidate.machine)
            .unwrap()
            .is_some(),
        one
    );
    assert_eq!(
        store
            .grant(&second_snapshot.candidate.machine)
            .unwrap()
            .is_some(),
        two
    );
}

#[test]
fn mailbox_real_store_confirmation_recovery_and_revoke() {
    committed(Mailbox::new);
}
#[test]
fn append_log_real_store_confirmation_recovery_and_revoke() {
    committed(AppendLog::new);
}
#[test]
fn mailbox_real_store_receipt_failure_rolls_back() {
    rollback(Mailbox::new);
}
#[test]
fn append_log_real_store_receipt_failure_rolls_back() {
    rollback(AppendLog::new);
}
#[test]
fn mailbox_real_store_rechecks_owner_in_writer() {
    owner_race(Mailbox::new);
}
#[test]
fn append_log_real_store_rechecks_owner_in_writer() {
    owner_race(AppendLog::new);
}

#[test]
fn mailbox_real_store_same_offer_race_has_one_member_and_receipt() {
    competing_candidates(Mailbox::new);
}
#[test]
fn append_log_real_store_same_offer_race_has_one_member_and_receipt() {
    competing_candidates(AppendLog::new);
}
