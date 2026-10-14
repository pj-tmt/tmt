//! Constructor seam only. The project owner supplies verified configuration,
//! declaration and Auth, and captures its shared runtime/model in the factory.
use crate::sealed_sync::{AuthPort, SealedSyncPort, SyncError};
use serde_json::Value;
use std::sync::Arc;

pub struct BindInput<'a> {
    pub public_config: Value,
    pub declaration: &'a [u8],
    pub declaration_digest: &'a str,
    pub space: &'a str,
    pub auth: Arc<dyn AuthPort>,
}
/// Pure construction of the existing SyncContext/NativeClient. No provider I/O,
/// signup, new runtime, budget pool, timer or independently owned project state.
pub type ClientFactory =
    Arc<dyn for<'a> Fn(BindInput<'a>) -> Result<Arc<dyn SealedSyncPort>, SyncError> + Send + Sync>;

/// Trusted project-owner registration, not wire-supplied config or credentials.
/// The owner obtains the release declaration through InstalledDeclarations and
/// verifies its envelope before registration; it owns Auth cancellation, its
/// wakeable minute checkpoint and final finish after ObjectService shutdown.
pub struct Registration {
    pub(super) public_config: Value,
    pub(super) declaration: Vec<u8>,
    pub(super) digest: String,
    pub(super) extension: String,
    pub(super) auth: Arc<dyn AuthPort>,
    pub(super) factory: ClientFactory,
}
impl Registration {
    pub fn new(
        public_config: Value,
        declaration: Vec<u8>,
        declaration_digest: String,
        auth: Arc<dyn AuthPort>,
        factory: ClientFactory,
    ) -> Result<Self, SyncError> {
        let parsed = crate::declaration::parse(&declaration).map_err(|_| SyncError::Invalid)?;
        if parsed.backend != crate::declaration::Backend::Firestore
            || crate::deploy_plan::sha256_hex(&declaration) != declaration_digest
        {
            return Err(SyncError::Invalid);
        }
        if public_config.get("projectId").and_then(Value::as_str)
            != Some(auth.identity().project.as_str())
        {
            return Err(SyncError::Denied);
        }
        Ok(Self {
            public_config,
            declaration,
            digest: declaration_digest,
            extension: parsed.extension,
            auth,
            factory,
        })
    }
    pub(in crate::object_service) fn extension(&self) -> &str {
        &self.extension
    }
    pub(super) fn bind(&self, space: &str) -> Result<Arc<dyn SealedSyncPort>, SyncError> {
        let identity = self.auth.identity();
        if self.public_config.get("projectId").and_then(Value::as_str)
            != Some(identity.project.as_str())
        {
            return Err(SyncError::Denied);
        }
        let port = (self.factory)(BindInput {
            public_config: self.public_config.clone(),
            declaration: &self.declaration,
            declaration_digest: &self.digest,
            space,
            auth: self.auth.clone(),
        })?;
        let context = port.context();
        if context.config() != &self.public_config
            || context.extension() != self.extension
            || context.declaration_digest() != self.digest
            || context.space() != space
            || context.identity() != &identity
        {
            return Err(SyncError::Denied);
        }
        Ok(port)
    }
}
