//! Transport capabilities, not authority. An adapter authenticates its provider
//! evidence; only the separate confirmation sink can create durable authority.
use super::{BackendBinding, Candidate, Context};
use crate::{error::RemoteError, objects::IoBudget, store::Grant};

/// Native possession signing without exposing private key material. The shared
/// proof owner constructs the bytes and verifies the returned signature.
pub trait NativeSigner {
    fn public_key(&self) -> [u8; 32];
    fn sign_possession(&self, message: &[u8]) -> [u8; 64];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, PartialEq, Eq)]
pub enum BackendFailure {
    Refused,
    Unavailable,
    InvalidEvidence,
    Cancelled,
    Deadline,
    /// Sanitized adapter refusal/uncertainty with its shipped error identity.
    Provider(RemoteError),
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
pub struct OwnerActor {
    pub(super) authority: Actor,
    pub(super) operation: Option<OwnerOperation>,
    pub(super) admission: Option<crate::admission::BrowserMessageFence>,
}

/// Frozen identity of an already adopted, signed browser confirmation. It holds
/// no invitation code or response capability and is never decoded from JSON.
pub(crate) struct OwnerOperation {
    pub(crate) id: String,
    pub(crate) intent: [u8; 32],
    pub(crate) offer: String,
    pub(crate) snapshot: [u8; 32],
    pub(crate) matched: bool,
    pub(crate) sending: bool,
}

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
        Self {
            authority: Actor::LocalInteractive,
            operation: None,
            admission: None,
        }
    }
    pub fn owner_browser(client: String, key: [u8; 32], revision: u64) -> Self {
        Self {
            authority: Actor::OwnerBrowser {
                client,
                key,
                revision,
            },
            operation: None,
            admission: None,
        }
    }
    /// The signed management adapter must adopt this exact intent first. The
    /// writer independently verifies the adopted row and its member ownership.
    pub fn owner_browser_decision(
        grant: &Grant,
        operation_id: String,
        intent: [u8; 32],
        snapshot: &Snapshot,
        decision: OwnerDecision,
    ) -> Result<Self, RemoteError> {
        if !crate::canonical::is_core_id(&operation_id)
            || operation_id == snapshot.context.offer
            || grant.kind != "browser"
            || (!decision.matched && decision.enable_sending)
        {
            return Err(super::invalid());
        }
        let mut actor =
            Self::owner_browser(grant.client_id.clone(), grant.public_key, grant.revision);
        actor.operation = Some(OwnerOperation {
            id: operation_id,
            intent,
            offer: snapshot.context.offer.clone(),
            snapshot: snapshot.digest,
            matched: decision.matched,
            sending: decision.enable_sending,
        });
        Ok(actor)
    }
    pub(crate) fn with_admission(
        mut self,
        permit: &crate::admission::MessagePermit,
    ) -> Result<Self, RemoteError> {
        let matches = match &self.authority {
            Actor::OwnerBrowser {
                client,
                key,
                revision,
            } => {
                *client == permit.grant.client_id
                    && *key == permit.grant.public_key
                    && *revision == permit.grant.revision
            }
            _ => false,
        };
        let envelope = permit.message.envelope();
        let bound = match &self.operation {
            Some(op) => {
                envelope.operation == "remote.invites.confirm"
                    && envelope.id == op.id
                    && crate::journal::intent(&permit.message)? == op.intent
            }
            None => matches!(
                envelope.operation,
                "remote.invites.pending" | "remote.invites.confirm"
            ),
        };
        if !matches || !bound {
            return Err(super::invalid());
        }
        self.admission = Some(permit.browser_fence()?);
        Ok(self)
    }
    pub fn automated() -> Self {
        Self {
            authority: Actor::Automated,
            operation: None,
            admission: None,
        }
    }
    pub(super) fn interactive(&self) -> bool {
        match &self.authority {
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

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub context: Context,
    pub candidate: Candidate,
    pub digest: [u8; 32],
}

pub struct Confirmation<'a> {
    pub(super) verified: &'a super::VerifiedCandidate,
    pub snapshot: &'a Snapshot,
    pub actor: &'a OwnerActor,
    pub grant: &'a Grant,
    pub receipt_key: &'a ReceiptKey,
    /// Separate admitted owner choice, never a candidate policy option.
    pub sending: bool,
    pub io: &'a IoBudget<'a>,
}

/// Constructed by the admitted owner interaction, without a provider/JSON codec.
#[derive(Clone, Copy)]
pub struct OwnerDecision {
    pub matched: bool,
    pub enable_sending: bool,
}

/// Scoped MAC capability. It is never persisted, formatted or sent to a backend.
pub struct ReceiptKey(pub(super) [u8; 32]);
impl ReceiptKey {
    pub fn prove(&self, receipt: &[u8]) -> Result<[u8; 32], RemoteError> {
        crate::crypto::server_proof(&self.0, receipt).map_err(|_| super::invalid())
    }
    pub(crate) fn capability(&self, framed_context: &[u8]) -> [u8; 32] {
        crate::crypto::mac(&self.0, framed_context)
    }
}
impl Drop for ReceiptKey {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// Exact original receipt and publication bytes returned by the writer. Routes
/// are an opaque publication capability, not a backend-created member/grant.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Only a proven absence of commit is a definite refusal. An uncertain commit
/// retains its original identity and can be settled only by journal observation.
#[derive(Debug)]
pub enum ConfirmationFailure {
    Refused(RemoteError),
    CommitUnknown(RemoteError),
}

/// The concrete implementation belongs on the existing Store. Confirmation
/// rechecks owner/expiry/candidate and inserts member, grant, original receipt
/// and policy in ONE IMMEDIATE transaction. Publication claims also persist
/// before I/O; an ambiguous attempt can only be observed, never repeated.
pub trait ConfirmationSink: Send + Sync {
    fn authorize(&self, context: &Context, actor: &OwnerActor) -> Result<(), RemoteError>;
    fn confirm(&self, input: Confirmation<'_>) -> Result<Committed, ConfirmationFailure>;
    fn decline(
        &self,
        actor: &OwnerActor,
        snapshot: &Snapshot,
        io: &IoBudget<'_>,
    ) -> Result<(), ConfirmationFailure> {
        io.check()
            .map_err(|_| ConfirmationFailure::Refused(super::invalid()))?;
        self.authorize(&snapshot.context, actor)
            .map_err(ConfirmationFailure::Refused)
    }
    fn original(&self, snapshot: &Snapshot) -> Result<Option<Committed>, RemoteError>;
    fn claim(&self, committed: &Committed, part: Part) -> Result<Claim, RemoteError>;
    fn acknowledge(&self, committed: &Committed, part: Part) -> Result<bool, RemoteError>;
}
