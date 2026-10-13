//! v3's neutral sealed-document wire. Fields remain opaque to the carrier;
//! provider admission and cryptographic authority belong to the applications.
//!
//! The selected carrier version is 3; frame schema `version` remains 1. The body
//! limit is 397,312 bytes, canonical fields per document 393,216, and non-fields
//! metadata 4,096. The four-byte length prefix is excluded. Query results contain
//! at most 256 documents and one frame; continuation is at most 1,024 graphic ASCII
//! bytes. There is no staging, batch handle, document-event queue or wire drain.
//!
//! Generation and binding IDs are lowercase UUIDv4. RPC IDs are positive canonical
//! u64 decimal strings, shared with object requests. A subscription's ID is its
//! subscribe RPC ID, so it cannot be reused in a generation; notice sequences are
//! positive canonical u64 strings increasing within that interest. Cursor sequences
//! and numeric milliseconds are unsigned JSON-safe integers (at most 2^53-1).
//! Space/uid/stream names are 1..128 ASCII alphanumeric/underscore/plus/hyphen
//! bytes, document IDs 1..256, and project/extension/resource/shape names 1..64.
//! Fields and scope maps have at most 64 entries. Provider declaration/codec
//! admission, Rules and cryptographic authority are outside the leaf.
//!
//! One channel retains at most eight bindings and three page interests, with twelve
//! interests per installation. Interests count pages, never writers; the bus keeps
//! its existing eight-frame/524,288-body-byte queue and shared 8/32 request caps.
//! Every encoder/decoder uses these bounds, including actual encoded metadata.
use crate::{Counter, ErrorClass, Sha256Hex, Uuid4, limits};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// Nullable members are present in the wire schema. Serde's default Option
// handling would also admit a missing member, unlike the strict frame grammar.
fn required_option<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub resource: String,
    pub id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Document {
    pub target: Target,
    pub fields: Map<String, Value>,
    pub digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamPosition {
    pub stream: String,
    pub seq: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Query {
    Stream {
        resource: String,
        scope: Map<String, Value>,
        stream: String,
        #[serde(deserialize_with = "required_option")]
        cursor: Option<StreamPosition>,
    },
    Discovery {
        resource: String,
        name: String,
        scope: Map<String, Value>,
        #[serde(deserialize_with = "required_option")]
        continuation: Option<String>,
    },
}
/// One whole-field mutation; no staging, stamp or hidden retry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Mutation {
    Create { document: Document },
    Put { document: Document },
    Delete { target: Target },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Published {
    Confirmed,
    AlreadyPresent,
    Unknown,
}
/// One page interest; no stream, cursor, continuation or delivery position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interest {
    pub resource: String,
    pub scope: Map<String, Value>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "notice", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InterestNotice {
    Changed,
    Terminal { error: Error },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    #[serde(rename = "sync.bind")]
    Bind,
    #[serde(rename = "sync.read")]
    Read,
    #[serde(rename = "sync.query")]
    Query,
    #[serde(rename = "sync.subscribe")]
    Subscribe,
    #[serde(rename = "sync.cancel")]
    Cancel,
    #[serde(rename = "sync.publish")]
    Publish,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "method",
    content = "input",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Call {
    #[serde(rename = "sync.bind")]
    Bind {
        project: String,
        extension: String,
        declaration_digest: String,
        space: String,
    },
    #[serde(rename = "sync.read")]
    Read { binding: String, target: Target },
    #[serde(rename = "sync.query")]
    Query { binding: String, query: Query },
    #[serde(rename = "sync.subscribe")]
    Subscribe { binding: String, interest: Interest },
    #[serde(rename = "sync.cancel")]
    Cancel {
        binding: String,
        subscription_id: String,
    },
    #[serde(rename = "sync.publish")]
    Publish { binding: String, mutation: Mutation },
}
impl Call {
    pub fn method(&self) -> Method {
        match self {
            Self::Bind { .. } => Method::Bind,
            Self::Read { .. } => Method::Read,
            Self::Query { .. } => Method::Query,
            Self::Subscribe { .. } => Method::Subscribe,
            Self::Cancel { .. } => Method::Cancel,
            Self::Publish { .. } => Method::Publish,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundContext {
    pub id: String,
    pub project: String,
    pub extension: String,
    pub declaration_digest: String,
    pub space: String,
    pub uid: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Code {
    #[serde(rename = "REMOTE_INPUT_INVALID")]
    Invalid,
    #[serde(rename = "REMOTE_SCOPE_DENIED")]
    Denied,
    #[serde(rename = "REMOTE_STATE_UNAVAILABLE")]
    Unavailable,
    #[serde(rename = "REMOTE_BUDGET_EXHAUSTED")]
    Budget,
    #[serde(rename = "REMOTE_RATE_LIMITED")]
    Rate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Cancelled,
    Deadline,
    CatchUpRequired,
    Conflict,
    Unsupported,
    Unenrolled,
    Expired,
    SignInNeeded,
    Revoking,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Error {
    pub code: Code,
    #[serde(deserialize_with = "required_option")]
    pub state: Option<State>,
    #[serde(deserialize_with = "required_option")]
    pub reset_at_ms: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    pub retry_after_ms: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Outcome {
    Binding {
        context: BoundContext,
    },
    Read {
        #[serde(deserialize_with = "required_option")]
        document: Option<Document>,
    },
    Stream {
        documents: Vec<Document>,
        #[serde(deserialize_with = "required_option")]
        cursor: Option<StreamPosition>,
    },
    Discovery {
        documents: Vec<Document>,
        #[serde(deserialize_with = "required_option")]
        continuation: Option<String>,
    },
    Error {
        error: Error,
    },
    Subscribed {
        id: String,
    },
    Cancelled {
        id: String,
    },
    Published {
        status: Published,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Frame {
    #[serde(rename = "sync-request")]
    Request {
        version: u8,
        generation: String,
        request_id: String,
        call: Call,
    },
    #[serde(rename = "sync-result")]
    Result {
        version: u8,
        generation: String,
        request_id: String,
        method: Method,
        outcome: Outcome,
    },
    #[serde(rename = "sync-notice")]
    Notice {
        version: u8,
        generation: String,
        subscription_id: String,
        sequence: String,
        notice: InterestNotice,
    },
}
impl Frame {
    pub fn generation(&self) -> Result<Uuid4, ErrorClass> {
        Uuid4::parse(match self {
            Self::Request { generation, .. }
            | Self::Result { generation, .. }
            | Self::Notice { generation, .. } => generation,
        })
    }
    pub fn request_id(&self) -> Result<Counter, ErrorClass> {
        Counter::parse(match self {
            Self::Request { request_id, .. } | Self::Result { request_id, .. } => request_id,
            Self::Notice { sequence, .. } => sequence,
        })
    }
    pub fn method(&self) -> Method {
        match self {
            Self::Request { call, .. } => call.method(),
            Self::Result { method, .. } => *method,
            Self::Notice { .. } => Method::Subscribe,
        }
    }
    fn documents(&self) -> Vec<&Document> {
        match self {
            Self::Request {
                call:
                    Call::Publish {
                        mutation: Mutation::Create { document } | Mutation::Put { document },
                        ..
                    },
                ..
            } => vec![document],
            Self::Result {
                outcome: Outcome::Read { document },
                ..
            } => document.iter().collect(),
            Self::Result {
                outcome: Outcome::Stream { documents, .. } | Outcome::Discovery { documents, .. },
                ..
            } => documents.iter().collect(),
            _ => Vec::new(),
        }
    }
    fn check(&self) -> Result<(), ErrorClass> {
        let version = match self {
            Self::Request { version, .. }
            | Self::Result { version, .. }
            | Self::Notice { version, .. } => *version,
        };
        if version != 1 {
            return Err(ErrorClass::Value);
        }
        self.generation()?;
        self.request_id()?;
        match self {
            Self::Request { call, .. } => match call {
                Call::Bind {
                    project,
                    extension,
                    declaration_digest,
                    space,
                } => {
                    id(project, 64)?;
                    id(extension, 64)?;
                    id(space, 128)?;
                    Sha256Hex::parse(declaration_digest)?;
                }
                Call::Read { binding, target } => {
                    Uuid4::parse(binding)?;
                    target.check()?;
                }
                Call::Query { binding, query } => {
                    Uuid4::parse(binding)?;
                    query.check()?;
                }
                Call::Subscribe { binding, interest } => {
                    Uuid4::parse(binding)?;
                    id(&interest.resource, 64)?;
                    scope_check(&interest.scope)?;
                }
                Call::Cancel {
                    binding,
                    subscription_id,
                } => {
                    Uuid4::parse(binding)?;
                    Counter::parse(subscription_id)?;
                }
                Call::Publish { binding, mutation } => {
                    Uuid4::parse(binding)?;
                    match mutation {
                        Mutation::Create { document } | Mutation::Put { document } => {
                            document.check()?
                        }
                        Mutation::Delete { target } => target.check()?,
                    }
                }
            },
            Self::Result {
                method, outcome, ..
            } => {
                let allowed = matches!(
                    (method, outcome),
                    (Method::Bind, Outcome::Binding { .. })
                        | (Method::Read, Outcome::Read { .. })
                        | (
                            Method::Query,
                            Outcome::Stream { .. } | Outcome::Discovery { .. }
                        )
                        | (Method::Subscribe, Outcome::Subscribed { .. })
                        | (Method::Cancel, Outcome::Cancelled { .. })
                        | (Method::Publish, Outcome::Published { .. })
                        | (_, Outcome::Error { .. })
                );
                if !allowed {
                    return Err(ErrorClass::Shape);
                }
                match outcome {
                    Outcome::Binding { context } => {
                        Uuid4::parse(&context.id)?;
                        id(&context.project, 64)?;
                        id(&context.extension, 64)?;
                        id(&context.space, 128)?;
                        id(&context.uid, 128)?;
                        Sha256Hex::parse(&context.declaration_digest)?;
                    }
                    Outcome::Stream {
                        cursor: Some(cursor),
                        ..
                    } => cursor.check()?,
                    Outcome::Discovery { continuation, .. } => continuation_check(continuation)?,
                    Outcome::Error { error } => error.check()?,
                    Outcome::Subscribed { id } | Outcome::Cancelled { id } => {
                        Counter::parse(id)?;
                    }
                    _ => {}
                }
            }
            Self::Notice {
                subscription_id,
                notice,
                ..
            } => {
                Counter::parse(subscription_id)?;
                if let InterestNotice::Terminal { error } = notice {
                    error.check()?
                }
            }
        }
        let docs = self.documents();
        if docs.len() > limits::SYNC_PAGE_DOCUMENTS {
            return Err(ErrorClass::Length);
        }
        for doc in docs {
            doc.check()?;
        }
        Ok(())
    }
}
impl Error {
    fn check(&self) -> Result<(), ErrorClass> {
        let budget = self.code == Code::Budget;
        if budget != (self.reset_at_ms.is_some() && self.retry_after_ms.is_some())
            || (!budget && (self.reset_at_ms.is_some() || self.retry_after_ms.is_some()))
            || (self.state.is_some() && self.code != Code::Unavailable)
        {
            return Err(ErrorClass::Shape);
        }
        Ok(())
    }
}
impl Target {
    fn check(&self) -> Result<(), ErrorClass> {
        id(&self.resource, 64)?;
        id(&self.id, 256)
    }
}
impl StreamPosition {
    fn check(&self) -> Result<(), ErrorClass> {
        id(&self.stream, 128)?;
        if self.seq > limits::MAX_SAFE_INTEGER {
            return Err(ErrorClass::Value);
        }
        Ok(())
    }
}
impl Document {
    fn check(&self) -> Result<(), ErrorClass> {
        self.target.check()?;
        Sha256Hex::parse(&self.digest)?;
        if self.fields.len() > 64
            || serde_json::to_vec(&self.fields)
                .map_err(|_| ErrorClass::Syntax)?
                .len()
                > limits::SYNC_DOCUMENT_BYTES
        {
            return Err(ErrorClass::Length);
        }
        Ok(())
    }
}
impl Query {
    fn check(&self) -> Result<(), ErrorClass> {
        match self {
            Self::Stream {
                resource,
                scope,
                stream,
                cursor,
            } => {
                id(resource, 64)?;
                id(stream, 128)?;
                scope_check(scope)?;
                if let Some(cursor) = cursor {
                    cursor.check()?;
                    if cursor.stream != *stream {
                        return Err(ErrorClass::Value);
                    }
                }
            }
            Self::Discovery {
                resource,
                name,
                scope,
                continuation,
            } => {
                id(resource, 64)?;
                id(name, 64)?;
                if name == "stream" {
                    return Err(ErrorClass::Value);
                }
                scope_check(scope)?;
                continuation_check(continuation)?;
            }
        }
        Ok(())
    }
}
fn scope_check(scope: &Map<String, Value>) -> Result<(), ErrorClass> {
    if scope.len() > 64 {
        return Err(ErrorClass::Length);
    }
    Ok(())
}
fn id(value: &str, max: usize) -> Result<(), ErrorClass> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
    {
        return Err(ErrorClass::Value);
    }
    Ok(())
}
fn continuation_check(value: &Option<String>) -> Result<(), ErrorClass> {
    if let Some(value) = value
        && (value.is_empty()
            || value.len() > limits::SYNC_CONTINUATION_BYTES
            || !value.bytes().all(|b| b.is_ascii_graphic()))
    {
        return Err(ErrorClass::Value);
    }
    Ok(())
}
/// Actual encoded size, including all metadata, excluding the length prefix.
pub fn encoded_size(frame: &Frame) -> Result<usize, ErrorClass> {
    frame.check()?;
    let body = serde_json::to_vec(frame).map_err(|_| ErrorClass::Syntax)?;
    let fields = frame.documents().iter().try_fold(0usize, |sum, doc| {
        let size = serde_json::to_vec(&doc.fields)
            .map_err(|_| ErrorClass::Syntax)?
            .len();
        sum.checked_add(size).ok_or(ErrorClass::Length)
    })?;
    crate::codec::strict_value_with_limit(&body, limits::V3_FRAME_BYTES)?;
    if body.len().saturating_sub(fields) > limits::SYNC_METADATA_BYTES
        || body.len() > limits::V3_FRAME_BYTES
    {
        return Err(ErrorClass::Length);
    }
    Ok(body.len())
}
pub fn encode(frame: &Frame) -> Result<Vec<u8>, ErrorClass> {
    let size = encoded_size(frame)?;
    let mut bytes = Vec::with_capacity(size + 4);
    bytes.extend_from_slice(&(size as u32).to_be_bytes());
    bytes.extend_from_slice(&serde_json::to_vec(frame).map_err(|_| ErrorClass::Syntax)?);
    Ok(bytes)
}
pub fn decode(body: &[u8]) -> Result<Frame, ErrorClass> {
    let value = crate::codec::strict_value_with_limit(body, limits::V3_FRAME_BYTES)?;
    let frame: Frame = serde_json::from_value(value).map_err(|_| ErrorClass::Shape)?;
    encoded_size(&frame)?;
    received_size(body, &frame)?;
    Ok(frame)
}
/// Count actual received payload spans, so whitespace or escape spellings cannot
/// turn a metadata-heavy input into a smaller admitted reencoding.
fn received_size(body: &[u8], frame: &Frame) -> Result<(), ErrorClass> {
    use serde_json::value::RawValue;
    fn member<'a>(object: &'a RawValue, name: &str) -> Result<&'a RawValue, ErrorClass> {
        let members: std::collections::BTreeMap<String, &'a RawValue> =
            serde_json::from_str(object.get()).map_err(|_| ErrorClass::Shape)?;
        members.get(name).copied().ok_or(ErrorClass::Shape)
    }
    let root: &RawValue = serde_json::from_slice(body).map_err(|_| ErrorClass::Syntax)?;
    let documents: Vec<&RawValue> = match frame {
        Frame::Request {
            call:
                Call::Publish {
                    mutation: Mutation::Create { .. } | Mutation::Put { .. },
                    ..
                },
            ..
        } => {
            let input = member(member(root, "call")?, "input")?;
            vec![member(member(input, "mutation")?, "document")?]
        }
        Frame::Result {
            outcome: Outcome::Read { document: Some(_) },
            ..
        } => vec![member(member(root, "outcome")?, "document")?],
        Frame::Result {
            outcome: Outcome::Stream { .. } | Outcome::Discovery { .. },
            ..
        } => serde_json::from_str(member(member(root, "outcome")?, "documents")?.get())
            .map_err(|_| ErrorClass::Shape)?,
        _ => Vec::new(),
    };
    let mut payload = 0usize;
    for document in documents {
        let bytes = member(document, "fields")?.get().len();
        if bytes > limits::SYNC_DOCUMENT_BYTES {
            return Err(ErrorClass::Length);
        }
        payload = payload.checked_add(bytes).ok_or(ErrorClass::Length)?;
    }
    if body.len().saturating_sub(payload) > limits::SYNC_METADATA_BYTES {
        return Err(ErrorClass::Length);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
