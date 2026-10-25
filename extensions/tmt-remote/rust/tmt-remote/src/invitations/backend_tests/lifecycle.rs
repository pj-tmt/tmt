//! Shared lifecycle and original-receipt recovery, with no backend-specific owner.
use super::*;

fn lifecycle<B: ScenarioBackend>(make: impl Fn() -> B) {
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let owner = OwnerActor::local_interactive();
    for terminal in [State::Refused, State::Cancelled, State::Expired] {
        let backend = make();
        let sink = Sink::default();
        let (coordinator, delivery, snapshot) = pending(backend.clone(), sink.clone(), &io);
        let status = match terminal {
            State::Refused => coordinator
                .revoke(&owner, &delivery.context.offer, 3, &io)
                .unwrap(),
            State::Cancelled => coordinator
                .disconnect(&delivery.context.offer, 3, &io)
                .unwrap(),
            _ => {
                assert!(coordinator.expire(&delivery.context.offer, 3, &io).is_err());
                coordinator
                    .expire(&delivery.context.offer, delivery.context.expires_at_ms, &io)
                    .unwrap()
            }
        };
        assert_eq!(status.state, terminal);
        assert!(!coordinator.retains_secret().unwrap());
        assert!(coordinator.deliver(&owner, 3).is_err());
        assert!(
            coordinator
                .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
                .is_err()
        );
        // Subsequent terminal signals preserve the first transition.
        assert_eq!(
            coordinator
                .cancel(&owner, &delivery.context.offer, 3, &io)
                .unwrap()
                .state,
            terminal
        );
        assert_eq!(
            coordinator
                .disconnect(&delivery.context.offer, 3, &io)
                .unwrap()
                .state,
            terminal
        );
        assert_eq!(sink.ledger.lock().unwrap().grants, 0);
        assert_eq!(backend.writes(Part::Receipt), 0);
    }

    let backend = make();
    let sink = Sink::default();
    let (coordinator, delivery, _) = pending(backend.clone(), sink.clone(), &io);
    coordinator
        .cancel(&owner, &delivery.context.offer, 3, &io)
        .unwrap();
    let mut replacement = delivery.context.clone();
    replacement.offer = "66666666-6666-4666-8666-666666666666".into();
    coordinator
        .create(&owner, replacement.clone(), 3, &io)
        .unwrap();
    assert!(
        coordinator
            .disconnect(&delivery.context.offer, 4, &io)
            .is_err()
    );
    assert!(coordinator.deliver(&owner, 4).unwrap().context == replacement);
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);

    // A new distinct offer may replace an expired one without a separate poll.
    let backend = make();
    let sink = Sink::default();
    let (coordinator, delivery, _) = pending(backend, sink.clone(), &io);
    let mut replacement = delivery.context.clone();
    replacement.offer = "66666666-6666-4666-8666-666666666666".into();
    replacement.issued_at_ms = delivery.context.expires_at_ms;
    replacement.expires_at_ms = replacement.issued_at_ms + 60_000;
    coordinator
        .create(&owner, replacement.clone(), replacement.issued_at_ms, &io)
        .unwrap();
    assert!(
        coordinator
            .deliver(&owner, replacement.issued_at_ms)
            .unwrap()
            .context
            == replacement
    );
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);

    // A pending comparison disappears on restart, and cannot be recovered from
    // a backend candidate as though it were a private confirmed receipt.
    let backend = make();
    let sink = Sink::default();
    let (coordinator, _, snapshot) = pending(backend.clone(), sink.clone(), &io);
    drop(coordinator);
    let restarted = Coordinator::new(backend, sink.clone());
    assert!(restarted.recover(&owner, &snapshot, 3, &io).is_err());
    assert!(restarted.status(3).unwrap().offer.is_none());
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);

    // A confirmed member and original receipt survive the memory owner. Lost
    // publication is observed, never repeated; recovery cannot extend expiry.
    let backend = make();
    let sink = Sink::default();
    let (coordinator, delivery, snapshot) = pending(backend.clone(), sink.clone(), &io);
    backend.lose_next_receipt();
    assert!(
        coordinator
            .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
            .unwrap()
            .local_confirmation_known
    );
    let original = sink.original(&snapshot).unwrap().unwrap();
    drop(coordinator);
    let restarted = Coordinator::new(backend.clone(), sink.clone());
    let mut changed = snapshot.clone();
    changed.context.issued_at_ms += 1;
    changed.context.expires_at_ms += 1;
    assert!(restarted.recover(&owner, &changed, 4, &io).is_err());
    assert!(
        restarted
            .recover(&OwnerActor::automated(), &snapshot, 4, &io)
            .is_err()
    );
    let now = snapshot.context.expires_at_ms - 1000;
    assert!(
        restarted
            .recover(&owner, &snapshot, now, &io)
            .unwrap()
            .local_confirmation_known
    );
    assert!(
        restarted.original_deadline().unwrap().unwrap()
            <= sink.ledger.lock().unwrap().last_original_read.unwrap()
                + Duration::from_millis(snapshot.context.expires_at_ms - now),
        "receipt lookup renewed the invitation deadline"
    );
    assert!(!restarted.retains_secret().unwrap());
    restarted.observe(now, &io).unwrap();
    assert_eq!(restarted.status(now).unwrap().state, State::Confirmed);
    assert!(restarted.deliver(&owner, now).is_err());
    assert!(
        restarted
            .confirm(&owner, &snapshot, true, &PairingPolicy::default(), now, &io)
            .is_err()
    );
    assert!(
        restarted
            .revoke(&owner, &snapshot.context.offer, now, &io)
            .is_err()
    );
    assert_eq!(sink.ledger.lock().unwrap().grants, 1);
    assert_eq!(sink.ledger.lock().unwrap().confirms, 1);
    assert_eq!(backend.writes(Part::Receipt), 1);
    assert!(sink.original(&snapshot).unwrap().as_ref() == Some(&original));
    delivery.with_code(|code| {
        let key = crypto::response_key(
            code,
            &enrollment(&delivery.context, &snapshot.candidate).unwrap(),
        )
        .unwrap();
        crypto::verify_server_proof(&key, &original.receipt, &original.proof).unwrap();
    });
    drop(restarted);
    let expired = Coordinator::new(backend.clone(), sink.clone());
    assert!(
        expired
            .recover(&owner, &snapshot, snapshot.context.expires_at_ms, &io)
            .is_err()
    );
    assert_eq!(backend.writes(Part::Receipt), 1);
    assert_eq!(
        sink.ledger.lock().unwrap().grants,
        1,
        "expiry never rolls back durable pairing"
    );

    // A long caller budget cannot renew the original offer deadline while
    // confirmation waits for its writer. The sink sees that exact deadline.
    let backend = super::faults::Controlled::new(make());
    let sink = Sink::default();
    let long_io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(3600),
        cancelled: &stopped,
    };
    let (mut context, candidate, signing) = super::super::tests::fixture();
    context.binding = backend.binding();
    context.expires_at_ms = context.issued_at_ms + 60_000;
    let coordinator = Coordinator::new(backend.clone(), sink.clone());
    coordinator.create(&owner, context, 1, &long_io).unwrap();
    let original_deadline = coordinator.original_deadline().unwrap().unwrap();
    assert!(
        original_deadline
            <= sink.ledger.lock().unwrap().last_authorized.unwrap() + Duration::from_secs(60),
        "authorization renewed the invitation deadline"
    );
    let delivery = coordinator.deliver(&owner, 2).unwrap();
    backend
        .peer(candidate.principal.clone())
        .publish(
            &signed_publication(&delivery, &candidate, &signing),
            &long_io,
        )
        .unwrap();
    coordinator.observe(2, &long_io).unwrap();
    let snapshot = coordinator.candidate(2).unwrap().unwrap();
    coordinator
        .confirm(
            &owner,
            &snapshot,
            true,
            &PairingPolicy::default(),
            3,
            &long_io,
        )
        .unwrap();
    assert_eq!(
        sink.ledger.lock().unwrap().confirmation_deadline,
        Some(original_deadline),
        "confirmation renewed the invitation deadline"
    );
    let provider_deadlines = backend.owner_deadlines();
    assert!(!provider_deadlines.is_empty());
    assert!(
        provider_deadlines
            .iter()
            .all(|deadline| *deadline == original_deadline),
        "provider exchange renewed the invitation deadline"
    );
    assert!(original_deadline < long_io.deadline);

    // Compare-confirm races cancellation at the same shared transition owner.
    let backend = make();
    let sink = Sink::default();
    let (coordinator, _, snapshot) = pending(backend.clone(), sink.clone(), &io);
    let coordinator = Arc::new(coordinator);
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let c = coordinator.clone();
        let b = barrier.clone();
        let s = snapshot.clone();
        let io = &io;
        let confirm = scope.spawn(move || {
            b.wait();
            c.confirm(
                &OwnerActor::local_interactive(),
                &s,
                true,
                &PairingPolicy::default(),
                3,
                io,
            )
            .is_ok()
        });
        let c = coordinator.clone();
        let b = barrier.clone();
        let id = snapshot.context.offer.clone();
        let cancel = scope.spawn(move || {
            b.wait();
            c.disconnect(&id, 3, io).is_ok()
        });
        barrier.wait();
        let confirmed = confirm.join().unwrap();
        let cancelled = cancel.join().unwrap();
        assert_ne!(confirmed, cancelled);
        assert_eq!(sink.ledger.lock().unwrap().grants, usize::from(confirmed));
        assert_eq!(backend.writes(Part::Receipt), usize::from(confirmed));
    });
}
#[test]
fn mailbox_termination_restart_and_original_receipt_recovery() {
    let (context, _, _) = super::super::tests::fixture();
    lifecycle(|| Mailbox::new(context.binding.clone()));
}
#[test]
fn append_log_termination_restart_and_original_receipt_recovery() {
    let (mut context, _, _) = super::super::tests::fixture();
    context.binding.backend = "append-log".into();
    lifecycle(|| AppendLog::new(context.binding.clone()));
}
