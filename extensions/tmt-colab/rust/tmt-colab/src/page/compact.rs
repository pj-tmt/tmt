//! Self-compaction of the local writer's own stream (#1627). The device merges only its own
//! verified updates into one checkpoint per namespace at the stream's current head, so a page
//! edited many times by this device keeps a short tail. Other devices' streams are never touched,
//! and the shared document is never re-encoded: the merge is `merge_updates_v1` of this stream's
//! updates inside the isolated decoder.
use super::{
    Fault, PageUsage, hex,
    owner_fold::{self, Folded, Left},
    snapshot,
};
use crate::{
    Result,
    decoder::{Decoder, Namespace as DecoderNamespace},
    keyring::Keyring,
    store::{Envelope, Fault as StoreFault, Namespace, Store, StreamScope},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tmt_colab_model::{crypto, object};

/// A device compacts its own stream once this many updates sit after its last checkpoint.
pub const TRIGGER_UPDATES: usize = 50;
/// ...or once their plaintext adds up to this many bytes.
pub const TRIGGER_BYTES: usize = 1024 * 1024;

/// When a device's own tail is long enough to combine, and until when it may still publish.
#[derive(Clone, Copy)]
pub struct Trigger {
    pub updates: usize,
    pub bytes: usize,
    /// A combine that is still working at this instant publishes nothing, so a caller that
    /// answers by then never sees the page change after its answer.
    pub until: Option<std::time::Instant>,
}
impl Trigger {
    pub(crate) fn late(&self) -> bool {
        self.until
            .is_some_and(|until| std::time::Instant::now() >= until)
    }
}
impl Default for Trigger {
    fn default() -> Self {
        Self {
            updates: TRIGGER_UPDATES,
            bytes: TRIGGER_BYTES,
            until: None,
        }
    }
}

/// What one compaction published.
#[derive(Debug, PartialEq, Eq)]
pub struct Compacted {
    /// The shared stream position the checkpoints cover (1..=through).
    pub through: u64,
    /// Updates (and earlier checkpoints) merged away, over both namespaces.
    pub merged_objects: usize,
    /// Namespaces that now have a checkpoint at `through`.
    pub namespaces: usize,
}

/// Finish a deferred native content write's combine, never a cold creation or another
/// writer's tail. The offline caller holds its lifecycle lock through the next preparation.
pub fn deferred_write(
    store: &mut Store,
    key: &Keyring,
    page: &str,
    decoder: &mut Decoder,
) -> Result<Option<Compacted>> {
    let (stream, ..) = key.local_writer()?;
    let epoch = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        tx.current_epoch(page)
    })?;
    // Reuse bootstrap's paired-prefix selection; a lone namespace checkpoint is
    // not evidence that a previous write's combine finished.
    let checkpoint = store
        .namespace_next(
            StreamScope {
                page,
                epoch,
                stream: &stream,
            },
            crate::store::Namespace::Content,
            crate::store::NamespaceCursor::default(),
        )?
        .filter(|object| object.checkpoint)
        .map_or(0, |object| object.cursor.seq);
    let pending = store.owner_read(&key.space_id, &key.owner_public(), |tx| {
        tx.uncombined_content_write(page, epoch, &stream, checkpoint)
    })?;
    if !pending {
        return Ok(None);
    }
    compact(store, key, page, decoder, Trigger::default())
}

