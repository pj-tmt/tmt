//! The candidate uses the production submit/receipt exchange without authority.
use super::super::submission::SubmissionAttempt;
use super::faults::Controlled;
use super::*;

fn submissions<B: ScenarioBackend>(make: impl Fn() -> B) {
    let backend = make();
    let (mut context, candidate, signing) = super::super::tests::fixture();
    context.binding = backend.binding();
    context.issued_at_ms = crate::session::CLOCK_SKEW.as_millis() as u64 + 1;
    context.expires_at_ms = context.issued_at_ms + 600_000;
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let owner = OwnerActor::local_interactive();
    let sink = Sink::default();
    let coordinator = Coordinator::new(backend.clone(), sink.clone());
    coordinator
        .create(&owner, context.clone(), context.issued_at_ms, &io)
        .unwrap();
    let delivery = coordinator.deliver(&owner, context.issued_at_ms).unwrap();
    delivery.with_code(|code| {
        assert!(
            SubmissionAttempt::new(
                backend.peer(candidate.principal.clone()),
                context.clone(),
                candidate.clone(),
                code,
                &signing,
                0
            )
            .is_err(),
            "candidate accepted clock skew beyond the existing bound"
        );
        let mut attempt = SubmissionAttempt::new(
            backend.peer(candidate.principal.clone()),
            context.clone(),
            candidate.clone(),
            code,
            &signing,
            1,
        )
        .expect("candidate refused the existing clock-skew window");
        assert_eq!(attempt.submit(&io).unwrap(), WriteOutcome::Acknowledged);
    });
    coordinator.observe(context.issued_at_ms, &io).unwrap();
    assert_eq!(
        coordinator.status(context.issued_at_ms).unwrap().state,
        State::CandidateAwaitingOwner
    );
    assert!(
        coordinator
            .candidate(context.issued_at_ms)
            .unwrap()
            .unwrap()
            .candidate
            == candidate
    );
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);
    for wrong_principal in [true, false] {
        let backend = Controlled::new(make());
        let (mut context, candidate, signing) = super::super::tests::fixture();
        context.binding = backend.binding();
        let sink = Sink::default();
        let owner = OwnerActor::local_interactive();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: &stopped,
        };
        let coordinator = Coordinator::new(backend.clone(), sink.clone());
        coordinator.create(&owner, context, 1, &io).unwrap();
        let delivery = coordinator.deliver(&owner, 2).unwrap();
        let mut attempt = delivery
            .with_code(|code| {
                SubmissionAttempt::new(
                    backend.peer(candidate.principal.clone()),
                    delivery.context.clone(),
                    candidate.clone(),
                    code,
                    &signing,
                    2,
                )
            })
            .unwrap();
        backend.lose_candidate_answer();
        assert_eq!(attempt.submit(&io).unwrap(), WriteOutcome::Unconfirmed);
        assert_eq!(backend.writes(Part::Candidate), 1);
        assert_eq!(attempt.submit(&io).unwrap(), WriteOutcome::Acknowledged);
        assert_eq!(attempt.submit(&io).unwrap(), WriteOutcome::Acknowledged);
        assert_eq!(
            backend.writes(Part::Candidate),
            1,
            "unknown candidate publication was resent"
        );
        assert!(attempt.observe_receipt(&io).unwrap().is_none());
        coordinator.observe(2, &io).unwrap();
        let snapshot = coordinator.candidate(2).unwrap().unwrap();
        assert_eq!(
            sink.ledger.lock().unwrap().grants,
            0,
            "submission manufactured consent"
        );
        coordinator
            .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
            .unwrap();
        // The append-log consumer may defer visibility once; finish the owner
        // original observation before deliberately corrupting this peer's view.
        for _ in 0..2 {
            coordinator.observe(3, &io).unwrap();
        }
        backend.refuse_receipt(wrong_principal);
        assert!(
            attempt.observe_receipt(&io).is_err(),
            "peer accepted forged receipt evidence"
        );
        assert!(
            !attempt.retains_response_key(),
            "refused receipt retained its response capability"
        );
        assert_eq!(sink.ledger.lock().unwrap().grants, 1);
        assert_eq!(backend.writes(Part::Candidate), 1);
        assert_eq!(backend.writes(Part::Receipt), 1);
    }
}

#[test]
fn mailbox_candidate_unknown_write_and_receipt_refusals() {
    let (c, _, _) = super::super::tests::fixture();
    submissions(|| Mailbox::new(c.binding.clone()));
}
#[test]
fn append_log_candidate_unknown_write_and_receipt_refusals() {
    let (c, _, _) = super::super::tests::fixture();
    submissions(|| AppendLog::new(c.binding.clone()));
}
