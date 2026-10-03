//! Owner-local epoch advancement; request/socket composition remains separate.
use crate::{
    Result,
    decoder::{BaselineInput, Decoder},
    fold::{self, BaselineBody, Snapshot},
    keyring::Keyring,
    store::{
        Store,
        owner::{Mutation, OwnerFault},
    },
};
use std::{collections::BTreeMap, path::PathBuf};
use tmt_colab_model::{certificate, crypto, framing, values, wrap};

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
            let view = snapshot.materialize(key, request.page, decoder)?;
            let baseline = decoder.produce_baseline(
                BaselineInput {
                    source: view.source.as_bytes(),
                    title: &view.title,
                    source_digest: crypto::digest(view.source.as_bytes()),
                },
                None,
            )?;
            let epoch = snapshot.epoch.checked_add(1).ok_or(OwnerFault::Capacity)?;
            let revision = request
                .expected_revision
                .checked_add(1)
                .ok_or(OwnerFault::Capacity)?;
            let mut secret = [0; 32];
            getrandom::fill(&mut secret)?;
            let body = serde_json::to_vec(&BaselineBody {
                source: view.source,
                update: values::encode_binary(&baseline.update),
            })?;
            let object = key.seal_baseline(
                &fold::baseline_context(key, request.page, epoch, revision)?,
                &secret,
                &body,
            )?;
            let descriptor = serde_json::json!({"pageId":request.page,"epoch":epoch.to_string(),
                "sourceDigest":values::encode_binary(&baseline.source_digest),"baselineCommitment":values::encode_binary(&baseline.commitment),
                "title":view.title,"objectEnvelopeHash":values::encode_binary(&object.hash()?),"membershipRevision":revision.to_string()});
            let committed = store.owner_transaction(&key.space_id,&key.owner_public(),Mutation {
                operation_id:request.operation_id,digest,expected_revision:request.expected_revision,
            },|tx| {
                if tx.head() != Some(&snapshot.authority.head) || tx.current_epoch(request.page)? != snapshot.epoch
                    || tx.cuts(request.page,snapshot.epoch)? != snapshot.cuts
                    || serde_json::to_vec(&tx.devices()?)? != serde_json::to_vec(&snapshot.devices)? {
                    return Err(OwnerFault::StaleHead.into());
                }
                tx.pin_cuts(&snapshot.cuts)?;
                let mut targets = BTreeMap::new();
                for issuer in snapshot.authority.recipients.values().filter(|i| fold::eligible(&i.recipient,&snapshot.authority,request.page)) {
                    let r = &issuer.recipient;
                    targets.insert((r.kind.clone(),r.id.clone()),r.encryption_key);
                }
                for device in &snapshot.devices {
                    if device.revoked {continue;}
                    let chain = certificate::Chain::from_json(&device.chain)?;
                    let cert = chain.certificate()?;
                    if snapshot.authority.revoked_devices.contains(cert.device_id) {continue;}
                    let Some(issuer) = snapshot.authority.recipients.get(&(cert.issuer_kind.into(),cert.issuer_id.into())) else {continue;};
                    if !fold::eligible(&issuer.recipient,&snapshot.authority,request.page)
                        || now < cert.issued_at || now >= cert.expires_at {continue;}
                    fold::verify_chain(&chain,issuer,key,request.expected_revision)?;
                    if tx.registration(cert.device_id)?.is_some_and(|r| r.revoked) {continue;}
                    targets.insert(("device".into(),cert.device_id.into()),*cert.encryption_key);
                }
                if targets.len() > 512 {return Err(OwnerFault::Capacity.into());}
                let mut wraps = Vec::new();
                for ((kind,id),recipient_key) in targets {
                    wraps.push(key.seal_wrap(&wrap::Header {space:key.space_id.clone(),page:request.page.into(),epoch:epoch.to_string(),
                        recipient_kind:kind,recipient_id:id,recipient_key,signer_key:key.owner_public(),membership_revision:revision.to_string()},&secret)?);
                }
                let cuts = snapshot.cuts.iter().map(|c| c.payload()).collect::<Result<Vec<_>>>()?;
                let payload = serde_json::to_vec(&serde_json::json!({"pageId":request.page,"epoch":epoch.to_string(),
                    "cuts":cuts,"baseline":descriptor,"wraps":wraps}))?;
                if payload.len() > tmt_colab_model::payload::MAX_BYTES {return Err(OwnerFault::Capacity.into());}
                let statement = key.sign_statement(tx.head(),"epoch.advance",&payload)?;
                tx.append_statement(&statement)?;
                tx.put_epoch_secret(request.page,epoch,&secret)?;
                tx.put_baseline(&serde_json::to_vec(&descriptor)?,&object)?;
                for wrapped in &wraps {tx.put_wrap(wrapped)?;}
                tx.advance_epoch(request.page,snapshot.epoch)?;
                // Large baseline ciphertext is fetched separately by scoped hash;
                // the exact statement/wrap bytes are the durable mutation outcome.
                Ok(serde_json::to_vec(&serde_json::json!({"statements":[serde_json::from_slice::<serde_json::Value>(&statement.to_json()?)?],
                    "wrapLists":[wraps]}))?)
            });
            secret.fill(0);
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
