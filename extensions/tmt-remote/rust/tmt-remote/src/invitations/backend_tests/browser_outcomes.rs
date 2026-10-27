//! Browser operation recovery must commit with the actual invitation grant.
use super::durable_fixture::DurableFixture;
use super::*;
use crate::store::Grant;

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
        let browser = f.browser();
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
        let actor = OwnerActor::owner_browser_decision(
            &browser,
            id.clone(),
            intent,
            &snapshot,
            OwnerDecision {
                matched: true,
                enable_sending: sending,
            },
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
        assert_eq!(
            f.store
                .lock()
                .unwrap()
                .refuse_invitation_confirmation(&browser, &id, &intent, "REMOTE_INVITATION_DECIDED")
                .unwrap(),
            saved,
            "a late refusal overwrote a recorded confirmation"
        );
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
    let browser = f.browser();
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
        "CREATE TRIGGER fail_browser_settlement BEFORE UPDATE OF outcome ON management_receipts WHEN OLD.id='{id}' AND json_extract(NEW.outcome,'$.outcome.state')='committed' BEGIN SELECT RAISE(ABORT,'settlement fault'); END;"
    )).unwrap();
    let actor = OwnerActor::owner_browser_decision(
        &browser,
        id.clone(),
        intent,
        &snapshot,
        OwnerDecision {
            matched: true,
            enable_sending: false,
        },
    )
    .unwrap();
    let error = match coordinator.confirm_owner_decision(
        &actor,
        &snapshot,
        OwnerDecision {
            matched: true,
            enable_sending: false,
        },
        &PairingPolicy::default(),
        crate::pairing::now_ms().unwrap(),
        &io,
    ) {
        Err(OwnerDecisionFailure::Refused(error)) => error,
        _ => panic!("proven rollback was not a typed definite refusal"),
    };
    assert_eq!(
        f.counts(),
        (1, 1, 1, 1),
        "failed settlement left partial invitation authority"
    );
    assert!(f.sink.original(&snapshot).unwrap().is_none());
    let refused = f
        .store
        .lock()
        .unwrap()
        .refuse_invitation_confirmation(&browser, &id, &intent, &error.code)
        .unwrap();
    assert_eq!(refused["state"], "refused");
    assert_eq!(refused["reason"], error.code);
    assert_eq!(outcome(&f, &browser, &id), refused);
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .refuse_invitation_confirmation(&browser, &id, &intent, "REMOTE_INVITATION_INVALID")
            .unwrap(),
        refused,
        "a second refusal rewrote the original code"
    );
    f.reopen();
    assert_eq!(f.counts(), (1, 1, 1, 1));
    assert_eq!(outcome(&f, &browser, &id), refused);
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
        let browser = f.browser();
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
        let mut actor = OwnerActor::owner_browser_decision(
            &browser,
            id.clone(),
            intent,
            &snapshot,
            OwnerDecision {
                matched: true,
                enable_sending: false,
            },
        )
        .unwrap();
        match changed {
            "intent" => actor.operation.as_mut().unwrap().intent[0] ^= 1,
            "snapshot" => actor.operation.as_mut().unwrap().snapshot[0] ^= 1,
            "sending" => actor.operation.as_mut().unwrap().sending = true,
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
    let browser = f.browser();
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
    let actor = OwnerActor::owner_browser_decision(
        &browser,
        id.clone(),
        intent,
        &snapshot,
        OwnerDecision {
            matched: true,
            enable_sending: false,
        },
    )
    .unwrap();
    let (verified, snapshot, grant, receipt_key) =
        super::durable_fixture::admitted(&delivery, &snapshot.candidate);
    let error = match f.sink.confirm(Confirmation {
        verified: &verified,
        snapshot: &snapshot,
        actor: &actor,
        grant: &grant,
        receipt_key: &receipt_key,
        sending: false,
        io: &io,
    }) {
        Err(ConfirmationFailure::Refused(error)) => error,
        Err(_) => panic!("terminal winner became uncertain"),
        Ok(_) => panic!("browser claimed terminal original"),
    };
    assert_eq!(error.code, "REMOTE_INVITATION_DECIDED");
    let refused = f
        .store
        .lock()
        .unwrap()
        .refuse_invitation_confirmation(&browser, &id, &intent, &error.code)
        .unwrap();
    assert_eq!(refused["state"], "refused");
    assert_eq!(refused["reason"], "REMOTE_INVITATION_DECIDED");
    assert_eq!(outcome(&f, &browser, &id), refused);
    f.reopen();
    assert_eq!(outcome(&f, &browser, &id), refused);
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

fn browser_commit_unknown_reads_only_the_recorded_original<B: ScenarioBackend>(
    make: impl Fn(BackendBinding) -> B,
) {
    for committed in [false, true] {
        let f = DurableFixture::new();
        let browser = f.browser();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let sink = super::confirmation::UncertainConfirmation {
            sink: f.sink.clone(),
            committed,
        };
        let (coordinator, _, snapshot) = f.pending(make(f.context.binding.clone()), sink, &io);
        let id = crate::store::uuid_v4().unwrap();
        let intent = [28; 32];
        f.store
            .lock()
            .unwrap()
            .adopt_invitation_confirmation(&browser, &id, &intent)
            .unwrap();
        let actor = OwnerActor::owner_browser_decision(
            &browser,
            id.clone(),
            intent,
            &snapshot,
            OwnerDecision {
                matched: true,
                enable_sending: false,
            },
        )
        .unwrap();
        assert!(matches!(
            coordinator.confirm_owner_decision(
                &actor,
                &snapshot,
                OwnerDecision {
                    matched: true,
                    enable_sending: false
                },
                &PairingPolicy::default(),
                crate::pairing::now_ms().unwrap(),
                &io
            ),
            Err(OwnerDecisionFailure::CommitUnknown(_))
        ));
        let original = outcome(&f, &browser, &id);
        assert_eq!(
            original["state"],
            if committed { "committed" } else { "unknown" }
        );
        assert_eq!(
            f.counts(),
            if committed {
                (2, 2, 2, 2)
            } else {
                (1, 1, 1, 1)
            }
        );
        coordinator
            .observe(crate::pairing::now_ms().unwrap(), &io)
            .unwrap();
        assert_eq!(
            outcome(&f, &browser, &id),
            original,
            "uncertainty overwrote the original outcome"
        );
        f.reopen();
        assert_eq!(outcome(&f, &browser, &id), original);
    }
}
#[test]
fn mailbox_browser_commit_unknown_never_settles_a_refusal() {
    browser_commit_unknown_reads_only_the_recorded_original(Mailbox::new);
}
#[test]
fn append_log_browser_commit_unknown_never_settles_a_refusal() {
    browser_commit_unknown_reads_only_the_recorded_original(AppendLog::new);
}
