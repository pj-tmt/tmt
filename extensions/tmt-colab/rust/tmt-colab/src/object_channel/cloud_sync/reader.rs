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
                return Err(Fault::Stuck("A page has more cloud records than it may."));
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
