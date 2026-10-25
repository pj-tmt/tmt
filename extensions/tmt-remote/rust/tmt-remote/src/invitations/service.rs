//! The shared memory owner. No terminal, browser, provider or Store transaction
//! runs its own competing ceremony; provider calls happen outside the state lock.
use super::port::*;
use super::{Context, invalid, proof, verify_enrollment};
use crate::{
    canonical, crypto,
    error::RemoteError,
    limits,
    objects::IoBudget,
    pairing::PairingPolicy,
    store::{DEFAULT_SCOPES, Grant},
};
use sha2::{Digest, Sha256};
use std::{
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    CandidateAwaitingOwner,
    Confirmed,
    Refused,
    Cancelled,
    Expired,
    ProviderOutcomeUnconfirmed,
}

/// Observation of this memory offer's physical withdrawal, never authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderCleanup {
    NotRequested,
    Acknowledged,
    Unconfirmed,
}

/// Deliberately no code, principal, provider locator or receipt projection.
pub struct Status {
    pub offer: Option<String>,
    pub expires_at_ms: Option<u64>,
    pub state: State,
    pub local_confirmation_known: bool,
    pub cleanup: ProviderCleanup,
}

pub struct Delivery {
    pub context: Context,
    code: [u8; 16],
}
struct NewCode([u8; 16]);
impl Drop for NewCode {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}
impl Delivery {
    /// The private code/link presenter owns the exported value's lifetime.
    pub fn with_code<T>(&self, use_code: impl FnOnce(&[u8; 16]) -> T) -> T {
        use_code(&self.code)
    }
}
impl Drop for Delivery {
    fn drop(&mut self) {
        self.code.fill(0);
    }
}

enum Phase {
    Open,
    Candidate(super::VerifiedCandidate),
    CommitUnknown(Snapshot),
    Confirmed(Committed),
    End(State),
}
struct Offer {
    context: Context,
    deadline: Instant,
    code: [u8; 16],
    has_code: bool,
    announcement: Option<Publication>,
    announcement_attempted: bool,
    announced: bool,
    routes_done: bool,
    receipt_done: bool,
    cleanup: ProviderCleanup,
    phase: Phase,
}
impl Offer {
    fn clear_code(&mut self) {
        self.code.fill(0);
        self.has_code = false;
    }
    fn expire(&mut self, now_ms: u64) {
        if (now_ms >= self.context.expires_at_ms || Instant::now() >= self.deadline)
            && !matches!(
                self.phase,
                Phase::Confirmed(_) | Phase::CommitUnknown(_) | Phase::End(_)
            )
        {
            self.clear_code();
            self.phase = Phase::End(State::Expired);
            self.cleanup = ProviderCleanup::Unconfirmed;
        }
    }
    fn status(&self) -> Status {
        let known = matches!(self.phase, Phase::Confirmed(_));
        let state = match self.phase {
            Phase::Open if !self.announced => State::ProviderOutcomeUnconfirmed,
            Phase::Open => State::Open,
            Phase::Candidate(_) => State::CandidateAwaitingOwner,
            Phase::CommitUnknown(_) => State::ProviderOutcomeUnconfirmed,
            Phase::Confirmed(_) if !self.routes_done || !self.receipt_done => {
                State::ProviderOutcomeUnconfirmed
            }
            Phase::Confirmed(_) => State::Confirmed,
            Phase::End(state) => state,
        };
        Status {
            offer: Some(self.context.offer.clone()),
            expires_at_ms: Some(self.context.expires_at_ms),
            state,
            local_confirmation_known: known,
            cleanup: self.cleanup,
        }
    }
}
impl Drop for Offer {
    fn drop(&mut self) {
        self.clear_code();
    }
}

pub struct Coordinator<B, S> {
    backend: B,
    sink: S,
    offer: Mutex<Option<Offer>>,
}

