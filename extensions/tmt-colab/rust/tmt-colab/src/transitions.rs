//! Root-local owner transitions; request/socket composition remains separate.
mod epoch;
mod links;
mod membership;
use crate::{
    Result,
    decoder::Decoder,
    fold::Snapshot,
    keyring::Keyring,
    store::{
        Store,
        owner::{Mutation, OwnerFault},
    },
};
pub use links::{LinkAction, LinkRequest, LinkSpec};
pub use membership::{DeviceRevoke, MemberAction, MemberRequest};
use std::{collections::BTreeMap, path::PathBuf};
use tmt_colab_model::{crypto, framing, values};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Invalid,
    Denied,
    Expired,
    Conflict,
    StaleHead,
    Capacity,
    Unavailable,
}
impl Code {
    pub fn text(self) -> &'static str {
        match self {
            Self::Invalid => "INVALID",
            Self::Denied => "DENIED",
            Self::Expired => "EXPIRED",
            Self::Conflict => "CONFLICT",
            Self::StaleHead => "STALE_HEAD",
            Self::Capacity => "CAPACITY",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}
#[derive(Debug)]
pub struct TransitionError {
    pub code: Code,
    cause: Box<dyn std::error::Error + Send + Sync>,
}
impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code.text())
    }
}
impl std::error::Error for TransitionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}
impl TransitionError {
    fn from_error(cause: Box<dyn std::error::Error + Send + Sync>) -> Self {
        let code = if let Some(f) = cause.downcast_ref::<OwnerFault>() {
            match f {
                OwnerFault::Conflict => Code::Conflict,
                OwnerFault::StaleHead => Code::StaleHead,
                OwnerFault::WrongOwner => Code::Denied,
                OwnerFault::Invalid => Code::Invalid,
                OwnerFault::Capacity => Code::Capacity,
            }
        } else if let Some(f) = cause.downcast_ref::<crate::store::Fault>() {
            match f {
                crate::store::Fault::Capacity => Code::Capacity,
                crate::store::Fault::StaleEpoch => Code::StaleHead,
                crate::store::Fault::Invalid => Code::Invalid,
                crate::store::Fault::Conflict => Code::Conflict,
                _ => Code::Unavailable,
            }
        } else if let Some(f) = cause.downcast_ref::<crate::decoder::DecodeFault>() {
            match f {
                crate::decoder::DecodeFault::InvalidInput
                | crate::decoder::DecodeFault::Rejected => Code::Invalid,
                crate::decoder::DecodeFault::Denied => Code::Denied,
                _ => Code::Unavailable,
            }
        } else if cause.is::<tmt_colab_model::Invalid>() {
            Code::Invalid
        } else {
            Code::Unavailable
        };
        Self { code, cause }
    }
}
/// Root-authorized local request, never an unsigned browser transport DTO.
pub struct EpochAdvance<'a> {
    pub operation_id: &'a str,
    pub expected_revision: u64,
    pub page: &'a str,
}
pub struct Engine {
    program: PathBuf,
    decoders: BTreeMap<String, Decoder>,
}
impl Engine {
    pub fn new(program: PathBuf) -> Result<Self> {
        Decoder::new(program.clone())?;
        Ok(Self {
            program,
            decoders: BTreeMap::new(),
        })
    }
    pub fn advance_epoch(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: EpochAdvance<'_>,
        now: u64,
    ) -> std::result::Result<Vec<u8>, TransitionError> {
        self.advance(store, key, request, now)
            .map_err(TransitionError::from_error)
    }
    fn advance(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: EpochAdvance<'_>,
        now: u64,
    ) -> Result<Vec<u8>> {
        values::generated_id(request.operation_id)?;
        values::generated_id(request.page)?;
        values::time(now)?;
        if request.expected_revision == 0 {
            return Err(OwnerFault::StaleHead.into());
        }
        let digest = crypto::digest(&framing::frame(&[
            b"tmt-colab-local-epoch-request-v1",
            key.space_id.as_bytes(),
            request.operation_id.as_bytes(),
            request.expected_revision.to_string().as_bytes(),
            request.page.as_bytes(),
        ])?);
        // Read-only replay fast path avoids creating new baseline identities. The
        // authoritative writer receipt check still owns conflicting/racing attempts.
        if let Some(saved) = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
            tx.saved_operation(request.operation_id, &digest)
        })? {
            return Ok(saved);
        }
        if !self.decoders.contains_key(request.page) {
            self.decoders
                .insert(request.page.into(), Decoder::new(self.program.clone())?);
        }
        let decoder = self
            .decoders
            .get_mut(request.page)
            .ok_or(OwnerFault::Invalid)?;
        for _ in 0..3 {
            let snapshot = Snapshot::capture(store, key, request.page)?;
            if snapshot.authority.head.revision != request.expected_revision {
                return Err(OwnerFault::StaleHead.into());
            }
            let prepared = epoch::Prepared::new(
                snapshot,
                key,
                request.page,
                request
                    .expected_revision
                    .checked_add(1)
                    .ok_or(OwnerFault::Capacity)?,
                decoder,
            )?;
            let committed = store.owner_transaction(&key.space_id,&key.owner_public(),Mutation {
                operation_id:request.operation_id,digest,expected_revision:request.expected_revision,
            },|tx| {
                prepared.recheck(tx)?;
                let (statement, wraps) = prepared.commit(tx, key, &prepared.snapshot.authority, now)?;
                Ok(serde_json::to_vec(&serde_json::json!({"statements":[serde_json::from_slice::<serde_json::Value>(&statement.to_json()?)?],
                    "wrapLists":[wraps]}))?)
            });
            match committed {
                Err(error)
                    if error.downcast_ref::<OwnerFault>() == Some(&OwnerFault::StaleHead) =>
                {
                    continue;
                }
                other => return other,
            }
        }
        Err(OwnerFault::StaleHead.into())
    }
}
