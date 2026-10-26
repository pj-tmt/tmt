//! Concrete Firestore confirmation over the real Store and shared coordinator.
//! The mailbox is only the exchange fixture; no provider/worker proof is claimed.
use super::durable_fixture::DurableFixture;
use super::*;
use crate::cloud_pairing::confirmation::{FirestoreConfirmation, RoutePublication};
use sha2::{Digest, Sha256};

fn fixture() -> (DurableFixture, FirestoreConfirmation) {
    let mut f = DurableFixture::new();
    f.context.binding = BackendBinding {
        backend: "firestore".into(),
        reference: b"demo-invitation-confirmation".to_vec(),
        owner_principal: b"owner-uid".to_vec(),
    };
    let sink = FirestoreConfirmation::new(f.store.clone(), f.context.owner_key,
        serde_json::json!({"projectId":"demo-invitation-confirmation","appId":"1:123:web:test","apiKey":"public-test"}),
        "owner-uid".into()).unwrap();
    (f, sink)
}
fn inventory(f: &DurableFixture) -> (i64, i64, i64, i64) {
    f.store.lock().unwrap().connection.query_row(
        "SELECT (SELECT COUNT(*) FROM cloud_peers),(SELECT COUNT(*) FROM cloud_peer_routes),(SELECT COUNT(*) FROM cloud_remotes),(SELECT COUNT(*) FROM cloud_owner_projects)", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap()
}
fn confirm(
    f: &DurableFixture,
    sink: &FirestoreConfirmation,
    sending: bool,
    io: &IoBudget<'_>,
) -> (Snapshot, Committed) {
    let backend = Mailbox::new(f.context.binding.clone());
    let (coordinator, _, snapshot) = f.pending(backend, sink.clone(), io);
    coordinator
        .confirm_owner_decision(
            &OwnerActor::local_interactive(),
            &snapshot,
            OwnerDecision {
                matched: true,
                enable_sending: sending,
            },
            &PairingPolicy::default(),
            crate::pairing::now_ms().unwrap(),
            io,
        )
        .unwrap();
    let original = sink.original(&snapshot).unwrap().unwrap();
    (snapshot, original)
}

#[test]
fn firestore_confirmation_commits_member_policy_route_tokens_and_original_together() {
    for sending in [false, true] {
        let (f, sink) = fixture();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let (snapshot, original) = confirm(&f, &sink, sending, &io);
        let capture = RoutePublication::read(
            original.routes.as_ref().unwrap(),
            &snapshot.context,
            &snapshot.digest,
        )
        .unwrap();
        assert_eq!(capture.member, original.member);
        let persisted = f
            .store
            .lock()
            .unwrap()
            .grant(&snapshot.candidate.machine)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.permits_scope("talk"), sending);
        assert_eq!(persisted.revision, original.grant_revision);
        let receipt: serde_json::Value = serde_json::from_slice(&original.receipt).unwrap();
        let expected: serde_json::Value = serde_json::from_slice(
            &crate::pairing::receipt(
                &persisted,
                &snapshot.context.owner,
                &snapshot.context.owner_key,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(receipt["grant"], expected["grant"]);
        let token =
            canonical::base64url_bytes(receipt["rerouteToken"].as_str().unwrap(), 32).unwrap();
        assert_eq!(
            capture.rows[0].reroute_hash.unwrap().as_slice(),
            Sha256::digest(token).as_slice()
        );
        assert_eq!(capture.rows[0].member.as_ref(), Some(&original.member));
        let connection = f
            .store
            .lock()
            .unwrap()
            .cloud_connection(&snapshot.candidate.machine)
            .unwrap()
            .unwrap();
        assert_eq!(connection.owner_key, snapshot.candidate.public_key);
        assert!(connection.confirmed && connection.reroute_token.is_some());
        assert_eq!(inventory(&f), (1, 1, 1, 1));
        f.reopen();
        assert!(sink.original(&snapshot).unwrap().as_ref() == Some(&original));
        let rows = f.store.lock().unwrap().cloud_route_changes().unwrap();
        assert_eq!(rows, capture.rows);
    }
}

#[test]
fn firestore_original_receipt_failure_rolls_back_grant_and_every_route_effect() {
    let (f, sink) = fixture();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let backend = Mailbox::new(f.context.binding.clone());
    let (coordinator, _, snapshot) = f.pending(backend.clone(), sink.clone(), &io);
    f.store.lock().unwrap().connection.execute_batch("CREATE TRIGGER fail_confirmation_receipt BEFORE INSERT ON management_receipts BEGIN SELECT RAISE(ABORT,'fixture receipt failure'); END").unwrap();
    assert!(
        coordinator
            .confirm_owner_decision(
                &OwnerActor::local_interactive(),
                &snapshot,
                OwnerDecision {
                    matched: true,
                    enable_sending: true
                },
                &PairingPolicy::default(),
                crate::pairing::now_ms().unwrap(),
                &io,
            )
            .is_err()
    );
    assert_eq!(
        coordinator
            .status(crate::pairing::now_ms().unwrap())
            .unwrap()
            .state,
        State::Refused
    );
    assert_eq!(backend.writes(Part::Routes), 0);
    assert_eq!(backend.writes(Part::Receipt), 0);
    f.reopen();
    assert_eq!(f.counts(), (0, 0, 0, 0));
    assert_eq!(inventory(&f), (0, 0, 0, 0));
    assert!(sink.original(&snapshot).unwrap().is_none());
}

#[test]
fn firestore_reinvitation_rotates_route_capability_and_fences_old_originals_and_ack() {
    let (mut f, sink) = fixture();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (old_snapshot, original) = confirm(&f, &sink, true, &io);
    let old_routes = RoutePublication::read(
        original.routes.as_ref().unwrap(),
        &old_snapshot.context,
        &old_snapshot.digest,
    )
    .unwrap();
    f.store
        .lock()
        .unwrap()
        .revoke(&old_snapshot.candidate.machine)
        .unwrap();
    f.context.offer = crate::store::uuid_v4().unwrap();
    let (snapshot, renewed) = confirm(&f, &sink, false, &io);
    assert_ne!(original.member, renewed.member);
    assert!(renewed.grant_revision > original.grant_revision);
    let routes = RoutePublication::read(
        renewed.routes.as_ref().unwrap(),
        &snapshot.context,
        &snapshot.digest,
    )
    .unwrap();
    assert_eq!(old_routes.rows[0].uid, routes.rows[0].uid);
    assert_eq!(old_routes.rows[0].generation, routes.rows[0].generation);
    assert_ne!(old_routes.rows[0].reroute_hash, routes.rows[0].reroute_hash);
    f.store
        .lock()
        .unwrap()
        .cloud_route_published(&old_routes.rows[0])
        .unwrap();
    let pending = f.store.lock().unwrap().cloud_route_changes().unwrap();
    assert_eq!(pending, routes.rows);
    assert_eq!(sink.claim(&original, Part::Receipt).unwrap(), Claim::Stale);
    assert!(sink.original(&old_snapshot).unwrap().as_ref() == Some(&original));
    let grant = f
        .store
        .lock()
        .unwrap()
        .grant(&snapshot.candidate.machine)
        .unwrap()
        .unwrap();
    assert!(!grant.permits_scope("talk"));
    assert_eq!(f.counts(), (1, 2, 1, 2));
    assert_eq!(inventory(&f), (1, 1, 1, 1));
    f.reopen();
    assert!(sink.original(&snapshot).unwrap().as_ref() == Some(&renewed));
}
