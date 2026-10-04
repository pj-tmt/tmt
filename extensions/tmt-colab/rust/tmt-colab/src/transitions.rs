//! Root-local owner transitions; request/socket composition remains separate.
mod create;
mod epoch;
mod links;
mod membership;
mod request;
mod sharing;
use crate::{
    Result,
    decoder::{Config as DecoderConfig, Decoder},
    fold::Snapshot,
    keyring::Keyring,
    store::{
        Store,
        owner::{Mutation, OwnerFault, OwnerTransaction},
    },
};
pub use links::{LinkAction, LinkRequest, LinkSpec};
pub use membership::{DeviceRevoke, MemberAction, MemberRequest};
pub use request::{
    Applied, HistoryMode, OwnerAction, OwnerRequest, Publication, RequestScope, ShareMode,
};
use std::{collections::BTreeMap, path::PathBuf};
use tmt_colab_model::values;

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
                OwnerFault::Capacity | OwnerFault::PageCapacity(_) => Code::Capacity,
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
        } else if let Some(f) = cause.downcast_ref::<crate::page::Fault>() {
            match f {
                crate::page::Fault::Denied => Code::Denied,
                crate::page::Fault::Capacity => Code::Capacity,
                crate::page::Fault::Invalid => Code::Invalid,
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
/// Revision-one initialization shared by registration and atomic page creation.
pub(crate) fn initialize_owner(
    tx: &mut OwnerTransaction<'_>,
    key: &Keyring,
) -> Result<Option<tmt_colab_model::statement::Envelope>> {
    if tx.head().is_some() {
        return Ok(None);
    }
    let (recipient, payload) = owner_genesis(key)?;
    let initial = key.sign_statement(None, "member.add", &payload)?;
    tx.append_statement(&initial)?;
    tx.put_recipient(&recipient)?;
    Ok(Some(initial))
}
pub(crate) fn owner_genesis(key: &Keyring) -> Result<(crate::store::owner::Recipient, Vec<u8>)> {
    let member = key.management_member()?;
    let recipient = crate::store::owner::Recipient {
        kind: "member".into(),
        id: member.id,
        role: Some("editor".into()),
        signing_key: member.signing_key,
        encryption_key: member.encryption_key,
        pages: vec![],
        revoked: false,
    };
    let payload = serde_json::to_vec(&serde_json::json!({
        "memberId":recipient.id,"role":"editor",
        "signKey":values::encode_binary(&recipient.signing_key),
        "encKey":values::encode_binary(&recipient.encryption_key),"pages":[]
    }))?;
    Ok((recipient, payload))
}
/// Root-authorized local request, never an unsigned browser transport DTO.
pub struct EpochAdvance<'a> {
    pub operation_id: &'a str,
    pub expected_revision: u64,
    pub page: &'a str,
}
pub struct Engine {
    decoder_config: DecoderConfig,
    decoders: BTreeMap<String, Decoder>,
}
impl Engine {
    pub fn new(program: PathBuf) -> Result<Self> {
        Self::with_decoder_config(DecoderConfig::new(program))
    }
    pub fn with_decoder_config(decoder_config: DecoderConfig) -> Result<Self> {
        Decoder::with_config(decoder_config.clone())?;
        Ok(Self {
            decoder_config,
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
        self.apply(
            store,
            key,
            OwnerRequest {
                operation_id: request.operation_id,
                expected_revision: request.expected_revision,
                action: OwnerAction::EpochAdvance { page: request.page },
                transport_digest: None,
                scope: None,
            },
            now,
        )
        .map(|applied| applied.outcome)
    }
    fn advance(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: EpochAdvance<'_>,
        now: u64,
        context: request::OwnerContext<'_>,
    ) -> Result<(Vec<u8>, bool)> {
        values::generated_id(request.operation_id)?;
        values::generated_id(request.page)?;
        values::time(now)?;
        if request.expected_revision == 0 {
            return Err(OwnerFault::StaleHead.into());
        }
        let digest = context.digest(
            key,
            "epoch.advance",
            &serde_json::json!({"pageId":request.page}),
        )?;
        self.run_transition(
            store,
            key,
            Mutation {
                operation_id: request.operation_id,
                digest,
                expected_revision: request.expected_revision,
            },
            |engine, store| {
                let snapshot = Snapshot::capture(store, key, request.page)?;
                context.check_scope(&[request.page.into()])?;
                if snapshot.authority.head.revision != request.expected_revision {
                    return Err(OwnerFault::StaleHead.into());
                }
                let decoder = engine.decoder(request.page)?;
                Ok(Some(epoch::Prepared::new(
                    snapshot,
                    key,
                    request.page,
                    request
                        .expected_revision
                        .checked_add(1)
                        .ok_or(OwnerFault::Capacity)?,
                    decoder,
                )?))
            },
            |tx, prepared| {
                prepared.recheck(tx)?;
                context.check_scope(&[request.page.into()])?;
                let (statements, wraps) =
                    prepared.commit(tx, key, &prepared.snapshot.authority, now)?;
                membership::outcome(&statements, wraps)
            },
        )
    }
    fn decoder(&mut self, page: &str) -> Result<&mut Decoder> {
        if !self.decoders.contains_key(page) {
            self.decoders.insert(
                page.into(),
                Decoder::with_config(self.decoder_config.clone())?,
            );
        }
        self.decoders
            .get_mut(page)
            .ok_or_else(|| OwnerFault::Invalid.into())
    }
    /// One receipt and writer lifecycle for root-local transitions. Preparation
    /// never holds the writer reservation; every commit rechecks its snapshots.
    fn run_transition<P>(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        mutation: Mutation<'_>,
        mut prepare: impl FnMut(&mut Self, &Store) -> Result<Option<P>>,
        mut commit: impl FnMut(&mut OwnerTransaction<'_>, &P) -> Result<Vec<u8>>,
    ) -> Result<(Vec<u8>, bool)> {
        if let Some(saved) = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
            tx.saved_operation(mutation.operation_id, &mutation.digest)
        })? {
            return Ok((saved, false));
        }
        for _ in 0..3 {
            let Some(prepared) = prepare(self, store)? else {
                return Ok((membership::outcome(&[], vec![])?, false));
            };
            let mut changed = false;
            let result = store.owner_transaction(
                &key.space_id,
                &key.owner_public(),
                Mutation {
                    operation_id: mutation.operation_id,
                    digest: mutation.digest,
                    expected_revision: mutation.expected_revision,
                },
                |tx| {
                    let outcome = commit(tx, &prepared)?;
                    changed = true;
                    Ok(outcome)
                },
            );
            match result {
                Err(e) if e.downcast_ref::<OwnerFault>() == Some(&OwnerFault::StaleHead) => {
                    continue;
                }
                other => return other.map(|outcome| (outcome, changed)),
            }
        }
        Err(OwnerFault::StaleHead.into())
    }
}