/// Combine the local writer's own stream if its tail passed `trigger`. Safe to repeat: a
/// checkpoint that already exists at the head is kept, and a crash between the two namespaces
/// leaves data intact (an unpaired checkpoint prunes nothing) and is completed by the next call.
pub fn compact(
    store: &mut Store,
    key: &Keyring,
    page: &str,
    decoder: &mut Decoder,
    trigger: Trigger,
) -> Result<Option<Compacted>> {
    let s = snapshot(store, key, page, true)?;
    let (id, _, _) = key.local_writer()?;
    let own: Vec<usize> = (0..s.cuts.len())
        .filter(|index| s.cuts[*index].stream == id)
        .collect();
    let head = own
        .iter()
        .map(|index| s.cuts[*index].tail_seq)
        .max()
        .unwrap_or(0);
    if head == 0 {
        return Ok(None);
    }
    let Some(head_hash) = own
        .iter()
        .map(|index| &s.cuts[*index])
        .find(|cut| cut.tail_seq == head)
        .map(|cut| cut.tail_hash)
    else {
        return Ok(None);
    };
    // The tail this device would combine: its objects after each namespace's checkpoint.
    let mut tail_updates = 0;
    let mut tail_bytes = 0;
    for (index, stored) in &s.objects {
        if own.contains(index) && !stored.checkpoint {
            tail_updates += 1;
            tail_bytes += stored.bytes.len() / 4 * 3;
        }
    }
    if tail_updates == 0 || (tail_updates < trigger.updates && tail_bytes < trigger.bytes) {
        return Ok(None);
    }
    let scope = StreamScope {
        page,
        epoch: s.epoch,
        stream: &id,
    };
    let mut namespaces = 0;
    let mut pending = Vec::new();
    for (name, namespace) in [("content", Namespace::Content), ("own", Namespace::Own)] {
        let Some(cut_index) = own.iter().copied().find(|i| s.cuts[*i].namespace == name) else {
            continue;
        };
        if s.cuts[cut_index].checkpoint_seq == head {
            namespaces += 1;
            continue;
        }
        let objects: Vec<_> = s.objects.iter().filter(|(i, _)| *i == cut_index).collect();
        if objects.is_empty() {
            continue;
        }
        let mut plaintexts = Vec::with_capacity(objects.len());
        for (index, stored) in &objects {
            plaintexts.push(s.open_object(key, page, *index, stored)?.plaintext);
        }
        let refs = plaintexts.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let merged = decoder.merge(
            match namespace {
                Namespace::Content => DecoderNamespace::Content,
                Namespace::Own => DecoderNamespace::Own,
            },
            &refs,
            None,
        )?;
        if merged.len() > object::MAX_PLAINTEXT {
            // Too large to carry as one object; leave the stream as it is.
            return Ok(None);
        }
        pending.push((name, namespace, cut_index, objects.len(), merged));
        namespaces += 1;
        if trigger.late() {
            return Ok(None);
        }
    }
    if namespaces == 0 {
        return Err(Fault::Missing.into());
    }
    if !pending.is_empty() {
        // A merge can be valid update-v1 yet omit or change content. Compare every projection
        // before the first checkpoint can make a paired prefix eligible for pruning.
        let current = s.materialize(key, page, decoder)?;
        if trigger.late() {
            return Ok(None);
        }
        let replacements = pending
            .iter()
            .map(|(_, _, index, _, merged)| (*index, merged.clone()))
            .collect::<BTreeMap<_, _>>();
        let candidate = match s.materialize_with_replacements(key, page, decoder, &replacements) {
            Ok(candidate) => candidate,
            Err(_) => return Ok(None),
        };
        if current.source != candidate.source
            || current.meta != candidate.meta
            || current.own != candidate.own
        {
            return Ok(None);
        }
    }
    // Everything above only read; from here the page changes.
    if trigger.late() {
        return Ok(None);
    }
    let mut merged_objects = 0;
    for (name, namespace, _, count, merged) in pending {
        let envelope = key.seal_content(
            &object::Context {
                space: key.space_id.clone(),
                page: page.into(),
                epoch: s.epoch.to_string(),
                kind: "checkpoint".into(),
                namespace: name.into(),
                author_device: id.clone(),
                membership_revision: s.authority.head.revision.to_string(),
                stream_seq: head.to_string(),
                prev_hash: head_hash,
            },
            &s.secret,
            &merged,
        )?;
        let published = store.checkpoint(&Envelope {
            scope,
            namespace,
            seq: head,
            hash: envelope.hash()?,
            previous: head_hash,
            bytes: &envelope.to_json()?,
        });
        match published {
            Ok(_) => {}
            // Another run already published this checkpoint; keep it.
            Err(StoreFault::Conflict | StoreFault::StaleCheckpoint) => {}
            Err(other) => return Err(other.into()),
        }
        merged_objects += count;
    }
    Ok(Some(Compacted {
        through: head,
        merged_objects,
        namespaces,
    }))
}

