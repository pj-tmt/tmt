//! Actual provider port + existing worker, with the shared injected HTTP fixture.
//! This is not Rules, emulator or public CLI acceptance.
use super::*;
use crate::{
    cloud_pairing::{confirmation::FirestoreConfirmation, invitation::Firestore},
    invitations::{
        BackendBinding, Candidate, Context,
        port::*,
        service::{Coordinator, State},
        submission::SubmissionAttempt,
    },
    pairing::PairingPolicy,
};

fn until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "bounded invitation observation did not become ready"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn v2_route_delegation_wakes_the_existing_worker_before_publishing_the_exact_receipt() {
    if crate::test_isolation::isolated_case(
        "cloud_pairing::service::tests::invitation_backend_tests::v2_route_delegation_wakes_the_existing_worker_before_publishing_the_exact_receipt",
    ) {
        return;
    }
    let core = core_fixture::Core::new();
    let wire = Arc::new(Wire::new());
    let ports = Arc::new(Ports {
        wire: wire.clone(),
        clock: SystemClock::default(),
    });
    let make_ports = || ProjectPorts {
        http: ports.clone(),
        clock: ports.clone(),
        jitter: ports.clone(),
    };
    let (owner, owner_lease) = service(&core.root.join("owner"), &core, make_ports());
    let (peer, peer_lease) = service(&core.root.join("peer"), &core, make_ports());
    let owner_provider = Arc::new(owner.provider(&config(), &owner.machine, true).unwrap());
    let peer_provider = Arc::new(peer.provider(&config(), &owner.machine, true).unwrap());
    let now = crate::pairing::now_ms().unwrap();
    let descriptor = owner.keys.advertised(now).unwrap();
    owner
        .register(&owner_provider, descriptor.bytes(), None, &owner.io())
        .unwrap();
    let context = Context {
        binding: BackendBinding {
            backend: "firestore".into(),
            reference: PROJECT.as_bytes().to_vec(),
            owner_principal: owner_provider.uid().as_bytes().to_vec(),
        },
        owner: owner.machine.clone(),
        owner_key: owner.key.public(),
        offer: crate::store::uuid_v4().unwrap(),
        nonce: [2; 16],
        issued_at_ms: now,
        expires_at_ms: now + 600_000,
    };
    let sink = FirestoreConfirmation::new(
        owner.store.clone(),
        owner.key.public(),
        config(),
        owner_provider.uid().into(),
    )
    .unwrap();
    let backend = Firestore::new(
        owner_provider.clone(),
        context.clone(),
        sink.clone(),
        owner.store.clone(),
        owner.wake.clone(),
    )
    .unwrap();
    let peer_sink = FirestoreConfirmation::new(
        peer.store.clone(),
        context.owner_key,
        config(),
        owner_provider.uid().into(),
    )
    .unwrap();
    let peer_backend = Firestore::new(
        peer_provider.clone(),
        context.clone(),
        peer_sink,
        peer.store.clone(),
        peer.wake.clone(),
    )
    .unwrap();
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(20),
        cancelled: &owner.stop,
    };
    let actor = OwnerActor::local_interactive();
    let coordinator = Coordinator::new(backend, sink.clone());
    coordinator
        .create(&actor, context.clone(), now, &io)
        .unwrap();
    let delivery = coordinator.deliver(&actor, now).unwrap();
    let candidate = Candidate {
        machine: peer.machine.clone(),
        public_key: peer.key.public(),
        principal: peer_provider.uid().as_bytes().to_vec(),
        name: "Peer".into(),
        nonce: [3; 16],
    };
    let mut attempt = delivery
        .with_code(|code| {
            SubmissionAttempt::new(
                peer_backend,
                context.clone(),
                candidate,
                code,
                &peer.key,
                now,
            )
        })
        .unwrap();
    assert_eq!(attempt.submit(&io).unwrap(), WriteOutcome::Acknowledged);
    coordinator
        .observe(crate::pairing::now_ms().unwrap(), &io)
        .unwrap();
    let snapshot = coordinator
        .candidate(crate::pairing::now_ms().unwrap())
        .unwrap()
        .unwrap();

    // Give the existing worker one old confirmed peer. Its first enrollment
    // query proves the initial flush has already run before the new grant is
    // committed. A subsequent flush inside 10 s therefore needs the wake;
    // the normal idle interval is 60 s.
    let dummy = crate::store::Grant {
        client_id: crate::store::uuid_v4().unwrap(),
        public_key: ed25519_dalek::SigningKey::from_bytes(&[22; 32])
            .verifying_key()
            .to_bytes(),
        kind: "cli".into(),
        origin: "cli".into(),
        name: "Existing peer".into(),
        agents: "all".into(),
        scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
        mode: "direct".into(),
        issued_at_ms: now,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    };
    {
        let mut store = owner.store().unwrap();
        store.insert_grant(&dummy).unwrap();
        store.connection.execute("INSERT INTO cloud_peers(project,client_id,owner,owner_key,generation,reroute_hash) VALUES(?1,?2,?3,?4,1,?5)", rusqlite::params![PROJECT,dummy.client_id,owner.machine,owner.key.public().as_slice(),[1u8;32].as_slice()]).unwrap();
        store.remember_cloud_owner(PROJECT, &config()).unwrap();
    }
    let worker = owner.worker().unwrap();
    until(|| {
        wire.queries
            .lock()
            .unwrap()
            .iter()
            .any(|q| q["structuredQuery"]["from"][0]["collectionId"] == "enroll")
    });
    let status = coordinator
        .confirm_owner_decision(
            &actor,
            &snapshot,
            OwnerDecision {
                matched: true,
                enable_sending: true,
            },
            &PairingPolicy::default(),
            crate::pairing::now_ms().unwrap(),
            &io,
        )
        .unwrap();
    assert_eq!(status.state, State::ProviderOutcomeUnconfirmed);
    assert!(status.local_confirmation_known);
    until(|| {
        owner
            .store()
            .unwrap()
            .cloud_route_changes()
            .unwrap()
            .is_empty()
    });
    let acknowledged = coordinator
        .observe(crate::pairing::now_ms().unwrap(), &io)
        .unwrap();
    assert_eq!(acknowledged.state, State::Confirmed);
    let original = sink.original(&snapshot).unwrap().unwrap();
    assert_eq!(
        attempt.observe_receipt(&io).unwrap(),
        Some(original.receipt.clone())
    );
    let receiving = peer.store().unwrap().grant(&owner.machine).unwrap();
    assert!(
        receiving.is_none(),
        "transport proof manufactured reverse local consent"
    );
    let reads = wire.reads.lock().unwrap().clone();
    assert!(
        !reads.iter().any(|name| name.contains("/peers/")),
        "route recovery added a Rules read"
    );
    let response_name = format!(
        "projects/{PROJECT}/databases/(default)/documents/m/{}/enroll/{}-response",
        owner.machine, context.offer
    );
    let receipt_writes = wire
        .commits
        .lock()
        .unwrap()
        .iter()
        .flat_map(|c| c["writes"].as_array().unwrap())
        .filter(|w| w["update"]["name"] == response_name)
        .count();
    assert_eq!(receipt_writes, 1);
    assert_eq!(core.sends(), 0);
    drop(worker);
    drop(attempt);
    drop(coordinator);
    drop(sink);
    drop(owner_provider);
    drop(peer_provider);
    drop(owner);
    drop(peer);
    drop(owner_lease);
    drop(peer_lease);
}
