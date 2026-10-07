//! Typed request and result frames of the generic object channel: seven methods,
//! each with exact input, result and error associations. A frame carries
//! correlation only; no field names a principal, permission, retry or scope.
//!
//! Decoding and encoding apply one set of checks, so bytes that decode re-encode
//! to the same canonical text and an invalid frame cannot be encoded. Callback and
//! lifecycle kinds are not part of this slice and decode as an unknown kind.
use crate::{
    Bytes32, Chunk, Counter, ErrorClass, Policy, Sha256Hex, Uuid4,
    limits::{
        BACKEND_ID_BYTES, CHUNK_BYTES, FRAME_BYTES, MAX_SAFE_INTEGER, PAYLOAD_BYTES, PREFIX_BYTES,
    },
};

mod read;
mod write;

/// One decoded frame body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Request(Request),
    Result(ResultFrame),
}

/// Decode one frame body of exactly the length [`crate::decode_length`] announced.
pub fn decode(body: &[u8]) -> Result<Frame, ErrorClass> {
    let frame = read::frame(&crate::codec::strict_value(body)?)?;
    frame.check()?;
    Ok(frame)
}

/// The length prefix followed by the compact canonical body.
pub fn encode(frame: &Frame) -> Result<Vec<u8>, ErrorClass> {
    frame.check()?;
    let body = write::frame(frame);
    if body.len() > FRAME_BYTES {
        return Err(ErrorClass::Length);
    }
    let prefix = u32::try_from(body.len()).map_err(|_| ErrorClass::Length)?;
    let mut bytes = Vec::with_capacity(PREFIX_BYTES + body.len());
    bytes.extend_from_slice(&prefix.to_be_bytes());
    bytes.extend_from_slice(body.as_bytes());
    Ok(bytes)
}

/// The seven generic object operations, spelled `objects.<name>` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Config,
    Begin,
    Part,
    Commit,
    Status,
    Read,
    Discard,
}
impl Method {
    /// Whether results and errors of this method repeat the original transfer id.
    fn carries_transfer(self) -> bool {
        !matches!(self, Self::Config | Self::Read)
    }
}

/// Where the request entered the extension. Correlation of the connection, never
/// a statement about who may do what.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    LocalExtension,
    Mounted(Uuid4),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub generation: Uuid4,
    pub request_id: Counter,
    pub origin: Origin,
    pub call: Call,
}

/// A method together with its input, so a request cannot name one and carry another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Config(ConfigInput),
    Begin(BeginInput),
    Part(PartInput),
    Commit(TransferInput),
    Status(StatusInput),
    Read(ReadInput),
    Discard(TransferInput),
}
impl Call {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigInput {
    pub namespace: Bytes32,
    pub policy: Policy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginInput {
    pub transfer_id: Uuid4,
    pub namespace: Bytes32,
    pub opaque_key: Bytes32,
    pub policy: Policy,
    pub payload_sha256: Sha256Hex,
    pub payload_bytes: u64,
}
/// One part. `index` is any `u32`: ordering, full parts and the final remainder are
/// the backend's to enforce, not a property of the transport ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartInput {
    pub transfer_id: Uuid4,
    pub index: u32,
    pub bytes: Chunk,
}
/// Input of `commit` and `discard`: the original transfer id alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferInput {
    pub transfer_id: Uuid4,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusInput {
    pub transfer_id: Uuid4,
    pub namespace: Bytes32,
    pub policy: Policy,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadInput {
    pub namespace: Bytes32,
    pub opaque_key: Bytes32,
    pub policy: Policy,
    pub payload_sha256: Sha256Hex,
    pub payload_bytes: u64,
    pub offset: u64,
    pub count: u32,
}

/// The answer to one request. `transfer_id` is present exactly for the methods
/// that name a transfer, on success and on error alike.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultFrame {
    pub generation: Uuid4,
    pub request_id: Counter,
    pub method: Method,
    pub transfer_id: Option<Uuid4>,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success(Success),
    Failure(ErrorCode),
}

/// What a successful result carries; the allowed methods are in [`Success::allowed_for`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Success {
    Config(Config),
    /// `expires_at_ms` is the actual stored expiry and appears only when answering
    /// `status`; a begin never fabricates one.
    Pending {
        next_index: u32,
        received: u64,
        expires_at_ms: Option<u64>,
    },
    Progress {
        next_index: u32,
        received: u64,
    },
    Committed {
        opaque_key: Bytes32,
        payload_sha256: Sha256Hex,
        payload_bytes: u64,
    },
    State(State),
    Read {
        offset: u64,
        total_bytes: u64,
        bytes: Chunk,
    },
}
impl Success {
    fn allowed_for(&self, method: Method) -> bool {
        match self {
            Self::Config(_) => method == Method::Config,
            Self::Pending { .. } => matches!(method, Method::Begin | Method::Status),
            Self::Progress { .. } => method == Method::Part,
            Self::Committed { .. } => {
                matches!(method, Method::Begin | Method::Commit | Method::Status)
            }
            Self::State(State::Discarded) => {
                matches!(method, Method::Begin | Method::Status | Method::Discard)
            }
            Self::State(_) => matches!(method, Method::Begin | Method::Status),
            Self::Read { .. } => method == Method::Read,
        }
    }
}