impl<B: Backend, S: ConfirmationSink> Coordinator<B, S> {
    pub fn new(backend: B, sink: S) -> Self {
        Self {
            backend,
            sink,
            offer: Mutex::new(None),
        }
    }
    fn lock(&self) -> Result<MutexGuard<'_, Option<Offer>>, RemoteError> {
        self.offer.lock().map_err(|_| unavailable())
    }

    #[cfg(test)]
    pub(super) fn retains_secret(&self) -> Result<bool, RemoteError> {
        Ok(self
            .lock()?
            .as_ref()
            .is_some_and(|offer| offer.has_code || offer.code.iter().any(|byte| *byte != 0)))
    }

    #[cfg(test)]
    pub(super) fn original_deadline(&self) -> Result<Option<Instant>, RemoteError> {
        Ok(self.lock()?.as_ref().map(|offer| offer.deadline))
    }

    /// Context is captured by the admitted owner composition. New code entropy
    /// is generated here and is never given to the provider port.
    pub fn create(
        &self,
        actor: &OwnerActor,
        context: Context,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        let started = Instant::now();
        check(io)?;
        context.validate()?;
        if !actor.interactive()
            || self.backend.binding() != context.binding
            || now_ms >= context.expires_at_ms
            || context.issued_at_ms > now_ms
        {
            return Err(invalid());
        }
        let deadline = started + Duration::from_millis(context.expires_at_ms - now_ms);
        let creation_io = offer_io(io, deadline);
        self.sink.authorize(&context, actor)?;
        check(&creation_io)?;
        let mut state = self.lock()?;
        if let Some(previous) = state.as_mut() {
            previous.expire(now_ms);
        }
        if state
            .as_ref()
            .is_some_and(|offer| !matches!(offer.phase, Phase::End(_) | Phase::Confirmed(_)))
        {
            return Err(RemoteError::new(
                "REMOTE_INVITATION_BUSY",
                "An invitation is already open.",
            ));
        }
        let mut code = NewCode([0; 16]);
        getrandom::fill(&mut code.0)
            .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not create an invitation."))?;
        let gate = gate(&context, &code.0)?;
        let announcement = Publication {
            exchange: InvitationExchange {
                context: context.clone(),
                part: Part::Offer,
            },
            payload: Payload::Offer {
                gate_hash: Sha256::digest(gate).into(),
            },
        };
        check(&creation_io)?;
        let offer = Offer {
            deadline,
            context,
            code: code.0,
            has_code: true,
            announcement: Some(announcement),
            announcement_attempted: false,
            announced: false,
            routes_done: false,
            receipt_done: false,
            cleanup: ProviderCleanup::NotRequested,
            phase: Phase::Open,
        };
        drop(code);
        *state = Some(offer);
        drop(state);
        self.observe(now_ms, io)
    }

    pub fn status(&self, now_ms: u64) -> Result<Status, RemoteError> {
        let mut state = self.lock()?;
        Ok(match state.as_mut() {
            Some(offer) => {
                offer.expire(now_ms);
                offer.status()
            }
            None => Status {
                offer: None,
                expires_at_ms: None,
                state: State::Cancelled,
                local_confirmation_known: false,
                cleanup: ProviderCleanup::NotRequested,
            },
        })
    }

    pub fn deliver(&self, actor: &OwnerActor, now_ms: u64) -> Result<Delivery, RemoteError> {
        let mut state = self.lock()?;
        let offer = state.as_mut().ok_or_else(invalid)?;
        offer.expire(now_ms);
        if !actor.interactive()
            || !offer.announced
            || !offer.has_code
            || !matches!(offer.phase, Phase::Open | Phase::Candidate(_))
        {
            return Err(invalid());
        }
        self.sink.authorize(&offer.context, actor)?;
        Ok(Delivery {
            context: offer.context.clone(),
            code: offer.code,
        })
    }

    pub fn candidate(&self, now_ms: u64) -> Result<Option<Snapshot>, RemoteError> {
        let mut state = self.lock()?;
        let Some(offer) = state.as_mut() else {
            return Ok(None);
        };
        offer.expire(now_ms);
        Ok(match &offer.phase {
            Phase::Candidate(verified) => Some(snapshot(verified)),
            _ => None,
        })
    }

    pub fn observe(&self, now_ms: u64, io: &IoBudget<'_>) -> Result<Status, RemoteError> {
        check(io)?;
        let next = {
            let mut state = self.lock()?;
            let Some(offer) = state.as_mut() else {
                return Ok(empty_status());
            };
            offer.expire(now_ms);
            if !offer.announced && matches!(offer.phase, Phase::Open) {
                let first = !offer.announcement_attempted;
                offer.announcement_attempted = true;
                Some((
                    offer.announcement.clone().ok_or_else(invalid)?,
                    first,
                    offer.deadline,
                ))
            } else {
                None
            }
        };
        if let Some((publication, first, deadline)) = next {
            let exchange_io = offer_io(io, deadline);
            let ack = self.publish_or_observe(&publication, first, &exchange_io)?;
            let mut state = self.lock()?;
            if let Some(offer) = state.as_mut().filter(|offer| {
                offer.context == publication.exchange.context && matches!(offer.phase, Phase::Open)
            }) {
                offer.expire(now_ms);
                if matches!(offer.phase, Phase::Open) {
                    offer.announced = ack;
                }
            }
            return Ok(state.as_ref().map_or_else(empty_status, Offer::status));
        }
        let context = {
            let state = self.lock()?;
            state
                .as_ref()
                .filter(|offer| offer.announced && matches!(offer.phase, Phase::Open))
                .map(|offer| (offer.context.clone(), offer.deadline))
        };
        if let Some((context, deadline)) = context {
            let exchange_io = offer_io(io, deadline);
            let exchange = InvitationExchange {
                context: context.clone(),
                part: Part::Candidate,
            };
            let observed = self
                .backend
                .observe(&exchange, &exchange_io)
                .map_err(provider)?;
            check(&exchange_io)?;
            if let Some(observed) = observed {
                if observed.publication.exchange != exchange {
                    return Err(invalid());
                }
                let Payload::Candidate(submitted) = observed.publication.payload else {
                    return Err(invalid());
                };
                let mut state = self.lock()?;
                let offer = state
                    .as_mut()
                    .filter(|offer| offer.context == context)
                    .ok_or_else(invalid)?;
                offer.expire(now_ms);
                if matches!(offer.phase, Phase::Open) {
                    if !offer.has_code || gate(&context, &offer.code)? != submitted.gate {
                        return Err(invalid());
                    }
                    let verified = verify_enrollment(
                        &context,
                        &submitted.candidate,
                        &observed.principal,
                        &offer.code,
                        &submitted.mac,
                        &submitted.signature,
                        now_ms,
                    )?;
                    offer.phase = Phase::Candidate(verified);
                }
            }
        }
        let unknown = {
            let state = self.lock()?;
            state.as_ref().and_then(|offer| {
                if let Phase::CommitUnknown(snapshot) = &offer.phase {
                    Some(snapshot.clone())
                } else {
                    None
                }
            })
        };
        if let Some(snapshot) = unknown
            && let Some(committed) = self.sink.original(&snapshot)?
        {
            if !valid_commit(&committed, &snapshot) {
                return Err(invalid());
            }
            let mut state = self.lock()?;
            if let Some(offer) = state.as_mut().filter(
                |offer| matches!(&offer.phase, Phase::CommitUnknown(pinned) if *pinned == snapshot),
            ) {
                offer.phase = Phase::Confirmed(committed);
            }
        }
        self.flush(now_ms, io)?;
        self.status(now_ms)
    }

    pub fn confirm(
        &self,
        actor: &OwnerActor,
        expected: &Snapshot,
        matched: bool,
        policy: &PairingPolicy,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        self.confirm_owner_decision(
            actor,
            expected,
            OwnerDecision {
                matched,
                enable_sending: false,
            },
            policy,
            now_ms,
            io,
        )
    }

    pub fn confirm_owner_decision(
        &self,
        actor: &OwnerActor,
        expected: &Snapshot,
        decision: OwnerDecision,
        policy: &PairingPolicy,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        check(io)?;
        if !actor.interactive() {
            return Err(invalid());
        }
        if policy.talk || policy.hold || policy.agents.is_some() {
            return Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "New invitations start with sending off and direct delivery. Change member sending after confirmation.",
            ));
        }
        let mut state = self.lock()?;
        let offer = state.as_mut().ok_or_else(invalid)?;
        offer.expire(now_ms);
        let Phase::Candidate(verified) = &offer.phase else {
            return Err(invalid());
        };
        if snapshot(verified) != *expected {
            return Err(invalid());
        }
        self.sink.authorize(&offer.context, actor)?;
        if !decision.matched {
            let context = offer.context.clone();
            offer.clear_code();
            offer.phase = Phase::End(State::Refused);
            offer.cleanup = ProviderCleanup::Unconfirmed;
            let result = offer.status();
            drop(state);
            return self.finish_withdrawal(&context, result, io);
        }
        let grant = grant(expected, now_ms);
        let transcript = proof::enrollment(&expected.context, &expected.candidate)?;
        let receipt_key =
            ReceiptKey(crypto::response_key(&offer.code, &transcript).map_err(|_| invalid())?);
        let confirmation_io = offer_io(io, offer.deadline);
        check(&confirmation_io)?;
        let committed = self.sink.confirm(Confirmation {
            verified,
            snapshot: expected,
            actor,
            grant: &grant,
            receipt_key: &receipt_key,
            sending: decision.enable_sending,
            io: &confirmation_io,
        });
        offer.clear_code();
        offer.phase = Phase::CommitUnknown(expected.clone());
        let committed = match committed {
            Ok(committed) => committed,
            Err(ConfirmationFailure::CommitUnknown(error)) => return Err(error),
            Err(ConfirmationFailure::Refused(error)) => {
                let context = offer.context.clone();
                offer.phase = Phase::End(State::Refused);
                offer.cleanup = ProviderCleanup::Unconfirmed;
                let status = offer.status();
                drop(state);
                // Physical cleanup must not replace the original refusal.
                let _ = self.finish_withdrawal(&context, status, io);
                return Err(error);
            }
        };
        if !valid_commit(&committed, expected)
            || crypto::verify_server_proof(&receipt_key.0, &committed.receipt, &committed.proof)
                .is_err()
        {
            return Err(invalid());
        }
        offer.phase = Phase::Confirmed(committed);
        drop(receipt_key);
        drop(state);
        self.flush(now_ms, io)?;
        self.status(now_ms)
    }

    /// Resume only an exact original held by the private confirmation journal.
    /// This cannot restore an unconfirmed code or perform another confirmation.
    pub fn recover(
        &self,
        actor: &OwnerActor,
        expected: &Snapshot,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        let started = Instant::now();
        check(io)?;
        expected.context.validate()?;
        proof::enrollment(&expected.context, &expected.candidate)?;
        if !actor.interactive()
            || self.backend.binding() != expected.context.binding
            || now_ms < expected.context.issued_at_ms
            || now_ms >= expected.context.expires_at_ms
        {
            return Err(invalid());
        }
        let deadline = started + Duration::from_millis(expected.context.expires_at_ms - now_ms);
        let recovery_io = offer_io(io, deadline);
        let mut state = self.lock()?;
        if let Some(offer) = state.as_ref()
            && !matches!(offer.phase, Phase::End(_))
        {
            return Err(invalid());
        }
        self.sink.authorize(&expected.context, actor)?;
        check(&recovery_io)?;
        let committed = self.sink.original(expected)?.ok_or_else(invalid)?;
        if !valid_commit(&committed, expected) {
            return Err(invalid());
        }
        check(&recovery_io)?;
        *state = Some(Offer {
            context: expected.context.clone(),
            deadline,
            code: [0; 16],
            has_code: false,
            announcement: None,
            announcement_attempted: true,
            announced: true,
            routes_done: false,
            receipt_done: false,
            cleanup: ProviderCleanup::NotRequested,
            phase: Phase::Confirmed(committed),
        });
        drop(state);
        self.flush(now_ms, io)?;
        self.status(now_ms)
    }

    /// Cancelling an already committed confirmation never rolls back pairing.
    pub fn cancel(
        &self,
        actor: &OwnerActor,
        expected_offer: &str,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        self.end(Some(actor), expected_offer, State::Cancelled, now_ms, io)
    }

    /// Withdraw an unconfirmed invitation, not the durable member it may have
    /// created. Member revocation and its route cleanup retain their own owner.
    pub fn revoke(
        &self,
        actor: &OwnerActor,
        expected_offer: &str,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        self.end(Some(actor), expected_offer, State::Refused, now_ms, io)
    }

    /// Trusted owner-lifetime callback, never a browser/JSON consent action.
    /// Stale disconnects cannot terminate a replacement offer.
    pub fn disconnect(
        &self,
        expected_offer: &str,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        self.end(None, expected_offer, State::Cancelled, now_ms, io)
    }

    pub fn expire(
        &self,
        expected_offer: &str,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        let mut state = self.lock()?;
        let offer = state.as_mut().ok_or_else(invalid)?;
        if offer.context.offer != expected_offer {
            return Err(invalid());
        }
        offer.expire(now_ms);
        if !matches!(offer.phase, Phase::End(State::Expired)) {
            return Err(invalid());
        }
        let context = offer.context.clone();
        let result = offer.status();
        drop(state);
        self.finish_withdrawal(&context, result, io)
    }

    fn end(
        &self,
        actor: Option<&OwnerActor>,
        expected_offer: &str,
        terminal: State,
        now_ms: u64,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        let mut state = self.lock()?;
        let offer = state.as_mut().ok_or_else(invalid)?;
        if offer.context.offer != expected_offer {
            return Err(invalid());
        }
        if let Some(actor) = actor {
            if !actor.interactive() {
                return Err(invalid());
            }
            self.sink.authorize(&offer.context, actor)?;
        }
        offer.expire(now_ms);
        if matches!(offer.phase, Phase::Confirmed(_) | Phase::CommitUnknown(_)) {
            return Err(invalid());
        }
        if matches!(offer.phase, Phase::End(_)) {
            return Ok(offer.status());
        }
        let context = offer.context.clone();
        offer.clear_code();
        offer.phase = Phase::End(terminal);
        offer.cleanup = ProviderCleanup::Unconfirmed;
        let result = offer.status();
        drop(state);
        self.finish_withdrawal(&context, result, io)
    }

    /// Physical gate cleanup after a locally committed or uncertain outcome.
    /// This never changes that authority outcome into cancellation or rollback.
    pub fn withdraw(
        &self,
        actor: &OwnerActor,
        expected_offer: &str,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        let state = self.lock()?;
        let offer = state.as_ref().ok_or_else(invalid)?;
        if !actor.interactive()
            || offer.context.offer != expected_offer
            || !matches!(
                offer.phase,
                Phase::Confirmed(_) | Phase::CommitUnknown(_) | Phase::End(_)
            )
        {
            return Err(invalid());
        }
        self.sink.authorize(&offer.context, actor)?;
        let context = offer.context.clone();
        let status = offer.status();
        drop(state);
        self.finish_withdrawal(&context, status, io)
    }

    fn finish_withdrawal(
        &self,
        context: &Context,
        mut result: Status,
        io: &IoBudget<'_>,
    ) -> Result<Status, RemoteError> {
        result.cleanup = match self.backend.withdraw(context, io) {
            Ok(WriteOutcome::Acknowledged) => ProviderCleanup::Acknowledged,
            _ => ProviderCleanup::Unconfirmed,
        };
        let mut state = self.lock()?;
        if let Some(offer) = state.as_mut().filter(|offer| offer.context == *context) {
            offer.cleanup = result.cleanup;
        }
        Ok(result)
    }

    fn flush(&self, now_ms: u64, io: &IoBudget<'_>) -> Result<(), RemoteError> {
        for part in [Part::Routes, Part::Receipt] {
            check(io)?;
            let record = {
                let state = self.lock()?;
                state.as_ref().and_then(|offer| match &offer.phase {
                    Phase::Confirmed(record)
                        if now_ms < offer.context.expires_at_ms
                            && Instant::now() < offer.deadline
                            && !(if part == Part::Routes {
                                offer.routes_done
                            } else {
                                offer.receipt_done
                            }) =>
                    {
                        Some((record.clone(), offer.deadline))
                    }
                    _ => None,
                })
            };
            let Some((record, deadline)) = record else {
                continue;
            };
            if part == Part::Routes && record.routes.is_none() {
                let mut state = self.lock()?;
                if let Some(offer) = state.as_mut().filter(
                    |offer| matches!(&offer.phase, Phase::Confirmed(current) if *current == record),
                ) {
                    offer.routes_done = true;
                }
                continue;
            }
            let claim = self.sink.claim(&record, part)?;
            if claim == Claim::Stale {
                return Ok(());
            }
            let publication = publication(&record, part);
            let publication_io = offer_io(io, deadline);
            if !self.publish_or_observe(
                &publication,
                claim == Claim::FirstAttempt,
                &publication_io,
            )? {
                return Ok(());
            }
            if !self.sink.acknowledge(&record, part)? {
                return Ok(());
            }
            let mut state = self.lock()?;
            if let Some(offer) = state.as_mut().filter(
                |offer| matches!(&offer.phase, Phase::Confirmed(current) if *current == record),
            ) {
                if part == Part::Routes {
                    offer.routes_done = true;
                } else {
                    offer.receipt_done = true;
                }
            }
        }
        Ok(())
    }

    fn publish_or_observe(
        &self,
        publication: &Publication,
        first: bool,
        io: &IoBudget<'_>,
    ) -> Result<bool, RemoteError> {
        check(io)?;
        if first {
            let result = self.backend.publish(publication, io).map_err(provider)?;
            check(io)?;
            Ok(result == WriteOutcome::Acknowledged)
        } else {
            let observed = self
                .backend
                .observe(&publication.exchange, io)
                .map_err(provider)?;
            check(io)?;
            match observed {
                None => Ok(false),
                Some(observed)
                    if observed.principal
                        == publication.exchange.context.binding.owner_principal
                        && observed.publication == *publication =>
                {
                    Ok(true)
                }
                Some(_) => Err(invalid()),
            }
        }
    }
}

