//! Real serving Store fixture; it owns the one opener and disposes its root.
use super::*;
use crate::{
    invitations::store::StoreConfirmation,
    state::{Layout, Serving},
    store::Store,
};
use std::path::PathBuf;

pub(super) struct DurableFixture {
    pub(super) store: Arc<Mutex<Store>>,
    pub(super) sink: StoreConfirmation,
    pub(super) context: Context,
    serving: Serving,
    root: PathBuf,
}
impl DurableFixture {
    pub(super) fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("tmt-i2-{}", crate::store::uuid_v4().unwrap()));
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let mut store = Store::open(&serving).unwrap();
        let (mut context, _, _) = super::super::tests::fixture();
        context.owner = store.machine().unwrap().id;
        context.issued_at_ms = crate::pairing::now_ms().unwrap();
        context.expires_at_ms = context.issued_at_ms + 600_000;
        let store = Arc::new(Mutex::new(store));
        let sink = StoreConfirmation::new(store.clone(), context.owner_key).unwrap();
        Self {
            store,
            sink,
            context,
            serving,
            root,
        }
    }
    pub(super) fn browser(&self) -> crate::store::Grant {
        let now = crate::pairing::now_ms().unwrap();
        let grant = crate::store::Grant {
            client_id: crate::store::uuid_v4().unwrap(),
            public_key: ed25519_dalek::SigningKey::from_bytes(&[24; 32])
                .verifying_key()
                .to_bytes(),
            kind: "browser".into(),
            origin: "http://127.0.0.1:49152".into(),
            name: "Owner".into(),
            agents: "all".into(),
            scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
            mode: "direct".into(),
            issued_at_ms: now,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        let mut store = self.store.lock().unwrap();
        store.insert_grant(&grant).unwrap();
        store
            .designate(&grant.client_id, &grant.origin, now)
            .unwrap();
        store.grant(&grant.client_id).unwrap().unwrap()
    }
    pub(super) fn reopen(&self) {
        let mut store = self.store.lock().unwrap();
        store.connection = rusqlite::Connection::open_in_memory().unwrap();
        *store = Store::open(&self.serving).unwrap();
    }
    pub(super) fn machine_key(&self) -> crate::state::MachineKey {
        crate::state::MachineKey::open(&Layout::open(&self.root).unwrap()).unwrap()
    }
    pub(super) fn counts(&self) -> (i64, i64, i64, i64) {
        self.store.lock().unwrap().connection.query_row(
            "SELECT (SELECT COUNT(*) FROM grants),(SELECT COUNT(*) FROM members),(SELECT COUNT(*) FROM member_machines),(SELECT COUNT(*) FROM management_receipts)",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).unwrap()
    }
    pub(super) fn pending<B: ScenarioBackend, S: ConfirmationSink>(
        &self,
        backend: B,
        sink: S,
        io: &IoBudget<'_>,
    ) -> (Coordinator<B, S>, super::super::service::Delivery, Snapshot) {
        let (_, candidate, signing) = super::super::tests::fixture();
        self.pending_candidate(backend, sink, candidate, signing, io)
    }
    pub(super) fn pending_candidate<B: ScenarioBackend, S: ConfirmationSink>(
        &self,
        backend: B,
        sink: S,
        candidate: Candidate,
        signing: ed25519_dalek::SigningKey,
        io: &IoBudget<'_>,
    ) -> (Coordinator<B, S>, super::super::service::Delivery, Snapshot) {
        let coordinator = Coordinator::new(backend.clone(), sink);
        let now = crate::pairing::now_ms().unwrap();
        coordinator
            .create(
                &OwnerActor::local_interactive(),
                self.context.clone(),
                now,
                io,
            )
            .unwrap();
        let delivery = coordinator
            .deliver(&OwnerActor::local_interactive(), now)
            .unwrap();
        let mut attempt = delivery
            .with_code(|code| {
                super::super::submission::SubmissionAttempt::new(
                    backend.peer(candidate.principal.clone()),
                    delivery.context.clone(),
                    candidate,
                    code,
                    &signing,
                    now,
                )
            })
            .unwrap();
        assert_eq!(attempt.submit(io).unwrap(), WriteOutcome::Acknowledged);
        coordinator.observe(now, io).unwrap();
        let snapshot = coordinator.candidate(now).unwrap().unwrap();
        (coordinator, delivery, snapshot)
    }
}

pub(super) fn admitted(
    delivery: &super::super::service::Delivery,
    candidate: &Candidate,
) -> (
    super::super::VerifiedCandidate,
    Snapshot,
    crate::store::Grant,
    ReceiptKey,
) {
    let (_, _, signing) = super::super::tests::fixture();
    let Payload::Candidate(submission) = signed_publication(delivery, candidate, &signing).payload
    else {
        unreachable!()
    };
    let verified = delivery
        .with_code(|code| {
            verify_enrollment(
                &delivery.context,
                candidate,
                &candidate.principal,
                code,
                &submission.mac,
                &submission.signature,
                delivery.context.issued_at_ms,
            )
        })
        .unwrap();
    let snapshot = Snapshot {
        context: delivery.context.clone(),
        candidate: candidate.clone(),
        digest: *verified.digest(),
    };
    let grant = crate::store::Grant {
        client_id: candidate.machine.clone(),
        public_key: candidate.public_key,
        kind: "cli".into(),
        origin: "cli".into(),
        name: candidate.name.clone(),
        agents: "all".into(),
        scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
        mode: "direct".into(),
        issued_at_ms: delivery.context.issued_at_ms,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    };
    let key = delivery.with_code(|code| {
        ReceiptKey(
            crypto::response_key(code, &enrollment(&snapshot.context, candidate).unwrap()).unwrap(),
        )
    });
    (verified, snapshot, grant, key)
}
impl Drop for DurableFixture {
    fn drop(&mut self) {
        self.store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .connection = rusqlite::Connection::open_in_memory().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
