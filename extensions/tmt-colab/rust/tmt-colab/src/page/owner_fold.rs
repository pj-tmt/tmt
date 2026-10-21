//! The owner's fold (#3983, colab-v1 "Owner fold"): a checkpoint this machine's local writer signs
//! for another owner-member device's stream, so `page compact` can clear a tail that only that
//! device could combine. It merges the covered stream's verified updates 1..n per namespace, the
//! way a device combines its own stream (`compact::compact`), and publishes them as folds only
//! after the source, the metadata and every writer's own projection are the same with the folds
//! as without them.
//!
//! A step is bounded: at most [`MERGES_PER_RUN`] namespace merges and the caller's deadline; the
//! streams it did not reach are counted in [`Folds::unfolded`] and the next run continues.
use super::{compact::Trigger, snapshot};
use crate::{
    Result,
    decoder::{Decoder, Namespace as DecoderNamespace},
    keyring::Keyring,
    store::{
        Envelope, Fault as StoreFault, Namespace, Store, StreamScope, owner::epoch::StoredObject,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tmt_colab_model::object;

/// Namespace merges one run performs; each is one isolated decoder call.
pub const MERGES_PER_RUN: usize = 16;

/// Why a stream with changes to combine was left as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Why {
    /// A bridge, link or commenter device, or one with no admitted owner chain: only its own
    /// device can combine its changes.
    NotOwnerDevice,
    /// The combined changes would be too large for one object.
    TooLarge,
    /// The changes did not merge, or the fold did not verify, so nothing of this stream was written.
    Refused,
}

/// A stream whose changes one run combined.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Folded {
    /// The device whose stream was combined.
    pub stream: String,
    /// The fold covers changes 1..=through of that stream.
    pub through: u64,
    pub merged_objects: usize,
}

/// A stream a run left as it is, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Left {
    pub stream: String,
    pub why: Why,
}

/// What one run did for other devices' streams.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Folds {
    pub folded: Vec<Folded>,
    pub left: Vec<Left>,
    /// Streams with changes to combine that this run did not reach (its merges or its time ran out).
    pub unfolded: usize,
}

