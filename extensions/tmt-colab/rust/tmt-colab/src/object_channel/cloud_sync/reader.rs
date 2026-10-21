//! Reads of the active page from the cloud: its signed authority, discovered and reassembled,
//! and the log tail of one foreign stream. Everything returned is exact original bytes that
//! still has to be admitted; no field read here is trusted.
use super::{
    PageRef,
    publisher::{Fault, refusal},
};
use crate::{
    SealedClient,
    sealed_documents::{self, Resource, originals},
};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, time::Instant};
use tmt_extension_objects::sync;

/// One discovery run reads at most this many documents, which is a page's declared entry cap.
const RUN_DOCUMENTS: usize = 1024;

/// What a tail query found after the position it was given.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Tail {
    /// The next contiguous originals, oldest first, with their sequences. Empty when caught up.
    Originals(Vec<(u64, Vec<u8>)>),
    /// The provider no longer holds the sequence after the position: only a bootstrap helps.
    CatchUpRequired,
    /// A record of this stream disagrees with its own routing or signed bytes. That is this
    /// writer's doing: the stream is refused, and no other stream is.
    Malformed,
}

/// One complete checkpoint or fold object found in the cloud, still to be admitted.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// The stream it covers: a fold's covered device, else its author.
    pub stream: String,
    pub namespace: String,
    pub fold: bool,
    pub seq: u64,
    /// The update hash at `seq` that it binds.
    pub previous: [u8; 32],
    pub bytes: Vec<u8>,
}
/// Every checkpoint object of a page the cloud holds whole, from one discovery run. A record that
/// cannot be read as one belongs to no stream and is left out.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Checkpoints {
    pub found: Vec<Found>,
}
/// What the cloud holds for the stream a bootstrap was asked for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Boot {
    /// The latest pair of the stream, content then own, as exact originals; and, for a fold, the
    /// exact update it binds.
    Pair {
        pair: Vec<Vec<u8>>,
        anchor: Option<Vec<u8>>,
    },
    /// Only part of the latest pair is in the cloud yet: the next change of the checkpoints asks
    /// again.
    Waiting,
    /// The cloud holds nothing this stream can start from.
    Unavailable,
    /// A record of this stream disagrees with its routing or with the other half of its pair.
    Malformed,
}

