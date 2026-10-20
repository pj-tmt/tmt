//! The publisher's local side over the Store: which originals exist, in what order they are
//! due, and where confirmations are recorded. Positions live in the B2 publication metadata;
//! nothing here keeps its own copy.
use super::{
    PageRef,
    publisher::{Confirm, Data, Fault, Local, Retirement, Row},
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
/// The longest a page's cloud copy lives without activity: what the declaration and the Rules
/// admit (365 days from the request). A page with no local expiry (forever) is given this much,
/// since the cloud lifetime has no forever option; the policy's own 30-day default is its
/// retention, not a fallback here.
const CLOUD_MAX_DAYS: u64 = 365;
/// The cloud lifetime of a page with no activity, in days: its finite retention within one
/// through [`CLOUD_MAX_DAYS`], and the maximum for a page kept forever locally.
fn cloud_days(retention_days: Option<u64>) -> u64 {
    retention_days.map_or(CLOUD_MAX_DAYS, |days| days.clamp(1, CLOUD_MAX_DAYS))
}
/// How soon after a write the page row is written again on activity: at most a day, and half
/// the lifetime of a short one, so the cloud expiry stays close to the last activity.
fn row_refresh(days: u64) -> std::time::Duration {
    std::time::Duration::from_millis((days * DAY_MS / 2).min(DAY_MS))
}
/// A live link of the page as its cloud row and wraps need it.
struct LiveLink {
    id: String,
    role: String,
    token: String,
    /// The signed wraps addressed to the link, exactly as stored.
    wraps: Vec<Vec<u8>>,
}
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
    /// The root-local writer this machine's keyring names.
    writer: String,
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
        writer: String,
        space: String,
        bound: Bound,
        now: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            store,
            owner,
            writer,
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
    /// The streams of this page that this machine writes (its root-local writer and the devices
    /// it registered), in a stable order.
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
                    if tx.is_local_device(&self.writer, &stream)? {
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
        let mut anchored = false;
        let mut signers = std::collections::BTreeSet::new();
        for namespace in [Namespace::Content, Namespace::Own] {
            let object = self
                .store
                .namespace_next(scope, namespace, NamespaceCursor::default())
                .map_err(store_fault)?
                .filter(|object| object.checkpoint && object.cursor.seq == seq)
                // Compaction moved the cut between the position and this read: look again.
                .ok_or(Fault::Retry)?;
            let envelope = mapping(
                object::Envelope::from_json(&object.bytes).map_err(|_| documents::Invalid),
            )?;
            let header =
                mapping(object::Header::decode(envelope.header()).map_err(|_| documents::Invalid))?;
            if let Some(covered) = &header.covered_device {
                signers.insert(header.context.author_device.clone());
                if covered != stream {
                    return Err(Fault::Retry);
                }
                if !anchored {
                    let anchor = self
                        .store
                        .anchor(scope, seq)
                        .map_err(store_fault)?
                        .ok_or(Fault::Retry)?;
                    documents.insert(
                        0,
                        mapping(originals::log_document(&routing, &anchor.bytes))?,
                    );
                    anchored = true;
                }
            }
            let base = mapping(originals::checkpoint_base(&routing, &object.bytes))?;
            documents.extend(mapping(originals::checkpoint_documents(
                base,
                &object.bytes,
            ))?);
            digests.push(crypto::digest(&object.bytes));
        }
        // Freeze the chosen folds' proof dependencies with this batch. A newer compaction
        // between next() and preamble() must not substitute another signer's chain.
        let mut dependencies = Vec::new();
        for signer in signers {
            let chain = self
                .store
                .owner_read(&self.space, &self.owner, |tx| tx.author_chain(&signer))
                .map_err(|error| read_fault(error.as_ref()))?;
            dependencies.extend(mapping(originals::authority_documents(
                &routing,
                Authority::Chain,
                "",
                &chain,
            ))?);
        }
        dependencies.extend(documents);
        Ok(Data {
            documents: dependencies,
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
            links: Vec<LiveLink>,
            /// The epoch's exact signed baseline; none at the first epoch, which has none.
            baseline: Option<Vec<u8>>,
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
                let mut links = Vec::new();
                for (id, token) in tx.link_admissions()? {
                    let Some(link) = tx.recipient("link", &id)? else {
                        continue;
                    };
                    let (false, true, Some(role)) = (
                        link.revoked,
                        link.pages.contains(&page.page),
                        link.role.clone(),
                    ) else {
                        continue;
                    };
                    chains.extend(tx.link_device_chains(&id)?);
                    links.push(LiveLink {
                        wraps: tx.owner_link_wraps(&page.page, page.epoch, &head, &id)?,
                        id,
                        role,
                        token,
                    });
                }
                Ok(Some(Snapshot {
                    retention_days: policy.retention_days,
                    archived: policy.archived,
                    statements: tx.membership_originals(&head)?,
                    chains,
                    wraps: tx.owner_member_wraps(&page.page, page.epoch, &head)?,
                    links,
                    baseline: if page.epoch > 1 {
                        tx.baseline(&page.page, page.epoch)?.map(|b| b.envelope)
                    } else {
                        None
                    },
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
        let days = cloud_days(snapshot.retention_days);
        let expires_ms = now + days * DAY_MS;
        rows.push(Row::Put {
            days,
            refresh: row_refresh(days),
            expires_ms,
            document: admission(
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
                    ("expiresAt", mapping(Field::timestamp(expires_ms))?),
                ],
            )?,
        });
        rows.push(Row::Create(admission(
            Resource::Members,
            vec![
                ("space", text(&self.space)),
                ("page", text(&page.page)),
                ("uid", text(uid)),
                ("role", text("editor")),
                ("link", Field::Null(())),
                ("token", Field::Null(())),
                ("seq", mapping(Field::number(1))?),
            ],
        )?));
        // A link's row opens enrollment, so its removal is recorded before the row is written.
        let tracked = self
            .store
            .link_publications(&page.page)
            .map_err(store_fault)?;
        for link in &snapshot.links {
            if !tracked.contains(&link.id) {
                self.store
                    .track_link_publication(&page.page, &link.id)
                    .map_err(store_fault)?;
            }
            rows.push(Row::Create(admission(
                Resource::Links,
                vec![
                    ("space", text(&self.space)),
                    ("page", text(&page.page)),
                    ("link", text(&link.id)),
                    ("role", text(&link.role)),
                    ("token", text(&link.token)),
                ],
            )?));
        }
        let routing = self.routing(page, self.entry_expiry()?);
        for (kind, recipient, originals) in [
            (Authority::Membership, "", &snapshot.statements),
            (Authority::Chain, "", &snapshot.chains),
        ] {
            for bytes in originals {
                for document in mapping(originals::authority_documents(
                    &routing, kind, recipient, bytes,
                ))? {
                    rows.push(Row::Create(document));
                }
            }
        }
        // The epoch's baseline precedes its wraps: whoever can read a wrap can read the baseline
        // that wrap's key opens.
        if let Some(bytes) = &snapshot.baseline {
            for document in mapping(originals::baseline_documents(&routing, bytes))? {
                rows.push(Row::Create(document));
            }
        }
        for bytes in &snapshot.wraps {
            for document in mapping(originals::authority_documents(
                &routing,
                Authority::Wrap,
                uid,
                bytes,
            ))? {
                rows.push(Row::Create(document));
            }
        }
        for link in &snapshot.links {
            let recipient = format!("{}{}", documents::LINK_RECIPIENT, link.id);
            for bytes in &link.wraps {
                for document in mapping(originals::authority_documents(
                    &routing,
                    Authority::Wrap,
                    &recipient,
                    bytes,
                ))? {
                    rows.push(Row::Create(document));
                }
            }
        }
        Ok(rows)
    }
    fn retirements(&mut self, page: &PageRef) -> Result<Vec<Retirement>, Fault> {
        let tracked = self
            .store
            .link_publications(&page.page)
            .map_err(store_fault)?;
        if tracked.is_empty() {
            return Ok(Vec::new());
        }
        let live = self
            .store
            .owner_read(&self.space, &self.owner, |tx| {
                let mut live = Vec::new();
                for (id, _) in tx.link_admissions()? {
                    if tx
                        .recipient("link", &id)?
                        .is_some_and(|link| !link.revoked && link.pages.contains(&page.page))
                    {
                        live.push(id);
                    }
                }
                Ok(live)
            })
            .map_err(|error| read_fault(error.as_ref()))?;
        tracked
            .into_iter()
            .filter(|link| !live.contains(link))
            .map(|link| {
                Ok(Retirement {
                    target: mapping(documents::link_target(&self.space, &page.page, &link))?,
                    scope: mapping(documents::holders_scope(&self.space, &page.page, &link))?,
                    link,
                })
            })
            .collect()
    }
    fn retired(&mut self, page: &PageRef, link: &str) -> Result<(), Fault> {
        self.store
            .end_link_publication(&page.page, link)
            .map_err(store_fault)
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
    fn page_row_confirmed(&mut self, page: &PageRef, expires_ms: u64) -> Result<(), Fault> {
        self.store
            .confirm_page_row(&page.page, page.epoch, expires_ms)
            .map_err(store_fault)
    }
}

#[cfg(test)]
mod tests;
