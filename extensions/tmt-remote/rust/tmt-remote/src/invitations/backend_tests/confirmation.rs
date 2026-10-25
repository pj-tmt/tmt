//! Fake atomic confirmation journal, separate from scenario assertions.
use super::*;

#[derive(Default)]
pub(super) struct Ledger {
    pub(super) records: BTreeMap<String, Committed>,
    pub(super) grant_rows: BTreeMap<String, crate::store::Grant>,
    pub(super) claims: BTreeSet<Key>,
    pub(super) grants: usize,
    pub(super) confirms: usize,
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
        && match &actor.0 {
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
        if actor_allowed(context, actor, &self.ledger.lock().unwrap()) {
            Ok(())
        } else {
            Err(invalid())
        }
    }
    fn confirm(&self, input: Confirmation<'_>) -> Result<Committed, crate::error::RemoteError> {
        let mut ledger = self.ledger.lock().unwrap();
        if !actor_allowed(&input.snapshot.context, input.actor, &ledger) {
            return Err(invalid());
        }
        input.io.check().map_err(|_| invalid())?;
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
        if ledger.lose_commit_answer {
            return Err(invalid());
        }
        Ok(record)
    }
    fn original(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Option<Committed>, crate::error::RemoteError> {
        Ok(self
            .ledger
            .lock()
            .unwrap()
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