pub(crate) struct Reader {
    client: SealedClient,
    binding: String,
}
fn malformed() -> Fault {
    Fault::Stuck("A cloud record is malformed.")
}
impl Reader {
    pub(crate) fn new(client: SealedClient, binding: String) -> Self {
        Self { client, binding }
    }
    fn request(&self, call: sync::Call) -> Result<sync::Outcome, Fault> {
        self.client
            .sealed_request(call, Instant::now() + super::REQUEST_BOUND)
            .map(|(_, outcome)| outcome)
            .map_err(|_| Fault::Retry)
    }
    /// Every document of one discovery run, in the provider's order, within the page cap.
    fn discover(
        &self,
        resource: &str,
        name: &str,
        scope: Map<String, Value>,
    ) -> Result<Vec<sync::Document>, Fault> {
        let mut documents = Vec::new();
        let mut continuation = None;
        loop {
            let outcome = self.request(sync::Call::Query {
                binding: self.binding.clone(),
                query: sync::Query::Discovery {
                    resource: resource.into(),
                    name: name.into(),
                    scope: scope.clone(),
                    continuation: continuation.take(),
                },
            })?;
            let sync::Outcome::Discovery {
                documents: page,
                continuation: next,
            } = outcome
            else {
                return match outcome {
                    sync::Outcome::Error { error } => Err(refusal(&error)),
                    _ => Err(Fault::Retry),
                };
            };
            documents.extend(page);
            if documents.len() > RUN_DOCUMENTS {
                return Err(Fault::Stuck(
                    "This page has more cloud records than the limit allows.",
                ));
            }
            match next {
                Some(next) => continuation = Some(next),
                None => return Ok(documents),
            }
        }
    }
    /// The exact signed originals of one authority kind, each reassembled from its pieces.
    /// The order is the provider's, which says nothing about revision: callers sort what they
    /// verify.
    pub(crate) fn authority(&self, page: &PageRef, kind: &str) -> Result<Vec<Vec<u8>>, Fault> {
        let scope =
            sealed_documents::authority_scope(&page.space, &page.page, page.epoch, kind, "")
                .map_err(|_| malformed())?;
        let mut subjects: BTreeMap<String, Vec<(u64, sync::Document)>> = BTreeMap::new();
        for document in self.discover("authority", "records", scope)? {
            let fields = sealed_documents::decode(Resource::Authority, &document)
                .map_err(|_| malformed())?;
            let (Some(sealed_documents::Field::String(subject)), Some(seq)) =
                (fields.get("subject"), fields.get("seq"))
            else {
                return Err(malformed());
            };
            let sealed_documents::Field::Integer(seq) = seq else {
                return Err(malformed());
            };
            let seq: u64 = seq.parse().map_err(|_| malformed())?;
            subjects
                .entry(subject.clone())
                .or_default()
                .push((seq, document));
        }
        let mut out = Vec::new();
        for (_, mut pieces) in subjects {
            pieces.sort_by_key(|(seq, _)| *seq);
            let pieces: Vec<_> = pieces.into_iter().map(|(_, document)| document).collect();
            let (found, bytes) = originals::authority(&pieces).map_err(|_| malformed())?;
            let expected = match kind {
                "membership" => originals::Authority::Membership,
                "chain" => originals::Authority::Chain,
                _ => originals::Authority::Wrap,
            };
            if found != expected {
                return Err(malformed());
            }
            out.push(bytes);
        }
        Ok(out)
    }
    /// The document with this exact ID, if the provider holds it.
    fn read(&self, resource: &str, id: &str) -> Result<Option<sync::Document>, Fault> {
        match self.request(sync::Call::Read {
            binding: self.binding.clone(),
            target: sync::Target {
                resource: resource.into(),
                id: id.into(),
            },
        })? {
            sync::Outcome::Read { document } => Ok(document),
            // Colab's Rules allow a get only on a document that exists, so the provider refuses
            // the get of an absent one, and a refusal here reads as absent.
            sync::Outcome::Error { error } if error.code == sync::Code::Denied => Ok(None),
            sync::Outcome::Error { error } => Err(refusal(&error)),
            _ => Err(Fault::Retry),
        }
    }
    /// Every checkpoint object of the page that the cloud holds whole, in one discovery run. An
    /// object whose chunks are still being written is not listed: the publisher confirms only a
    /// whole one, and the next change of the checkpoints brings the rest.
    pub(crate) fn checkpoints(&self, page: &PageRef) -> Result<Checkpoints, Fault> {
        let scope = sealed_documents::page_scope(&page.space, &page.page, page.epoch)
            .map_err(|_| malformed())?;
        let mut objects: BTreeMap<String, Vec<(u64, u64, sync::Document)>> = BTreeMap::new();
        for document in self.discover("checkpoints", "objects", scope)? {
            let Ok(fields) = sealed_documents::decode(Resource::Checkpoints, &document) else {
                continue;
            };
            let (
                Some(sealed_documents::Field::String(object)),
                Some(sealed_documents::Field::Integer(index)),
                Some(sealed_documents::Field::Integer(count)),
            ) = (
                fields.get("object"),
                fields.get("index"),
                fields.get("count"),
            )
            else {
                continue;
            };
            let (Ok(index), Ok(count)) = (index.parse::<u64>(), count.parse::<u64>()) else {
                continue;
            };
            objects
                .entry(object.clone())
                .or_default()
                .push((index, count, document));
        }
        let mut found = Vec::new();
        for (_, mut chunks) in objects {
            chunks.sort_by_key(|(index, ..)| *index);
            let partial = (chunks.len() as u64) < chunks[0].1
                && chunks
                    .iter()
                    .enumerate()
                    .all(|(index, chunk)| chunk.0 == index as u64);
            if partial {
                continue;
            }
            let documents: Vec<_> = chunks.into_iter().map(|(.., document)| document).collect();
            let Ok(original) = originals::checkpoint(&documents) else {
                continue;
            };
            let context = &original.header.context;
            let Ok(seq) = context.stream_seq.parse::<u64>() else {
                continue;
            };
            found.push(Found {
                stream: original
                    .header
                    .covered_device
                    .clone()
                    .unwrap_or_else(|| context.author_device.clone()),
                namespace: context.namespace.clone(),
                fold: original.header.covered_device.is_some(),
                seq,
                previous: context.prev_hash,
                bytes: original.bytes,
            });
        }
        Ok(Checkpoints { found })
    }
    /// What `stream` can start from, out of the page's checkpoints: its latest pair of both
    /// namespaces at one sequence and head, and for a fold the update it binds, read by exact ID
    /// from the log. Nothing here is admitted.
    pub(crate) fn bootstrap(
        &self,
        page: &PageRef,
        checkpoints: &Checkpoints,
        stream: &str,
    ) -> Result<Boot, Fault> {
        let mine: Vec<&Found> = checkpoints
            .found
            .iter()
            .filter(|found| found.stream == stream)
            .collect();
        let Some(latest) = mine.iter().map(|found| found.seq).max() else {
            return Ok(Boot::Unavailable);
        };
        let at: Vec<&Found> = mine.into_iter().filter(|f| f.seq == latest).collect();
        let content = at.iter().find(|found| found.namespace == "content");
        let own = at.iter().find(|found| found.namespace == "own");
        let (Some(content), Some(own)) = (content, own) else {
            return Ok(if at.len() == 1 {
                Boot::Waiting
            } else {
                Boot::Malformed
            });
        };
        if at.len() != 2 || content.previous != own.previous {
            return Ok(Boot::Malformed);
        }
        let anchor = if content.fold || own.fold {
            let id = format!(
                "{}_{}_{}_{stream}_{latest}",
                page.space, page.page, page.epoch
            );
            let Some(document) = self.read("log", &id)? else {
                return Ok(Boot::Unavailable);
            };
            let Ok(original) = originals::log(&document) else {
                return Ok(Boot::Malformed);
            };
            let context = &original.header.context;
            if context.author_device != stream
                || context.page != page.page
                || context.stream_seq != latest.to_string()
            {
                return Ok(Boot::Malformed);
            }
            Some(original.bytes)
        } else {
            None
        };
        Ok(Boot::Pair {
            pair: vec![content.bytes.clone(), own.bytes.clone()],
            anchor,
        })
    }
    /// The contiguous log originals of `stream` after `after`, at most one provider page.
    pub(crate) fn tail(&self, page: &PageRef, stream: &str, after: u64) -> Result<Tail, Fault> {
        let scope = sealed_documents::stream_scope(&page.space, &page.page, page.epoch, stream)
            .map_err(|_| malformed())?;
        let outcome = self.request(sync::Call::Query {
            binding: self.binding.clone(),
            query: sync::Query::Stream {
                resource: "log".into(),
                scope,
                stream: stream.into(),
                cursor: Some(sync::StreamPosition {
                    stream: stream.into(),
                    seq: after,
                }),
            },
        })?;
        let documents = match outcome {
            sync::Outcome::Stream { documents, .. } => documents,
            sync::Outcome::Error { error } if error.state == Some(sync::State::CatchUpRequired) => {
                return Ok(Tail::CatchUpRequired);
            }
            sync::Outcome::Error { error } => return Err(refusal(&error)),
            _ => return Err(Fault::Retry),
        };
        let mut originals_out = Vec::new();
        for (index, document) in documents.iter().enumerate() {
            let Ok(original) = originals::log(document) else {
                return Ok(Tail::Malformed);
            };
            let context = &original.header.context;
            let seq = after + index as u64 + 1;
            if context.author_device != stream
                || context.page != page.page
                || context.stream_seq != seq.to_string()
            {
                return Ok(Tail::Malformed);
            }
            originals_out.push((seq, original.bytes));
        }
        Ok(Tail::Originals(originals_out))
    }
}
#[cfg(test)]
mod tests;