/// A transfer's terminal or unobserved state. None proves that a possibly
/// published effect did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Expired,
    Discarded,
    Unavailable,
    Unknown,
    NotObserved,
}

/// The server-selected backend, its capabilities and effective bounds. Display
/// only: it grants no way to change the backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// `^[a-z][a-z0-9-]{0,31}$`.
    pub backend_id: String,
    pub immutable_create: bool,
    pub chunked_read: bool,
    pub recover_by_original_id: bool,
    pub limits: Limits,
}
/// Browser-projected bounds, or the owner's with the quotas added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limits {
    Browser(TransferBounds),
    Local {
        bounds: TransferBounds,
        namespace_bytes: u64,
        extension_bytes: u64,
        installation_bytes: u64,
    },
}
/// `chunk_bytes` is the backend's canonical part size, never a clamp to the transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferBounds {
    pub payload_bytes: u64,
    pub chunk_bytes: u32,
}

/// Why a request failed. `Unknown` is possible only where an effect may have
/// happened, which is the four mutating methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    Denied,
    Unavailable,
    Invalid,
    Conflict,
    Capacity(Limit),
    NotFound,
    Unknown,
}
impl ErrorCode {
    fn allowed_for(self, method: Method) -> bool {
        match self {
            Self::NotFound => method == Method::Read,
            Self::Unknown => matches!(
                method,
                Method::Begin | Method::Part | Method::Commit | Method::Discard
            ),
            _ => true,
        }
    }
}
/// The bound a `capacity` error names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    NamespaceBytes,
    ExtensionBytes,
    InstallationBytes,
    NamespaceEntries,
    ExtensionEntries,
    InstallationEntries,
    ActiveIntents,
    RetainedExtension,
    RetainedInstallation,
    Requests,
}

impl Frame {
    /// The grammar and association rules shared by decode and encode.
    fn check(&self) -> Result<(), ErrorClass> {
        match self {
            Self::Request(request) => request.call.check(),
            Self::Result(result) => result.check(),
        }
    }
}
fn require(condition: bool) -> Result<(), ErrorClass> {
    condition.then_some(()).ok_or(ErrorClass::Value)
}
impl Call {
    fn check(&self) -> Result<(), ErrorClass> {
        match self {
            Self::Begin(input) => require(input.payload_bytes <= PAYLOAD_BYTES),
            Self::Part(input) => require(!input.bytes.as_bytes().is_empty()),
            Self::Read(input) => {
                // The count is a request for at most one chunk. `offset == payload`
                // is a valid start only for the empty payload, which the backend answers
                // with an empty read.
                require(
                    input.payload_bytes <= PAYLOAD_BYTES
                        && (1..=CHUNK_BYTES as u32).contains(&input.count)
                        && (input.offset < input.payload_bytes
                            || (input.offset == 0 && input.payload_bytes == 0)),
                )
            }
            _ => Ok(()),
        }
    }
}
impl ResultFrame {
    fn check(&self) -> Result<(), ErrorClass> {
        require(self.transfer_id.is_some() == self.method.carries_transfer())?;
        match &self.outcome {
            Outcome::Failure(code) => require(code.allowed_for(self.method)),
            Outcome::Success(success) => {
                require(success.allowed_for(self.method))?;
                success.check(self.method)
            }
        }
    }
}
impl Success {
    fn check(&self, method: Method) -> Result<(), ErrorClass> {
        match self {
            Self::Config(config) => config.check(),
            Self::Pending {
                received,
                expires_at_ms,
                ..
            } => require(
                *received <= PAYLOAD_BYTES
                    && match expires_at_ms {
                        None => true,
                        Some(expiry) => method == Method::Status && *expiry <= MAX_SAFE_INTEGER,
                    },
            ),
            Self::Progress { received, .. } => require(*received <= PAYLOAD_BYTES),
            Self::Committed { payload_bytes, .. } => require(*payload_bytes <= PAYLOAD_BYTES),
            Self::State(_) => Ok(()),
            Self::Read {
                offset,
                total_bytes,
                bytes,
            } => require(
                *total_bytes <= PAYLOAD_BYTES
                    && offset.saturating_add(bytes.as_bytes().len() as u64) <= *total_bytes,
            ),
        }
    }
}
impl Config {
    fn check(&self) -> Result<(), ErrorClass> {
        let id = self.backend_id.as_bytes();
        let id_ok = !id.is_empty()
            && id.len() <= BACKEND_ID_BYTES
            && id[0].is_ascii_lowercase()
            && id[1..]
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-');
        let (Limits::Browser(bounds) | Limits::Local { bounds, .. }) = self.limits;
        let quotas = match self.limits {
            Limits::Browser(_) => [1; 3],
            Limits::Local {
                namespace_bytes,
                extension_bytes,
                installation_bytes,
                ..
            } => [namespace_bytes, extension_bytes, installation_bytes],
        };
        require(
            id_ok
                && (1..=PAYLOAD_BYTES).contains(&bounds.payload_bytes)
                && (1..=CHUNK_BYTES as u32).contains(&bounds.chunk_bytes)
                && quotas
                    .iter()
                    .all(|quota| (1..=MAX_SAFE_INTEGER).contains(quota)),
        )
    }
}

#[cfg(test)]
mod tests;
