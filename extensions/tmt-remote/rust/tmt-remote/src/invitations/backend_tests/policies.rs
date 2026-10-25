//! The provider candidate cannot supply policy or outlive owner designation.
use super::*;

#[derive(Clone)]
struct AdmissionRace {
    sink: Sink,
    entered: Arc<Barrier>,
    resume: Arc<Barrier>,
}
impl ConfirmationSink for AdmissionRace {
    fn authorize(
        &self,
        context: &Context,
        actor: &OwnerActor,
    ) -> Result<(), crate::error::RemoteError> {
        self.sink.authorize(context, actor)
    }
    fn confirm(&self, input: Confirmation<'_>) -> Result<Committed, crate::error::RemoteError> {
        self.entered.wait();
        self.resume.wait();
        self.sink.confirm(input)
    }
    fn original(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Option<Committed>, crate::error::RemoteError> {
        self.sink.original(snapshot)
    }
    fn claim(&self, record: &Committed, part: Part) -> Result<Claim, crate::error::RemoteError> {
        self.sink.claim(record, part)
    }
    fn acknowledge(
        &self,
        record: &Committed,
        part: Part,
    ) -> Result<bool, crate::error::RemoteError> {
        self.sink.acknowledge(record, part)
    }
}

fn policies<B: ScenarioBackend>(make: impl Fn() -> B) {
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let owner = OwnerActor::local_interactive();
    let backend = make();
    let sink = Sink::default();
    let (coordinator, _, snapshot) = pending(backend.clone(), sink.clone(), &io);
    coordinator
        .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
        .unwrap();
    let original = sink.original(&snapshot).unwrap().unwrap();
    {
        let ledger = sink.ledger.lock().unwrap();
        let persisted = ledger.grant_rows.get(&snapshot.candidate.machine).unwrap();
        assert_eq!(
            original.receipt,
            crate::pairing::receipt(
                persisted,
                &snapshot.context.owner,
                &snapshot.context.owner_key
            )
            .unwrap()
        );
        assert_eq!(persisted.mode, "direct");
        assert!(!persisted.permits_scope("talk"));
    }
    let receipt: serde_json::Value = serde_json::from_slice(&original.receipt).unwrap();
    assert_eq!(receipt["grant"]["mode"], "direct");
    assert_eq!(receipt["grant"]["agents"], "all");
    assert_eq!(
        receipt["grant"]["scopes"],
        serde_json::json!(crate::store::DEFAULT_SCOPES)
    );
    assert_eq!(receipt["grant"]["clientId"], snapshot.candidate.machine);
    assert_eq!(
        receipt["grant"]["publicKey"],
        canonical::base64url(&snapshot.candidate.public_key)
    );
    assert_eq!(receipt["grant"]["machineId"], snapshot.context.owner);
    // A separately authorized later member-policy change cannot replace the
    // original confirmation receipt. This is fake state, not that public API.
    {
        let mut ledger = sink.ledger.lock().unwrap();
        let current = ledger
            .grant_rows
            .get_mut(&snapshot.candidate.machine)
            .unwrap();
        current.scopes.push("talk".into());
        current.revision += 1;
    }
    coordinator.observe(4, &io).unwrap();
    assert!(sink.original(&snapshot).unwrap().as_ref() == Some(&original));
    assert_eq!(sink.ledger.lock().unwrap().grants, 1);

    // Reject every unsupported option before a writer or provider mutation,
    // including an empty agents list and a mismatched comparison request.
    for policy in [
        PairingPolicy {
            talk: true,
            ..PairingPolicy::default()
        },
        PairingPolicy {
            hold: true,
            ..PairingPolicy::default()
        },
        PairingPolicy {
            agents: Some(vec![]),
            ..PairingPolicy::default()
        },
        PairingPolicy {
            agents: Some(vec!["44444444-4444-4444-8444-444444444444".into()]),
            ..PairingPolicy::default()
        },
        PairingPolicy {
            talk: true,
            hold: true,
            agents: Some(vec!["44444444-4444-4444-8444-444444444444".into()]),
        },
    ] {
        for matched in [false, true] {
            let backend = super::faults::Controlled::new(make());
            let sink = Sink::default();
            let (coordinator, _, snapshot) = pending(backend.clone(), sink.clone(), &io);
            let before = backend.write_attempts();
            let error = match coordinator.confirm(&owner, &snapshot, matched, &policy, 3, &io) {
                Ok(_) => panic!("new invitation accepted sending options"),
                Err(error) => error,
            };
            assert_eq!(error.code, "REMOTE_INPUT_INVALID");
            let ledger = sink.ledger.lock().unwrap();
            assert_eq!(ledger.confirms, 0);
            assert_eq!(ledger.grants, 0);
            assert!(ledger.records.is_empty());
            assert!(ledger.grant_rows.is_empty());
            drop(ledger);
            assert_eq!(
                backend.write_attempts(),
                before,
                "refusal attempted a provider mutation"
            );
            assert_eq!(
                coordinator.status(3).unwrap().state,
                State::CandidateAwaitingOwner
            );
            coordinator
                .cancel(&owner, &snapshot.context.offer, 3, &io)
                .unwrap();
        }
    }

    // Admit the owner browser, then revoke its designation before the writer
    // acquires the fake transaction. It must recheck, not reuse admission.
    let backend = make();
    let sink = Sink::default();
    let (mut context, candidate, signing) = super::super::tests::fixture();
    context.binding = backend.binding();
    let browser_id = "55555555-5555-4555-8555-555555555555";
    sink.ledger.lock().unwrap().owner_browser = Some((browser_id.into(), context.owner_key, 1));
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let coordinator = Coordinator::new(
        backend.clone(),
        AdmissionRace {
            sink: sink.clone(),
            entered: entered.clone(),
            resume: resume.clone(),
        },
    );
    coordinator.create(&owner, context.clone(), 1, &io).unwrap();
    let delivery = coordinator.deliver(&owner, 2).unwrap();
    backend
        .peer(candidate.principal.clone())
        .publish(&signed_publication(&delivery, &candidate, &signing), &io)
        .unwrap();
    coordinator.observe(2, &io).unwrap();
    let snapshot = coordinator.candidate(2).unwrap().unwrap();
    std::thread::scope(|scope| {
        let job = scope.spawn(|| {
            coordinator
                .confirm(
                    &OwnerActor::owner_browser(browser_id.into(), context.owner_key, 1),
                    &snapshot,
                    true,
                    &PairingPolicy::default(),
                    3,
                    &io,
                )
                .is_err()
        });
        entered.wait();
        sink.ledger.lock().unwrap().owner_browser = None;
        resume.wait();
        assert!(job.join().unwrap());
    });
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);
    assert_eq!(backend.writes(Part::Receipt), 0);
}
#[test]
fn mailbox_policy_and_owner_revocation_fence() {
    let (c, _, _) = super::super::tests::fixture();
    policies(|| Mailbox::new(c.binding.clone()));
}
#[test]
fn append_log_policy_and_owner_revocation_fence() {
    let (mut c, _, _) = super::super::tests::fixture();
    c.binding.backend = "append-log".into();
    policies(|| AppendLog::new(c.binding.clone()));
}
