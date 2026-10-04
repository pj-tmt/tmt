//! Prepare outside the writer lock; commit against the rechecked snapshot.
use crate::{
    Result,
    decoder::{BaselineInput, Decoder},
    fold::{self, Authority, BaselineBody, Snapshot},
    keyring::Keyring,
    store::owner::{OwnerFault, OwnerTransaction},
};
use std::collections::BTreeMap;
use tmt_colab_model::{certificate, crypto, object, statement, values, wrap};

pub(super) struct Prepared {
    pub snapshot: Snapshot,
    page: String,
    epoch: u64,
    secret: [u8; 32],
    descriptor: serde_json::Value,
    object: object::Envelope,
}
impl Drop for Prepared {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}
impl Prepared {
    pub fn new(
        snapshot: Snapshot,
        key: &Keyring,
        page: &str,
        revision: u64,
        decoder: &mut Decoder,
    ) -> Result<Self> {
        let view = snapshot.materialize(key, page, decoder)?;
        let baseline = decoder.produce_baseline(
            BaselineInput {
                source: view.source.as_bytes(),
                title: &view.title,
                publisher_agent: view.publisher_agent.as_deref(),
                source_digest: crypto::digest(view.source.as_bytes()),
            },
            None,
        )?;
        let epoch = snapshot.epoch.checked_add(1).ok_or(OwnerFault::Capacity)?;
        let body = serde_json::to_vec(&BaselineBody {
            source: view.source,
            update: values::encode_binary(&baseline.update),
        })?;
        let mut secret = [0; 32];
        let result = (|| {
            getrandom::fill(&mut secret)?;
            let object = key.seal_baseline(
                &fold::baseline_context(key, page, epoch, revision)?,
                &secret,
                &body,
            )?;
            let descriptor = serde_json::json!({"pageId":page,"epoch":epoch.to_string(),
                "sourceDigest":values::encode_binary(&baseline.source_digest),"baselineCommitment":values::encode_binary(&baseline.commitment),
                "title":view.title,"objectEnvelopeHash":values::encode_binary(&object.hash()?),"membershipRevision":revision.to_string()});
            Ok(Self {
                snapshot,
                page: page.into(),
                epoch,
                secret,
                descriptor,
                object,
            })
        })();
        secret.fill(0);
        result
    }
    pub fn recheck(&self, tx: &OwnerTransaction<'_>) -> Result<()> {
        recheck(tx, &self.snapshot, &self.page)
    }
    pub fn commit(
        &self,
        tx: &mut OwnerTransaction<'_>,
        key: &Keyring,
        authority: &Authority,
        now: u64,
    ) -> Result<(Vec<statement::Envelope>, Vec<wrap::Envelope>)> {
        let revision = tx
            .head()
            .ok_or(OwnerFault::Invalid)?
            .revision
            .checked_add(1)
            .ok_or(OwnerFault::Capacity)?;
        if values::decimal(
            self.descriptor["membershipRevision"]
                .as_str()
                .ok_or(OwnerFault::Invalid)?,
            false,
        )? != revision
        {
            return Err(OwnerFault::StaleHead.into());
        }
        tx.pin_cuts(&self.snapshot.cuts)?;
        let targets = targets(tx, &self.snapshot.devices, key, authority, &self.page, now)?;
        if targets.len() > 512 {
            return Err(OwnerFault::Capacity.into());
        }
        let mut wraps = Vec::new();
        for ((kind, id), recipient_key) in targets {
            wraps.push(key.seal_wrap(
                &wrap::Header {
                    space: key.space_id.clone(),
                    page: self.page.clone(),
                    epoch: self.epoch.to_string(),
                    recipient_kind: kind,
                    recipient_id: id,
                    recipient_key,
                    signer_key: key.owner_public(),
                    membership_revision: revision.to_string(),
                },
                &self.secret,
            )?);
        }
        let cuts = self
            .snapshot
            .cuts
            .iter()
            .map(|c| c.payload())
            .collect::<Result<Vec<_>>>()?;
        let payload = serde_json::to_vec(
            &serde_json::json!({"pageId":self.page,"epoch":self.epoch.to_string(),
            "cuts":cuts,"baseline":self.descriptor,"wraps":wraps}),
        )?;
        if payload.len() > tmt_colab_model::payload::MAX_BYTES {
            return Err(OwnerFault::Capacity.into());
        }
        let statement = key.sign_statement(tx.head(), "epoch.advance", &payload)?;
        tx.append_statement(&statement)?;
        tx.put_epoch_secret(&self.page, self.epoch, &self.secret)?;
        tx.put_baseline(&serde_json::to_vec(&self.descriptor)?, &self.object)?;
        for wrapped in &wraps {
            tx.put_wrap(wrapped)?;
        }
        tx.advance_epoch(&self.page, self.snapshot.epoch)?;
        let mut statements = vec![statement];
        if authority.policy.public_mode {
            let payload = super::sharing::share_payload(
                tx,
                &self.page,
                super::ShareMode::Public,
                authority.policy.history_current,
            )?;
            let published =
                key.sign_statement(tx.head(), "page.share", &serde_json::to_vec(&payload)?)?;
            tx.append_statement(&published)?;
            statements.push(published);
        }
        Ok((statements, wraps))
    }
}
pub(super) fn recheck(tx: &OwnerTransaction<'_>, snapshot: &Snapshot, page: &str) -> Result<()> {
    if tx.head() != Some(&snapshot.authority.head)
        || tx.current_epoch(page)? != snapshot.epoch
        || tx.cuts(page, snapshot.epoch)? != snapshot.cuts
        || serde_json::to_vec(&tx.devices()?)? != serde_json::to_vec(&snapshot.devices)?
    {
        return Err(OwnerFault::StaleHead.into());
    }
    Ok(())
}
pub(super) fn targets(
    tx: &OwnerTransaction<'_>,
    devices: &[crate::store::owner::Device],
    key: &Keyring,
    authority: &Authority,
    page: &str,
    now: u64,
) -> Result<BTreeMap<(String, String), [u8; 32]>> {
    let mut targets = BTreeMap::new();
    for issuer in authority
        .recipients
        .values()
        .filter(|i| fold::eligible(&i.recipient, authority, page))
    {
        let r = &issuer.recipient;
        targets.insert((r.kind.clone(), r.id.clone()), r.encryption_key);
    }
    for device in devices {
        if device.revoked {
            continue;
        }
        let chain = certificate::Chain::from_json(&device.chain)?;
        let cert = chain.certificate()?;
        if authority.revoked_devices.contains(cert.device_id) {
            continue;
        }
        let Some(issuer) = authority
            .recipients
            .get(&(cert.issuer_kind.into(), cert.issuer_id.into()))
        else {
            continue;
        };
        if !fold::eligible(&issuer.recipient, authority, page)
            || now < cert.issued_at
            || now >= cert.expires_at
        {
            continue;
        }
        fold::verify_chain(&chain, issuer, key, authority.head.revision)?;
        if tx.registration(cert.device_id)?.is_some_and(|r| r.revoked) {
            continue;
        }
        targets.insert(
            ("device".into(), cert.device_id.into()),
            *cert.encryption_key,
        );
    }
    Ok(targets)
}
