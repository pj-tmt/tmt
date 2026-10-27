//! Browser operation recovery must commit with the actual invitation grant.
use super::durable_fixture::DurableFixture;
use super::*;
use crate::store::{DEFAULT_SCOPES, Grant};

fn browser(f: &DurableFixture) -> Grant {
    let now = crate::pairing::now_ms().unwrap();
    let grant = Grant {
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
    let mut store = f.store.lock().unwrap();
    store.insert_grant(&grant).unwrap();
    store
        .designate(&grant.client_id, &grant.origin, now)
        .unwrap();
    store.grant(&grant.client_id).unwrap().unwrap()
}
fn outcome(f: &DurableFixture, browser: &Grant, operation: &str) -> serde_json::Value {
    crate::management::receipt(
        &f.store.lock().unwrap().connection,
        &browser.client_id,
        operation,
        crate::pairing::now_ms().unwrap(),
    )
    .unwrap()
}
fn browser_original<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    for sending in [false, true] {
        let f = DurableFixture::new();
        let browser = browser(&f);
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let (coordinator, delivery, snapshot) =
            f.pending(make(f.context.binding.clone()), f.sink.clone(), &io);
        let id = crate::store::uuid_v4().unwrap();
        let intent = [21; 32];
        assert!(
            f.store
                .lock()
                .unwrap()
                .adopt_invitation_confirmation(&browser, &id, &intent)
                .unwrap()
                .is_none()
        );
        assert_eq!(outcome(&f, &browser, &id)["state"], "unknown");
        let actor = OwnerActor::owner_browser_confirmation(
            &browser,
            id.clone(),
            intent,
            &snapshot,
            sending,
        )
        .unwrap();
        let status = coordinator
            .confirm_owner_decision(
                &actor,
                &snapshot,
                OwnerDecision {
                    matched: true,
                    enable_sending: sending,
                },
                &PairingPolicy::default(),
                crate::pairing::now_ms().unwrap(),
                &io,
            )
            .unwrap();
        assert!(status.local_confirmation_known);
        let saved = outcome(&f, &browser, &id);
        assert_eq!(
            saved["state"], "committed",
            "browser original did not settle with the invitation grant"
        );
        assert_eq!(
            saved["result"]["invitation"]["offerId"],
            snapshot.context.offer
        );
        assert_eq!(
            saved["result"]["invitation"]["clientId"],
            snapshot.candidate.machine
        );
        assert_eq!(saved["result"]["invitation"]["sendingEnabled"], sending);
        let original = f.sink.original(&snapshot).unwrap().unwrap();
        assert_eq!(saved["result"]["invitation"]["memberId"], original.member);
        assert_eq!(f.counts(), (2, 2, 2, 2));
        let journal: String = f
            .store
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT outcome FROM management_receipts WHERE id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let actor_member = f
            .store
            .lock()
            .unwrap()
            .member_for_client(&browser.client_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&journal).unwrap()["actorMemberId"],
            actor_member.member_id
        );
        delivery.with_code(|code| assert!(!journal.contains(&canonical::base64url(code))));
        drop(coordinator);
        f.reopen();
        assert_eq!(outcome(&f, &browser, &id), saved);
        assert_eq!(
            f.sink.original(&snapshot).unwrap().unwrap().receipt,
            original.receipt
        );
        assert!(
            f.store
                .lock()
                .unwrap()
                .adopt_invitation_confirmation(&browser, &id, &intent)
                .unwrap()
                .as_ref()
                == Some(&saved)
        );
        assert_eq!(
            f.counts(),
            (2, 2, 2, 2),
            "original lookup replayed confirmation"
        );
    }
}
fn browser_settlement_rollback<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    let f = DurableFixture::new();
    let browser = browser(&f);
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (coordinator, _, snapshot) =
        f.pending(make(f.context.binding.clone()), f.sink.clone(), &io);
    let id = crate::store::uuid_v4().unwrap();
    let intent = [22; 32];
    f.store
        .lock()
        .unwrap()
        .adopt_invitation_confirmation(&browser, &id, &intent)
        .unwrap();
    f.store.lock().unwrap().connection.execute_batch(&format!(
        "CREATE TRIGGER fail_browser_settlement BEFORE UPDATE OF outcome ON management_receipts WHEN OLD.id='{id}' BEGIN SELECT RAISE(ABORT,'settlement fault'); END;"
    )).unwrap();
    let actor =
        OwnerActor::owner_browser_confirmation(&browser, id.clone(), intent, &snapshot, false)
            .unwrap();
    assert!(
        coordinator
            .confirm(
                &actor,
                &snapshot,
                true,
                &PairingPolicy::default(),
                crate::pairing::now_ms().unwrap(),
                &io
            )
            .is_err(),
        "invitation committed despite failed browser settlement"
    );
    assert_eq!(
        f.counts(),
        (1, 1, 1, 1),
        "failed settlement left partial invitation authority"
    );
    assert!(f.sink.original(&snapshot).unwrap().is_none());
    assert_eq!(outcome(&f, &browser, &id)["state"], "unknown");
    f.reopen();
    assert_eq!(f.counts(), (1, 1, 1, 1));
}
#[test]
fn mailbox_browser_original_is_atomic_and_survives_reopen() {
    browser_original(Mailbox::new);
}
#[test]
fn append_log_browser_original_is_atomic_and_survives_reopen() {
    browser_original(AppendLog::new);
}
#[test]
fn mailbox_browser_settlement_failure_rolls_back_invitation() {
    browser_settlement_rollback(Mailbox::new);
}
#[test]
fn append_log_browser_settlement_failure_rolls_back_invitation() {
    browser_settlement_rollback(AppendLog::new);
}

