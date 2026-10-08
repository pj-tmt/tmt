//! Complete committed ciphertext reads. Remote's exact digest/length checks and
//! the final local digest are independent of Colab asset authentication.
use super::{CallbackOwner, Client};
use crate::{Result, attachments::CommittedObjectVerifier, page};
use std::{sync::Arc, time::Instant};
use tmt_colab_model::crypto;
use tmt_extension_objects::{
    Bytes32, Call, Origin, Outcome, Policy, ReadInput, Sha256Hex, Success, limits,
};

pub(crate) struct CommittedReader {
    pub client: Client,
    pub origin: Origin,
    pub owner: Arc<dyn CallbackOwner>,
    pub namespace: [u8; 32],
    pub key: [u8; 32],
    pub policy: Policy,
    pub digest: [u8; 32],
    pub bytes: u64,
}
impl CommittedObjectVerifier for CommittedReader {
    fn read_committed(
        &self,
        namespace: &[u8; 32],
        key: &[u8; 32],
        deadline: Instant,
    ) -> Result<Vec<u8>> {
        if namespace != &self.namespace
            || key != &self.key
            || self.bytes == 0
            || self.bytes > limits::PAYLOAD_BYTES
        {
            return Err(page::Fault::Invalid.into());
        }
        let mut raw = Vec::with_capacity(usize::try_from(self.bytes)?);
        while (raw.len() as u64) < self.bytes {
            let offset = raw.len() as u64;
            let count = (self.bytes - offset).min(limits::CHUNK_BYTES as u64) as u32;
            let result = self.client.request(
                self.origin,
                Call::Read(ReadInput {
                    namespace: Bytes32::from_bytes(self.namespace),
                    opaque_key: Bytes32::from_bytes(self.key),
                    policy: self.policy.clone(),
                    payload_sha256: Sha256Hex::from_bytes(self.digest),
                    payload_bytes: self.bytes,
                    offset,
                    count,
                }),
                Arc::clone(&self.owner),
                deadline,
            )?;
            let Outcome::Success(Success::Read {
                offset: returned,
                total_bytes,
                bytes,
            }) = result
            else {
                return Err(page::Fault::Unavailable.into());
            };
            if returned != offset
                || total_bytes != self.bytes
                || bytes.as_bytes().len() != count as usize
            {
                return Err(page::Fault::Invalid.into());
            }
            raw.extend_from_slice(bytes.as_bytes());
        }
        if Instant::now() >= deadline
            || !self.client.standing(self.origin)
            || crypto::digest(&raw) != self.digest
        {
            return Err(page::Fault::Unavailable.into());
        }
        Ok(raw)
    }
}
