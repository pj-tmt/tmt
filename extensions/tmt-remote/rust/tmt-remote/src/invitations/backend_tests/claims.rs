//! Candidate pre-I/O claims use the existing connection record, without changing
//! confirmed trust or introducing a provider-state table.
use super::durable_fixture::DurableFixture;
use super::*;
use crate::{
    cloud_pairing::{Connection, wire::v2},
    invitations::submission::SubmissionAttempt,
};

fn context(f: &DurableFixture) -> (Context, Candidate) {
    let (mut context, mut candidate, _) = super::super::tests::fixture();
    context.binding = BackendBinding {
        backend: "firestore".into(),
        reference: b"demo-native-claim".to_vec(),
        owner_principal: b"owner-uid".to_vec(),
    };
    context.issued_at_ms = crate::pairing::now_ms().unwrap();
    context.expires_at_ms = context.issued_at_ms + 600_000;
    candidate.machine = f.context.owner.clone();
    candidate.public_key = f.machine_key().public();
    candidate.principal = b"new-local-uid".to_vec();
    (context, candidate)
}
fn connection(context: &Context, candidate: &Candidate) -> Connection {
    Connection {
        version: 1,
        name: "peer".into(),
        owner: context.owner.clone(),
        owner_key: context.owner_key,
        config: serde_json::json!({"projectId":"demo-native-claim","appId":"1:123:web:test","apiKey":"public-test"}),
        peer: candidate.machine.clone(),
        uid: String::from_utf8(candidate.principal.clone()).unwrap(),
        request: Vec::new(),
        confirmed: false,
        descriptor: Vec::new(),
        reroute_token: None,
        pending_submission: None,
    }
}

#[test]
fn private_candidate_claim_survives_reopen_and_missing_original_never_resends() {
    let f = DurableFixture::new();
    let (context, candidate) = context(&f);
    let desired = connection(&context, &candidate);
    let backend = Mailbox::new(context.binding.clone());
    let attempt = SubmissionAttempt::new(
        backend.peer(candidate.principal.clone()),
        context.clone(),
        candidate.clone(),
        &[4; 16],
        &f.machine_key(),
        context.issued_at_ms,
    )
    .unwrap();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    assert!(
        f.store
            .lock()
            .unwrap()
            .claim_cloud_submission(&desired, &context, attempt.signed_submission(), &io)
            .unwrap()
    );
    drop(attempt);
    f.reopen();
    let saved = f
        .store
        .lock()
        .unwrap()
        .cloud_connection(&context.owner)
        .unwrap()
        .unwrap();
    assert!(!saved.confirmed);
    let pending = saved.pending_submission.unwrap();
    assert!(pending.context == context);
    let raw: serde_json::Value = serde_json::from_slice(&pending.request).unwrap();
    for secret in ["code", "gate", "responseKey"] {
        assert!(raw.get(secret).is_none());
    }
    let signed = v2::read_submission(
        &pending.request,
        &context,
        &desired.uid,
        crate::invitations::service::gate(&context, &[4; 16]).unwrap(),
    )
    .unwrap();
    assert!(
        !f.store
            .lock()
            .unwrap()
            .claim_cloud_submission(&desired, &context, &signed, &io)
            .unwrap()
    );
    let mut resumed = SubmissionAttempt::resume_original(
        backend.peer(candidate.principal),
        context.clone(),
        signed,
        &[4; 16],
        crate::pairing::now_ms().unwrap(),
    )
    .unwrap();
    assert_eq!(resumed.submit(&io).unwrap(), WriteOutcome::Unconfirmed);
    assert_eq!(backend.writes(Part::Candidate), 0);
    assert_eq!(f.counts(), (0, 0, 0, 0));
}

