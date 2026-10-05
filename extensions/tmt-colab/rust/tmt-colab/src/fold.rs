//! Owner-local authenticated fold. The opaque sync server never calls this module.
use crate::{
    Result,
    decoder::{BaselineInput, DecodeFault, Decoder, Namespace, Role, UpdateBatch},
    keyring::Keyring,
    store::{
        Store,
        owner::{Cut, Device, OwnerFault, Recipient, StoredBaseline, count, size},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tmt_colab_model::{
    certificate, object,
    payload::{self, Payload},
    statement, stream_cut, values,
};

#[derive(Clone)]
pub(crate) struct Issuer {
    pub recipient: Recipient,
    pub statement_hash: [u8; 32],
    pub revision: u64,
}
#[derive(Clone)]
pub(crate) struct Authority {
    pub head: statement::Head,
    pub recipients: Arc<BTreeMap<(String, String), Issuer>>,
    pub revoked_devices: Arc<BTreeSet<String>>,
    pub policy: PagePolicy,
}
/// One reducer for authenticated folds and transaction-local admission projections.
#[derive(Clone, Debug)]
pub(crate) struct PagePolicy {
    pub epoch: u64,
    pub link_mode: bool,
    pub public_mode: bool,
    pub history_current: bool,
    pub archived: bool,
    pub deleted: bool,
    pub retention_days: Option<u64>,
}
impl Default for PagePolicy {
    fn default() -> Self {
        Self {
            epoch: 1,
            link_mode: false,
            public_mode: false,
            history_current: false,
            archived: false,
            deleted: false,
            retention_days: Some(30),
        }
    }
}
impl PagePolicy {
    pub fn writable(&self) -> bool {
        !self.archived && !self.deleted
    }
    pub fn apply(&mut self, payload: &Payload, page: &str) -> Result<()> {
        match payload {
            Payload::EpochAdvance(p) if p.page_id == page => {
                let next = values::decimal(&p.epoch, false)?;
                if self.epoch.checked_add(1) != Some(next) || !self.writable() {
                    return Err(OwnerFault::Invalid.into());
                }
                self.epoch = next;
            }
            Payload::PageShare(p) if p.page_id == page => {
                if self.deleted || values::decimal(&p.epoch, false)? != self.epoch {
                    return Err(OwnerFault::Invalid.into());
                }
                self.link_mode = matches!(p.mode, payload::ShareMode::Link);
                self.public_mode = matches!(p.mode, payload::ShareMode::Public);
            }
            Payload::PageHistory(p) if p.page_id == page => {
                self.history_current = matches!(p.mode, payload::HistoryMode::Current);
            }
            Payload::RetentionSet(p) if p.page_id == page => {
                self.retention_days = match p.days {
                    payload::Days::Forever => None,
                    payload::Days::Count(n) => Some(n),
                };
            }
            Payload::Archive(p) if p.page_id == page => self.archived = true,
            Payload::Delete(p) if p.page_id == page => self.deleted = true,
            _ => {}
        }
        Ok(())
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaselineBody {
    pub source: String,
    pub update: String,
}
pub(crate) struct Snapshot {
    pub authority: Authority,
    pub epoch: u64,
    pub cuts: Vec<Cut>,
    pub secret: [u8; 32],
    pub devices: Vec<Device>,
    states: Vec<Authority>,
    payloads: Vec<Payload>,
    baseline: Option<StoredBaseline>,
    objects: Vec<(usize, crate::store::owner::epoch::StoredObject)>,
}
pub(crate) struct View {
    pub source: String,
    pub title: String,
    pub publisher_agent: Option<String>,
    pub update: Vec<u8>,
    pub memory_limit: crate::decoder::MemoryLimit,
    /// Each authenticated writer's decoded `own` projection (threads, messages, intents,
    /// replies), as the isolated decoder returned and validated it.
    pub own: BTreeMap<String, serde_json::Value>,
    /// Exact merged structs for isolated preparation in the local writer's document.
    pub local_own_update: Option<Vec<u8>>,
    /// Owner-member provenance from verified, cut-admitted own envelopes at their
    /// membership revision. Historical keys alone do not grant status authority.
    pub status_writers: BTreeSet<String>,
    /// Each writer's historical signing key, from its cut-admitted envelopes. It gives no
    /// fresh write authority; it only lets a reader verify what the writer signed.
    pub signing_keys: BTreeMap<String, [u8; 32]>,
}
impl Snapshot {
    pub fn require_update_capacity(&self, page: &str) -> Result<()> {
        if self.objects.len() >= crate::decoder::WRITE_TAIL_UPDATES {
            return Err(OwnerFault::too_large_to_edit(
                page,
                format!(
                    "it has {} changes, the most one page can hold",
                    count(self.objects.len())
                ),
            )
            .into());
        }
        Ok(())
    }
    pub fn capture(store: &Store, key: &Keyring, page: &str) -> Result<Self> {
        store.owner_read(&key.space_id, &key.owner_public(), |tx| {
            let (states, payloads) = verify_log(&tx.log()?, key, page)?;
            let authority = states.last().ok_or(OwnerFault::Invalid)?.clone();
            let epoch = tx.current_epoch(page)?;
            if tx.head() != Some(&authority.head)
                || !authority.policy.writable()
                || epoch != authority.policy.epoch
            {
                return Err(OwnerFault::Invalid.into());
            }
            for payload in &payloads {
                let cuts = match payload {
                    Payload::MemberRole(p) => Some(p.cuts.as_slice()),
                    Payload::MemberRemove(p) => Some(p.cuts.as_slice()),
                    Payload::LinkRemove(p) => Some(p.cuts.as_slice()),
                    Payload::DeviceRevoke(p) => Some(p.cuts.as_slice()),
                    _ => None,
                };
                for cut in cuts
                    .into_iter()
                    .flatten()
                    .filter(|c| c.page_id == page && c.epoch == epoch.to_string())
                {
                    tx.validate_retained_cut(cut)?;
                }
            }
            let secret = tx.epoch_secret(page, epoch)?.ok_or(OwnerFault::Invalid)?;
            let cuts = tx.cuts(page, epoch)?;
            let mut objects = Vec::new();
            for (index, cut) in cuts.iter().enumerate() {
                for object in tx.cut_objects(cut)? {
                    objects.push((index, object));
                }
            }
            if objects.len() > crate::decoder::UPDATES {
                return Err(OwnerFault::too_large(
                    page,
                    format!(
                        "it has {} changes since its last baseline (limit {})",
                        count(objects.len()),
                        count(crate::decoder::UPDATES)
                    ),
                )
                .into());
            }
            Ok(Self {
                authority,
                epoch,
                cuts,
                secret,
                devices: tx.devices()?,
                states,
                payloads,
                baseline: tx.baseline(page, epoch)?,
                objects,
            })
        })
    }
    pub fn materialize(&self, key: &Keyring, page: &str, decoder: &mut Decoder) -> Result<View> {
        self.materialize_edit(key, page, decoder, None)
    }
    pub fn materialize_edit(
        &self,
        key: &Keyring,
        page: &str,
        decoder: &mut Decoder,
        edit: Option<crate::decoder::ContentEdit<'_>>,
    ) -> Result<View> {
        let mut baseline = Vec::new();
        if let Some(saved) = &self.baseline {
            let d = payload::decode_baseline(&saved.descriptor)?;
            let rev = values::decimal(&d.membership_revision, false)?;
            let Payload::EpochAdvance(advance) = self
                .payloads
                .get(usize::try_from(rev - 1).map_err(|_| OwnerFault::Invalid)?)
                .ok_or(OwnerFault::Invalid)?
            else {
                return Err(OwnerFault::Invalid.into());
            };
            let envelope = object::Envelope::from_json(&saved.envelope)?;
            let h = object::Header::decode(envelope.header())?;
            let expected = &advance.baseline;
            if d.page_id != page
                || d.epoch != self.epoch.to_string()
                || d.source_digest != expected.source_digest
                || d.baseline_commitment != expected.baseline_commitment
                || d.title != expected.title
                || d.object_envelope_hash != expected.object_envelope_hash
                || d.membership_revision != expected.membership_revision
                || h.context != baseline_context(key, page, self.epoch, rev)?
                || values::binary(&d.object_envelope_hash, 32)? != envelope.hash()?
            {
                return Err(OwnerFault::Invalid.into());
            }
            let body: BaselineBody = serde_json::from_slice(&object::open(
                &envelope,
                &h.context,
                &self.secret,
                &key.management_member()?.signing_key,
            )?)?;
            baseline = values::binary(&body.update, crate::decoder::BASELINE_UPDATE_BYTES)?;
            decoder.verify_baseline(
                BaselineInput {
                    source: body.source.as_bytes(),
                    title: &d.title,
                    publisher_agent: None,
                    source_digest: binary32(&d.source_digest)?,
                },
                &baseline,
                binary32(&d.baseline_commitment)?,
                None,
            )?;
        } else if self.epoch != 1 {
            return Err(OwnerFault::Invalid.into());
        }
        let mut updates = Vec::new();
        let mut own_updates: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
        let mut signing_keys: BTreeMap<String, [u8; 32]> = BTreeMap::new();
        let mut owner_provenance: BTreeMap<String, bool> = BTreeMap::new();
        for (index, stored) in &self.objects {
            let cut = &self.cuts[*index];
            let envelope = object::Envelope::from_json(&stored.bytes)?;
            let h = object::Header::decode(envelope.header())?;
            let c = &h.context;
            let revision = values::decimal(&c.membership_revision, false)?;
            let at_write = self
                .states
                .get(usize::try_from(revision - 1).map_err(|_| OwnerFault::Invalid)?)
                .ok_or(OwnerFault::Invalid)?;
            if c.space != key.space_id
                || c.page != page
                || c.epoch != self.epoch.to_string()
                || c.author_device != cut.stream
                || c.namespace != cut.namespace
                || c.kind
                    != if stored.checkpoint {
                        "checkpoint"
                    } else {
                        "update"
                    }
                || c.stream_seq != stored.seq.to_string()
                || c.prev_hash != stored.previous
                || envelope.hash()? != stored.hash
                || at_write.policy.epoch != self.epoch
                || at_write.revoked_devices.contains(&c.author_device)
            {
                return Err(OwnerFault::Invalid.into());
            }
            let bridge = at_write
                .recipients
                .get(&("bridge".into(), cut.stream.clone()));
            let (issuer, key_bytes, owner_device) = if let Some(bridge) = bridge {
                if c.namespace != "own" {
                    return Err(OwnerFault::Invalid.into());
                }
                (bridge, bridge.recipient.signing_key, false)
            } else {
                let device = self
                    .devices
                    .iter()
                    .find(|d| {
                        certificate::Chain::from_json(&d.chain)
                            .and_then(|ch| Ok(ch.certificate()?.device_id == c.author_device))
                            .unwrap_or(false)
                    })
                    .ok_or(OwnerFault::Invalid)?;
                let chain = certificate::Chain::from_json(&device.chain)?;
                let cert = chain.certificate()?;
                let issuer = at_write
                    .recipients
                    .get(&(cert.issuer_kind.into(), cert.issuer_id.into()))
                    .ok_or(OwnerFault::Invalid)?;
                verify_chain(&chain, issuer, key, revision)?;
                (
                    issuer,
                    *cert.signing_key,
                    cert.issuer_kind == "member" && cert.issuer_id == at_write.head.owner_member.id,
                )
            };
            if !eligible(&issuer.recipient, at_write, page)
                || (c.namespace == "content" && issuer.recipient.role.as_deref() != Some("editor"))
                || issuer.recipient.role.as_deref() == Some("viewer")
            {
                return Err(OwnerFault::Invalid.into());
            }
            // Every later authority reduction must commit this exact prefix.
            for (index, payload) in self
                .payloads
                .iter()
                .enumerate()
                .skip(usize::try_from(revision).map_err(|_| OwnerFault::Invalid)?)
            {
                let affected = match payload {
                    Payload::MemberRole(p)
                        if issuer.recipient.kind == "member"
                            && p.member_id == issuer.recipient.id
                            && role_rank(role_name(&p.role))
                                < role_rank(
                                    self.states[index - 1]
                                        .recipients
                                        .get(&("member".into(), p.member_id.clone()))
                                        .ok_or(OwnerFault::Invalid)?
                                        .recipient
                                        .role
                                        .as_deref()
                                        .ok_or(OwnerFault::Invalid)?,
                                ) =>
                    {
                        Some(p.cuts.as_slice())
                    }
                    Payload::MemberRemove(p)
                        if issuer.recipient.kind == "member"
                            && p.member_id == issuer.recipient.id =>
                    {
                        Some(p.cuts.as_slice())
                    }
                    Payload::LinkRemove(p)
                        if issuer.recipient.kind == "link" && p.link_id == issuer.recipient.id =>
                    {
                        Some(p.cuts.as_slice())
                    }
                    Payload::DeviceRevoke(p) if p.device_id == c.author_device => {
                        Some(p.cuts.as_slice())
                    }
                    _ => None,
                };
                if let Some(cuts) = affected {
                    let bound = cuts
                        .iter()
                        .find(|p| {
                            p.page_id == page
                                && p.epoch == c.epoch
                                && p.namespace == c.namespace
                                && values::binary(&p.cut, 1024)
                                    .and_then(|bytes| {
                                        Ok(stream_cut::decode(&bytes)?.stream_id == c.author_device)
                                    })
                                    .unwrap_or(false)
                        })
                        .ok_or(OwnerFault::Invalid)?;
                    let bytes = values::binary(&bound.cut, 1024)?;
                    let committed = stream_cut::decode(&bytes)?;
                    if stored.seq > values::decimal(committed.tail_head_seq, true)?
                        || (stored.checkpoint
                            && (stored.seq != values::decimal(committed.checkpoint_seq, true)?
                                || committed.checkpoint_hash != Some(&stored.hash)))
                    {
                        return Err(OwnerFault::Invalid.into());
                    }
                }
            }
            let plaintext = object::open(&envelope, c, &self.secret, &key_bytes)?;
            if c.namespace == "content" {
                updates.push(plaintext);
            } else {
                signing_keys.insert(c.author_device.clone(), key_bytes);
                // An ambiguous writer never gains status authority. Later revocation
                // does not erase provenance of an earlier cut-admitted envelope.
                owner_provenance
                    .entry(c.author_device.clone())
                    .and_modify(|owner| *owner &= owner_device)
                    .or_insert(owner_device);
                own_updates
                    .entry(c.author_device.clone())
                    .or_default()
                    .push(plaintext);
            }
        }
        let state = baseline.len() + updates.iter().map(Vec::len).sum::<usize>();
        let tail = updates.iter().map(Vec::len).sum::<usize>();
        if edit.is_some() {
            // A write adds one update and must not leave a page the browser cannot open; reads
            // accept more. The new update's own size is checked once it is prepared.
            let detail = if self.objects.len() >= crate::decoder::WRITE_TAIL_UPDATES {
                Some(format!(
                    "it has {} changes, the most one page can hold",
                    count(self.objects.len())
                ))
            } else if tail >= crate::decoder::WRITE_TAIL_BYTES {
                Some(format!(
                    "its changes add up to {}; one page holds at most {}",
                    size(tail),
                    size(crate::decoder::WRITE_TAIL_BYTES)
                ))
            } else if baseline.len() > crate::decoder::BASELINE_BYTES {
                Some(format!(
                    "its content is {}, the most one page can hold is {}",
                    size(baseline.len()),
                    size(crate::decoder::BASELINE_BYTES)
                ))
            } else {
                None
            };
            if let Some(detail) = detail {
                return Err(OwnerFault::too_large_to_edit(page, detail).into());
            }
        }
        if state > crate::decoder::STATE_BYTES {
            return Err(OwnerFault::too_large(
                page,
                format!(
                    "its state is {} (limit {})",
                    size(state),
                    size(crate::decoder::STATE_BYTES)
                ),
            )
            .into());
        }
        let mut threads = 0;
        let mut own_views = BTreeMap::new();
        let local_writer = key.local_writer()?.0;
        let mut local_own_update = None;
        for (writer, own) in &own_updates {
            let discussion = own.iter().map(Vec::len).sum::<usize>();
            if discussion > crate::decoder::STATE_BYTES {
                return Err(OwnerFault::too_large(
                    page,
                    format!(
                        "one writer's discussion state is {} (limit {})",
                        size(discussion),
                        size(crate::decoder::STATE_BYTES)
                    ),
                )
                .into());
            }
            let refs = own.iter().map(Vec::as_slice).collect::<Vec<_>>();
            let decoded = decoder.decode(
                UpdateBatch {
                    namespace: Namespace::Own,
                    baseline: &[],
                    updates: &refs,
                },
                Role::Commenter,
                None,
            )?;
            threads += decoded.projection["threads"]
                .as_object()
                .ok_or(OwnerFault::Invalid)?
                .len();
            if threads > 1000 {
                return Err(OwnerFault::Capacity.into());
            }
            if writer == &local_writer {
                local_own_update = Some(decoded.merged);
            }
            own_views.insert(writer.clone(), decoded.projection);
        }
        let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let batch = UpdateBatch {
            namespace: Namespace::Content,
            baseline: &baseline,
            updates: &refs,
        };
        let folded = if let Some(edit) = edit {
            decoder
                .prepare(batch, edit, None)
                .map_err(|fault| match fault {
                    // A source bigger than one update can carry replaces more than one update holds.
                    DecodeFault::Rejected
                        if edit.source.len() > crate::decoder::UPDATE_BYTES - 1024 =>
                    {
                        OwnerFault::too_large_to_edit(
                            page,
                            format!(
                                "this edit changes more than the {} one change can carry; make it in smaller steps",
                                size(crate::decoder::UPDATE_BYTES)
                            ),
                        )
                        .into()
                    }
                    other => Box::<dyn std::error::Error + Send + Sync>::from(other),
                })?
        } else {
            decoder
                .decode(batch, Role::Editor, None)
                .map_err(|fault| match fault {
                    // The decoder's own deadline is the containment; say which page hit it.
                    DecodeFault::Invoke(ref invoke)
                        if invoke.kind == tmt_invoke::FailureKind::Deadline =>
                    {
                        OwnerFault::too_large(
                            page,
                            format!(
                                "decoding its {} changes ({}) did not finish within {} s",
                                count(updates.len()),
                                size(state),
                                crate::decoder::DEADLINE.as_secs()
                            ),
                        )
                        .into()
                    }
                    other => Box::<dyn std::error::Error + Send + Sync>::from(other),
                })?
        };
        if edit.is_some() && tail + folded.merged.len() > crate::decoder::WRITE_TAIL_BYTES {
            return Err(OwnerFault::too_large_to_edit(
                page,
                format!(
                    "this edit would take its changes to {}, more than the {} one page can hold",
                    size(tail + folded.merged.len()),
                    size(crate::decoder::WRITE_TAIL_BYTES)
                ),
            )
            .into());
        }
        Ok(View {
            publisher_agent: folded.projection["meta"]["publisherAgent"]
                .as_str()
                .map(str::to_owned),
            update: folded.merged,
            memory_limit: folded.memory_limit,
            own: own_views,
            local_own_update,
            status_writers: owner_provenance
                .into_iter()
                .filter_map(|(writer, owner)| owner.then_some(writer))
                .collect(),
            signing_keys,
            source: folded.projection["html"]
                .as_str()
                .ok_or(OwnerFault::Invalid)?
                .into(),
            title: folded.projection["meta"]["title"]
                .as_str()
                .unwrap_or("")
                .into(),
        })
    }
}
pub(crate) fn verify_log(
    log: &[statement::Envelope],
    key: &Keyring,
    page: &str,
) -> Result<(Vec<Authority>, Vec<Payload>)> {
    let mut states: Vec<Authority> = Vec::new();
    let mut payloads = Vec::new();
    let mut recipients: Arc<BTreeMap<(String, String), Issuer>> = Arc::new(BTreeMap::new());
    let mut revoked_devices = Arc::new(BTreeSet::new());
    let mut policy = PagePolicy::default();
    for item in log {
        let verified = item.verify_next(
            &key.space_id,
            &key.owner_public(),
            states.last().map(|s| &s.head),
        )?;
        let mut addition = None;
        match &verified.payload {
            Payload::MemberAdd(p) => {
                addition = Some(recipient(
                    "member",
                    &p.member_id,
                    Some(role_name(&p.role)),
                    &p.sign_key,
                    &p.enc_key,
                    p.pages.as_slice(),
                )?)
            }
            Payload::LinkAdd(p) => {
                addition = Some(recipient(
                    "link",
                    &p.link_id,
                    Some(role_name(&p.role)),
                    &p.link_sign_key,
                    &p.link_enc_key,
                    p.pages.as_slice(),
                )?)
            }
            Payload::BridgeAdd(p) => {
                addition = Some(recipient(
                    "bridge",
                    &p.machine_id,
                    None,
                    &p.machine_sign_key,
                    &p.enc_key,
                    p.pages.as_slice(),
                )?)
            }
            Payload::MemberRole(p) => {
                Arc::make_mut(&mut recipients)
                    .get_mut(&("member".into(), p.member_id.clone()))
                    .ok_or(OwnerFault::Invalid)?
                    .recipient
                    .role = Some(role_name(&p.role).into())
            }
            Payload::MemberRemove(p) => {
                Arc::make_mut(&mut recipients)
                    .get_mut(&("member".into(), p.member_id.clone()))
                    .ok_or(OwnerFault::Invalid)?
                    .recipient
                    .revoked = true
            }
            Payload::LinkRemove(p) => {
                Arc::make_mut(&mut recipients)
                    .get_mut(&("link".into(), p.link_id.clone()))
                    .ok_or(OwnerFault::Invalid)?
                    .recipient
                    .revoked = true
            }
            Payload::DeviceRevoke(p) => {
                Arc::make_mut(&mut revoked_devices).insert(p.device_id.clone());
            }
            _ => {}
        }
        policy.apply(&verified.payload, page)?;
        if let Some(recipient) = addition {
            let id = (recipient.kind.clone(), recipient.id.clone());
            if Arc::make_mut(&mut recipients)
                .insert(
                    id,
                    Issuer {
                        recipient,
                        statement_hash: item.hash()?,
                        revision: verified.head.revision,
                    },
                )
                .is_some()
            {
                return Err(OwnerFault::Invalid.into());
            }
        }
        states.push(Authority {
            head: verified.head,
            recipients: recipients.clone(),
            revoked_devices: revoked_devices.clone(),
            policy: policy.clone(),
        });
        payloads.push(verified.payload);
    }
    let member = key.management_member()?;
    if states.last().is_none_or(|s| s.head.owner_member != member) {
        return Err(OwnerFault::WrongOwner.into());
    }
    Ok((states, payloads))
}
fn recipient(
    kind: &str,
    id: &str,
    role: Option<&str>,
    signing: &str,
    encryption: &str,
    pages: &[String],
) -> Result<Recipient> {
    Ok(Recipient {
        kind: kind.into(),
        id: id.into(),
        role: role.map(str::to_owned),
        signing_key: binary32(signing)?,
        encryption_key: binary32(encryption)?,
        pages: pages.to_vec(),
        revoked: false,
    })
}
pub(crate) fn eligible(r: &Recipient, a: &Authority, page: &str) -> bool {
    !r.revoked
        && a.policy.writable()
        && ((r.kind == "member" && r.id == a.head.owner_member.id)
            || r.pages.iter().any(|p| p == page))
        && (r.kind != "link" || a.policy.link_mode)
}
pub(crate) fn verify_chain(
    chain: &certificate::Chain,
    issuer: &Issuer,
    key: &Keyring,
    revision: u64,
) -> Result<()> {
    let cert = chain.certificate()?;
    if cert.space != key.space_id
        || values::decimal(cert.membership_revision, false)? > revision
        || values::decimal(cert.membership_revision, false)? < issuer.revision
        || cert.issuer_kind != issuer.recipient.kind
        || cert.issuer_id != issuer.recipient.id
    {
        return Err(OwnerFault::Invalid.into());
    }
    chain.verify(&issuer.statement_hash, &cert, &issuer.recipient.signing_key)?;
    Ok(())
}
pub(crate) fn baseline_context(
    key: &Keyring,
    page: &str,
    epoch: u64,
    revision: u64,
) -> Result<object::Context> {
    Ok(object::Context {
        space: key.space_id.clone(),
        page: page.into(),
        epoch: epoch.to_string(),
        kind: "html".into(),
        namespace: "content".into(),
        author_device: key.management_member()?.id,
        membership_revision: revision.to_string(),
        stream_seq: "0".into(),
        prev_hash: [0; 32],
    })
}
pub(crate) fn binary32(text: &str) -> Result<[u8; 32]> {
    values::binary(text, 32)?
        .try_into()
        .map_err(|_| OwnerFault::Invalid.into())
}
fn role_name(role: &payload::Role) -> &'static str {
    match role {
        payload::Role::Viewer => "viewer",
        payload::Role::Commenter => "commenter",
        payload::Role::Editor => "editor",
    }
}
fn role_rank(role: &str) -> u8 {
    match role {
        "editor" => 2,
        "commenter" => 1,
        _ => 0,
    }
}
