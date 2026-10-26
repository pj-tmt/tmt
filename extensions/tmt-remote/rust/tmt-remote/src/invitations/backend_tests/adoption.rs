//! Two private Stores and the real common coordinator; the mailbox supplies only
//! typed transport. Receipt adoption must commit or roll back the entire reverse
//! authority and consume exactly the durably claimed candidate.
use super::{durable_fixture::DurableFixture, *};
use crate::{
    cloud_pairing::{Connection, adoption::ReceiptAdoption, confirmation::FirestoreConfirmation},
    invitations::submission::SubmissionAttempt,
};
use serde_json::json;

struct Exchange {
    owner: DurableFixture,
    peer: DurableFixture,
    delivery: super::super::service::Delivery,
    attempt: SubmissionAttempt<Mailbox>,
    connection: Connection,
    submitted: Submission,
    backend: Mailbox,
}
impl Exchange {
    fn new(io: &IoBudget<'_>) -> Self {
        let mut owner = DurableFixture::new();
        let peer = DurableFixture::new();
        owner.context.binding = BackendBinding {
            backend: "firestore".into(),
            reference: b"demo-receipt-adoption".to_vec(),
            owner_principal: b"owner-uid".to_vec(),
        };
        let config = json!({"projectId":"demo-receipt-adoption","appId":"1:123:web:test","apiKey":"public-test"});
        let sink = FirestoreConfirmation::new(
            owner.store.clone(),
            owner.context.owner_key,
            config.clone(),
            "owner-uid".into(),
        )
        .unwrap();
        let backend = Mailbox::new(owner.context.binding.clone());
        let coordinator = Coordinator::new(backend.clone(), sink);
        let actor = OwnerActor::local_interactive();
        let now = crate::pairing::now_ms().unwrap();
        coordinator
            .create(&actor, owner.context.clone(), now, io)
            .unwrap();
        let delivery = coordinator.deliver(&actor, now).unwrap();
        let (_, mut candidate, _) = super::super::tests::fixture();
        candidate.machine = peer.context.owner.clone();
        candidate.public_key = peer.machine_key().public();
        candidate.principal = b"peer-uid".to_vec();
        let mut attempt = delivery
            .with_code(|code| {
                SubmissionAttempt::new(
                    backend.peer(candidate.principal.clone()),
                    delivery.context.clone(),
                    candidate.clone(),
                    code,
                    &peer.machine_key(),
                    now,
                )
            })
            .unwrap();
        let submitted = attempt.signed_submission().clone();
        let connection = Connection {
            version: 1,
            name: "remote-owner".into(),
            owner: owner.context.owner.clone(),
            owner_key: owner.context.owner_key,
            config,
            peer: candidate.machine,
            uid: "peer-uid".into(),
            request: Vec::new(),
            confirmed: false,
            descriptor: Vec::new(),
            reroute_token: None,
            pending_submission: None,
        };
        assert!(
            peer.store
                .lock()
                .unwrap()
                .claim_cloud_submission(&connection, &owner.context, &submitted, io)
                .unwrap()
        );
        attempt.submit(io).unwrap();
        coordinator.observe(now, io).unwrap();
        let snapshot = coordinator.candidate(now).unwrap().unwrap();
        coordinator
            .confirm(&actor, &snapshot, true, &PairingPolicy::default(), now, io)
            .unwrap();
        Self {
            owner,
            peer,
            delivery,
            attempt,
            connection,
            submitted,
            backend,
        }
    }
    fn resume(&mut self) {
        self.attempt = self
            .delivery
            .with_code(|code| {
                SubmissionAttempt::resume_original(
                    self.backend
                        .peer(self.submitted.candidate.principal.clone()),
                    self.delivery.context.clone(),
                    self.submitted.clone(),
                    code,
                    crate::pairing::now_ms().unwrap(),
                )
            })
            .unwrap();
    }
    fn adopt(
        &mut self,
        sending: bool,
        io: &IoBudget<'_>,
    ) -> Result<Option<Connection>, crate::error::RemoteError> {
        self.attempt.observe_receipt_with(io, |bytes, key, budget| {
            self.peer.store.lock().unwrap().adopt_cloud_receipt(
                ReceiptAdoption {
                    context: &self.delivery.context,
                    submission: &self.submitted,
                    receipt: bytes,
                    key,
                    connection: &self.connection,
                    owner_name: "Owner display name",
                    enable_sending: sending,
                },
                budget,
            )
        })
    }
}
fn routes(f: &DurableFixture) -> i64 {
    f.store
        .lock()
        .unwrap()
        .connection
        .query_row("SELECT COUNT(*) FROM cloud_peer_routes", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn verified_receipt_adopts_reverse_member_policy_routes_and_consumes_original_atomically() {
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    for sending in [false, true] {
        let mut exchange = Exchange::new(&io);
        assert_eq!(exchange.peer.counts(), (0, 0, 0, 0));
        let adopted = exchange.adopt(sending, &io).unwrap().unwrap();
        assert!(
            adopted.confirmed
                && adopted.pending_submission.is_none()
                && adopted.reroute_token.is_some()
        );
        assert_eq!(
            adopted.request,
            crate::cloud_pairing::wire::v2::submission(
                &exchange.delivery.context,
                &exchange.submitted
            )
            .unwrap()
        );
        assert!(
            exchange.adopt(true, &io).is_err(),
            "one live attempt cannot reapply local consent"
        );
        exchange.peer.reopen();
        let store = exchange.peer.store.lock().unwrap();
        let grant = store.grant(&exchange.connection.owner).unwrap().unwrap();
        assert_eq!(grant.permits_scope("talk"), sending);
        assert_eq!(grant.name, "Owner display name");
        assert!(
            store
                .cloud_connection(&exchange.connection.owner)
                .unwrap()
                .unwrap()
                .pending_submission
                .is_none()
        );
        let rows = store.cloud_route_changes().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].uid, "owner-uid");
        assert!(rows[0].member.is_some() && rows[0].reroute_hash.is_some());
        drop(store);
        assert_eq!(exchange.peer.counts(), (1, 1, 1, 0));
        assert_eq!(routes(&exchange.peer), 1);
        assert_eq!(exchange.owner.counts(), (1, 1, 1, 1));
    }
}

