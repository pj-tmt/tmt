//! Page-scoped read capabilities. This owner never signs or grants write authority.
use crate::{
    keyring::Keyring,
    registration::Code,
    store::{
        Store,
        owner::{Device, OwnerTransaction},
    },
    sync::{SyncScope, WrapRecipients},
};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use tmt_colab_model::{certificate, crypto, framing, payload, values};

pub const CHALLENGE_PATH: &str = "/api/readers/challenge";
pub const SESSION_PATH: &str = "/api/readers/session";
const CHALLENGE_MS: u64 = 60_000;
const SESSION_MS: u64 = 600_000;
const CAP: usize = 64;
const TOKEN_DOMAIN: &[u8] = b"tmt-colab-reader-token-v1";
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum ChallengeRequest {
    Public {
        space: String,
        page: String,
    },
    Link {
        space: String,
        page: String,
        chain: String,
    },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum ReaderExchange {
    Public {
        #[serde(rename = "challengeId")]
        id: String,
    },
    Link {
        #[serde(rename = "challengeId")]
        id: String,
        signature: String,
    },
}
struct Reader {
    scope: SyncScope,
    chain: Option<Vec<u8>>,
    expires: u64,
    phase: Phase,
}
enum Phase {
    Challenge { input: Vec<u8> },
    Ticket([u8; 32]),
    Active,
}
#[derive(Default)]
pub(crate) struct Sessions(BTreeMap<String, Reader>);
fn random<const N: usize>() -> Result<[u8; N], Code> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| Code::Unavailable)?;
    Ok(bytes)
}
fn uuid() -> Result<String, Code> {
    let mut bytes = random::<16>()?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}