#[test]
fn pending_claim_never_replaces_confirmed_pin_uid_policy_or_a_concurrent_claim() {
    let f = DurableFixture::new();
    let (context, candidate) = context(&f);
    let desired = connection(&context, &candidate);
    let mut old = desired.clone();
    old.confirmed = true;
    old.uid = "old-local-uid".into();
    old.reroute_token = Some([7; 32]);
    f.store.lock().unwrap().save_cloud_connection(&old).unwrap();
    let grant = crate::store::Grant {
        client_id: context.owner.clone(),
        public_key: context.owner_key,
        kind: "cli".into(),
        origin: "cli".into(),
        name: "Existing owner".into(),
        agents: "all".into(),
        scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
        mode: "direct".into(),
        issued_at_ms: context.issued_at_ms,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    };
    f.store.lock().unwrap().insert_grant(&grant).unwrap();
    let original_grant = f
        .store
        .lock()
        .unwrap()
        .talk(&context.owner, true)
        .unwrap()
        .unwrap();
    let stale_worker_snapshot = old.clone();
    let backend = Mailbox::new(context.binding.clone());
    let attempt = SubmissionAttempt::new(
        backend,
        context.clone(),
        candidate.clone(),
        &[4; 16],
        &f.machine_key(),
        context.issued_at_ms,
    )
    .unwrap();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    assert!(
        f.store
            .lock()
            .unwrap()
            .claim_cloud_submission(&desired, &context, attempt.signed_submission(), &io)
            .unwrap()
    );
    f.store
        .lock()
        .unwrap()
        .save_cloud_connection(&stale_worker_snapshot)
        .unwrap();
    let saved = f
        .store
        .lock()
        .unwrap()
        .cloud_connection(&context.owner)
        .unwrap()
        .unwrap();
    assert!(saved.confirmed && saved.pending_submission.is_some());
    assert_eq!(saved.owner_key, old.owner_key);
    assert_eq!(saved.uid, old.uid);
    assert_eq!(saved.reroute_token, old.reroute_token);
    assert_eq!(saved.request, old.request);
    let pending = saved.pending_submission.unwrap();
    let mut other = candidate;
    other.nonce[0] ^= 1;
    let changed = SubmissionAttempt::new(
        Mailbox::new(context.binding.clone()),
        context.clone(),
        other.clone(),
        &[4; 16],
        &f.machine_key(),
        context.issued_at_ms,
    )
    .unwrap();
    let error = f
        .store
        .lock()
        .unwrap()
        .claim_cloud_submission(&desired, &context, changed.signed_submission(), &io)
        .unwrap_err();
    assert_eq!(error.code, "REMOTE_ID_CONFLICT");
    let mut other_offer = context.clone();
    other_offer.offer = crate::store::uuid_v4().unwrap();
    let competing = SubmissionAttempt::new(
        Mailbox::new(other_offer.binding.clone()),
        other_offer.clone(),
        other,
        &[4; 16],
        &f.machine_key(),
        context.issued_at_ms,
    )
    .unwrap();
    let error = f
        .store
        .lock()
        .unwrap()
        .claim_cloud_submission(&desired, &other_offer, competing.signed_submission(), &io)
        .unwrap_err();
    assert_eq!(error.code, "REMOTE_INVITATION_BUSY");
    f.reopen();
    let saved = f
        .store
        .lock()
        .unwrap()
        .cloud_connection(&context.owner)
        .unwrap()
        .unwrap();
    assert!(saved.pending_submission.as_ref() == Some(&pending));
    let persisted_grant = f
        .store
        .lock()
        .unwrap()
        .grant(&context.owner)
        .unwrap()
        .unwrap();
    assert_eq!(persisted_grant, original_grant);
    assert_eq!(f.counts(), (1, 1, 1, 0));
}

#[test]
fn failed_candidate_claim_or_cancelled_budget_leaves_no_partial_connection() {
    let f = DurableFixture::new();
    let (context, candidate) = context(&f);
    let desired = connection(&context, &candidate);
    let attempt = SubmissionAttempt::new(
        Mailbox::new(context.binding.clone()),
        context.clone(),
        candidate,
        &[4; 16],
        &f.machine_key(),
        context.issued_at_ms,
    )
    .unwrap();
    let stopped = AtomicBool::new(true);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    assert!(
        f.store
            .lock()
            .unwrap()
            .claim_cloud_submission(&desired, &context, attempt.signed_submission(), &io)
            .is_err()
    );
    stopped.store(false, Ordering::Release);
    f.store.lock().unwrap().connection.execute_batch("CREATE TRIGGER fail_claim BEFORE INSERT ON cloud_remotes BEGIN SELECT RAISE(ABORT,'fixture claim failure'); END").unwrap();
    assert!(
        f.store
            .lock()
            .unwrap()
            .claim_cloud_submission(&desired, &context, attempt.signed_submission(), &io)
            .is_err()
    );
    f.reopen();
    let saved = f
        .store
        .lock()
        .unwrap()
        .cloud_connection(&context.owner)
        .unwrap();
    assert!(saved.is_none());
    assert_eq!(f.counts(), (0, 0, 0, 0));
}