#[test]
fn failed_connection_consumption_rolls_back_reverse_member_policy_and_routes() {
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let mut exchange = Exchange::new(&io);
    exchange.peer.store.lock().unwrap().connection.execute_batch(
        "CREATE TRIGGER fail_adoption BEFORE UPDATE ON cloud_remotes BEGIN SELECT RAISE(ABORT,'fixture adoption failure'); END"
    ).unwrap();
    assert!(exchange.adopt(true, &io).is_err());
    exchange.peer.reopen();
    assert_eq!(exchange.peer.counts(), (0, 0, 0, 0));
    assert_eq!(routes(&exchange.peer), 0);
    let saved = exchange
        .peer
        .store
        .lock()
        .unwrap()
        .cloud_connection(&exchange.connection.owner)
        .unwrap()
        .unwrap();
    assert!(
        saved.pending_submission.is_some() && !saved.confirmed && saved.reroute_token.is_none()
    );
    assert!(
        exchange.adopt(false, &io).is_err(),
        "failed handoff consumed the scoped key"
    );
}

#[test]
fn active_independent_reverse_policy_is_preserved_and_wrong_claim_is_never_consumed() {
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let mut exchange = Exchange::new(&io);
    let grant = crate::store::Grant {
        client_id: exchange.connection.owner.clone(),
        public_key: exchange.connection.owner_key,
        kind: "cli".into(),
        origin: "cli".into(),
        name: "Independent owner".into(),
        agents: "all".into(),
        scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
        mode: "direct".into(),
        issued_at_ms: crate::pairing::now_ms().unwrap(),
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    };
    let original = {
        let mut store = exchange.peer.store.lock().unwrap();
        store.confirm_member_grant(&grant).unwrap();
        store.talk(&grant.client_id, true).unwrap().unwrap()
    };
    exchange.adopt(false, &io).unwrap().unwrap();
    assert_eq!(
        exchange
            .peer
            .store
            .lock()
            .unwrap()
            .grant(&grant.client_id)
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(exchange.peer.counts(), (1, 1, 1, 0));

    let mut other = Exchange::new(&io);
    let old = other.submitted.clone();
    let mut different = other.submitted.candidate.clone();
    different.nonce[0] ^= 1;
    // This proof is itself valid. Only the durable-original fence can reject
    // its substitution during the already authenticated original handoff.
    other.submitted = other
        .delivery
        .with_code(|code| {
            SubmissionAttempt::new(
                other.backend.peer(different.principal.clone()),
                other.delivery.context.clone(),
                different,
                code,
                &other.peer.machine_key(),
                crate::pairing::now_ms().unwrap(),
            )
        })
        .unwrap()
        .signed_submission()
        .clone();
    assert!(other.adopt(true, &io).is_err());
    assert_eq!(other.peer.counts(), (0, 0, 0, 0));
    let saved = other
        .peer
        .store
        .lock()
        .unwrap()
        .cloud_connection(&other.connection.owner)
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.pending_submission.unwrap().request,
        crate::cloud_pairing::wire::v2::submission(&other.delivery.context, &old).unwrap()
    );
}

