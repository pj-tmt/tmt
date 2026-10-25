//! Provider transport and writer uncertainty use the same two backend consumers.
use super::*;

#[derive(Default)]
struct Faults {
    lose_offer: bool,
    lose_candidate: bool,
    wrong_receipt_principal: bool,
    bad_receipt_proof: bool,
    wrong_candidate_principal: bool,
    revoke_after_routes: Option<Sink>,
    publishes: BTreeMap<u8, usize>,
    withdrawals: usize,
    fail_withdrawal: bool,
    pause_withdrawal: Option<(Arc<Barrier>, Arc<Barrier>)>,
    owner_deadlines: Vec<Instant>,
}
#[derive(Clone)]
pub(super) struct Controlled<B> {
    inner: B,
    faults: Arc<Mutex<Faults>>,
}
impl<B> Controlled<B> {
    pub(super) fn new(inner: B) -> Self {
        Self {
            inner,
            faults: Default::default(),
        }
    }
    pub(super) fn write_attempts(&self) -> usize {
        let faults = self.faults.lock().unwrap();
        faults.publishes.values().sum::<usize>() + faults.withdrawals
    }
    pub(super) fn fail_next_withdrawal(&self) {
        self.faults.lock().unwrap().fail_withdrawal = true;
    }
    pub(super) fn pause_next_withdrawal(&self, entered: Arc<Barrier>, resume: Arc<Barrier>) {
        self.faults.lock().unwrap().pause_withdrawal = Some((entered, resume));
    }
    pub(super) fn owner_deadlines(&self) -> Vec<Instant> {
        self.faults.lock().unwrap().owner_deadlines.clone()
    }
    pub(super) fn lose_candidate_answer(&self) {
        self.faults.lock().unwrap().lose_candidate = true;
    }
    pub(super) fn refuse_receipt(&self, principal: bool) {
        let mut faults = self.faults.lock().unwrap();
        faults.wrong_receipt_principal = principal;
        faults.bad_receipt_proof = !principal;
    }
    fn publications(&self, part: Part) -> usize {
        self.faults
            .lock()
            .unwrap()
            .publishes
            .get(&(part as u8))
            .copied()
            .unwrap_or(0)
    }
}
impl<B: Backend> Backend for Controlled<B> {
    fn binding(&self) -> BackendBinding {
        self.inner.binding()
    }
    fn publish(
        &self,
        publication: &Publication,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure> {
        // Candidate publication is the peer, not this owner coordinator.
        if publication.exchange.part != Part::Candidate {
            self.faults
                .lock()
                .unwrap()
                .owner_deadlines
                .push(io.deadline);
        }
        *self
            .faults
            .lock()
            .unwrap()
            .publishes
            .entry(publication.exchange.part as u8)
            .or_default() += 1;
        let result = self.inner.publish(publication, io)?;
        let mut faults = self.faults.lock().unwrap();
        if publication.exchange.part == Part::Offer && std::mem::take(&mut faults.lose_offer) {
            return Ok(WriteOutcome::Unconfirmed);
        }
        if publication.exchange.part == Part::Candidate
            && std::mem::take(&mut faults.lose_candidate)
        {
            return Ok(WriteOutcome::Unconfirmed);
        }
        if publication.exchange.part == Part::Routes
            && let Some(sink) = faults.revoke_after_routes.take()
        {
            sink.ledger.lock().unwrap().revoked = true;
        }
        Ok(result)
    }
    fn observe(
        &self,
        exchange: &InvitationExchange,
        io: &IoBudget<'_>,
    ) -> Result<Option<Observation>, BackendFailure> {
        self.faults
            .lock()
            .unwrap()
            .owner_deadlines
            .push(io.deadline);
        let mut row = self.inner.observe(exchange, io)?;
        if exchange.part == Part::Receipt
            && let Some(row) = row.as_mut()
        {
            let faults = self.faults.lock().unwrap();
            if faults.wrong_receipt_principal {
                row.principal = b"not the owner principal".to_vec();
            }
            if faults.bad_receipt_proof
                && let Payload::Receipt { proof, .. } = &mut row.publication.payload
            {
                proof[0] ^= 1;
            }
        }
        if exchange.part == Part::Candidate
            && self.faults.lock().unwrap().wrong_candidate_principal
            && let Some(row) = row.as_mut()
        {
            row.principal = b"not the authenticated candidate".to_vec();
        }
        Ok(row)
    }
    fn withdraw(
        &self,
        context: &Context,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure> {
        self.faults.lock().unwrap().withdrawals += 1;
        let pause = self.faults.lock().unwrap().pause_withdrawal.take();
        if let Some((entered, resume)) = pause {
            entered.wait();
            resume.wait();
        }
        if std::mem::take(&mut self.faults.lock().unwrap().fail_withdrawal) {
            return Err(BackendFailure::Unavailable);
        }
        self.inner.withdraw(context, io)
    }
}
impl<B: ScenarioBackend> ScenarioBackend for Controlled<B> {
    fn peer(&self, principal: Vec<u8>) -> Self {
        Self {
            inner: self.inner.peer(principal),
            faults: self.faults.clone(),
        }
    }
    fn lose_next_receipt(&self) {
        self.inner.lose_next_receipt();
    }
    fn writes(&self, part: Part) -> usize {
        self.publications(part)
    }
}

fn faults<B: ScenarioBackend>(make: impl Fn() -> B) {
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled: &stopped,
    };
    let owner = OwnerActor::local_interactive();

    // An offer that may already exist is observed at its original identity.
    // It is never republished and cannot expose a secret until observed.
    let backend = Controlled::new(make());
    backend.faults.lock().unwrap().lose_offer = true;
    let sink = Sink::default();
    let coordinator = Coordinator::new(backend.clone(), sink.clone());
    let (mut context, _, _) = super::super::tests::fixture();
    context.binding = backend.binding();
    assert_eq!(
        coordinator.create(&owner, context, 1, &io).unwrap().state,
        State::ProviderOutcomeUnconfirmed
    );
    assert!(coordinator.deliver(&owner, 2).is_err());
    assert_eq!(coordinator.observe(2, &io).unwrap().state, State::Open);
    assert_eq!(backend.publications(Part::Offer), 1);
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);
    // Restarting an unconfirmed offer must not reconstitute its code or state.
    drop(coordinator);
    let restarted = Coordinator::new(backend.clone(), sink.clone());
    assert!(restarted.status(3).unwrap().offer.is_none());
    assert!(restarted.deliver(&owner, 3).is_err());
    assert_eq!(backend.publications(Part::Offer), 1);

