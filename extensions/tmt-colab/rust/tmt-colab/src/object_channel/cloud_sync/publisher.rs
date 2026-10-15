//! Outbound publication of one active page: the rows and signed authority every reader needs
//! first, then each local original in sequence. Progress moves only after the exact original
//! is confirmed or read back identical; an uncertain effect is retried on the same ID and
//! bytes, never on a new one.
use super::PageRef;
use crate::SealedClient;
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use tmt_extension_objects::sync;

/// The documents one pass may send before it yields; the next wake continues.
const PASS_DOCUMENTS: usize = 64;

/// Why a pass stopped before the page was fully published.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    /// No answer, or an effect that may or may not have happened: nothing advanced, and the
    /// same originals are tried again later.
    Retry,
    /// Remote's budget guard refused: do not call again before this many milliseconds pass.
    Budget(u64),
    /// The page is gone or at another epoch than the one asked for. Nothing is retried: the
    /// next touch of the page names its current epoch.
    Moved,
    /// Rules refused, an immutable original conflicts, or local state is unusable. Publication
    /// of this page stops until restart; the line is what a person may be told.
    Stuck(&'static str),
}

/// A document that must exist before the page's data.
pub(crate) enum Row {
    /// Created only when absent. An existing document with the same fields (expiry aside) is
    /// published; a different one is a conflict.
    Create(sync::Document),
    /// The owner's admission row: written whole, again when anything but its expiry changed
    /// or its lifetime in `days` did (an owner override), and when `refresh` has passed since
    /// it was last written, which is an activity refresh of the page's cloud lifetime.
    Put {
        document: sync::Document,
        days: u64,
        refresh: Duration,
    },
}
/// A link that ended: its cloud `links` row goes first, so nobody can enroll again, then the
/// member rows it enrolled, which `holders` discovery finds by link.
#[derive(Clone)]
pub(crate) struct Retirement {
    pub link: String,
    pub target: sync::Target,
    pub scope: Map<String, Value>,
}
/// What a published batch confirms in the Store.
pub(crate) enum Confirm {
    Update {
        stream: String,
        seq: u64,
        hash: [u8; 32],
    },
    Checkpoint {
        stream: String,
        seq: u64,
        content: [u8; 32],
        own: [u8; 32],
    },
}
/// One local original as the documents that carry it, with the confirmation they earn.
pub(crate) struct Data {
    pub documents: Vec<sync::Document>,
    pub confirm: Confirm,
}
/// The publisher's view of durable local state. The Store owns every position; this only
/// reads originals and records confirmations.
pub(crate) trait Local {
    /// Rows and signed authority, in the order they must exist. A stream's chain is part of it
    /// from that stream's first original on, so it is read after the original it must precede.
    fn preamble(&mut self, page: &PageRef) -> Result<Vec<Row>, Fault>;
    /// The next unconfirmed original of a local stream, or none when the page is current.
    fn next(&mut self, page: &PageRef) -> Result<Option<Data>, Fault>;
    fn confirm(&mut self, page: &PageRef, confirm: &Confirm) -> Result<(), Fault>;
    /// Links whose cloud rows may still exist although the link ended.
    fn retirements(&mut self, page: &PageRef) -> Result<Vec<Retirement>, Fault>;
    /// The link's cloud rows are gone for this page.
    fn retired(&mut self, page: &PageRef, link: &str) -> Result<(), Fault>;
}

