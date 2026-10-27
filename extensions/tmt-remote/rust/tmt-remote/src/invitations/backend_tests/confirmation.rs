//! Fake atomic confirmation journal, separate from scenario assertions.
use super::*;

#[derive(Default)]
pub(super) struct Ledger {
    pub(super) records: BTreeMap<String, Committed>,
    pub(super) grant_rows: BTreeMap<String, crate::store::Grant>,
    pub(super) claims: BTreeSet<Key>,
    pub(super) grants: usize,
    pub(super) confirms: usize,
    pub(super) confirmation_deadline: Option<Instant>,
    pub(super) last_authorized: Option<Instant>,
    pub(super) last_original_read: Option<Instant>,
    pub(super) revoked: bool,
    pub(super) owner_browser: Option<(String, [u8; 32], u64)>,
    pub(super) fail_commit: bool,
    pub(super) lose_commit_answer: bool,
}
#[derive(Clone, Default)]
pub(super) struct Sink {
    pub(super) ledger: Arc<Mutex<Ledger>>,
    pub(super) routes: bool,
}
fn actor_allowed(context: &Context, actor: &OwnerActor, ledger: &Ledger) -> bool {
    context.owner == "11111111-1111-4111-8111-111111111111"
        && !ledger.revoked
        && match &actor.authority {
            Actor::LocalInteractive => true,
            Actor::OwnerBrowser {
                client,
                key,
                revision,
            } => ledger.owner_browser.as_ref() == Some(&(client.clone(), *key, *revision)),
            _ => false,
        }
}
impl ConfirmationSink for Sink {
    fn authorize(
        &self,
        context: &Context,
        actor: &OwnerActor,
    ) -> Result<(), crate::error::RemoteError> {
        let mut ledger = self.ledger.lock().unwrap();
        if actor_allowed(context, actor, &ledger) {
            ledger.last_authorized = Some(Instant::now());
            Ok(())
        } else {
            Err(invalid())
        }
    }
    fn confirm(&self, input: Confirmation<'_>) -> Result<Committed, ConfirmationFailure> {
        let mut ledger = self.ledger.lock().unwrap();
        let record = (|| -> Result<Committed, crate::error::RemoteError> {
            if !actor_allowed(&input.snapshot.context, input.actor, &ledger) {
                return Err(invalid());
            }
            input.io.check().map_err(|_| invalid())?;
            ledger.confirmation_deadline = Some(input.io.deadline);
            ledger.confirms += 1;
            if ledger.fail_commit {
                return Err(invalid());
            }
            if let Some(original) = ledger.records.get(&input.snapshot.context.offer) {
                if original.snapshot != *input.snapshot {
                    return Err(invalid());
                }
                return Ok(original.clone());
            }
            // Model M1's enforced initial grant, then serialize exactly the values
            // committed to the fake row rather than the incoming request options.
            let mut persisted = input.grant.clone();
            persisted.scopes.retain(|scope| scope != "talk");
            persisted.mode = "direct".into();
            if input.sending {
                persisted.scopes.push("talk".into());
                persisted.revision += 1;
            }
            let receipt = crate::pairing::receipt(
                &persisted,
                &input.snapshot.context.owner,
                &input.snapshot.context.owner_key,
            )?;
            let record = Committed {
                snapshot: input.snapshot.clone(),
                member: crate::store::uuid_v4()?,
                grant_revision: persisted.revision,
                proof: input.receipt_key.prove(&receipt)?,
                receipt,
                routes: self.routes.then(|| b"opaque route capability".to_vec()),
            };
            ledger.grants += 1;
            ledger
                .grant_rows
                .insert(persisted.client_id.clone(), persisted);
            ledger
                .records
                .insert(input.snapshot.context.offer.clone(), record.clone());
            Ok(record)
        })()
        .map_err(ConfirmationFailure::Refused)?;
        if ledger.lose_commit_answer {
            return Err(ConfirmationFailure::CommitUnknown(invalid()));
        }
        Ok(record)
    }
    fn original(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Option<Committed>, crate::error::RemoteError> {
        let mut ledger = self.ledger.lock().unwrap();
        ledger.last_original_read = Some(Instant::now());
        Ok(ledger
            .records
            .get(&snapshot.context.offer)
            .filter(|record| record.snapshot == *snapshot)
            .cloned())
    }
    fn claim(&self, record: &Committed, part: Part) -> Result<Claim, crate::error::RemoteError> {
        let mut ledger = self.ledger.lock().unwrap();
        if ledger.revoked || ledger.records.get(&record.snapshot.context.offer) != Some(record) {
            return Ok(Claim::Stale);
        }
        Ok(
            if ledger
                .claims
                .insert((record.snapshot.context.offer.clone(), part as u8))
            {
                Claim::FirstAttempt
            } else {
                Claim::ObserveOnly
            },
        )
    }
    fn acknowledge(
        &self,
        record: &Committed,
        _part: Part,
    ) -> Result<bool, crate::error::RemoteError> {
        let ledger = self.ledger.lock().unwrap();
        Ok(!ledger.revoked && ledger.records.get(&record.snapshot.context.offer) == Some(record))
    }
}

#[derive(Clone)]
pub(super) struct AdmissionRace<S> {
    pub(super) sink: S,
    pub(super) entered: Arc<Barrier>,
    pub(super) resume: Arc<Barrier>,
}
impl<S: ConfirmationSink> ConfirmationSink for AdmissionRace<S> {
    fn authorize(
        &self,
        context: &Context,
        actor: &OwnerActor,
    ) -> Result<(), crate::error::RemoteError> {
        self.sink.authorize(context, actor)
    }
    fn confirm(&self, input: Confirmation<'_>) -> Result<Committed, ConfirmationFailure> {
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

/// A lost writer answer, on either side of the real Store commit boundary.
#[derive(Clone)]
pub(super) struct UncertainConfirmation {
    pub(super) sink: crate::invitations::store::StoreConfirmation,
    pub(super) committed: bool,
}
impl ConfirmationSink for UncertainConfirmation {
    fn authorize(
        &self,
        context: &Context,
        actor: &OwnerActor,
    ) -> Result<(), crate::error::RemoteError> {
        self.sink.authorize(context, actor)
    }
    fn confirm(&self, input: Confirmation<'_>) -> Result<Committed, ConfirmationFailure> {
        if self.committed {
            self.sink.confirm(input)?;
        }
        Err(ConfirmationFailure::CommitUnknown(invalid()))
    }
    fn decline(
        &self,
        actor: &OwnerActor,
        snapshot: &Snapshot,
        io: &IoBudget<'_>,
    ) -> Result<(), ConfirmationFailure> {
        self.sink.decline(actor, snapshot, io)
    }
    fn original(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Option<Committed>, crate::error::RemoteError> {
        self.sink.original(snapshot)
    }
    fn claim(&self, committed: &Committed, part: Part) -> Result<Claim, crate::error::RemoteError> {
        self.sink.claim(committed, part)
    }
    fn acknowledge(
        &self,
        committed: &Committed,
        part: Part,
    ) -> Result<bool, crate::error::RemoteError> {
        self.sink.acknowledge(committed, part)
    }
}