/// Why a requested compaction published nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Skipped {
    /// This device has no changes after its checkpoints; nothing to combine.
    NothingToCombine,
    /// This device has changes to combine, but the combine could not be published (the merge
    /// would be too large for one checkpoint, or it did not finish in time).
    NotCombined,
}

/// What a requested compaction did, for the person who asked: whose stream was combined into
/// which range, the page's usage before and after, and the source hash that did not change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Report {
    pub page_id: String,
    pub compacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<Skipped>,
    /// The device whose own stream was combined: this one.
    pub by: String,
    /// The range of that stream the checkpoints now hold: 1..=through (0 when nothing was combined).
    pub through: u64,
    /// Changes merged away, in this device's stream and in the streams folded below.
    pub merged_objects: usize,
    /// Other devices' streams combined into owner folds, each with the range it now covers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folded: Vec<Folded>,
    /// Streams with changes this run left as they are, each with the reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<Left>,
    /// Streams with changes this run did not reach; running it again continues.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unfolded: usize,
    pub source_sha256: String,
    pub before: PageUsage,
    pub after: PageUsage,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Combine this device's own stream now, whatever the size of its tail, then fold the streams of
/// other owner-member devices (`owner_fold`), and report what changed. Each step compares every
/// projection of the merge with the page's own before its first checkpoint, so the source is the
/// same before and after or nothing is published. Repeating it finds no tail and is a no-op.
/// `after` shows what remains: streams no fold can combine are named in `left`.
pub fn request(
    store: &mut Store,
    key: &Keyring,
    page: &str,
    decoder: &mut Decoder,
    until: Option<std::time::Instant>,
) -> Result<Report> {
    let by = key.local_writer()?.0;
    let usage_before = snapshot(store, key, page, true)?.usage(key, page, decoder)?;
    let trigger = Trigger {
        updates: 1,
        bytes: usize::MAX,
        until,
    };
    let done = compact(store, key, page, decoder, trigger)?;
    let folds = owner_fold::fold_others(store, key, page, decoder, trigger)?;
    let after = snapshot(store, key, page, true)?;
    let usage_after = after.usage(key, page, decoder)?;
    let source_sha256 = hex(&crypto::digest(
        after.materialize(key, page, decoder)?.source.as_bytes(),
    ));
    let own_tail = usage_before.updates - usage_before.other_device_updates;
    let compacted = done.is_some() || !folds.folded.is_empty();
    // A device that only it can combine is no work left undone; the rest of what was left is.
    let unreached = folds
        .left
        .iter()
        .any(|left| left.why != owner_fold::Why::NotOwnerDevice)
        || folds.unfolded > 0;
    Ok(Report {
        page_id: page.into(),
        compacted,
        skipped: match (compacted, own_tail, unreached) {
            (true, ..) => None,
            (false, 0, false) => Some(Skipped::NothingToCombine),
            (false, ..) => Some(Skipped::NotCombined),
        },
        by,
        through: done.as_ref().map_or(0, |done| done.through),
        merged_objects: done.as_ref().map_or(0, |done| done.merged_objects)
            + folds.folded.iter().map(|f| f.merged_objects).sum::<usize>(),
        folded: folds.folded,
        left: folds.left,
        unfolded: folds.unfolded,
        source_sha256,
        before: usage_before,
        after: usage_after,
    })
}
