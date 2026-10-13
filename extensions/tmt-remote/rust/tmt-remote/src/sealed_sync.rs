//! Proposed sealed-document ports and pure binding. No provider or credential implementation.
//! Contract: Remote channel, extension-scoped sealed-document client.
use crate::{
    declaration::{self, Backend, Resource},
    deploy_plan::sha256_hex,
    hosting,
    objects::IoBudget,
};
use serde_json::Value;
use std::{future::Future, pin::Pin, sync::Arc};

pub type Pending<'a, T> = Pin<Box<dyn Future<Output = Result<T, SyncError>> + Send + 'a>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncError {
    Invalid,
    Denied,
    Unavailable,
    Cancelled,
    Deadline,
    BudgetExhausted(crate::firestore_budget::Refusal),
    RateLimited,
}
impl SyncError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "REMOTE_INPUT_INVALID",
            Self::Denied => "REMOTE_SCOPE_DENIED",
            Self::BudgetExhausted(_) => "REMOTE_BUDGET_EXHAUSTED",
            Self::RateLimited => "REMOTE_RATE_LIMITED",
            Self::Unavailable | Self::Cancelled | Self::Deadline => "REMOTE_STATE_UNAVAILABLE",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthIdentity {
    pub project: String,
    pub uid: String,
}
/// Deliberately lacks Debug/Serialize: secrets must not enter logs, receipts or operation IDs.
pub struct AuthToken {
    pub identity: AuthIdentity,
    pub token: String,
    pub expires_at_ms: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthState {
    Expired,
    SignInNeeded,
    Unavailable,
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthFailure {
    pub state: AuthState,
}
impl AuthFailure {
    pub fn code(self) -> &'static str {
        "REMOTE_STATE_UNAVAILABLE"
    }
}
/// Already authenticated identity; acquiring a token must never silently change its uid.
pub trait AuthPort: Send + Sync {
    fn identity(&self) -> AuthIdentity;
    fn token<'a>(
        &'a self,
        io: &'a IoBudget<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>>;
}
/// Caller supplies verified public config; parsing alone cannot attest an origin or admission.
pub struct SyncContext {
    config: Value,
    extension: String,
    declaration_digest: String,
    space: String,
    identity: AuthIdentity,
    resources: Vec<Resource>,
    auth: Arc<dyn AuthPort>,
}
impl SyncContext {
    pub fn bind(
        config: Value,
        bytes: &[u8],
        digest: &str,
        space: &str,
        auth: Arc<dyn AuthPort>,
    ) -> Result<Self, SyncError> {
        if sha256_hex(bytes) != digest || !id(space, 128) {
            return Err(SyncError::Invalid);
        }
        let declaration = declaration::parse(bytes).map_err(|_| SyncError::Invalid)?;
        for (i, resource) in declaration.resources.iter().enumerate() {
            let segments = resource.path.split('/').count();
            if segments > 7
                || segments % 2 == 0
                || declaration.resources[..i].iter().any(|other| {
                    resource.path == other.path
                        || resource.path.starts_with(&format!("{}/", other.path))
                        || other.path.starts_with(&format!("{}/", resource.path))
                })
            {
                return Err(SyncError::Invalid);
            }
        }
        let identity = auth.identity();
        if declaration.backend != Backend::Firestore || !id(&identity.uid, 128) {
            return Err(SyncError::Invalid);
        }
        let project = config
            .get("projectId")
            .and_then(Value::as_str)
            .ok_or(SyncError::Invalid)?;
        let app = config
            .get("appId")
            .and_then(Value::as_str)
            .ok_or(SyncError::Invalid)?;
        if !crate::deploy_run::project_ok(project)
            || !hosting::public_config_ok(Some(&config), project, app)
        {
            return Err(SyncError::Invalid);
        }
        if project != identity.project {
            return Err(SyncError::Denied);
        }
        Ok(Self {
            config,
            extension: declaration.extension,
            declaration_digest: digest.into(),
            space: space.into(),
            identity,
            resources: declaration.resources,
            auth,
        })
    }
    pub fn config(&self) -> &Value {
        &self.config
    }
    pub fn extension(&self) -> &str {
        &self.extension
    }
    pub fn declaration_digest(&self) -> &str {
        &self.declaration_digest
    }
    pub fn space(&self) -> &str {
        &self.space
    }
    pub fn identity(&self) -> &AuthIdentity {
        &self.identity
    }
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }
    pub fn auth(&self) -> &dyn AuthPort {
        self.auth.as_ref()
    }
    pub fn document_path(&self, target: &Target) -> Result<String, SyncError> {
        let resource = self
            .resources
            .iter()
            .find(|r| r.name == target.resource)
            .ok_or(SyncError::Invalid)?;
        if !id(&target.id, 256) {
            return Err(SyncError::Invalid);
        }
        Ok(format!(
            "x/{}/{}/{}",
            self.extension, resource.path, target.id
        ))
    }
}
fn id(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub resource: String,
    pub id: String,
}
/// Canonical restricted REST Document.fields bytes; sealed content stays encrypted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub target: Target,
    pub bytes: Vec<u8>,
    pub digest: String,
}
/// Last observed declared per-stream sequence; never a clock or lexical document-ID position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncCursor {
    pub stream: String,
    pub seq: u64,
}
pub struct Query {
    pub resource: String,
    pub scope: std::collections::BTreeMap<String, Value>,
    pub stream: String,
    pub cursor: Option<SyncCursor>,
}
pub struct Page {
    pub documents: Vec<Document>,
    pub cursor: Option<SyncCursor>,
}
pub enum Change {
    Document(Document),
    CatchUpRequired,
}
pub enum Published {
    Confirmed,
    Unknown,
}
pub trait Subscription: Send {
    fn next<'a>(&'a mut self, io: &'a IoBudget<'a>) -> Pending<'a, Change>;
    fn close(&mut self);
}
/// Later browser/native implementations own I/O and bounds behind this context, not operations.
pub trait SealedSyncPort: Send + Sync {
    fn context(&self) -> &SyncContext;
    fn read<'a>(
        &'a self,
        target: &'a Target,
        io: &'a IoBudget<'a>,
    ) -> Pending<'a, Option<Document>>;
    fn query<'a>(&'a self, query: &'a Query, io: &'a IoBudget<'a>) -> Pending<'a, Page>;
    fn subscribe<'a>(
        &'a self,
        query: &'a Query,
        io: &'a IoBudget<'a>,
    ) -> Pending<'a, Box<dyn Subscription>>;
    /// Only pre-effect refusal is Err; a possibly applied write returns Unknown, even on cancellation.
    fn publish<'a>(
        &'a self,
        document: &'a Document,
        io: &'a IoBudget<'a>,
    ) -> Pending<'a, Published>;
    fn close(&self);
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectManifest {
    pub original: String,
    pub count: u64,
    pub bytes: u64,
    pub digest: String,
    pub chunk_bytes: u64,
}
/// Pure complete-or-absent validation. No provider access or partial-data disclosure.
pub fn verify_object(
    original: &str,
    manifest: &ObjectManifest,
    chunks: &[Vec<u8>],
    max_object_bytes: u64,
) -> Result<(), SyncError> {
    if !id(original, 256)
        || manifest.original != original
        || max_object_bytes == 0
        || max_object_bytes > crate::limits::OBJECT_PAYLOAD_BYTES
        || manifest.chunk_bytes == 0
        || manifest.chunk_bytes > max_object_bytes
        || manifest.bytes > max_object_bytes
        || manifest.count > 513
        || manifest.count != manifest.bytes.div_ceil(manifest.chunk_bytes)
        || u64::try_from(chunks.len()).ok() != Some(manifest.count)
    {
        return Err(SyncError::Invalid);
    }
    let mut data = Vec::new();
    let mut offset = 0u64;
    for chunk in chunks {
        let expected = manifest.chunk_bytes.min(manifest.bytes - offset);
        if u64::try_from(chunk.len()).ok() != Some(expected) {
            return Err(SyncError::Invalid);
        }
        data.extend_from_slice(chunk);
        offset += expected;
    }
    if sha256_hex(&data) != manifest.digest {
        return Err(SyncError::Invalid);
    }
    Ok(())
}

mod codec;
pub use codec::{DocumentAccess, validate_document, validate_fields, validate_query};