/// One stream this run may fold: its head, and the namespaces still to bring to it.
struct Candidate {
    stream: String,
    head: u64,
    head_hash: [u8; 32],
    /// `(name, namespace, cut index)` of each namespace whose checkpoint is behind the head.
    namespaces: Vec<(&'static str, Namespace, usize)>,
    anchor: Vec<u8>,
}

fn context(
    key: &Keyring,
    page: &str,
    epoch: u64,
    revision: u64,
    signer: &str,
    candidate: &Candidate,
    namespace: &str,
) -> object::Context {
    object::Context {
        space: key.space_id.clone(),
        page: page.into(),
        epoch: epoch.to_string(),
        kind: object::FOLD.into(),
        namespace: namespace.into(),
        author_device: signer.into(),
        membership_revision: revision.to_string(),
        stream_seq: candidate.head.to_string(),
        prev_hash: candidate.head_hash,
    }
}

/// The run's time ran out before it wrote anything: every eligible stream that did not end in
/// `left` is still to do, so the report says to run the command again.
fn stopped(mut folds: Folds, to_do: usize) -> Folds {
    folds.unfolded = to_do;
    folds
}

/// Fold the streams of other devices of `page` that this machine's local writer may combine, one
/// bounded step. Nothing is written unless the folded page equals the page as it is.
pub fn fold_others(
    store: &mut Store,
    key: &Keyring,
    page: &str,
    decoder: &mut Decoder,
    trigger: Trigger,
) -> Result<Folds> {
    let mut s = snapshot(store, key, page, true)?;
    let (signer, ..) = key.local_writer()?;
    let revision = s.authority.head.revision;
    let mut folds = Folds::default();

    // The streams with a tail to combine, and the namespaces still behind each one's head.
    let mut heads: BTreeMap<&str, (u64, [u8; 32])> = BTreeMap::new();
    for cut in s.cuts.iter().filter(|cut| cut.stream != signer) {
        let head = heads.entry(cut.stream.as_str()).or_insert((0, [0; 32]));
        if cut.tail_seq > head.0 {
            *head = (cut.tail_seq, cut.tail_hash);
        }
    }
    let mut candidates = Vec::new();
    for (stream, (head, head_hash)) in heads {
        let indexes: Vec<usize> = (0..s.cuts.len())
            .filter(|i| s.cuts[*i].stream == stream)
            .collect();
        let has_tail = s
            .objects
            .iter()
            .any(|(i, object)| indexes.contains(i) && !object.checkpoint);
        if head == 0 || !has_tail {
            continue;
        }
        let namespaces = [("content", Namespace::Content), ("own", Namespace::Own)]
            .into_iter()
            .filter_map(|(name, namespace)| {
                let index = indexes
                    .iter()
                    .copied()
                    .find(|i| s.cuts[*i].namespace == name)?;
                let objects = s.objects.iter().any(|(i, _)| *i == index);
                (objects && s.cuts[index].checkpoint_seq != head)
                    .then_some((name, namespace, index))
            })
            .collect::<Vec<_>>();
        if namespaces.is_empty() {
            continue;
        }
        let anchor = store.anchor(
            StreamScope {
                page,
                epoch: s.epoch,
                stream,
            },
            head,
        )?;
        let Some(anchor) = anchor else {
            folds.left.push(Left {
                stream: stream.into(),
                why: Why::Refused,
            });
            continue;
        };
        candidates.push(Candidate {
            stream: stream.into(),
            head,
            head_hash,
            namespaces,
            anchor: anchor.bytes,
        });
    }
    if candidates.is_empty() {
        return Ok(folds);
    }

    let mut eligible = Vec::new();
    for candidate in candidates {
        // The fold must verify as the readers will check it: seal an empty one for this stream and
        // open it with the anchor in place, before any merge is spent on it.
        s.with_anchor(&candidate.stream, candidate.head, candidate.anchor.clone());
        let mut why = None;
        for (name, ..) in &candidate.namespaces {
            let probe = key.seal_fold(
                &context(key, page, s.epoch, revision, &signer, &candidate, name),
                &candidate.stream,
                &s.secret,
                &[],
            )?;
            let stored = StoredObject {
                seq: candidate.head,
                hash: probe.hash()?,
                previous: candidate.head_hash,
                checkpoint: true,
                bytes: probe.to_json()?,
            };
            if s.open_stream_object(key, page, &candidate.stream, name, &stored)
                .is_err()
            {
                why = Some(
                    if s.is_owner_member_device(key, page, &candidate.stream, name) {
                        Why::Refused
                    } else {
                        Why::NotOwnerDevice
                    },
                );
                break;
            }
        }
        match why {
            Some(why) => folds.left.push(Left {
                stream: candidate.stream,
                why,
            }),
            None => eligible.push(candidate),
        }
    }

    // Merge, within the step's bounds.
    struct Pending {
        candidate: usize,
        name: &'static str,
        namespace: Namespace,
        index: usize,
        count: usize,
        merged: Vec<u8>,
    }
    let mut pending: Vec<Pending> = Vec::new();
    let mut merges = 0;
    let mut reached = Vec::new();
    // Eligible streams that ended in `left` here; every other one is either folded below or, when
    // the run stops before its writes, still to do (`stopped`).
    let mut failed = 0;
    for (position, candidate) in eligible.iter().enumerate() {
        if trigger.late() || (merges > 0 && merges + candidate.namespaces.len() > MERGES_PER_RUN) {
            folds.unfolded += 1;
            continue;
        }
        let mut merged_stream = Vec::new();
        let mut why = None;
        for (name, namespace, index) in &candidate.namespaces {
            let objects: Vec<_> = s.objects.iter().filter(|(i, _)| i == index).collect();
            let mut plaintexts = Vec::with_capacity(objects.len());
            for (i, stored) in &objects {
                plaintexts.push(s.open_object(key, page, *i, stored)?.plaintext);
            }
            let refs = plaintexts.iter().map(Vec::as_slice).collect::<Vec<_>>();
            merges += 1;
            let merged = decoder.merge(
                match namespace {
                    Namespace::Content => DecoderNamespace::Content,
                    Namespace::Own => DecoderNamespace::Own,
                },
                &refs,
                None,
            );
            match merged {
                Ok(merged) if merged.len() <= object::MAX_PLAINTEXT => {
                    merged_stream.push(Pending {
                        candidate: position,
                        name,
                        namespace: *namespace,
                        index: *index,
                        count: objects.len(),
                        merged,
                    });
                }
                Ok(_) => {
                    why = Some(Why::TooLarge);
                    break;
                }
                Err(_) => {
                    why = Some(Why::Refused);
                    break;
                }
            }
            if trigger.late() {
                return Ok(stopped(folds, eligible.len() - failed));
            }
        }
        match why {
            Some(why) => {
                failed += 1;
                folds.left.push(Left {
                    stream: candidate.stream.clone(),
                    why,
                });
            }
            None => {
                reached.push(position);
                pending.extend(merged_stream);
            }
        }
    }
    if pending.is_empty() {
        return Ok(folds);
    }

    // A merge can be valid update-v1 yet omit or change content: compare every projection before
    // the first fold can make a paired prefix eligible for pruning.
    let current = s.materialize(key, page, decoder)?;
    if trigger.late() {
        return Ok(stopped(folds, eligible.len() - failed));
    }
    let replacements = pending
        .iter()
        .map(|p| (p.index, p.merged.clone()))
        .collect::<BTreeMap<_, _>>();
    let same = match s.materialize_with_replacements(key, page, decoder, &replacements) {
        Ok(candidate) => {
            current.source == candidate.source
                && current.meta == candidate.meta
                && current.own == candidate.own
        }
        Err(_) => false,
    };
    if !same {
        for position in reached {
            folds.left.push(Left {
                stream: eligible[position].stream.clone(),
                why: Why::Refused,
            });
        }
        return Ok(folds);
    }
    // Everything above only read; from here the page changes.
    if trigger.late() {
        return Ok(stopped(folds, eligible.len() - failed));
    }
    for position in reached {
        let candidate = &eligible[position];
        let mut merged_objects = 0;
        for p in pending.iter().filter(|p| p.candidate == position) {
            let envelope = key.seal_fold(
                &context(key, page, s.epoch, revision, &signer, candidate, p.name),
                &candidate.stream,
                &s.secret,
                &p.merged,
            )?;
            let published = store.fold(
                &Envelope {
                    scope: StreamScope {
                        page,
                        epoch: s.epoch,
                        stream: &candidate.stream,
                    },
                    namespace: p.namespace,
                    seq: candidate.head,
                    hash: envelope.hash()?,
                    previous: candidate.head_hash,
                    bytes: &envelope.to_json()?,
                },
                &signer,
            );
            match published {
                Ok(_) => {}
                // Another run already published this fold or a checkpoint at this sequence.
                Err(StoreFault::Conflict | StoreFault::StaleCheckpoint) => {}
                Err(other) => return Err(other.into()),
            }
            merged_objects += p.count;
        }
        folds.folded.push(Folded {
            stream: candidate.stream.clone(),
            through: candidate.head,
            merged_objects,
        });
    }
    Ok(folds)
}
