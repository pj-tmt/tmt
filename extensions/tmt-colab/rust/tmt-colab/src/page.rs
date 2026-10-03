//! Root-local source access: isolated preparation, then fenced ciphertext commit.
pub mod ipc;
use crate::{
    Result,
    decoder::{Decoder, MemoryLimit},
    fold::{self, Snapshot},
    keyring::Keyring,
    store::{
        Accepted, Envelope, Namespace, Store, StreamScope,
        owner::{Cut, Device, OwnerTransaction},
    },
};
use serde::{Deserialize, Serialize};
use tmt_colab_model::{certificate, crypto, framing, object, statement, values};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    StaleBase,
    Invalid,
    Capacity,
    Missing,
    Inactive,
    Denied,
    Unavailable,
}
impl Fault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleBase => "COLAB_STALE_BASE",
            Self::Invalid => "COLAB_INPUT_INVALID",
            Self::Capacity => "COLAB_CAPACITY",
            Self::Missing => "COLAB_STATE_MISSING",
            Self::Inactive => "COLAB_PAGE_INACTIVE",
            Self::Denied => "COLAB_DENIED",
            Self::Unavailable => "COLAB_UNAVAILABLE",
        }
    }
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::StaleBase => "Page changed since the editing base; read it again before writing.",
            Self::Invalid => "Invalid page source or input.",
            Self::Capacity => "Page source, update or decoder capacity exceeded.",
            Self::Missing => "Existing Colab state is required.",
            Self::Inactive => "Archived or deleted pages cannot be written.",
            Self::Denied => {
                "The local writer is revoked or its certificate does not match the owner."
            }
            Self::Unavailable => {
                "Serving page write unavailable; no offline fallback was attempted."
            }
        })
    }
}
impl std::error::Error for Fault {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Head {
    pub revision: String,
    pub statement_hash: String,
}
impl From<&statement::Head> for Head {
    fn from(head: &statement::Head) -> Self {
        Self {
            revision: head.revision.to_string(),
            statement_hash: hex(&head.hash),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub space_id: String,
    pub page_id: String,
    pub source: String,
    pub title: String,
    pub epoch: String,
    pub membership_head: Head,
    pub revision: String,
    pub memory_limit: MemoryLimit,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Prepared {
    version: u8,
    operation_id: String,
    pub space_id: String,
    pub page_id: String,
    pub epoch: String,
    membership_head: Head,
    base_revision: String,
    source_sha256: String,
    memory_limit: MemoryLimit,
    pub chain: String,
    pub envelope: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Receipt {
    pub space_id: String,
    pub page_id: String,
    pub epoch: String,
    pub membership_head: Head,
    pub revision: String,
    pub stream_id: String,
    pub seq: String,
    pub envelope_hash: String,
    pub source_sha256: String,
    pub memory_limit: MemoryLimit,
}
pub struct Committed {
    pub receipt: Receipt,
    pub accepted: Accepted,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn token(
    space: &str,
    page: &str,
    head: &statement::Head,
    epoch: u64,
    cuts: &[Cut],
) -> Result<String> {
    let cuts = cuts.iter().map(Cut::payload).collect::<Result<Vec<_>>>()?;
    Ok(format!(
        "v1:{}",
        hex(&crypto::digest(&framing::frame(&[
            b"tmt-colab-page-revision-v1",
            space.as_bytes(),
            page.as_bytes(),
            head.revision.to_string().as_bytes(),
            &head.hash,
            epoch.to_string().as_bytes(),
            &serde_json::to_vec(&cuts)?,
        ])?))
    ))
}
fn current_token(tx: &OwnerTransaction<'_>, key: &Keyring, page: &str) -> Result<String> {
    let epoch = tx.current_epoch(page)?;
    token(
        &key.space_id,
        page,
        tx.head().ok_or(Fault::Missing)?,
        epoch,
        &tx.cuts(page, epoch)?,
    )
}
fn snapshot(store: &Store, key: &Keyring, page: &str, writing: bool) -> Result<Snapshot> {
    values::generated_id(page)?;
    store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        let (states, _) = fold::verify_log(&tx.log()?, key, page)?;
        if states.last().is_some_and(|a| !a.policy.writable()) && writing {
            return Err(Fault::Inactive.into());
        }
        Ok(())
    })?;
    Snapshot::capture(store, key, page)
}
pub fn read(store: &Store, key: &Keyring, page: &str, decoder: &mut Decoder) -> Result<Page> {
    let s = snapshot(store, key, page, false)?;
    let view = s.materialize(key, page, decoder)?;
    Ok(Page {
        space_id: key.space_id.clone(),
        page_id: page.into(),
        source: view.source,
        title: view.title,
        epoch: s.epoch.to_string(),
        membership_head: Head::from(&s.authority.head),
        revision: token(&key.space_id, page, &s.authority.head, s.epoch, &s.cuts)?,
        memory_limit: view.memory_limit,
    })
}
pub fn prepare(
    store: &Store,
    key: &Keyring,
    page: &str,
    source: &str,
    expected: Option<&str>,
    decoder: &mut Decoder,
    now: u64,
) -> Result<Prepared> {
    if source.len() > crate::decoder::BASELINE_BYTES {
        return Err(Fault::Capacity.into());
    }
    let s = snapshot(store, key, page, true)?;
    let revision = token(&key.space_id, page, &s.authority.head, s.epoch, &s.cuts)?;
    if expected.is_some_and(|r| r != revision) {
        return Err(Fault::StaleBase.into());
    }
    let (id, _, _) = key.local_writer()?;
    if s.authority.revoked_devices.contains(&id) {
        return Err(Fault::Denied.into());
    }
    let issuer = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        Ok(tx.statement(1)?.ok_or(Fault::Missing)?.hash()?)
    })?;
    let existing = s.devices.iter().find(|d| {
        certificate::Chain::from_json(&d.chain)
            .and_then(|ch| Ok(ch.certificate()?.device_id == id))
            .unwrap_or(false)
    });
    let reusable = if let Some(device) = existing {
        if device.revoked {
            return Err(Fault::Denied.into());
        }
        let chain = certificate::Chain::from_json(&device.chain)?;
        let expiry = local_chain(&chain, key, &s.authority.head, &issuer, now)?;
        (expiry > now).then(|| device.chain.clone())
    } else {
        None
    };
    let chain = if let Some(chain) = reusable {
        chain
    } else {
        writer_chain(key, &s.authority.head, &issuer, now)?
    };
    let view = s.materialize_edit(key, page, decoder, Some(source))?;
    let head = s
        .cuts
        .iter()
        .filter(|c| c.stream == id)
        .max_by_key(|c| c.tail_seq);
    let seq = head
        .map_or(0, |c| c.tail_seq)
        .checked_add(1)
        .ok_or(Fault::Capacity)?;
    let envelope = key.seal_content(
        &object::Context {
            space: key.space_id.clone(),
            page: page.into(),
            epoch: s.epoch.to_string(),
            kind: "update".into(),
            namespace: "content".into(),
            author_device: id,
            membership_revision: s.authority.head.revision.to_string(),
            stream_seq: seq.to_string(),
            prev_hash: head.map_or([0; 32], |c| c.tail_hash),
        },
        &s.secret,
        &view.update,
    )?;
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let h = hex(&bytes);
    let operation_id = format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    );
    Ok(Prepared {
        version: 1,
        operation_id,
        space_id: key.space_id.clone(),
        page_id: page.into(),
        epoch: s.epoch.to_string(),
        membership_head: Head::from(&s.authority.head),
        base_revision: revision,
        source_sha256: hex(&crypto::digest(source.as_bytes())),
        memory_limit: view.memory_limit,
        chain: values::encode_binary(&chain),
        envelope: values::encode_binary(&envelope.to_json()?),
    })
}
pub(crate) fn writer_chain(
    key: &Keyring,
    head: &statement::Head,
    issuer: &[u8; 32],
    now: u64,
) -> Result<Vec<u8>> {
    let (id, sign, enc) = key.local_writer()?;
    let cert = certificate::Certificate {
        space: &key.space_id,
        issuer_kind: "member",
        issuer_id: &head.owner_member.id,
        device_id: &id,
        signing_key: &sign,
        encryption_key: &enc,
        membership_revision: "1",
        issued_at: now,
        expires_at: now
            .checked_add(crate::registration::CERTIFICATE_MS)
            .ok_or(Fault::Invalid)?,
    };
    Ok(serde_json::to_vec(
        &serde_json::json!({"version":1,"issuerStatement":values::encode_binary(issuer),
            "deviceCertificate":values::encode_binary(&certificate::input(&cert)?),
            "issuerSignature":values::encode_binary(&key.sign_device_certificate(&cert)?)}),
    )?)
}
fn local_chain(
    chain: &certificate::Chain,
    key: &Keyring,
    head: &statement::Head,
    issuer: &[u8; 32],
    now: u64,
) -> Result<u64> {
    let cert = chain.certificate()?;
    let (id, sign, enc) = key.local_writer()?;
    if cert.space != key.space_id
        || cert.issuer_kind != "member"
        || cert.issuer_id != head.owner_member.id
        || cert.device_id != id
        || cert.signing_key != &sign
        || cert.encryption_key != &enc
        || cert.membership_revision != "1"
        || cert.issued_at > now
    {
        return Err(Fault::Denied.into());
    }
    chain.verify(issuer, &cert, &head.owner_member.signing_key)?;
    Ok(cert.expires_at)
}
pub fn commit(
    store: &mut Store,
    key: &Keyring,
    prepared: &Prepared,
    now: u64,
) -> Result<Committed> {
    let p = prepared;
    if p.version != 1 || p.space_id != key.space_id {
        return Err(Fault::Invalid.into());
    }
    values::generated_id(&p.page_id)?;
    values::generated_id(&p.operation_id)?;
    let epoch = values::decimal(&p.epoch, false)?;
    let envelope =
        object::Envelope::from_json(&values::binary(&p.envelope, crate::limits::UPDATE_BYTES)?)?;
    let chain_bytes = values::binary(&p.chain, 16 * 1024)?;
    let chain = certificate::Chain::from_json(&chain_bytes)?;
    let header = object::Header::decode(envelope.header())?;
    let c = &header.context;
    if c.space != p.space_id
        || c.page != p.page_id
        || c.epoch != p.epoch
        || c.namespace != "content"
        || c.kind != "update"
        || c.membership_revision != p.membership_head.revision
    {
        return Err(Fault::Invalid.into());
    }
    let seq = values::decimal(&c.stream_seq, false)?;
    let hash = envelope.hash()?;
    let bytes = envelope.to_json()?;
    let digest = crypto::digest(&framing::frame(&[
        b"tmt-colab-page-write-v1",
        &serde_json::to_vec(p)?,
    ])?);
    let mut accepted = Accepted::Replay;
    let outcome = store.device_transaction(&key.space_id, &key.owner_public(), |tx| {
        let head = tx.head().ok_or(Fault::Missing)?;
        let issuer = tx.statement(1)?.ok_or(Fault::Missing)?.hash()?;
        if local_chain(&chain, key, head, &issuer, now)? <= now {
            return Err(Fault::Denied.into());
        }
        crypto::verify_signature(
            chain.certificate()?.signing_key,
            &envelope.signature_input()?,
            envelope.signature(),
        )?;
        let (states, _) = fold::verify_log(&tx.log()?, key, &p.page_id)?;
        let authority = states.last().ok_or(Fault::Missing)?;
        if authority.revoked_devices.contains(&c.author_device)
            || tx.device(&c.author_device)?.is_some_and(|d| d.revoked)
        {
            return Err(Fault::Denied.into());
        }
        if let Some(saved) = tx.saved_operation(&p.operation_id, &digest)? {
            return Ok(saved);
        }
        if !authority.policy.writable() {
            return Err(Fault::Inactive.into());
        }
        if p.membership_head.statement_hash != hex(&head.hash)
            || p.base_revision != current_token(tx, key, &p.page_id)?
        {
            return Err(Fault::StaleBase.into());
        }
        if c.author_device != key.local_writer()?.0 {
            return Err(Fault::Denied.into());
        }
        tx.put_device(&Device {
            chain: chain_bytes.clone(),
            revoked: false,
        })?;
        accepted = tx.append_content(&Envelope {
            scope: StreamScope {
                page: &p.page_id,
                epoch,
                stream: &c.author_device,
            },
            namespace: Namespace::Content,
            seq,
            hash,
            previous: c.prev_hash,
            bytes: &bytes,
        })?;
        let receipt = Receipt {
            space_id: p.space_id.clone(),
            page_id: p.page_id.clone(),
            epoch: p.epoch.clone(),
            membership_head: p.membership_head.clone(),
            revision: current_token(tx, key, &p.page_id)?,
            stream_id: c.author_device.clone(),
            seq: c.stream_seq.clone(),
            envelope_hash: values::encode_binary(&hash),
            source_sha256: p.source_sha256.clone(),
            memory_limit: p.memory_limit,
        };
        let outcome = serde_json::to_vec(&receipt)?;
        tx.save_operation(&p.operation_id, &digest, &outcome)?;
        Ok(outcome)
    })?;
    Ok(Committed {
        receipt: serde_json::from_slice(&outcome)?,
        accepted,
    })
}
