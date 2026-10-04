//! Self-compaction of the local writer's own stream (#1627). The device merges only its own
//! verified updates into one checkpoint per namespace at the stream's current head, so a page
//! edited many times by this device keeps a short tail. Other devices' streams are never touched,
//! and the shared document is never re-encoded: the merge is `merge_updates_v1` of this stream's
//! updates inside the isolated decoder.
use super::{Fault, snapshot};
use crate::{
    Result,
    decoder::{Decoder, Namespace as DecoderNamespace},
    keyring::Keyring,
    store::{Envelope, Fault as StoreFault, Namespace, Store, StreamScope},
};
use tmt_colab_model::object;

/// A device compacts its own stream once this many updates sit after its last checkpoint.
pub const TRIGGER_UPDATES: usize = 50;
/// ...or once their plaintext adds up to this many bytes.
pub const TRIGGER_BYTES: usize = 1024 * 1024;

/// When a device's own tail is long enough to combine.
#[derive(Clone, Copy)]
pub struct Trigger {
    pub updates: usize,
    pub bytes: usize,
}
impl Default for Trigger {
    fn default() -> Self {
        Self {
            updates: TRIGGER_UPDATES,
            bytes: TRIGGER_BYTES,
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
    let mut merged_objects = 0;
    let mut namespaces = 0;
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
        merged_objects += objects.len();
        namespaces += 1;
    }
    if namespaces == 0 {
        return Err(Fault::Missing.into());
    }
    Ok(Some(Compacted {
        through: head,
        merged_objects,
        namespaces,
    }))
}