fn error(e: Box<dyn std::error::Error + Send + Sync>) -> Code {
    e.downcast_ref::<Code>()
        .copied()
        .unwrap_or(Code::Unavailable)
}
/// The carrier is isolated here; never return or echo its secret subprotocol.
pub(crate) fn upgrade_token(protocols: &[String]) -> Result<Option<[u8; 32]>, Code> {
    let tokens: Vec<_> = protocols
        .iter()
        .filter_map(|p| p.strip_prefix("colab-reader-v1."))
        .collect();
    match tokens.as_slice() {
        [] => Ok(None),
        [token] => Ok(Some(
            values::binary(token, 32)?
                .try_into()
                .map_err(|_| Code::Invalid)?,
        )),
        _ => Err(Code::Invalid),
    }
}
impl Sessions {
    pub(crate) fn challenge(
        &mut self,
        store: &Store,
        key: &Keyring,
        body: &[u8],
        now: u64,
    ) -> Result<Vec<u8>, Code> {
        values::time(now)?;
        let request: ChallengeRequest = serde_json::from_slice(body).map_err(|_| Code::Invalid)?;
        let (space, page, chain) = match request {
            ChallengeRequest::Public { space, page } => (space, page, None),
            ChallengeRequest::Link { space, page, chain } => {
                (space, page, Some(values::binary(&chain, 16 * 1024)?))
            }
        };
        values::space_id(&space)?;
        values::generated_id(&page)?;
        if space != key.space_id {
            return Err(Code::Denied);
        }
        self.0
            .retain(|_, r| r.expires > now || matches!(r.phase, Phase::Active));
        if self.0.len() >= CAP {
            return Err(Code::Capacity);
        }
        let epoch = store
            .owner_read(&space, &key.owner_public(), |tx| {
                Ok(tx.page_epoch(&page)?.ok_or(Code::Denied)?)
            })
            .map_err(error)?;
        let scope = SyncScope { space, page, epoch };
        let id = uuid()?;
        // Reader IDs cannot alias a registered writer, even if entropy is faulty.
        if self.0.contains_key(&id)
            || store
                .owner_read(&key.space_id, &key.owner_public(), |tx| {
                    Ok(tx.device(&id)?.is_some() || tx.registration(&id)?.is_some())
                })
                .map_err(error)?
        {
            return Err(Code::Unavailable);
        }
        let expires = now.checked_add(CHALLENGE_MS).ok_or(Code::Invalid)?;
        values::time(expires)?;
        let nonce = random::<32>()?;
        let digest = chain
            .as_ref()
            .map(|c| certificate::Chain::from_json(c)?.digest())
            .transpose()?
            .unwrap_or([0; 32]);
        let input = framing::frame(&[
            b"tmt-colab-reader-session-v1",
            b"1",
            id.as_bytes(),
            &nonce,
            scope.space.as_bytes(),
            scope.page.as_bytes(),
            scope.epoch.as_bytes(),
            &digest,
            expires.to_string().as_bytes(),
        ])?;
        let reader = Reader {
            scope,
            chain,
            expires,
            phase: Phase::Challenge { input },
        };
        reader.check(store, key, now)?;
        let response = serde_json::to_vec(&json!({"challengeId":id,"nonce":values::encode_binary(&nonce),"space":reader.scope.space,"page":reader.scope.page,"epoch":reader.scope.epoch,"chainDigest":values::encode_binary(&digest),"expiresAt":expires})).map_err(|_| Code::Unavailable)?;
        self.0.insert(id, reader);
        Ok(response)
    }
    pub(crate) fn exchange(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        body: &[u8],
        now: u64,
    ) -> Result<Vec<u8>, Code> {
        values::time(now)?;
        let request: ReaderExchange = serde_json::from_slice(body).map_err(|_| Code::Invalid)?;
        let (id, signature) = match request {
            ReaderExchange::Public { id } => (id, None),
            ReaderExchange::Link { id, signature } => (id, Some(values::binary(&signature, 64)?)),
        };
        values::generated_id(&id)?;
        // A failed proof consumes its challenge too; it can never mint two tickets.
        let mut reader = self.0.remove(&id).ok_or(Code::Denied)?;
        let Phase::Challenge { ref input } = reader.phase else {
            self.0.insert(id, reader);
            return Err(Code::Denied);
        };
        reader.check(store, key, now)?;
        match (&reader.chain, signature) {
            (Some(bytes), Some(signature)) => {
                let chain = certificate::Chain::from_json(bytes)?;
                crypto::verify_signature(chain.certificate()?.signing_key, input, &signature)
                    .map_err(|_| Code::Denied)?;
                store
                    .device_transaction(&key.space_id, &key.owner_public(), |tx| {
                        reader.check_tx(tx, now)?;
                        let cert = chain.certificate()?;
                        if let Some(old) = tx.device(cert.device_id)? {
                            if old.revoked
                                || certificate::Chain::from_json(&old.chain)?.digest()?
                                    != chain.digest()?
                            {
                                return Err(Code::Denied.into());
                            }
                        } else {
                            tx.put_device(&Device {
                                chain: bytes.clone(),
                                revoked: false,
                            })?;
                        }
                        Ok(Vec::new())
                    })
                    .map_err(error)?;
            }
            (None, None) => {}
            _ => return Err(Code::Denied),
        }
        let token = random::<32>()?;
        reader.phase = Phase::Ticket(crypto::digest(&token));
        reader.expires = now.checked_add(SESSION_MS).ok_or(Code::Invalid)?;
        values::time(reader.expires)?;
        let response = serde_json::to_vec(&json!({"principal":id,"token":values::encode_binary(&token),"space":reader.scope.space,"page":reader.scope.page,"epoch":reader.scope.epoch,"ownerKey":values::encode_binary(&key.owner_public()),"expiresAt":reader.expires})).map_err(|_| Code::Unavailable)?;
        self.0.insert(id, reader);
        Ok(response)
    }
    pub(crate) fn upgrade(
        &mut self,
        store: &Store,
        key: &Keyring,
        token: &[u8; 32],
        now: u64,
    ) -> Result<(String, String), Code> {
        let hash = crypto::digest(token);
        // The existing model HMAC verifier compares fixed-size confirmation tags
        // in constant time; no new crypto dependency or early-exit byte loop.
        let confirmation = crypto::mac(&hash, TOKEN_DOMAIN);
        let mut matched = None;
        for (id, r) in &self.0 {
            if let Phase::Ticket(saved) = &r.phase
                && crypto::verify_mac(saved, TOKEN_DOMAIN, &confirmation).is_ok()
            {
                matched = Some(id.clone());
            }
        }
        let id = matched.ok_or(Code::Denied)?;
        let reader = self.0.get_mut(&id).ok_or(Code::Denied)?;
        reader.check(store, key, now)?;
        reader.phase = Phase::Active;
        let device = match &reader.chain {
            Some(c) => certificate::Chain::from_json(c)?
                .certificate()?
                .device_id
                .to_owned(),
            None => id.clone(),
        };
        Ok((id, device))
    }
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }
    pub(crate) fn check(
        &self,
        id: &str,
        scope: Option<&SyncScope>,
        store: &Store,
        key: &Keyring,
        now: u64,
    ) -> Result<(), Code> {
        let reader = self.0.get(id).ok_or(Code::Denied)?;
        if !matches!(reader.phase, Phase::Active) || scope.is_some_and(|s| s != &reader.scope) {
            return Err(Code::Denied);
        }
        reader.check(store, key, now)
    }
    pub(crate) fn recipients(&self, id: &str) -> Result<WrapRecipients, Code> {
        Ok(match &self.0.get(id).ok_or(Code::Denied)?.chain {
            None => WrapRecipients::None,
            Some(c) => WrapRecipients::Link(
                certificate::Chain::from_json(c)?
                    .certificate()?
                    .issuer_id
                    .into(),
            ),
        })
    }
    /// Identity of this active Session, not a transferable read token. Current
    /// scope, policy and expiry are checked separately by check().
    pub(crate) fn attachment_context(&self, id: &str) -> crate::Result<[u8; 32]> {
        let reader = self.0.get(id).ok_or(Code::Denied)?;
        if !matches!(reader.phase, Phase::Active) {
            return Err(Code::Denied.into());
        }
        Ok(crypto::digest(&framing::frame(&[
            b"tmt-colab-attachment-reader-context-v1",
            id.as_bytes(),
            &serde_json::to_vec(&reader.scope)?,
            reader.chain.as_deref().unwrap_or(&[]),
            reader.expires.to_string().as_bytes(),
        ])?))
    }
    pub(crate) fn release(&mut self, id: &str) {
        self.0.remove(id);
    }
}
impl Reader {
    fn check(&self, store: &Store, key: &Keyring, now: u64) -> Result<(), Code> {
        store
            .owner_read(&key.space_id, &key.owner_public(), |tx| {
                self.check_tx(tx, now)?;
                Ok(())
            })
            .map_err(error)
    }
    fn check_tx(&self, tx: &OwnerTransaction<'_>, now: u64) -> crate::Result<()> {
        if now >= self.expires {
            return Err(Code::Expired.into());
        }
        let head = tx.head().ok_or(Code::Denied)?;
        let policy = tx.page_policy_at(&self.scope.page, head.revision)?;
        if policy.deleted || tx.page_epoch(&self.scope.page)?.as_deref() != Some(&self.scope.epoch)
        {
            return Err(Code::Denied.into());
        }
        let Some(bytes) = &self.chain else {
            return if policy.public_mode {
                Ok(())
            } else {
                Err(Code::Denied.into())
            };
        };
        if !(policy.link_mode || policy.public_mode) {
            return Err(Code::Denied.into());
        }
        let chain = certificate::Chain::from_json(bytes)?;
        let cert = chain.certificate()?;
        if cert.space != self.scope.space
            || cert.issuer_kind != "link"
            || now < cert.issued_at
            || now >= cert.expires_at
        {
            return Err(Code::Denied.into());
        }
        if tx.registration(cert.device_id)?.is_some_and(|r| r.revoked)
            || tx.device(cert.device_id)?.is_some_and(|d| d.revoked)
        {
            return Err(Code::Denied.into());
        }
        let link = tx.recipient("link", cert.issuer_id)?.ok_or(Code::Denied)?;
        if link.revoked || !link.pages.contains(&self.scope.page) {
            return Err(Code::Denied.into());
        }
        let rev = values::decimal(cert.membership_revision, false)?;
        let issuer = tx.statement(rev)?.ok_or(Code::Denied)?;
        let raw: serde_json::Value = serde_json::from_slice(&issuer.to_json()?)?;
        let input = values::binary(raw["statement"].as_str().ok_or(Code::Denied)?, 1024)?;
        let header = tmt_colab_model::statement::decode(&input)?;
        let Payload::LinkAdd(add) = payload::decode(
            header.operation,
            &values::binary(
                raw["payload"].as_str().ok_or(Code::Denied)?,
                payload::MAX_BYTES,
            )?,
        )?
        else {
            return Err(Code::Denied.into());
        };
        if add.link_id != cert.issuer_id
            || values::binary(&add.link_sign_key, 32)? != link.signing_key
            || values::binary(&add.link_enc_key, 32)? != link.encryption_key
        {
            return Err(Code::Denied.into());
        }
        if matches!(self.phase, Phase::Challenge { .. }) {
            chain
                .verify(&issuer.hash()?, &cert, &link.signing_key)
                .map_err(|_| Code::Denied)?;
        }
        Ok(())
    }
}
use payload::Payload;

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
