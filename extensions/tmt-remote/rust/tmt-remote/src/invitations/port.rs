//! Transport capabilities, not authority. An adapter authenticates its provider
//! evidence; only the separate confirmation sink can create durable authority.
use super::{BackendBinding, Candidate, Context};
use crate::{error::RemoteError, objects::IoBudget, pairing::PairingPolicy, store::Grant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Offer,
    Candidate,
    Routes,
    Receipt,
}

/// One offer/part identity, including after a lost answer. Candidate publications
/// additionally bind their native machine identity. An adapter may expose a
/// bounded candidate collection, but never substitutes another backend reference.
#[derive(Clone, PartialEq, Eq)]
pub struct InvitationExchange {
    pub context: Context,
    pub part: Part,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Submission {
    pub candidate: Candidate,
    pub gate: [u8; 32],
    pub mac: [u8; 32],
    pub signature: [u8; 64],
}

/// No variant contains the invitation secret or the derived response key.
#[derive(Clone, PartialEq, Eq)]
pub enum Payload {
    Offer {
        gate_hash: [u8; 32],
    },
    Candidate(Submission),
    Routes {
        snapshot: [u8; 32],
        bytes: Vec<u8>,
    },
    Receipt {
        snapshot: [u8; 32],
        bytes: Vec<u8>,
        proof: [u8; 32],
    },
}

#[derive(Clone, PartialEq, Eq)]
pub struct Publication {
    pub exchange: InvitationExchange,
    pub payload: Payload,
}

/// Authentication comes from the provider response, never from payload fields.
#[derive(Clone, PartialEq, Eq)]
pub struct Observation {
    pub principal: Vec<u8>,
    pub publication: Publication,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Acknowledged,
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendFailure {
    Refused,
    Unavailable,
    InvalidEvidence,
    Cancelled,
    Deadline,
}

/// Each operation obeys the supplied absolute budget. Providers store immutable
/// originals and expose observations; none of these methods can supply consent.
pub trait Backend: Send + Sync {
    fn binding(&self) -> BackendBinding;
    fn publish(
        &self,
        publication: &Publication,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure>;
    fn observe(
        &self,
        exchange: &InvitationExchange,
        io: &IoBudget<'_>,
    ) -> Result<Option<Observation>, BackendFailure>;
    fn withdraw(
        &self,
        context: &Context,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure>;
}

/// This value is made by the admitted control composition, not deserialized from
/// a provider row or a JSON consent flag. The Store rechecks browser designation.
pub struct OwnerActor(pub(super) Actor);

pub(super) enum Actor {
    LocalInteractive,
    OwnerBrowser {
        client: String,
        key: [u8; 32],
        revision: u64,
    },
    Automated,
}

impl OwnerActor {
    /// Trusted local control only; the sink still owns admission and rechecks it.
    pub fn local_interactive() -> Self {
        Self(Actor::LocalInteractive)
    }
    pub fn owner_browser(client: String, key: [u8; 32], revision: u64) -> Self {
        Self(Actor::OwnerBrowser {
            client,
            key,
            revision,
        })
    }
    pub fn automated() -> Self {
        Self(Actor::Automated)
    }
    pub(super) fn interactive(&self) -> bool {
        match &self.0 {
            Actor::LocalInteractive => true,
            Actor::OwnerBrowser {
                client,
                key,
                revision,
            } => {
                crate::canonical::uuid(client).is_ok()
                    && crate::crypto::public_key(key).is_ok()
                    && *revision > 0
            }
            Actor::Automated => false,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub context: Context,
    pub candidate: Candidate,
    pub digest: [u8; 32],
}

pub struct Confirmation<'a> {
    pub snapshot: &'a Snapshot,
    pub actor: &'a OwnerActor,
    pub policy: &'a PairingPolicy,
    pub grant: &'a Grant,
    pub receipt_key: &'a ReceiptKey,
    pub io: &'a IoBudget<'a>,
}

/// Scoped MAC capability. It is never persisted, formatted or sent to a backend.
pub struct ReceiptKey(pub(super) [u8; 32]);
impl ReceiptKey {
    pub fn prove(&self, receipt: &[u8]) -> Result<[u8; 32], RemoteError> {
        crate::crypto::server_proof(&self.0, receipt).map_err(|_| super::invalid())
    }
}
impl Drop for ReceiptKey {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// Exact original receipt and publication bytes returned by the writer. Routes
/// are an opaque publication capability, not a backend-created member/grant.
#[derive(Clone, PartialEq, Eq)]
pub struct Committed {
    pub snapshot: Snapshot,
    pub member: String,
    pub grant_revision: u64,
    pub receipt: Vec<u8>,
    pub proof: [u8; 32],
    pub routes: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    FirstAttempt,
    ObserveOnly,
    Stale,
}

/// The concrete implementation belongs on the existing Store. Confirmation
/// rechecks owner/expiry/candidate and inserts member, grant, original receipt
/// and policy in ONE IMMEDIATE transaction. Publication claims also persist
/// before I/O; an ambiguous attempt can only be observed, never repeated.
pub trait ConfirmationSink: Send + Sync {
    fn authorize(&self, context: &Context, actor: &OwnerActor) -> Result<(), RemoteError>;
    fn confirm(&self, input: Confirmation<'_>) -> Result<Committed, RemoteError>;
    fn original(&self, snapshot: &Snapshot) -> Result<Option<Committed>, RemoteError>;
    fn claim(&self, committed: &Committed, part: Part) -> Result<Claim, RemoteError>;
    fn acknowledge(&self, committed: &Committed, part: Part) -> Result<bool, RemoteError>;
}