#[test]
fn late_independent_reverse_grant_keeps_policy_despite_an_earlier_sending_yes() {
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    for independent_sending in [false, true] {
        let mut exchange = Exchange::new(&io);
        // The answer was obtained while no reverse member existed. Another
        // owner action establishes independent policy before adoption's tx.
        let grant = crate::store::Grant {
            client_id: exchange.connection.owner.clone(),
            public_key: exchange.connection.owner_key,
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Independent owner".into(),
            agents: "all".into(),
            scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
            mode: "direct".into(),
            issued_at_ms: crate::pairing::now_ms().unwrap(),
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        let original = {
            let mut store = exchange.peer.store.lock().unwrap();
            store.confirm_member_grant(&grant).unwrap();
            if independent_sending {
                store.talk(&grant.client_id, true).unwrap().unwrap()
            } else {
                store.grant(&grant.client_id).unwrap().unwrap()
            }
        };
        let adopted = exchange.adopt(true, &io).unwrap().unwrap();
        assert!(adopted.confirmed && adopted.pending_submission.is_none());
        exchange.peer.reopen();
        assert_eq!(
            exchange
                .peer
                .store
                .lock()
                .unwrap()
                .grant(&grant.client_id)
                .unwrap()
                .unwrap(),
            original,
            "the earlier answer cannot change independent policy found inside adoption"
        );
        assert_eq!(exchange.peer.counts(), (1, 1, 1, 0));
        assert_eq!(routes(&exchange.peer), 1);
    }
}

#[test]
fn original_receipt_after_reopen_never_reapplies_policy_or_revives_revoked_reverse_authority() {
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    for revoke in [false, true] {
        let mut exchange = Exchange::new(&io);
        exchange.adopt(true, &io).unwrap().unwrap();
        let (grant, rows) = {
            let mut store = exchange.peer.store.lock().unwrap();
            let grant = if revoke {
                store.revoke(&exchange.connection.owner).unwrap().unwrap()
            } else {
                store
                    .talk(&exchange.connection.owner, false)
                    .unwrap()
                    .unwrap()
            };
            (grant, store.cloud_route_changes().unwrap())
        };
        exchange.peer.reopen();
        exchange.resume();
        assert_eq!(
            exchange.attempt.submit(&io).unwrap(),
            WriteOutcome::Acknowledged
        );
        assert!(exchange.adopt(true, &io).unwrap().unwrap().confirmed);
        let store = exchange.peer.store.lock().unwrap();
        assert_eq!(
            store.grant(&exchange.connection.owner).unwrap().unwrap(),
            grant
        );
        assert_eq!(store.cloud_route_changes().unwrap(), rows);
        assert_eq!(exchange.backend.writes(Part::Candidate), 1);
    }
}
