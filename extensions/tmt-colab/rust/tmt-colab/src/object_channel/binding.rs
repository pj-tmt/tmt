//! Colab policy bytes stay here; the neutral leaf and Remote never parse them.
#[cfg(test)]
use super::{CallbackOwner, Client};
use crate::{Result, attachments, page};
use serde::Serialize;
#[cfg(test)]
use std::sync::Arc;
use tmt_colab_model::{
    attachment::{AttachmentSelector, Descriptor, Source},
    values,
};
use tmt_extension_objects::{
    AdmitInput, BeginInput, Bytes32, Call, Chunk, PartAdmit, PartInput, Policy, Retained,
    Sha256Hex, StatusInput, TransferAdmit, TransferInput, Uuid4,
};

#[cfg(test)]
use tmt_extension_objects::{Origin, Outcome, Success};

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
    payload_bytes: u64,
    #[cfg(test)]
    bytes: Vec<u8>,
}
impl FrozenUpload {
    #[cfg(test)]
    pub(crate) fn new(
        descriptor: Descriptor,
        base: String,
        transfer: &str,
        bytes: Vec<u8>,
    ) -> Result<Self> {
        let mut frozen = Self::metadata(descriptor, base, transfer)?;
        if bytes.len() as u64 != frozen.payload_bytes
            || tmt_colab_model::crypto::digest(&bytes) != *frozen.digest.as_bytes()
        {
            return Err(page::Fault::Invalid.into());
        }
        frozen.bytes = bytes;
        Ok(frozen)
    }
    /// Metadata capture allocates no backend row and grants no authority. The
    /// backend checks streamed ciphertext; publication independently reads and
    /// authenticates the complete committed payload before exposing a reference.
    pub(crate) fn metadata(descriptor: Descriptor, base: String, transfer: &str) -> Result<Self> {
        descriptor.validate()?;
        if !base.starts_with("v1:") {
            return Err(page::Fault::Invalid.into());
        }
        values::object_id(&base[3..])?;
        let payload_bytes = values::decimal(&descriptor.payload_bytes, false)?;
        let digest =
            Sha256Hex::parse(&descriptor.payload_sha256).map_err(|_| page::Fault::Invalid)?;
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
            payload_bytes,
            #[cfg(test)]
            bytes: Vec::new(),
        })
    }
    pub(crate) fn begin(&self) -> Call {
        Call::Begin(BeginInput {
            transfer_id: self.transfer,
            namespace: self.namespace,
            opaque_key: self.key,
            policy: self.policy.clone(),
            payload_sha256: self.digest,
            payload_bytes: self.payload_bytes,
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
    #[cfg(test)]
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
    pub(crate) fn streamed_part(&self, index: u32, bytes: Vec<u8>) -> Result<Call> {
        if bytes.is_empty() {
            return Err(page::Fault::Invalid.into());
        }
        Ok(Call::Part(PartInput {
            transfer_id: self.transfer,
            index,
            bytes: Chunk::new(bytes).map_err(|_| page::Fault::Invalid)?,
        }))
    }
    /// The callback must agree with the exact original, including part size.
    /// Remote's retained ledger is checked independently by the neutral carrier.
    pub(crate) fn admission_input(&self, call: &Call) -> Result<AdmitInput> {
        let retained = Retained {
            namespace: self.namespace,
            opaque_key: self.key,
            policy: self.policy.clone(),
            payload_sha256: self.digest,
            payload_bytes: self.payload_bytes,
        };
        match call {
            Call::Begin(input) if *call == self.begin() => Ok(AdmitInput::Begin(input.clone())),
            Call::Status(input) if *call == self.status() => Ok(AdmitInput::Status(input.clone())),
            Call::Part(input) if input.transfer_id == self.transfer => {
                Ok(AdmitInput::Part(PartAdmit {
                    transfer_id: self.transfer,
                    index: input.index,
                    length: u32::try_from(input.bytes.as_bytes().len())?,
                    retained,
                }))
            }
            Call::Commit(input) if input.transfer_id == self.transfer => {
                Ok(AdmitInput::Commit(TransferAdmit {
                    transfer_id: self.transfer,
                    retained,
                }))
            }
            Call::Discard(input) if input.transfer_id == self.transfer => {
                Ok(AdmitInput::Discard(TransferAdmit {
                    transfer_id: self.transfer,
                    retained,
                }))
            }
            _ => Err(page::Fault::Invalid.into()),
        }
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
    #[cfg(test)]
    pub(crate) fn committed(&self, outcome: &Outcome) -> bool {
        matches!(outcome, Outcome::Success(Success::Committed { opaque_key, payload_sha256, payload_bytes })
            if *opaque_key == self.key && *payload_sha256 == self.digest && *payload_bytes == self.payload_bytes)
    }
    /// Verification reads before publication bind the same retained upload, not
    /// a reference that does not exist yet. The callback owner still requires
    /// the actual OwnerSession and fresh Colab write authority.
    #[cfg(test)]
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
            bytes: self.payload_bytes,
        })
    }
    pub(crate) fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    pub(crate) fn base(&self) -> &str {
        &self.base
    }
}
