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
        let mut adopted = false;
        assert!(
            attempt
                .observe_receipt_with(&io, |_, _, _| {
                    adopted = true;
                    Ok(())
                })
                .is_err(),
            "peer accepted forged receipt evidence"
        );
        assert!(!adopted, "forged receipt reached local authority adoption");
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

#[test]
fn opaque_machine_signer_uses_the_shared_proof_and_refuses_invalid_output() {
    struct InvalidSigner([u8; 32]);
    impl NativeSigner for InvalidSigner {
        fn public_key(&self) -> [u8; 32] {
            self.0
        }
        fn sign_possession(&self, _: &[u8]) -> [u8; 64] {
            [0; 64]
        }
    }
    let fixture = super::durable_fixture::DurableFixture::new();
    let machine_key = fixture.machine_key();
    let (context, mut candidate, _) = super::super::tests::fixture();
    candidate.public_key = machine_key.public();
    let backend = Mailbox::new(context.binding.clone());
    let sink = Sink::default();
    let coordinator = Coordinator::new(backend.clone(), sink.clone());
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let actor = OwnerActor::local_interactive();
    coordinator.create(&actor, context, 1, &io).unwrap();
    let delivery = coordinator.deliver(&actor, 2).unwrap();
    let mut attempt = delivery.with_code(|code| {
        assert!(
            SubmissionAttempt::new(
                backend.peer(candidate.principal.clone()),
                delivery.context.clone(),
                candidate.clone(),
                code,
                &InvalidSigner(candidate.public_key),
                2,
            )
            .is_err(),
            "opaque signer output bypassed native possession verification"
        );
        let mut wrong_pin = candidate.clone();
        wrong_pin.public_key = super::super::tests::fixture().1.public_key;
        assert!(
            SubmissionAttempt::new(
                backend.peer(candidate.principal.clone()),
                delivery.context.clone(),
                wrong_pin,
                code,
                &machine_key,
                2,
            )
            .is_err(),
            "signer key differed from the frozen candidate pin"
        );
        SubmissionAttempt::new(
            backend.peer(candidate.principal.clone()),
            delivery.context.clone(),
            candidate.clone(),
            code,
            &machine_key,
            2,
        )
        .unwrap()
    });
    assert_eq!(attempt.submit(&io).unwrap(), WriteOutcome::Acknowledged);
    coordinator.observe(2, &io).unwrap();
    assert!(coordinator.candidate(2).unwrap().unwrap().candidate == candidate);
    assert_eq!(
        sink.ledger.lock().unwrap().grants,
        0,
        "opaque native signing supplied owner consent"
    );
    assert_eq!(fixture.counts().0, 0);
}

fn restored_submission<B: ScenarioBackend>(make: impl Fn() -> B) {
    for original_reached_provider in [false, true] {
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
        coordinator.create(&owner, context.clone(), 1, &io).unwrap();
        let delivery = coordinator.deliver(&owner, 2).unwrap();
        let mut attempt = delivery
            .with_code(|code| {
                SubmissionAttempt::new(
                    backend.peer(candidate.principal.clone()),
                    context.clone(),
                    candidate.clone(),
                    code,
                    &signing,
                    2,
                )
            })
            .unwrap();
        let signed = attempt.signed_submission().clone();
        if original_reached_provider {
            backend.lose_candidate_answer();
            assert_eq!(attempt.submit(&io).unwrap(), WriteOutcome::Unconfirmed);
        }
        // A persisted claim can precede I/O. Dropping the process's attempt is
        // not proof that the provider has or has not committed its exact row.
        drop(attempt);
        let mut restored = delivery
            .with_code(|code| {
                assert!(
                    SubmissionAttempt::resume_original(
                        backend.peer(candidate.principal.clone()),
                        context.clone(),
                        signed.clone(),
                        &[0; 16],
                        3
                    )
                    .is_err()
                );
                assert!(
                    SubmissionAttempt::resume_original(
                        backend.peer(candidate.principal.clone()),
                        context.clone(),
                        signed.clone(),
                        code,
                        context.expires_at_ms
                    )
                    .is_err()
                );
                SubmissionAttempt::resume_original(
                    backend.peer(candidate.principal.clone()),
                    context.clone(),
                    signed,
                    code,
                    3,
                )
            })
            .unwrap();
        for _ in 0..3 {
            assert_eq!(
                restored.submit(&io).unwrap(),
                if original_reached_provider {
                    WriteOutcome::Acknowledged
                } else {
                    WriteOutcome::Unconfirmed
                }
            );
        }
        assert_eq!(
            backend.writes(Part::Candidate),
            usize::from(original_reached_provider),
            "restored candidate claim published another original"
        );
        assert_eq!(sink.ledger.lock().unwrap().grants, 0);
        if original_reached_provider {
            coordinator.observe(3, &io).unwrap();
            let snapshot = coordinator.candidate(3).unwrap().unwrap();
            coordinator
                .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 4, &io)
                .unwrap();
            let original = sink.original(&snapshot).unwrap().unwrap();
            assert_eq!(
                restored.observe_receipt(&io).unwrap(),
                Some(original.receipt)
            );
        } else {
            assert!(coordinator.candidate(3).unwrap().is_none());
            assert!(restored.observe_receipt(&io).unwrap().is_none());
        }
    }
}
#[test]
fn mailbox_restored_submission_claim_observes_only_even_when_no_original_is_visible() {
    let (c, _, _) = super::super::tests::fixture();
    restored_submission(|| Mailbox::new(c.binding.clone()));
}
#[test]
fn append_log_restored_submission_claim_observes_only_even_when_no_original_is_visible() {
    let (c, _, _) = super::super::tests::fixture();
    restored_submission(|| AppendLog::new(c.binding.clone()));
}