pub fn gate(context: &Context, code: &[u8; 16]) -> Result<[u8; 32], RemoteError> {
    // Purpose is separate from enrollment and the response proof, even though
    // every context field is also bound by the eventual candidate transcript.
    context.validate()?;
    let bytes = canonical::framed(&[
        b"tmt-invitation-gate-v2",
        context.binding.backend.as_bytes(),
        &context.binding.reference,
        &context.binding.owner_principal,
        context.owner.as_bytes(),
        &context.owner_key,
        context.offer.as_bytes(),
        &context.nonce,
        context.issued_at_ms.to_string().as_bytes(),
        context.expires_at_ms.to_string().as_bytes(),
    ])
    .map_err(|_| invalid())?;
    if bytes.len() > limits::CLOUD_ENROLLMENT_BYTES {
        return Err(invalid());
    }
    Ok(crypto::mac(code, &bytes))
}
fn snapshot(verified: &super::VerifiedCandidate) -> Snapshot {
    Snapshot {
        context: verified.context().clone(),
        candidate: verified.candidate().clone(),
        digest: *verified.digest(),
    }
}
fn grant(snapshot: &Snapshot, now: u64) -> Grant {
    Grant {
        client_id: snapshot.candidate.machine.clone(),
        public_key: snapshot.candidate.public_key,
        kind: "cli".into(),
        origin: "cli".into(),
        name: snapshot.candidate.name.clone(),
        agents: "all".into(),
        scopes: DEFAULT_SCOPES
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect(),
        mode: "direct".into(),
        issued_at_ms: now,
        expires_at_ms: None,
        revision: 1,
        disabled: false,
    }
}