fn browser_intent_fence<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    for changed in ["intent", "snapshot", "sending"] {
        let f = DurableFixture::new();
        let browser = browser(&f);
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let (coordinator, _, snapshot) =
            f.pending(make(f.context.binding.clone()), f.sink.clone(), &io);
        let id = crate::store::uuid_v4().unwrap();
        let intent = [23; 32];
        f.store
            .lock()
            .unwrap()
            .adopt_invitation_confirmation(&browser, &id, &intent)
            .unwrap();
        let mut actor =
            OwnerActor::owner_browser_confirmation(&browser, id.clone(), intent, &snapshot, false)
                .unwrap();
        match changed {
            "intent" => actor.1.as_mut().unwrap().intent[0] ^= 1,
            "snapshot" => actor.1.as_mut().unwrap().snapshot[0] ^= 1,
            "sending" => actor.1.as_mut().unwrap().sending = true,
            _ => unreachable!(),
        }
        let error = match coordinator.confirm(
            &actor,
            &snapshot,
            true,
            &PairingPolicy::default(),
            crate::pairing::now_ms().unwrap(),
            &io,
        ) {
            Ok(_) => panic!("changed {changed} was applied to the original decision"),
            Err(error) => error,
        };
        assert_eq!(
            error.code,
            if changed == "intent" {
                "REMOTE_INTENT_CONFLICT"
            } else {
                "REMOTE_INPUT_INVALID"
            }
        );
        assert_eq!(f.counts(), (1, 1, 1, 1));
        assert_eq!(outcome(&f, &browser, &id)["state"], "unknown");
    }
}
#[test]
fn mailbox_browser_original_fences_changed_intent_snapshot_and_sending() {
    browser_intent_fence(Mailbox::new);
}
#[test]
fn append_log_browser_original_fences_changed_intent_snapshot_and_sending() {
    browser_intent_fence(AppendLog::new);
}

fn browser_cannot_claim_terminal_original<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    let f = DurableFixture::new();
    let browser = browser(&f);
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (coordinator, delivery, snapshot) =
        f.pending(make(f.context.binding.clone()), f.sink.clone(), &io);
    coordinator
        .confirm(
            &OwnerActor::local_interactive(),
            &snapshot,
            true,
            &PairingPolicy::default(),
            crate::pairing::now_ms().unwrap(),
            &io,
        )
        .unwrap();
    let original = f.sink.original(&snapshot).unwrap().unwrap();
    let id = crate::store::uuid_v4().unwrap();
    let intent = [24; 32];
    f.store
        .lock()
        .unwrap()
        .adopt_invitation_confirmation(&browser, &id, &intent)
        .unwrap();
    let actor =
        OwnerActor::owner_browser_confirmation(&browser, id.clone(), intent, &snapshot, false)
            .unwrap();
    let (verified, snapshot, grant, receipt_key) =
        super::durable_fixture::admitted(&delivery, &snapshot.candidate);
    assert!(matches!(
        f.sink.confirm(Confirmation {
            verified: &verified,
            snapshot: &snapshot,
            actor: &actor,
            grant: &grant,
            receipt_key: &receipt_key,
            sending: false,
            io: &io,
        }),
        Err(ConfirmationFailure::Refused(_))
    ));
    assert_eq!(outcome(&f, &browser, &id)["state"], "unknown");
    assert_eq!(
        f.sink.original(&snapshot).unwrap().unwrap().receipt,
        original.receipt
    );
    assert_eq!(f.counts(), (2, 2, 2, 2));
}
#[test]
fn mailbox_browser_operation_cannot_claim_a_terminal_confirmation() {
    browser_cannot_claim_terminal_original(Mailbox::new);
}
#[test]
fn append_log_browser_operation_cannot_claim_a_terminal_confirmation() {
    browser_cannot_claim_terminal_original(AppendLog::new);
}
