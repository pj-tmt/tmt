//! Colab policy bytes stay here; the neutral leaf and Remote never parse them.
use super::{CallbackOwner, Client};
use crate::{Result, attachments, page};
use serde::Serialize;
use std::sync::Arc;
use tmt_colab_model::{
    attachment::{AttachmentSelector, Descriptor, Source},
    crypto, values,
};
use tmt_extension_objects::{
    BeginInput, Bytes32, Call, Chunk, Origin, Outcome, PartInput, Policy, Sha256Hex, StatusInput,
    Success, TransferInput, Uuid4,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadPolicy<'a> {
    version: u8,
    space: &'a str,
    page: &'a str,
    epoch: &'a str,
    target: &'a Source,
    attachment_id: &'a str,
    descriptor_hash: String,
    object_id: &'a str,
    envelope_hash: &'a str,
    payload_sha256: &'a str,
    payload_bytes: &'a str,
    plaintext_bytes: &'a str,
}
#[derive(Serialize)]
struct ReadPolicy<'a> {
    version: u8,
    space: &'a str,
    page: &'a str,
    epoch: &'a str,
    reference: &'a AttachmentSelector,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn key(descriptor: &Descriptor) -> Result<Bytes32> {
    let hash = Sha256Hex::parse(&descriptor.object_id).map_err(|_| page::Fault::Invalid)?;
    Ok(Bytes32::from_bytes(*hash.as_bytes()))
}
fn policy(value: &impl Serialize) -> Result<Policy> {
    Policy::new(serde_json::to_vec(value)?).map_err(|_| page::Fault::Capacity.into())
}
pub(crate) fn read_policy(
    descriptor: &Descriptor,
    peer_epoch: &str,
    reference: &AttachmentSelector,
) -> Result<Policy> {
    descriptor.validate()?;
    reference.validate()?;
    values::decimal(peer_epoch, false)?;
    policy(&ReadPolicy {
        version: 1,
        space: &descriptor.space,
        page: &descriptor.page,
        epoch: peer_epoch,
        reference,
    })
}
/// A retained ciphertext/target binding, NEVER an allow or a caller-provided
/// committed flag. Actual peer authority must be checked at every bus callback.
pub(crate) struct FrozenUpload {
    descriptor: Descriptor,
    base: String,
    transfer: Uuid4,
    namespace: Bytes32,
    key: Bytes32,
    policy: Policy,
    digest: Sha256Hex,
    bytes: Vec<u8>,
}
impl FrozenUpload {
    pub(crate) fn new(
        descriptor: Descriptor,
        base: String,
        transfer: &str,
        bytes: Vec<u8>,
    ) -> Result<Self> {
        descriptor.validate()?;
        // The native/browser admission owner checks the actual head, signature,
        // source, creator and key; this constructor only freezes byte bindings.
        if !base.starts_with("v1:") {
            return Err(page::Fault::Invalid.into());
        }
        values::object_id(&base[3..])?;
        let digest =
            Sha256Hex::parse(&descriptor.payload_sha256).map_err(|_| page::Fault::Invalid)?;
        if bytes.len() as u64 != values::decimal(&descriptor.payload_bytes, false)?
            || crypto::digest(&bytes) != *digest.as_bytes()
        {
            return Err(page::Fault::Invalid.into());
        }
        let input = policy(&UploadPolicy {
            version: 1,
            space: &descriptor.space,
            page: &descriptor.page,
            epoch: &descriptor.epoch,
            target: &descriptor.source,
            attachment_id: &descriptor.attachment_id,
            descriptor_hash: hex(&descriptor.hash()?),
            object_id: &descriptor.object_id,
            envelope_hash: &descriptor.envelope_hash,
            payload_sha256: &descriptor.payload_sha256,
            payload_bytes: &descriptor.payload_bytes,
            plaintext_bytes: &descriptor.plaintext_bytes,
        })?;
        Ok(Self {
            namespace: Bytes32::from_bytes(attachments::namespace(
                &descriptor.space,
                &descriptor.page,
            )?),
            key: key(&descriptor)?,
            descriptor,
            base,
            transfer: Uuid4::parse(transfer).map_err(|_| page::Fault::Invalid)?,
            policy: input,
            digest,
            bytes,
        })
    }
    pub(crate) fn begin(&self) -> Call {
        Call::Begin(BeginInput {
            transfer_id: self.transfer,
            namespace: self.namespace,
            opaque_key: self.key,
            policy: self.policy.clone(),
            payload_sha256: self.digest,
            payload_bytes: self.bytes.len() as u64,
        })
    }
    pub(crate) fn status(&self) -> Call {
        // Recovery resends the frozen bytes, not a serialization of a new draft.
        Call::Status(StatusInput {
            transfer_id: self.transfer,
            namespace: self.namespace,
            policy: self.policy.clone(),
        })
    }
    pub(crate) fn part(&self, index: u32) -> Result<Call> {
        let start = usize::try_from(index)?
            .checked_mul(tmt_extension_objects::limits::CHUNK_BYTES)
            .ok_or(page::Fault::Invalid)?;
        let bytes = self
            .bytes
            .get(start..)
            .filter(|b| !b.is_empty())
            .ok_or(page::Fault::Invalid)?;
        Ok(Call::Part(PartInput {
            transfer_id: self.transfer,
            index,
            bytes: Chunk::new(
                bytes[..bytes.len().min(tmt_extension_objects::limits::CHUNK_BYTES)].to_vec(),
            )
            .map_err(|_| page::Fault::Invalid)?,
        }))
    }
    pub(crate) fn commit(&self) -> Call {
        Call::Commit(TransferInput {
            transfer_id: self.transfer,
        })
    }
    pub(crate) fn discard(&self) -> Call {
        Call::Discard(TransferInput {
            transfer_id: self.transfer,
        })
    }
    pub(crate) fn committed(&self, outcome: &Outcome) -> bool {
        matches!(outcome, Outcome::Success(Success::Committed { opaque_key, payload_sha256, payload_bytes })
            if *opaque_key == self.key && *payload_sha256 == self.digest && *payload_bytes == self.bytes.len() as u64)
    }
    /// Verification reads before publication bind the same retained upload, not
    /// a reference that does not exist yet. The callback owner still requires
    /// the actual OwnerSession and fresh Colab write authority.
    pub(crate) fn verifier(
        &self,
        client: Client,
        origin: Origin,
        owner: Arc<dyn CallbackOwner>,
    ) -> Result<super::CommittedReader> {
        if !matches!(origin, Origin::Mounted(_)) {
            return Err(page::Fault::Denied.into());
        }
        Ok(super::CommittedReader {
            client,
            origin,
            owner,
            namespace: *self.namespace.as_bytes(),
            key: *self.key.as_bytes(),
            policy: self.policy.clone(),
            digest: *self.digest.as_bytes(),
            bytes: self.bytes.len() as u64,
        })
    }
    pub(crate) fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    pub(crate) fn base(&self) -> &str {
        &self.base
    }
}
