//! Original identity shared by observational and mutating requests.
use crate::objects::{ExtensionId, IntentId};
use sha2::{Digest as _, Sha256};
use tmt_extension_objects::{Context, Uuid4};

/// Stable across owner-session/grant changes, but never across installed
/// extensions, principal kinds, owner devices or non-owner connections.
pub(super) fn original_id(extension: &ExtensionId, context: Context, transfer: Uuid4) -> IntentId {
    let (kind, principal): (&[u8], &[u8]) = match &context {
        Context::LocalExtension => (b"local", extension.as_str().as_bytes()),
        Context::OwnerSession { device_id, .. } => (b"owner", device_id.as_bytes()),
        Context::Mounted { origin_id } => (b"mounted", origin_id.as_bytes()),
    };
    let mut digest = Sha256::new();
    for field in [
        b"tmt.remote.original.v1".as_slice(),
        extension.as_str().as_bytes(),
        kind,
        principal,
        transfer.as_bytes(),
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    IntentId(digest.finalize().into())
}