fn publication(record: &Committed, part: Part) -> Publication {
    Publication {
        exchange: InvitationExchange {
            context: record.snapshot.context.clone(),
            part,
        },
        payload: if part == Part::Routes {
            Payload::Routes {
                snapshot: record.snapshot.digest,
                bytes: record.routes.clone().unwrap_or_default(),
            }
        } else {
            Payload::Receipt {
                snapshot: record.snapshot.digest,
                bytes: record.receipt.clone(),
                proof: record.proof,
            }
        },
    }
}
fn empty_status() -> Status {
    Status {
        offer: None,
        expires_at_ms: None,
        state: State::Cancelled,
        local_confirmation_known: false,
        cleanup: ProviderCleanup::NotRequested,
    }
}
fn valid_commit(record: &Committed, expected: &Snapshot) -> bool {
    record.snapshot == *expected
        && record.grant_revision > 0
        && canonical::uuid(&record.member).is_ok()
        && record.receipt.len() <= limits::CLOUD_ENROLLMENT_BYTES
        && record
            .routes
            .as_ref()
            .is_none_or(|bytes| bytes.len() <= limits::CLOUD_ENROLLMENT_BYTES)
}
fn unavailable() -> RemoteError {
    RemoteError::new(
        "REMOTE_INVITATION_UNAVAILABLE",
        "Invitation state is unavailable.",
    )
}
pub(super) fn offer_io<'a>(io: &IoBudget<'a>, deadline: Instant) -> IoBudget<'a> {
    IoBudget {
        deadline: io.deadline.min(deadline),
        cancelled: io.cancelled,
    }
}
pub(crate) fn check(io: &IoBudget<'_>) -> Result<(), RemoteError> {
    io.check().map_err(|_| {
        RemoteError::new(
            "REMOTE_INVITATION_STOPPED",
            "The invitation operation stopped.",
        )
    })
}
pub(super) fn provider(failure: BackendFailure) -> RemoteError {
    let code = match failure {
        BackendFailure::Provider(error) => return error,
        BackendFailure::Refused => "REMOTE_INVITATION_REFUSED",
        BackendFailure::InvalidEvidence => "REMOTE_INVITATION_INVALID",
        BackendFailure::Cancelled | BackendFailure::Deadline => "REMOTE_INVITATION_STOPPED",
        BackendFailure::Unavailable => "REMOTE_INVITATION_PROVIDER_UNAVAILABLE",
    };
    RemoteError::new(
        code,
        "The invitation provider operation could not be completed.",
    )
}