    // A valid self-asserted principal in the bytes cannot replace the port's
    // authenticated principal. The error is closed and contains no raw label.
    let backend = Controlled::new(make());
    let sink = Sink::default();
    let coordinator = Coordinator::new(backend.clone(), sink.clone());
    let (mut context, candidate, signing) = super::super::tests::fixture();
    context.binding = backend.binding();
    coordinator.create(&owner, context, 1, &io).unwrap();
    let delivery = coordinator.deliver(&owner, 2).unwrap();
    backend
        .peer(candidate.principal.clone())
        .publish(&signed_publication(&delivery, &candidate, &signing), &io)
        .unwrap();
    backend.faults.lock().unwrap().wrong_candidate_principal = true;
    let error = match coordinator.observe(2, &io) {
        Ok(_) => panic!("forged authentication was admitted"),
        Err(error) => error,
    };
    assert_eq!(error.code, "REMOTE_INVITATION_INVALID");
    assert!(!error.message.contains("authenticated candidate"));
    assert!(coordinator.candidate(2).unwrap().is_none());
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);
    backend.faults.lock().unwrap().wrong_candidate_principal = false;
    assert_eq!(
        coordinator.observe(2, &io).unwrap().state,
        State::CandidateAwaitingOwner
    );
    assert_eq!(sink.ledger.lock().unwrap().grants, 0);

    // A writer may fail before commit, or commit and lose its answer. In both
    // cases another confirm is forbidden; only the exact original is read.
    for after_commit in [false, true] {
        let backend = Controlled::new(make());
        let sink = Sink::default();
        let (coordinator, _, snapshot) = pending(backend.clone(), sink.clone(), &io);
        {
            let mut ledger = sink.ledger.lock().unwrap();
            ledger.fail_commit = !after_commit;
            ledger.lose_commit_answer = after_commit;
        }
        assert!(
            coordinator
                .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
                .is_err()
        );
        assert_eq!(
            coordinator.status(3).unwrap().state,
            State::ProviderOutcomeUnconfirmed
        );
        assert!(coordinator.deliver(&owner, 3).is_err());
        assert!(
            coordinator
                .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
                .is_err()
        );
        for now in [4, 5, 6] {
            coordinator.observe(now, &io).unwrap();
        }
        assert_eq!(sink.ledger.lock().unwrap().confirms, 1);
        assert_eq!(
            sink.ledger.lock().unwrap().grants,
            usize::from(after_commit)
        );
        assert_eq!(
            backend.publications(Part::Receipt),
            usize::from(after_commit)
        );
        assert_eq!(
            coordinator.status(6).unwrap().local_confirmation_known,
            after_commit
        );
    }

    // Revocation races a successful route response. The old acknowledgement
    // must not authorize receipt publication or another route write.
    let backend = Controlled::new(make());
    let sink = Sink {
        routes: true,
        ..Sink::default()
    };
    let (coordinator, _, snapshot) = pending(backend.clone(), sink.clone(), &io);
    backend.faults.lock().unwrap().revoke_after_routes = Some(sink.clone());
    assert_eq!(
        coordinator
            .confirm(&owner, &snapshot, true, &PairingPolicy::default(), 3, &io)
            .unwrap()
            .state,
        State::ProviderOutcomeUnconfirmed
    );
    for now in [4, 5, 6] {
        coordinator.observe(now, &io).unwrap();
    }
    assert_eq!(backend.publications(Part::Routes), 1);
    assert_eq!(backend.publications(Part::Receipt), 0);
    assert_eq!(sink.ledger.lock().unwrap().confirms, 1);
}

#[test]
fn mailbox_lost_originals_authentication_and_stale_acknowledgements() {
    let (context, _, _) = super::super::tests::fixture();
    faults(|| Mailbox::new(context.binding.clone()));
}
#[test]
fn append_log_lost_originals_authentication_and_stale_acknowledgements() {
    let (mut context, _, _) = super::super::tests::fixture();
    context.binding.backend = "append-log".into();
    faults(|| AppendLog::new(context.binding.clone()));
}
