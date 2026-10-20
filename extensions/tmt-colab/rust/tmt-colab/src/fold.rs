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
    time::Instant,
};
use tmt_colab_model::{
    certificate, crypto, object,
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
    pub(crate) objects: Vec<(usize, crate::store::owner::epoch::StoredObject)>,
    /// The update each retained owner fold binds, by covered stream and sequence.
    anchors: BTreeMap<(String, u64), Vec<u8>>,
}
/// Authenticated bytes from one snapshot, shared by reads, edits and causal preparation.
struct MaterializationInput {
    baseline: Vec<u8>,
    updates: Vec<Vec<u8>>,
    own_updates: BTreeMap<String, Vec<Vec<u8>>>,
    signing_keys: BTreeMap<String, [u8; 32]>,
    /// Whether every original own envelope of a writer came from an owner-member device.
    owner_provenance: BTreeMap<String, bool>,
    local_writer: String,
    tally: crate::page::UsageCounts,
}
impl crate::page::UsageCounts {
    /// Count one stored object: a checkpoint holds what the tail since the last one has not yet
    /// combined, and the write path refuses against the tail.
    fn add(&mut self, checkpoint: bool, own_device: bool, bytes: usize) -> Result<()> {
        let total = if checkpoint {
            &mut self.checkpoint_bytes
        } else {
            self.updates = self.updates.checked_add(1).ok_or(OwnerFault::Capacity)?;
            self.other_device_updates += usize::from(!own_device);
            &mut self.tail_bytes
        };
        *total = total.checked_add(bytes).ok_or(OwnerFault::Capacity)?;
        Ok(())
    }
}
pub(crate) struct View {
    pub source: String,
    pub title: String,
    pub publisher_agent: Option<String>,
    pub original_author: Option<String>,
    pub creation_recipient: Option<crate::decoder::CreationRecipient>,
    /// The complete content metadata projection, including keys not surfaced by the CLI.
    pub meta: serde_json::Value,
    pub memory_limit: crate::decoder::MemoryLimit,
    /// The counts and raw state size; the gzipped size only when read through
    /// [`Snapshot::materialize_measured`], because compressing the state costs as much as reading
    /// it and most callers (a page list, an export) never report it.
    pub usage: crate::page::PageUsage,
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
/// The gzipped size of the parts as one stream, the way a browser loads a page.
fn gzip_bytes<'a>(parts: impl Iterator<Item = &'a [u8]>) -> Result<usize> {
    use std::io::Write;
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut encoder = flate2::write::GzEncoder::new(Count(0), flate2::Compression::new(6));
    for part in parts {
        encoder.write_all(part)?;
    }
    Ok(encoder.finish()?.0)
}
/// The gzipped size of the parts, if it is over the page budget.
fn gzip_over_budget<'a>(parts: impl Iterator<Item = &'a [u8]>) -> Result<Option<usize>> {
    let compressed = gzip_bytes(parts)?;
    Ok((compressed > crate::decoder::PAGE_BUDGET_GZIP_BYTES).then_some(compressed))
}
/// The raw state size from which usage reports the gzipped size too.
const GZIP_MEASURED_FROM: usize = 256 * 1024;
pub(crate) struct OpenedObject {
    pub namespace: String,
    pub author_device: String,
    pub plaintext: Vec<u8>,
    pub key_bytes: [u8; 32],
    pub owner_device: bool,
}
impl Snapshot {
    pub fn require_update_capacity(&self, page: &str) -> Result<()> {
        let tail_count = self
            .objects
            .iter()
            .filter(|(_, object)| !object.checkpoint)
            .count();
        if tail_count >= crate::decoder::WRITE_TAIL_UPDATES {
            return Err(OwnerFault::tail_full(
                page,
                format!(
                    "it has {} changes, the most one page can hold",
                    count(tail_count)
                ),
            )
            .into());
        }
        Ok(())
    }
    /// How full the page is, from the same authenticated bytes and count a write is refused
    /// against.
    pub fn usage(
        &self,
        key: &Keyring,
        page: &str,
        decoder: &mut Decoder,
    ) -> Result<crate::page::PageUsage> {
        self.materialization_input(key, page, decoder, &BTreeMap::new(), None)?
            .usage(true)
    }
    /// [`Self::materialize`] with the gzipped state measured into `usage`, for a reader that
    /// reports it.
    pub(crate) fn materialize_measured(
        &self,
        key: &Keyring,
        page: &str,
        decoder: &mut Decoder,
    ) -> Result<View> {
        let input = self.materialization_input(key, page, decoder, &BTreeMap::new(), None)?;
        let usage = input.usage(true)?;
        let mut view = input.materialize(page, decoder, None)?;
        view.usage = usage;
        Ok(view)
    }
    pub fn capture(store: &Store, key: &Keyring, page: &str) -> Result<Self> {
        Self::capture_epoch(store, key, page, None, true)
    }
    /// Root-local read capture. Historical secrets are accessible to the actual
    /// management keyring, never as a substitute for a remote reader's wraps.
    pub(crate) fn capture_read(
        store: &Store,
        key: &Keyring,
        page: &str,
        epoch: Option<u64>,
    ) -> Result<Self> {
        Self::capture_epoch(store, key, page, epoch, false)
    }
    /// The authority for admitting originals of a page at its current epoch, with none of its
    /// stored objects: the verified owner log, the page secret and the retained devices, plus
    /// `extra` chains read from the cloud. Each chain is verified against the log when an
    /// original names its device, never here.
    pub(crate) fn admission(
        store: &Store,
        key: &Keyring,
        page: &str,
        extra: Vec<Device>,
    ) -> Result<Self> {
        values::generated_id(page)?;
        store.owner_read(&key.space_id, &key.owner_public(), |tx| {
            let (states, payloads) = verify_log(&tx.log()?, key, page)?;
            let authority = states.last().ok_or(OwnerFault::Invalid)?.clone();
            let epoch = tx.current_epoch(page)?;
            if tx.head() != Some(&authority.head)
                || authority.policy.deleted
                || epoch != authority.policy.epoch
            {
                return Err(OwnerFault::Invalid.into());
            }
            let secret = tx.epoch_secret(page, epoch)?.ok_or(OwnerFault::Invalid)?;
            let mut devices = tx.devices()?;
            devices.extend(extra);
            Ok(Self {
                authority,
                epoch,
                cuts: Vec::new(),
                secret,
                devices,
                states,
                payloads,
                baseline: None,
                objects: Vec::new(),
                anchors: BTreeMap::new(),
            })
        })
    }
    fn capture_epoch(
        store: &Store,
        key: &Keyring,
        page: &str,
        requested: Option<u64>,
        writing: bool,
    ) -> Result<Self> {
        values::generated_id(page)?;
        store.owner_read(&key.space_id, &key.owner_public(), |tx| {
            let (states, payloads) = verify_log(&tx.log()?, key, page)?;
            let authority = states.last().ok_or(OwnerFault::Invalid)?.clone();
            let current = tx.current_epoch(page)?;
            let epoch = requested.unwrap_or(current);
            if tx.head() != Some(&authority.head)
                || authority.policy.deleted
                || (writing && !authority.policy.writable())
                || current != authority.policy.epoch
                || epoch == 0
                || epoch > current
                || current - epoch >= 64
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
            let cuts = if writing {
                tx.cuts(page, epoch)?
            } else {
                tx.read_cuts(page, epoch)?
            };
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
                anchors: tx.fold_anchors(page, epoch)?,
            })
        })
    }
    /// One stored object, verified against the authority at the time it was written and opened.
    /// The same checks serve reads and the compaction of a device's own stream.
    pub(crate) fn open_object(
        &self,
        key: &Keyring,
        page: &str,
        index: usize,
        stored: &crate::store::owner::epoch::StoredObject,
    ) -> Result<OpenedObject> {
        let cut = &self.cuts[index];
        self.open_stream_object(key, page, &cut.stream, &cut.namespace, stored)
    }
    /// The same checks for an object of `stream` and `namespace` that this snapshot's cut list
    /// need not name, which is how an original read from the cloud is admitted before it is
    /// stored.
    pub(crate) fn open_stream_object(
        &self,
        key: &Keyring,
        page: &str,
        stream: &str,
        namespace: &str,
        stored: &crate::store::owner::epoch::StoredObject,
    ) -> Result<OpenedObject> {
        let envelope = object::Envelope::from_json(&stored.bytes)?;
        let h = object::Header::decode(envelope.header())?;
        if h.covered_device.is_some() {
            return self.open_fold_object(key, page, stream, namespace, stored, &envelope, &h);
        }
        let c = &h.context;
        let revision = values::decimal(&c.membership_revision, false)?;
        let at_write = self
            .states
            .get(usize::try_from(revision - 1).map_err(|_| OwnerFault::Invalid)?)
            .ok_or(OwnerFault::Invalid)?;
        if c.space != key.space_id
            || c.page != page
            || c.epoch != self.epoch.to_string()
            || c.author_device != stream
            || c.namespace != namespace
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
            .get(&("bridge".into(), stream.to_owned()));
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
                    if issuer.recipient.kind == "member" && p.member_id == issuer.recipient.id =>
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
                let bound = self
                    .signed_cut(index, cuts, revision, page, &c.namespace, &c.author_device)
                    .ok_or(OwnerFault::Invalid)?;
                let bytes = values::binary(&bound.cut, 1024)?;
                let committed = stream_cut::decode(&bytes)?;
                if stored.seq > values::decimal(committed.tail_head_seq, true)?
                    || (stored.checkpoint
                        && (stored.seq != values::decimal(committed.checkpoint_seq, true)?
                            || committed.checkpoint_hash != Some(&stored.hash)))
                    || (!stored.checkpoint
                        && stored.seq == values::decimal(committed.tail_head_seq, true)?
                        && stored.hash != *committed.tail_head_hash)
                {
                    return Err(OwnerFault::Invalid.into());
                }
            }
        }
        let plaintext = object::open(&envelope, c, &self.secret, &key_bytes)?;
        Ok(OpenedObject {
            namespace: c.namespace.clone(),
            author_device: c.author_device.clone(),
            plaintext,
            key_bytes,
            owner_device,
        })
    }
    /// Whether `device` is an owner-member device that may write `namespace` of the page now.
    pub(crate) fn is_owner_member_device(
        &self,
        key: &Keyring,
        page: &str,
        device: &str,
        namespace: &str,
    ) -> bool {
        self.owner_member_key(
            key,
            &self.authority,
            page,
            device,
            namespace,
            self.authority.head.revision,
        )
        .is_ok()
    }
    /// Hold the update at `seq` of `stream` as the anchor of a fold about to be checked, as the
    /// store hands it to a snapshot once the fold is stored.
    pub(crate) fn with_anchor(&mut self, stream: &str, seq: u64, bytes: Vec<u8>) {
        self.anchors.insert((stream.to_owned(), seq), bytes);
    }
    /// The cut that bounds `stream`'s objects in `namespace` of this epoch at the reduction `cuts`
    /// belongs to (the statement at payload `index`): the reduction's own or, as a later reduction
    /// need not repeat an already sealed epoch's cut, the one an earlier verified owner
    /// `epoch.advance` sealing this epoch signed after revision `after`. Unsealed or uncut history
    /// has none and is refused.
    fn signed_cut<'a>(
        &'a self,
        index: usize,
        cuts: &'a [payload::Cut],
        after: u64,
        page: &str,
        namespace: &str,
        stream: &str,
    ) -> Option<&'a payload::Cut> {
        let epoch = self.epoch.to_string();
        let matches = |p: &&payload::Cut| {
            p.page_id == page
                && p.epoch == epoch
                && p.namespace == namespace
                && values::binary(&p.cut, 1024)
                    .and_then(|bytes| Ok(stream_cut::decode(&bytes)?.stream_id == stream))
                    .unwrap_or(false)
        };
        cuts.iter().find(matches).or_else(|| {
            self.payloads[..index]
                .iter()
                .enumerate()
                .find_map(|(seal_index, p)| {
                    if seal_index < usize::try_from(after).ok()? {
                        return None;
                    }
                    match p {
                        Payload::EpochAdvance(seal)
                            if seal.page_id == page
                                && values::decimal(&seal.epoch, false).ok()
                                    == self.epoch.checked_add(1) =>
                        {
                            seal.cuts.as_slice().iter().find(matches)
                        }
                        _ => None,
                    }
                })
        })
    }
    /// The signing key of `device` as an owner-member device of the page at `revision`: its chain
    /// verifies against the owner member at that revision and the owner member may write the
    /// namespace. A bridge, a link-issued or commenter device and one with no chain are refused.
    fn owner_member_key(
        &self,
        key: &Keyring,
        at_write: &Authority,
        page: &str,
        device: &str,
        namespace: &str,
        revision: u64,
    ) -> Result<[u8; 32]> {
        if at_write
            .recipients
            .contains_key(&("bridge".into(), device.to_owned()))
        {
            return Err(OwnerFault::Invalid.into());
        }
        let found = self
            .devices
            .iter()
            .find_map(|d| {
                let chain = certificate::Chain::from_json(&d.chain).ok()?;
                (chain.certificate().ok()?.device_id == device).then_some(chain)
            })
            .ok_or(OwnerFault::Invalid)?;
        let cert = found.certificate()?;
        let issuer = at_write
            .recipients
            .get(&(cert.issuer_kind.into(), cert.issuer_id.into()))
            .ok_or(OwnerFault::Invalid)?;
        verify_chain(&found, issuer, key, revision)?;
        if cert.issuer_kind != "member"
            || cert.issuer_id != at_write.head.owner_member.id
            || !eligible(&issuer.recipient, at_write, page)
            || (namespace == "content" && issuer.recipient.role.as_deref() != Some("editor"))
            || issuer.recipient.role.as_deref() == Some("viewer")
        {
            return Err(OwnerFault::Invalid.into());
        }
        Ok(*cert.signing_key)
    }
    /// A stored owner fold of `stream` (colab-v1, "Owner fold"): the checkpoint of the covered
    /// stream that another owner-member device signed. It is admitted by the rules the browser
    /// applies (`Admission.readFold`):
    /// - signer and covered device are owner-member devices at the fold's revision;
    /// - a signer that is revoked folds only what its `device.revoke` lists, and only at a
    ///   revision before that revoke;
    /// - a covered device that is revoked is folded only up to its signed cut's tail head;
    /// - the fold binds an update the covered device itself signed (the anchor).
    ///
    /// The writer of what it holds is the covered device, never the signer.
    #[allow(clippy::too_many_arguments)]
    fn open_fold_object(
        &self,
        key: &Keyring,
        page: &str,
        stream: &str,
        namespace: &str,
        stored: &crate::store::owner::epoch::StoredObject,
        envelope: &object::Envelope,
        h: &object::Header,
    ) -> Result<OpenedObject> {
        let c = &h.context;
        let signer = c.author_device.as_str();
        let revision = values::decimal(&c.membership_revision, false)?;
        let at_write = self
            .states
            .get(usize::try_from(revision - 1).map_err(|_| OwnerFault::Invalid)?)
            .ok_or(OwnerFault::Invalid)?;
        if c.space != key.space_id
            || c.page != page
            || c.epoch != self.epoch.to_string()
            || !stored.checkpoint
            || c.kind != object::FOLD
            || h.covered_device.as_deref() != Some(stream)
            || c.namespace != namespace
            || c.stream_seq != stored.seq.to_string()
            || c.prev_hash != stored.previous
            || envelope.hash()? != stored.hash
            || at_write.policy.epoch != self.epoch
        {
            return Err(OwnerFault::Invalid.into());
        }
        let signer_key = self.owner_member_key(key, at_write, page, signer, namespace, revision)?;
        let covered_key =
            self.owner_member_key(key, at_write, page, stream, namespace, revision)?;
        for (index, payload) in self.payloads.iter().enumerate() {
            let Payload::DeviceRevoke(revoke) = payload else {
                continue;
            };
            if revoke.device_id == signer
                && !(revision <= index as u64 && revoke.lists_fold(&stored.hash))
            {
                return Err(OwnerFault::Invalid.into());
            }
            if revoke.device_id == stream {
                let cut = self
                    .signed_cut(
                        index,
                        revoke.cuts.as_slice(),
                        revision,
                        page,
                        namespace,
                        stream,
                    )
                    .ok_or(OwnerFault::Invalid)?;
                let bytes = values::binary(&cut.cut, 1024)?;
                let cut = stream_cut::decode(&bytes)?;
                let tail = values::decimal(cut.tail_head_seq, true)?;
                // The anchor is the update at the fold's sequence, so the cut's own tail rules apply.
                if stored.seq > tail
                    || (stored.seq == tail && *cut.tail_head_hash != stored.previous)
                {
                    return Err(OwnerFault::Invalid.into());
                }
            }
        }
        let anchor = self
            .anchors
            .get(&(stream.to_owned(), stored.seq))
            .ok_or(OwnerFault::Invalid)?;
        object::check_anchor(h, &object::Envelope::from_json(anchor)?, &covered_key)?;
        let plaintext = object::open_fold(envelope, c, stream, &self.secret, &signer_key)?;
        Ok(OpenedObject {
            namespace: c.namespace.clone(),
            author_device: stream.to_owned(),
            plaintext,
            key_bytes: covered_key,
            owner_device: true,
        })
    }
    /// Creator admission for zero-sequence assets. The caller must separately
    /// join an authenticated positive-sequence publication record; a device cut
    /// cannot itself prove when a zero-sequence asset was created.
    pub(crate) fn asset_author(
        &self,
        key: &Keyring,
        descriptor: &tmt_colab_model::attachment::Descriptor,
    ) -> Result<[u8; 32]> {
        descriptor.validate()?;
        let revision = values::decimal(&descriptor.membership_revision, false)?;
        let at = self
            .states
            .get(usize::try_from(revision - 1)?)
            .ok_or(OwnerFault::Invalid)?;
        if descriptor.space != key.space_id
            || descriptor.epoch != self.epoch.to_string()
            || at.policy.epoch != self.epoch
            || !at.policy.writable()
            || at.revoked_devices.contains(&descriptor.author_device)
        {
            return Err(OwnerFault::Invalid.into());
        }
        let device = self
            .devices
            .iter()
            .find(|d| {
                certificate::Chain::from_json(&d.chain)
                    .and_then(|ch| Ok(ch.certificate()?.device_id == descriptor.author_device))
                    .unwrap_or(false)
            })
            .ok_or(OwnerFault::Invalid)?;
        let chain = certificate::Chain::from_json(&device.chain)?;
        let cert = chain.certificate()?;
        let issuer = at
            .recipients
            .get(&(cert.issuer_kind.into(), cert.issuer_id.into()))
            .ok_or(OwnerFault::Invalid)?;
        verify_chain(&chain, issuer, key, revision)?;
        if cert.issuer_kind != "member"
            || cert.issuer_id != at.head.owner_member.id
            || issuer.recipient.role.as_deref() != Some("editor")
            || !eligible(&issuer.recipient, at, &descriptor.page)
        {
            return Err(OwnerFault::Invalid.into());
        }
        Ok(*cert.signing_key)
    }
    pub fn materialize(&self, key: &Keyring, page: &str, decoder: &mut Decoder) -> Result<View> {
        self.materialize_with_replacements(key, page, decoder, &BTreeMap::new())
    }
    pub(crate) fn materialize_until(
        &self,
        key: &Keyring,
        page: &str,
        decoder: &mut Decoder,
        deadline: Instant,
    ) -> Result<View> {
        self.materialization_input(key, page, decoder, &BTreeMap::new(), Some(deadline))?
            .materialize(page, decoder, Some(deadline))
    }
    /// Decode a candidate checkpoint in place of each selected stream's retained objects.
    /// Original objects are still opened and authenticated before the candidate is considered.
    pub(crate) fn materialize_with_replacements(
        &self,
        key: &Keyring,
        page: &str,
        decoder: &mut Decoder,
        replacements: &BTreeMap<usize, Vec<u8>>,
    ) -> Result<View> {
        self.materialization_input(key, page, decoder, replacements, None)?
            .materialize(page, decoder, None)
    }
    /// The verified genesis belongs to the same read snapshot as the page base.
    pub(crate) fn genesis_hash(&self) -> Result<[u8; 32]> {
        Ok(self.states.first().ok_or(OwnerFault::Invalid)?.head.hash)
    }
    pub(crate) fn prepare_content_batch(
        &self,
        key: &Keyring,
        page: &str,
        edit: crate::decoder::ContentEdit<'_>,
        base_sha256: Option<&[u8; 32]>,
        decoder: &mut Decoder,
    ) -> Result<crate::decoder::PreparedContent> {
        let input = self.materialization_input(key, page, decoder, &BTreeMap::new(), None)?;
        let base = input.materialize(page, decoder, None)?;
        if base.meta.get("pageType").is_some() {
            return Err(crate::page::Fault::Invalid.into());
        }
        // A caller that edited from a source it saw refuses to overwrite a different one.
        if base_sha256.is_some_and(|d| *d != crypto::digest(base.source.as_bytes())) {
            return Err(crate::page::Fault::StaleBase.into());
        }
        if let Some(change) = edit.attachments {
            // Added references must belong to this page and epoch and be witnessed by their
            // creator's own-stream proof; the decoder checks the change against the source.
            for descriptor in change.set.as_slice() {
                if descriptor.space != key.space_id
                    || descriptor.page != page
                    || descriptor.epoch != self.epoch.to_string()
                {
                    return Err(crate::page::Fault::Invalid.into());
                }
                crate::attachments::creation_proof(&base, descriptor)?;
            }
        }
        let expected_base = serde_json::json!({"html":base.source,"meta":base.meta});
        let refs = input.updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let prepared = decoder.prepare_content_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &input.baseline,
                updates: &refs,
            },
            &expected_base,
            edit,
            None,
        )?;
        if let crate::decoder::ContentBatch::Updates(updates) = &prepared.batch {
            input.admit_deltas(
                page,
                updates.len(),
                checked_bytes(updates.iter().map(Vec::len))?,
            )?;
            input.admit_gzip(page, updates.iter().map(Vec::as_slice))?;
        }
        Ok(prepared)
    }
    pub(crate) fn prepare_shell_batch(
        &self,
        key: &Keyring,
        page: &str,
        edit: &crate::decoder::ShellEdit,
        decoder: &mut Decoder,
    ) -> Result<crate::decoder::PreparedContent> {
        let input = self.materialization_input(key, page, decoder, &BTreeMap::new(), None)?;
        let base = input.materialize(page, decoder, None)?;
        if base.meta["pageType"].as_str() != Some("shell") {
            return Err(crate::page::Fault::Invalid.into());
        }
        // Validate the action before crossing the child boundary so callers retain the typed
        // input refusal, rather than an opaque decoder failure.
        crate::shell::apply(
            &base.meta["shell"],
            &edit.action,
            &edit.body,
            &edit.attribution,
        )
        .map_err(|_| crate::page::Fault::Invalid)?;
        let expected_base = serde_json::json!({"html":base.source,"meta":base.meta});
        let refs = input.updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let prepared = decoder.prepare_shell_batch(
            UpdateBatch {
                namespace: Namespace::Content,
                baseline: &input.baseline,
                updates: &refs,
            },
            &expected_base,
            edit,
            None,
        )?;
        if let crate::decoder::ContentBatch::Updates(updates) = &prepared.batch {
            input.admit_deltas(
                page,
                updates.len(),
                checked_bytes(updates.iter().map(Vec::len))?,
            )?;
            input.admit_gzip(page, updates.iter().map(Vec::as_slice))?;
        }
        Ok(prepared)
    }
    fn materialization_input(
        &self,
        key: &Keyring,
        page: &str,
        decoder: &mut Decoder,
        replacements: &BTreeMap<usize, Vec<u8>>,
        deadline: Option<Instant>,
    ) -> Result<MaterializationInput> {
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
            let view = BaselineInput {
                original_author: None,
                layout: None,
                attachments: None,
                source: body.source.as_bytes(),
                title: &d.title,
                publisher_agent: None,
                creation_recipient: None,
                source_digest: binary32(&d.source_digest)?,
            };
            if let Some(deadline) = deadline {
                decoder.verify_baseline_until(
                    view,
                    &baseline,
                    binary32(&d.baseline_commitment)?,
                    deadline,
                )?;
            } else {
                decoder.verify_baseline(
                    view,
                    &baseline,
                    binary32(&d.baseline_commitment)?,
                    None,
                )?;
            }
        } else if self.epoch != 1 {
            return Err(OwnerFault::Invalid.into());
        }
        let mut updates = Vec::new();
        let mut own_updates: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
        let mut signing_keys: BTreeMap<String, [u8; 32]> = BTreeMap::new();
        let mut owner_provenance: BTreeMap<String, bool> = BTreeMap::new();
        let mut replaced = BTreeSet::new();
        let local_writer = key.local_writer()?.0;
        let mut tally = crate::page::UsageCounts::default();
        for (index, stored) in &self.objects {
            let opened = self.open_object(key, page, *index, stored)?;
            if opened.namespace == "own" {
                // Authenticate every original envelope before a candidate replaces its
                // structs. Ambiguous writer provenance never grants status authority.
                owner_provenance
                    .entry(opened.author_device.clone())
                    .and_modify(|owner| *owner &= opened.owner_device)
                    .or_insert(opened.owner_device);
            }
            let plaintext = if let Some(merged) = replacements.get(index) {
                if !replaced.insert(*index) {
                    continue;
                }
                merged.clone()
            } else {
                opened.plaintext
            };
            tally.add(
                stored.checkpoint,
                opened.author_device == local_writer,
                plaintext.len(),
            )?;
            if opened.namespace == "content" {
                updates.push(plaintext);
            } else {
                signing_keys.insert(opened.author_device.clone(), opened.key_bytes);
                own_updates
                    .entry(opened.author_device)
                    .or_default()
                    .push(plaintext);
            }
        }
        if replaced.len() != replacements.len() {
            return Err(OwnerFault::Invalid.into());
        }
        Ok(MaterializationInput {
            baseline,
            updates,
            own_updates,
            signing_keys,
            owner_provenance,
            local_writer,
            tally,
        })
    }
}
/// Checked independently of allocation size, for raw admission and gzip's fastpath.
fn checked_bytes(mut lengths: impl Iterator<Item = usize>) -> Result<usize> {
    lengths
        .try_fold(0usize, usize::checked_add)
        .ok_or_else(|| OwnerFault::Capacity.into())
}
impl MaterializationInput {
    /// The page's usage: the counts of the stored objects and the state a browser loads. A state
    /// under [`GZIP_MEASURED_FROM`] is not gzipped: its raw size bounds the gzipped one by 5% of
    /// the budget, and a small page should not pay a compression on every read. `measure` is false
    /// for the callers that only need the counts.
    fn usage(&self, measure: bool) -> Result<crate::page::PageUsage> {
        let parts = || {
            std::iter::once(self.baseline.as_slice())
                .chain(self.updates.iter().map(Vec::as_slice))
                .chain(self.own_updates.values().flatten().map(Vec::as_slice))
        };
        let state = checked_bytes(parts().map(<[u8]>::len))?;
        let gzip = (measure && state >= GZIP_MEASURED_FROM)
            .then(|| gzip_bytes(parts()))
            .transpose()?;
        Ok(crate::page::PageUsage::new(self.tally, state, gzip))
    }
    fn admit_deltas(&self, page: &str, count: usize, bytes: usize) -> Result<()> {
        let count = self
            .tally
            .updates
            .checked_add(count)
            .ok_or(OwnerFault::Capacity)?;
        let bytes = self
            .tally
            .tail_bytes
            .checked_add(bytes)
            .ok_or(OwnerFault::Capacity)?;
        let tail_full =
            count > crate::decoder::WRITE_TAIL_UPDATES || bytes > crate::decoder::WRITE_TAIL_BYTES;
        let detail = if count > crate::decoder::WRITE_TAIL_UPDATES {
            Some(format!(
                "this edit would take its changes to {}, more than the {} one page can hold",
                count,
                crate::decoder::WRITE_TAIL_UPDATES
            ))
        } else if bytes > crate::decoder::WRITE_TAIL_BYTES {
            Some(format!(
                "this edit would take its changes to {} ({bytes} bytes), more than the {} ({} bytes) one page can hold",
                size(bytes),
                size(crate::decoder::WRITE_TAIL_BYTES),
                crate::decoder::WRITE_TAIL_BYTES
            ))
        } else if self.baseline.len() > crate::decoder::BASELINE_BYTES {
            Some(format!(
                "its content is {}, the most one page can hold is {}",
                size(self.baseline.len()),
                size(crate::decoder::BASELINE_BYTES)
            ))
        } else {
            None
        };
        if let Some(detail) = detail {
            return Err(if tail_full {
                OwnerFault::tail_full(page, detail)
            } else {
                OwnerFault::too_large_to_edit(page, detail)
            }
            .into());
        }
        Ok(())
    }
    fn admit_gzip<'a>(
        &'a self,
        page: &str,
        deltas: impl Iterator<Item = &'a [u8]> + Clone,
    ) -> Result<()> {
        let parts = std::iter::once(self.baseline.as_slice())
            .chain(self.updates.iter().map(Vec::as_slice))
            .chain(self.own_updates.values().flatten().map(Vec::as_slice))
            .chain(deltas);
        let after = checked_bytes(parts.clone().map(<[u8]>::len))?;
        if after > crate::decoder::PAGE_BUDGET_GZIP_BYTES
            && let Some(compressed) = gzip_over_budget(parts)?
        {
            return Err(OwnerFault::too_large_to_edit(page, format!("its content would be {} ({} compressed), more than the {} one page can hold compressed", size(after), size(compressed), size(crate::decoder::PAGE_BUDGET_GZIP_BYTES))).into());
        }
        Ok(())
    }
    fn materialize(
        &self,
        page: &str,
        decoder: &mut Decoder,
        deadline: Option<Instant>,
    ) -> Result<View> {
        let Self {
            baseline,
            updates,
            own_updates,
            signing_keys,
            owner_provenance,
            local_writer,
            ..
        } = self;
        let state =
            checked_bytes(std::iter::once(baseline.len()).chain(updates.iter().map(Vec::len)))?;
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
        let mut proposals = std::collections::BTreeSet::new();
        let mut own_views = BTreeMap::new();
        let mut local_own_update = None;
        for (writer, own) in own_updates {
            let discussion = checked_bytes(own.iter().map(Vec::len))?;
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
            let batch = UpdateBatch {
                namespace: Namespace::Own,
                baseline: &[],
                updates: &refs,
            };
            let decoded = if let Some(deadline) = deadline {
                decoder.decode_until(batch, Role::Commenter, None, deadline)
            } else {
                decoder.decode(batch, Role::Commenter, None)
            }?;
            threads += decoded.projection["threads"]
                .as_object()
                .ok_or(OwnerFault::Invalid)?
                .len();
            if threads > 1000 {
                return Err(OwnerFault::Capacity.into());
            }
            for record in decoded.projection["threads"]
                .as_object()
                .ok_or(OwnerFault::Invalid)?
                .values()
            {
                if record["kind"] == "thread"
                    && let Some(id) = record["proposal"]["proposalId"].as_str()
                {
                    proposals.insert((writer.clone(), id.to_owned()));
                }
            }
            if proposals.len() > crate::limits::PAGE_PROPOSALS {
                return Err(OwnerFault::too_large(
                    page,
                    format!(
                        "it retains {} proposals, more than the maximum {}",
                        proposals.len(),
                        crate::limits::PAGE_PROPOSALS
                    ),
                )
                .into());
            }
            if writer == local_writer {
                local_own_update = Some(decoded.merged);
            }
            own_views.insert(writer.clone(), decoded.projection);
        }
        let refs = updates.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let batch = UpdateBatch {
            namespace: Namespace::Content,
            baseline,
            updates: &refs,
        };
        let folded = (if let Some(deadline) = deadline {
            decoder.decode_until(batch, Role::Editor, None, deadline)
        } else {
            decoder.decode(batch, Role::Editor, None)
        })
        .map_err(|fault| match fault {
            // The decoder's own deadline is the containment; say which page hit it.
            DecodeFault::Invoke(ref invoke) if invoke.kind == tmt_invoke::FailureKind::Deadline => {
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
        })?;
        Ok(View {
            original_author: folded.projection["meta"]["originalAuthor"]
                .as_str()
                .map(str::to_owned),
            creation_recipient: folded.projection["meta"]
                .get("creationRecipient")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?,
            publisher_agent: folded.projection["meta"]["publisherAgent"]
                .as_str()
                .map(str::to_owned),
            memory_limit: folded.memory_limit,
            usage: self.usage(false)?,
            own: own_views,
            local_own_update,
            status_writers: owner_provenance
                .iter()
                .filter(|(_, owner)| **owner)
                .map(|(writer, _)| writer.clone())
                .collect(),
            signing_keys: signing_keys.clone(),
            meta: folded.projection["meta"].clone(),
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
#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn the_page_budget_counts_compressed_bytes_not_raw_ones() {
        // Text that compresses well is far under the budget at any raw size we allow.
        let repetitive = "<p>lorem ipsum dolor sit amet</p>\n".repeat(400_000);
        assert!(repetitive.len() > 10_000_000);
        assert_eq!(
            gzip_over_budget([repetitive.as_bytes()].into_iter()).unwrap(),
            None
        );
        // Incompressible bytes pass the budget only below it.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut noise = |n: usize| {
            (0..n)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect::<Vec<u8>>()
        };
        let under = noise(4_000_000);
        assert_eq!(
            gzip_over_budget([under.as_slice()].into_iter()).unwrap(),
            None
        );
        let (first, second) = (noise(3_000_000), noise(2_500_000));
        let over = gzip_over_budget([first.as_slice(), second.as_slice()].into_iter()).unwrap();
        assert!(over.is_some_and(|n| n > 5_000_000), "{over:?}");
    }
}

#[cfg(test)]
mod tests;
