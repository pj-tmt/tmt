//! Outbound publication of one active page: the rows and signed authority every reader needs
//! first, then each local original in sequence. Progress moves only after the exact original
//! is confirmed or read back identical; an uncertain effect is retried on the same ID and
//! bytes, never on a new one.
use super::PageRef;
use crate::SealedClient;
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tmt_extension_objects::sync;

/// The documents one pass may send before it yields; the next wake continues.
const PASS_DOCUMENTS: usize = 64;
/// A page row is rewritten after this long, so its expiry never lapses while serving.
const PAGE_ROW_REFRESH: Duration = Duration::from_secs(7 * 24 * 3600);

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
    /// The owner's admission row: written whole, again after [`PAGE_ROW_REFRESH`].
    Put(sync::Document),
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
    /// Rows and signed authority, in the order they must exist.
    fn preamble(&mut self, page: &PageRef) -> Result<Vec<Row>, Fault>;
    /// The next unconfirmed original of a local stream, or none when the page is current.
    fn next(&mut self, page: &PageRef) -> Result<Option<Data>, Fault>;
    fn confirm(&mut self, page: &PageRef, confirm: &Confirm) -> Result<(), Fault>;
}

pub(crate) struct Publisher {
    client: SealedClient,
    binding: String,
    /// Rows and authority already present or created in this process.
    present: HashMap<(String, String), Option<Instant>>,
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
            present: HashMap::new(),
        }
    }
    /// Publish what is due for the page. `Ok(true)` means more remains and the next wake
    /// continues; `Ok(false)` means the page is current.
    pub(crate) fn pass(&mut self, local: &mut dyn Local, page: &PageRef) -> Result<bool, Fault> {
        for row in local.preamble(page)? {
            match row {
                Row::Create(document) => self.create_once(document)?,
                Row::Put(document) => self.put_row(document)?,
            }
        }
        let mut sent = 0;
        while let Some(data) = local.next(page)? {
            for document in data.documents {
                self.create(&document)?;
                sent += 1;
            }
            local.confirm(page, &data.confirm)?;
            if sent >= PASS_DOCUMENTS {
                return Ok(true);
            }
        }
        Ok(false)
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
        if self.present.contains_key(&key) {
            return Ok(());
        }
        self.create(&document)?;
        self.present.insert(key, None);
        Ok(())
    }
    fn put_row(&mut self, document: sync::Document) -> Result<(), Fault> {
        let key = (document.target.resource.clone(), document.target.id.clone());
        if self
            .present
            .get(&key)
            .copied()
            .flatten()
            .is_some_and(|at| at.elapsed() < PAGE_ROW_REFRESH)
        {
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
        self.present.insert(key, Some(Instant::now()));
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
                "A cloud original conflicts with the one stored here.",
            )),
            None => Err(Fault::Retry),
        }
    }
}
/// Remote's typed refusal as the publisher's three outcomes.
fn refusal(error: &sync::Error) -> Fault {
    match error.code {
        sync::Code::Budget | sync::Code::Rate => {
            Fault::Budget(error.retry_after_ms.unwrap_or(60_000))
        }
        sync::Code::Denied => Fault::Stuck("Cloud sync was refused for this page."),
        sync::Code::Unavailable => match error.state {
            Some(sync::State::Conflict) => Fault::Stuck("A cloud original conflicts."),
            _ => Fault::Retry,
        },
        sync::Code::Invalid => Fault::Stuck("Cloud sync refused an original as invalid."),
    }
}

#[cfg(test)]
mod tests;