pub(crate) struct Publisher {
    client: SealedClient,
    binding: String,
    /// Authority and admission rows created in this process.
    created: HashSet<(String, String)>,
    /// Rows written whole in this process.
    written: HashMap<(String, String), Written>,
}
/// A row written whole: when, what (expiry aside) and for how many days.
struct Written {
    at: Instant,
    content: Map<String, Value>,
    days: u64,
}
fn comparable(fields: &Map<String, Value>) -> Map<String, Value> {
    let mut fields = fields.clone();
    fields.remove("expiresAt");
    fields
}
impl Publisher {
    pub(crate) fn new(client: SealedClient, binding: String) -> Self {
        Self {
            client,
            binding,
            created: HashSet::new(),
            written: HashMap::new(),
        }
    }
    /// Publish what is due for the page. `Ok(true)` means more remains and the next wake
    /// continues; `Ok(false)` means the page is current.
    pub(crate) fn pass(&mut self, local: &mut dyn Local, page: &PageRef) -> Result<bool, Fault> {
        let (mut sent, mut prepared) = (0, false);
        loop {
            // The original is read before the rows: a stream's chain exists from its first
            // original on, so the rows read after it always include the chain it needs, even
            // when that original arrived while this pass was running.
            let next = local.next(page)?;
            if next.is_some() || !prepared {
                for row in local.preamble(page)? {
                    match row {
                        Row::Create(document) => self.create_once(document)?,
                        Row::Put {
                            document,
                            days,
                            refresh,
                        } => self.put_row(document, days, refresh)?,
                    }
                }
                prepared = true;
                // Authority is current, so nobody can still be admitted by an ended link.
                if self.retire(local, page, &mut sent)? {
                    return Ok(true);
                }
            }
            let Some(data) = next else {
                return Ok(false);
            };
            for document in data.documents {
                self.create(&document)?;
                sent += 1;
            }
            local.confirm(page, &data.confirm)?;
            if sent >= PASS_DOCUMENTS {
                return Ok(true);
            }
        }
    }
    /// Remove what ended links left in the cloud. `Ok(true)` means the pass ran out of room and
    /// the next wake continues from the same discovery.
    fn retire(
        &mut self,
        local: &mut dyn Local,
        page: &PageRef,
        sent: &mut usize,
    ) -> Result<bool, Fault> {
        for retirement in local.retirements(page)? {
            self.delete(&retirement.target)?;
            let mut continuation = None;
            loop {
                let (holders, next) = self.holders(&retirement.scope, continuation)?;
                for holder in holders {
                    self.delete(&holder.target)?;
                    *sent += 1;
                    if *sent >= PASS_DOCUMENTS {
                        return Ok(true);
                    }
                }
                // The link row is gone, so a run that ended has found every holder there is.
                let Some(next) = next else { break };
                continuation = Some(next);
            }
            local.retired(page, &retirement.link)?;
        }
        Ok(false)
    }
    /// One run of the owner's holder discovery.
    fn holders(
        &self,
        scope: &Map<String, Value>,
        continuation: Option<String>,
    ) -> Result<(Vec<sync::Document>, Option<String>), Fault> {
        match self.request(sync::Call::Query {
            binding: self.binding.clone(),
            query: sync::Query::Discovery {
                resource: "members".into(),
                name: "holders".into(),
                scope: scope.clone(),
                continuation,
            },
        })? {
            sync::Outcome::Discovery {
                documents,
                continuation,
            } => Ok((documents, continuation)),
            sync::Outcome::Error { error } => Err(refusal(&error)),
            _ => Err(Fault::Retry),
        }
    }
    /// Delete a document. Rules read the document being deleted, so a delete of one that is not
    /// there is refused like any other: a refusal reads as already gone, as it does for a get.
    fn delete(&self, target: &sync::Target) -> Result<(), Fault> {
        match self.request(sync::Call::Publish {
            binding: self.binding.clone(),
            mutation: sync::Mutation::Delete {
                target: target.clone(),
            },
        })? {
            sync::Outcome::Published {
                status: sync::Published::Confirmed | sync::Published::AlreadyPresent,
            } => Ok(()),
            sync::Outcome::Published {
                status: sync::Published::Unknown,
            } => Err(Fault::Retry),
            sync::Outcome::Error { error } if error.code == sync::Code::Denied => Ok(()),
            sync::Outcome::Error { error } => Err(refusal(&error)),
            _ => Err(Fault::Retry),
        }
    }
    fn request(&self, call: sync::Call) -> Result<sync::Outcome, Fault> {
        self.client
            .sealed_request(call, Instant::now() + super::REQUEST_BOUND)
            .map(|(_, outcome)| outcome)
            .map_err(|_| Fault::Retry)
    }
    /// What the cloud holds at a target after a create whose outcome is in doubt. Rules refuse a
    /// get of a document that is not there, so a refusal reads as absent: the create that
    /// follows reports a denial that is real.
    fn read_back(&self, target: &sync::Target) -> Result<Option<sync::Document>, Fault> {
        match self.request(sync::Call::Read {
            binding: self.binding.clone(),
            target: target.clone(),
        })? {
            sync::Outcome::Read { document } => Ok(document),
            sync::Outcome::Error { error } if error.code == sync::Code::Denied => Ok(None),
            sync::Outcome::Error { error } => Err(refusal(&error)),
            _ => Err(Fault::Retry),
        }
    }
    /// Create what is not there yet. The carrier answers an identical document as already
    /// present and reads back an expiry-only difference, so no read comes first: Rules do not
    /// allow one for a document that does not exist. Authority and admission rows are created
    /// once per process.
    fn create_once(&mut self, document: sync::Document) -> Result<(), Fault> {
        let key = (document.target.resource.clone(), document.target.id.clone());
        if self.created.contains(&key) {
            return Ok(());
        }
        self.create(&document)?;
        self.created.insert(key);
        Ok(())
    }
    fn put_row(
        &mut self,
        document: sync::Document,
        days: u64,
        refresh: Duration,
    ) -> Result<(), Fault> {
        let key = (document.target.resource.clone(), document.target.id.clone());
        let content = comparable(&document.fields);
        if self.written.get(&key).is_some_and(|row| {
            row.content == content && row.days == days && row.at.elapsed() < refresh
        }) {
            return Ok(());
        }
        match self.request(sync::Call::Publish {
            binding: self.binding.clone(),
            mutation: sync::Mutation::Put {
                document: document.clone(),
            },
        })? {
            sync::Outcome::Published {
                status: sync::Published::Confirmed | sync::Published::AlreadyPresent,
            } => {}
            sync::Outcome::Published {
                status: sync::Published::Unknown,
            } => return Err(Fault::Retry),
            sync::Outcome::Error { error } => return Err(refusal(&error)),
            _ => return Err(Fault::Retry),
        }
        self.written.insert(
            key,
            Written {
                at: Instant::now(),
                content,
                days,
            },
        );
        Ok(())
    }
    /// Create one immutable original. An acknowledged or identical result publishes it; a
    /// possible effect is read back before anything advances.
    fn create(&self, document: &sync::Document) -> Result<(), Fault> {
        match self.request(sync::Call::Publish {
            binding: self.binding.clone(),
            mutation: sync::Mutation::Create {
                document: document.clone(),
            },
        })? {
            sync::Outcome::Published {
                status: sync::Published::Confirmed | sync::Published::AlreadyPresent,
            } => Ok(()),
            sync::Outcome::Published {
                status: sync::Published::Unknown,
            } => self.reconcile(document),
            sync::Outcome::Error { error } => match refusal(&error) {
                // The carrier saw other bytes at this ID; only an expiry difference is benign.
                Fault::Stuck(_) if error.state == Some(sync::State::Conflict) => {
                    self.reconcile(document)
                }
                fault => Err(fault),
            },
            _ => Err(Fault::Retry),
        }
    }
    fn reconcile(&self, document: &sync::Document) -> Result<(), Fault> {
        match self.read_back(&document.target)? {
            Some(existing) if comparable(&existing.fields) == comparable(&document.fields) => {
                Ok(())
            }
            Some(_) => Err(Fault::Stuck(
                "The cloud copy of this page's original differs from this machine's.",
            )),
            None => Err(Fault::Retry),
        }
    }
}
/// Remote's typed refusal as the publisher's three outcomes.
pub(super) fn refusal(error: &sync::Error) -> Fault {
    match error.code {
        sync::Code::Budget | sync::Code::Rate => {
            Fault::Budget(error.retry_after_ms.unwrap_or(60_000))
        }
        sync::Code::Denied => Fault::Stuck("The cloud refused sync for this page."),
        sync::Code::Unavailable => match error.state {
            Some(sync::State::Conflict) => {
                Fault::Stuck("Two copies of this page's original disagree in the cloud.")
            }
            _ => Fault::Retry,
        },
        sync::Code::Invalid => Fault::Stuck("The page original in the cloud is not valid."),
    }
}

#[cfg(test)]
mod tests;
