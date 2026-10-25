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
    let mut policy = PairingPolicy {
        talk: true,
        hold: true,
        agents: Some(vec![
            "44444444-4444-4444-8444-444444444444".into(),
            "55555555-5555-4555-8555-555555555555".into(),
        ]),
    };
    coordinator
        .confirm(&owner, &snapshot, true, &policy, 3, &io)
        .unwrap();
    let original = sink.original(&snapshot).unwrap().unwrap();
    let receipt: serde_json::Value = serde_json::from_slice(&original.receipt).unwrap();
    assert_eq!(receipt["grant"]["mode"], "hold");
    assert_eq!(receipt["grant"]["agents"], serde_json::json!(policy.agents));
    assert!(
        receipt["grant"]["scopes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("talk"))
    );
    assert_eq!(receipt["grant"]["clientId"], snapshot.candidate.machine);
    assert_eq!(
        receipt["grant"]["publicKey"],
        canonical::base64url(&snapshot.candidate.public_key)
    );
    assert_eq!(receipt["grant"]["machineId"], snapshot.context.owner);
    policy.talk = false;
    policy.hold = false;
    policy.agents = None;
    assert!(
        coordinator
            .confirm(&owner, &snapshot, true, &policy, 4, &io)
            .is_err()
    );
    coordinator.observe(4, &io).unwrap();
    assert!(sink.original(&snapshot).unwrap().as_ref() == Some(&original));
    assert_eq!(sink.ledger.lock().unwrap().grants, 1);

    // Invalid grant options cannot turn a refused comparison into an open offer.
    let backend = make();
    let sink = Sink::default();
    let (coordinator, _, snapshot) = pending(backend.clone(), sink.clone(), &io);
    let invalid_policy = PairingPolicy {
        talk: false,
        hold: true,
        agents: None,
    };
    assert!(
        coordinator
            .confirm(&owner, &snapshot, true, &invalid_policy, 3, &io)
            .is_err()
    );
    assert_eq!(sink.ledger.lock().unwrap().confirms, 0);
    assert_eq!(
        coordinator
            .confirm(&owner, &snapshot, false, &invalid_policy, 3, &io)
            .unwrap()
            .state,
        State::Refused
    );
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);
    assert_eq!(backend.writes(Part::Receipt), 0);

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
