//! The publisher's local side over the Store: which originals exist, in what order they are
//! due, and where confirmations are recorded. Positions live in the B2 publication metadata;
//! nothing here keeps its own copy.
use super::{
    PageRef,
    publisher::{Confirm, Data, Fault, Local, Row},
};
use crate::{
    object_channel::documents::{
        self, Field, Resource,
        originals::{self, Authority, Routing},
    },
    store::{self, Namespace, NamespaceCursor, Store, StreamScope, publication::CloudBinding},
};
use std::sync::Arc;
use tmt_colab_model::{crypto, object};

const DAY_MS: u64 = 24 * 3_600_000;
/// Entries live for this long from when they are written. Rules cap an entry at 365 days.
const ENTRY_DAYS: u64 = 360;
/// A page row defaults to 30 days when the owner set no retention.
const PAGE_DEFAULT_DAYS: u64 = 30;
/// The identity of one bound cloud client, frozen at bind.
#[derive(Clone)]
pub(crate) struct Bound {
    pub project: String,
    pub declaration_digest: String,
    pub uid: String,
}

pub(crate) struct StoreLocal {
    store: Store,
    owner: [u8; 32],
    space: String,
    bound: Bound,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
}
fn stuck_read() -> Fault {
    Fault::Stuck("Local page state could not be read for cloud sync.")
}
fn read_fault(error: &(dyn std::error::Error + 'static)) -> Fault {
    if store::contended(error) {
        Fault::Retry
    } else {
        stuck_read()
    }
}
fn store_fault(fault: store::Fault) -> Fault {
    match fault {
        store::Fault::StaleEpoch => Fault::Moved,
        store::Fault::ResyncRequired => Fault::Retry,
        store::Fault::Capacity => {
            Fault::Stuck("This page has more changes to publish than the limit allows.")
        }
        fault if store::contended(&fault) => Fault::Retry,
        _ => stuck_read(),
    }
}
fn mapping<T>(result: Result<T, documents::Invalid>) -> Result<T, Fault> {
    result.map_err(|_| Fault::Stuck("A stored original could not be mapped for cloud sync."))
}
fn text(value: &str) -> Field {
    Field::String(value.into())
}

impl StoreLocal {
    pub(crate) fn new(
        store: Store,
        owner: [u8; 32],
        space: String,
        bound: Bound,
        now: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            store,
            owner,
            space,
            bound,
            now,
        }
    }
    fn routing<'a>(&'a self, page: &'a PageRef, expires: Field) -> Routing<'a> {
        Routing {
            space: &self.space,
            page: &page.page,
            epoch: page.epoch,
            uid: &self.bound.uid,
            expires_at: expires,
        }
    }
    fn entry_expiry(&self) -> Result<Field, Fault> {
        mapping(Field::timestamp((self.now)() + ENTRY_DAYS * DAY_MS))
    }
    /// The streams of this page that this machine registered a device for, in a stable order.
    fn local_streams(&self, page: &PageRef) -> Result<Vec<String>, Fault> {
        let mut streams: Vec<String> = self
            .store
            .namespaces(&page.page, page.epoch)
            .map_err(store_fault)?
            .into_iter()
            .map(|(stream, _)| stream)
            .collect();
        streams.dedup();
        self.store
            .owner_read(&self.space, &self.owner, |tx| {
                let mut local = Vec::new();
                for stream in streams {
                    if tx.is_local_device(&stream)? {
                        local.push(stream);
                    }
                }
                Ok(local)
            })
            .map_err(|error| read_fault(error.as_ref()))
    }
    /// The paired checkpoint both namespaces hold at `seq`, split into its create-only chunks.
    fn checkpoint(&mut self, page: &PageRef, stream: &str, seq: u64) -> Result<Data, Fault> {
        let scope = StreamScope {
            page: &page.page,
            epoch: page.epoch,
            stream,
        };
        let routing = self.routing(page, self.entry_expiry()?);
        let (mut documents, mut digests) = (Vec::new(), Vec::new());
        for namespace in [Namespace::Content, Namespace::Own] {
            let object = self
                .store
                .namespace_next(scope, namespace, NamespaceCursor::default())
                .map_err(store_fault)?
                .filter(|object| object.checkpoint && object.cursor.seq == seq)
                // Compaction moved the cut between the position and this read: look again.
                .ok_or(Fault::Retry)?;
            let base = mapping(originals::checkpoint_base(&routing, &object.bytes))?;
            documents.extend(mapping(originals::checkpoint_documents(
                base,
                &object.bytes,
            ))?);
            digests.push(crypto::digest(&object.bytes));
        }
        Ok(Data {
            documents,
            confirm: Confirm::Checkpoint {
                stream: stream.to_owned(),
                seq,
                content: digests[0],
                own: digests[1],
            },
        })
    }
}
impl Local for StoreLocal {
    fn preamble(&mut self, page: &PageRef) -> Result<Vec<Row>, Fault> {
        let now = (self.now)();
        let local = self.local_streams(page)?;
        struct Snapshot {
            retention_days: Option<u64>,
            archived: bool,
            statements: Vec<Vec<u8>>,
            chains: Vec<Vec<u8>>,
            wraps: Vec<Vec<u8>>,
        }
        let snapshot = self
            .store
            .owner_read(&self.space, &self.owner, |tx| {
                let Some(head) = tx.head().cloned() else {
                    return Ok(None);
                };
                let policy = tx.page_policy_at(&page.page, head.revision)?;
                if policy.deleted
                    || tx.page_epoch(&page.page)?.as_deref() != Some(&page.epoch.to_string())
                {
                    return Ok(None);
                }
                let mut chains = Vec::new();
                for stream in &local {
                    chains.push(tx.author_chain(stream)?);
                }
                Ok(Some(Snapshot {
                    retention_days: policy.retention_days,
                    archived: policy.archived,
                    statements: tx.membership_originals(&head)?,
                    chains,
                    wraps: tx.owner_member_wraps(&page.page, page.epoch, &head)?,
                }))
            })
            .map_err(|error| read_fault(error.as_ref()))?
            .ok_or(Fault::Moved)?;
        let uid = &self.bound.uid;
        let mut rows = Vec::new();
        let admission = |resource: Resource, pairs: Vec<(&str, Field)>| {
            mapping(documents::encode(
                resource,
                pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
            ))
        };
        rows.push(Row::Create(admission(
            Resource::Spaces,
            vec![("space", text(&self.space)), ("owner", text(uid))],
        )?));
        let days = snapshot
            .retention_days
            .unwrap_or(PAGE_DEFAULT_DAYS)
            .clamp(1, 365);
        rows.push(Row::Put(admission(
            Resource::Pages,
            vec![
                ("space", text(&self.space)),
                ("page", text(&page.page)),
                ("epoch", mapping(Field::number(page.epoch))?),
                (
                    "state",
                    text(if snapshot.archived {
                        "archived"
                    } else {
                        "open"
                    }),
                ),
                ("expiresAt", mapping(Field::timestamp(now + days * DAY_MS))?),
            ],
        )?));
        rows.push(Row::Create(admission(
            Resource::Members,
            vec![
                ("space", text(&self.space)),
                ("page", text(&page.page)),
                ("uid", text(uid)),
                ("role", text("editor")),
                ("link", Field::Null(())),
                ("token", Field::Null(())),
            ],
        )?));
        let routing = self.routing(page, self.entry_expiry()?);
        for (kind, recipient, originals) in [
            (Authority::Membership, "", &snapshot.statements),
            (Authority::Chain, "", &snapshot.chains),
            (Authority::Wrap, uid.as_str(), &snapshot.wraps),
        ] {
            for bytes in originals {
                for document in mapping(originals::authority_documents(
                    &routing, kind, recipient, bytes,
                ))? {
                    rows.push(Row::Create(document));
                }
            }
        }
        Ok(rows)
    }
    fn next(&mut self, page: &PageRef) -> Result<Option<Data>, Fault> {
        for stream in self.local_streams(page)? {
            let scope = StreamScope {
                page: &page.page,
                epoch: page.epoch,
                stream: &stream,
            };
            let binding = CloudBinding {
                project: &self.bound.project,
                declaration_digest: &self.bound.declaration_digest,
                space: &self.space,
                uid: &self.bound.uid,
            };
            // Tracking is idempotent: it starts from the current paired cut and its tail.
            let progress = self
                .store
                .track_publication(scope, &binding)
                .map_err(store_fault)?;
            if progress.bootstrap_seq > progress.confirmed_seq {
                return self
                    .checkpoint(page, &stream, progress.bootstrap_seq)
                    .map(Some);
            }
            let seq = progress.confirmed_seq + 1;
            let Some(bytes) = self.store.payload(scope, seq).map_err(store_fault)? else {
                continue;
            };
            let envelope =
                mapping(object::Envelope::from_json(&bytes).map_err(|_| documents::Invalid))?;
            let hash = mapping(envelope.hash().map_err(|_| documents::Invalid))?;
            let routing = self.routing(page, self.entry_expiry()?);
            return Ok(Some(Data {
                documents: vec![mapping(originals::log_document(&routing, &bytes))?],
                confirm: Confirm::Update { stream, seq, hash },
            }));
        }
        Ok(None)
    }
    fn confirm(&mut self, page: &PageRef, confirm: &Confirm) -> Result<(), Fault> {
        let binding = CloudBinding {
            project: &self.bound.project,
            declaration_digest: &self.bound.declaration_digest,
            space: &self.space,
            uid: &self.bound.uid,
        };
        let result = match confirm {
            Confirm::Update { stream, seq, hash } => self.store.confirm_publication(
                StreamScope {
                    page: &page.page,
                    epoch: page.epoch,
                    stream,
                },
                &binding,
                *seq,
                hash,
            ),
            Confirm::Checkpoint {
                stream,
                seq,
                content,
                own,
            } => self.store.confirm_checkpoint_publication(
                StreamScope {
                    page: &page.page,
                    epoch: page.epoch,
                    stream,
                },
                &binding,
                *seq,
                content,
                own,
            ),
        };
        result.map(|_| ()).map_err(store_fault)
    }
}

#[cfg(test)]
mod tests;