fn receipt_adoption<B: ScenarioBackend>(make: impl Fn() -> B) {
    for failed_adoption in [false, true] {
        let backend = Controlled::new(make());
        let (mut context, candidate, signing) = super::super::tests::fixture();
        context.binding = backend.binding();
        let sink = Sink::default();
        let actor = OwnerActor::local_interactive();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(10),
            cancelled: &stopped,
        };
        let coordinator = Coordinator::new(backend.clone(), sink.clone());
        coordinator.create(&actor, context.clone(), 1, &io).unwrap();
        let delivery = coordinator.deliver(&actor, 2).unwrap();
        let mut attempt = delivery
            .with_code(|code| {
                SubmissionAttempt::new(
                    backend.peer(candidate.principal.clone()),
                    context.clone(),
                    candidate.clone(),
                    code,
                    &signing,
                    2,
                )
            })
            .unwrap();
        let signed = attempt.signed_submission().clone();
        attempt.submit(&io).unwrap();
        coordinator.observe(2, &io).unwrap();
        let snapshot = coordinator.candidate(2).unwrap().unwrap();
        coordinator
            .confirm(&actor, &snapshot, true, &PairingPolicy::default(), 3, &io)
            .unwrap();
        let original = sink.original(&snapshot).unwrap().unwrap();
        let mut calls = 0;
        let adopted = attempt.observe_receipt_with(&io, |bytes, key, bounded| {
            calls += 1;
            bounded.check().unwrap();
            assert_eq!(bytes, original.receipt);
            assert_eq!(key.prove(bytes).unwrap(), original.proof);
            if failed_adoption {
                return Err(crate::error::RemoteError::new(
                    "REMOTE_MEMBER_OWNER_REQUIRED",
                    "fixture definite local refusal",
                ));
            }
            Ok("local adoption")
        });
        assert_eq!(calls, 1);
        assert!(!attempt.retains_response_key());
        if failed_adoption {
            assert_eq!(adopted.err().unwrap().code, "REMOTE_MEMBER_OWNER_REQUIRED");
            let mut resumed = delivery
                .with_code(|code| {
                    SubmissionAttempt::resume_original(
                        backend.peer(candidate.principal.clone()),
                        context.clone(),
                        signed,
                        code,
                        4,
                    )
                })
                .unwrap();
            assert_eq!(
                resumed
                    .observe_receipt_with(&io, |bytes, key, _| {
                        assert_eq!(key.prove(bytes).unwrap(), original.proof);
                        Ok(bytes.to_vec())
                    })
                    .unwrap(),
                Some(original.receipt.clone())
            );
            assert!(!resumed.retains_response_key());
        } else {
            assert_eq!(adopted.unwrap(), Some("local adoption"));
            assert!(
                attempt
                    .observe_receipt_with(&io, |_, _, _| {
                        calls += 1;
                        Ok(())
                    })
                    .is_err()
            );
            assert_eq!(
                calls, 1,
                "consumed receipt capability repeated local adoption"
            );
            assert_eq!(
                attempt.observe_receipt(&io).unwrap(),
                Some(original.receipt)
            );
        }
        assert_eq!(backend.writes(Part::Candidate), 1);
        assert_eq!(sink.ledger.lock().unwrap().grants, 1);
    }
}
#[test]
fn mailbox_receipt_adoption_scopes_the_key_and_consumes_it_on_success_or_error() {
    let (c, _, _) = super::super::tests::fixture();
    receipt_adoption(|| Mailbox::new(c.binding.clone()));
}
#[test]
fn append_log_receipt_adoption_scopes_the_key_and_consumes_it_on_success_or_error() {
    let (c, _, _) = super::super::tests::fixture();
    receipt_adoption(|| AppendLog::new(c.binding.clone()));
}
