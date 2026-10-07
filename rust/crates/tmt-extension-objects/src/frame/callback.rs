//! Admission callback and origin lifecycle frames. An `Admit` frame asks the
//! extension to decide at one boundary of one operation; the answer is a closed
//! `Admission` decision. Neither carries payload bytes, and the decision carries no
//! principal, role or permit. An owner-session context does name the owner device and
//! grant revision, as Remote established them for the extension's own decision; it is
//! trusted-later plumbing, never selected by a browser or the caller, and grants no
//! authority.
use super::*;

/// Where in the lifetime of one request the extension is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checkpoint {
    /// Before work starts.
    Acquire,
    /// Before a durable change; only the mutating methods have one.
    Effect,
    /// Before a result leaves; the frame names the exact class about to be disclosed.
    Disclose,
}

/// The connection the request actually arrived on, as the service established it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// A paired owner device under one grant revision.
    OwnerSession {
        origin_id: Uuid4,
        device_id: Uuid4,
        /// Positive, at most 2^53-1.
        grant_revision: u64,
    },
    Mounted {
        origin_id: Uuid4,
    },
    LocalExtension,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admit {
    pub generation: Uuid4,
    pub callback_id: Counter,
    pub request_id: Counter,
    pub boundary: Checkpoint,
    pub context: Context,
    pub operation: Operation,
}

/// The operation under decision. `disclosure` is present exactly at the disclose
/// boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation {
    pub input: AdmitInput,
    pub disclosure: Option<Disclosure>,
}

/// The request input as the extension may see it: the same fields for `config`,
/// `begin`, `status` and `read`; for the others the transfer plus what was frozen at
/// its begin, and never the bytes of a part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmitInput {
    Config(ConfigInput),
    Begin(BeginInput),
    Part(PartAdmit),
    Commit(TransferAdmit),
    Status(StatusInput),
    Read(ReadInput),
    Discard(TransferAdmit),
}
impl AdmitInput {
    pub fn method(&self) -> Method {
        match self {
            Self::Config(_) => Method::Config,
            Self::Begin(_) => Method::Begin,
            Self::Part(_) => Method::Part,
            Self::Commit(_) => Method::Commit,
            Self::Status(_) => Method::Status,
            Self::Read(_) => Method::Read,
            Self::Discard(_) => Method::Discard,
        }
    }
}
/// A part under decision: its position and size, not its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartAdmit {
    pub transfer_id: Uuid4,
    pub index: u32,
    /// 1 to 32,768.
    pub length: u32,
    pub retained: Retained,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferAdmit {
    pub transfer_id: Uuid4,
    pub retained: Retained,
}
/// What the transfer froze when it began.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retained {
    pub namespace: Bytes32,
    pub opaque_key: Bytes32,
    pub policy: Policy,
    pub payload_sha256: Sha256Hex,
    pub payload_bytes: u64,
}

/// The exact class of result about to be disclosed, without its content: the bytes
/// of a read are only counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disclosure {
    Config {
        projection: Projection,
    },
    /// A pending answer to `begin` or `status`; the expiry only to `status`.
    Status {
        next_index: u32,
        received: u64,
        expires_at_ms: Option<u64>,
    },
    Progress {
        next_index: u32,
        received: u64,
    },
    Receipt {
        opaque_key: Bytes32,
        payload_sha256: Sha256Hex,
        payload_bytes: u64,
    },
    Terminal {
        state: State,
    },
    Bytes {
        offset: u64,
        /// At most 32,768; zero for an empty read.
        length: u32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Browser,
    Local,
}
impl Disclosure {
    fn answer(&self) -> Answer {
        match self {
            Self::Config { .. } => Answer::Config,
            Self::Status { .. } => Answer::Pending,
            Self::Progress { .. } => Answer::Progress,
            Self::Receipt { .. } => Answer::Committed,
            Self::Terminal { state } => Answer::State(*state),
            Self::Bytes { .. } => Answer::Read,
        }
    }
    fn check(&self, method: Method) -> Result<(), ErrorClass> {
        require(self.answer().allowed_for(method))?;
        match *self {
            Self::Status {
                received,
                expires_at_ms,
                ..
            } => require(
                received <= PAYLOAD_BYTES
                    && match expires_at_ms {
                        None => true,
                        Some(expiry) => method == Method::Status && expiry <= MAX_SAFE_INTEGER,
                    },
            ),
            Self::Progress { received, .. } => require(received <= PAYLOAD_BYTES),
            Self::Receipt { payload_bytes, .. } => require(payload_bytes <= PAYLOAD_BYTES),
            Self::Bytes { offset, length } => {
                require(offset <= PAYLOAD_BYTES && length as usize <= CHUNK_BYTES)
            }
            Self::Config { .. } | Self::Terminal { .. } => Ok(()),
        }
    }
}

impl Admit {
    pub(super) fn check(&self) -> Result<(), ErrorClass> {
        if let Context::OwnerSession { grant_revision, .. } = self.context {
            require((1..=MAX_SAFE_INTEGER).contains(&grant_revision))?;
        }
        let method = self.operation.input.method();
        self.operation.input.check()?;
        let boundary_exists = self.boundary != Checkpoint::Effect || method.mutates();
        require(boundary_exists)?;
        match (self.boundary, &self.operation.disclosure) {
            (Checkpoint::Disclose, Some(disclosure)) => disclosure.check(method),
            (Checkpoint::Disclose, None) | (_, Some(_)) => Err(ErrorClass::Value),
            (_, None) => Ok(()),
        }
    }
}
impl AdmitInput {
    fn check(&self) -> Result<(), ErrorClass> {
        match self {
            Self::Begin(input) => input.check(),
            Self::Read(input) => input.check(),
            Self::Part(input) => {
                input.retained.check()?;
                require((1..=CHUNK_BYTES as u32).contains(&input.length))
            }
            Self::Commit(input) | Self::Discard(input) => input.retained.check(),
            Self::Config(_) | Self::Status(_) => Ok(()),
        }
    }
}
impl Retained {
    fn check(&self) -> Result<(), ErrorClass> {
        require(self.payload_bytes <= PAYLOAD_BYTES)
    }
}

/// The extension's answer to one `Admit`. The three decisions are the whole
/// vocabulary: it carries no permit, principal, role, target or scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admission {
    pub generation: Uuid4,
    pub callback_id: Counter,
    pub request_id: Counter,
    pub decision: Decision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    /// The extension could not decide; neither permission nor a statement that no
    /// effect happened.
    Unavailable,
}

/// A change in the life of one origin, announced to the extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OriginState {
    pub generation: Uuid4,
    pub origin_id: Uuid4,
    pub phase: OriginPhase,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginPhase {
    Established,
    Closed,
}

#[cfg(test)]
mod tests;
