//! The Store side of an inbound pass: the cloud's membership log is compared with the owner
//! log, an original is admitted by the same authority fold that reads stored objects, and only
//! then does the serve's append path store and announce it.
use super::{Applied, Inbound, Reject};
use crate::{
    fold::Snapshot,
    keyring::{Keyring, Layout},
    object_channel::cloud_sync::{PageRef, publisher::Fault},
    store::{self, Namespace, Store, StreamScope, owner::Device},
    sync::{Admission, Bootstrapped, Code, Server, SyncScope},
};
use std::collections::{HashMap, HashSet};
use tmt_colab_model::{certificate, object, statement};

/// Why the cloud's membership log cannot be followed. Each stops the page and is shown as
/// such; adopting a cloud-only log or choosing between forks is not done here.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Stop {
    /// The cloud log has revisions this machine's log does not.
    Ahead,
    /// The cloud log and this machine's log differ at the same revision.
    Fork,
    /// The cloud log is not a valid chain from the owner.
    Invalid,
}
impl Stop {
    fn line(&self) -> &'static str {
        match self {
            Self::Ahead => "The cloud has newer sharing changes than this machine.",
            Self::Fork => "The cloud and this machine disagree about sharing.",
            Self::Invalid => "The cloud sharing record is not valid.",
        }
    }
}

pub(crate) struct StoreInbound<A: Admission> {
    store: Store,
    keyring: Keyring,
    server: Server<A>,
    space: String,
    /// Authority for the pass in progress, built once by `begin`.
    admission: Option<Snapshot>,
    /// The cloud's chains of the pass in progress, by device.
    chains: HashMap<String, Vec<u8>>,
    /// Writers whose chain this machine keeps, so it is stored once.
    retained: HashSet<String>,
    /// Pages this process has looked at once for edit requests left while it was not running.
    swept: HashSet<PageRef>,
    /// A link device's own original arrived, or the page is new to this process: its edit
    /// requests are due to be settled.
    requests_due: bool,
}
/// Whether a device chain was issued by a link, not by a member.
pub(super) fn link_issued(chain: &[u8]) -> bool {
    certificate::Chain::from_json(chain)
        .ok()
        .and_then(|chain| chain.certificate().ok().map(|c| c.issuer_kind == "link"))
        .unwrap_or(false)
}
fn read_fault(error: &(dyn std::error::Error + 'static)) -> Fault {
    if store::contended(error) {
        Fault::Retry
    } else {
        Fault::Stuck("Local page state could not be read for cloud sync.")
    }
}
/// One checkpoint or fold of a bootstrap, read from its header and not yet admitted.
struct Checkpoint<'a> {
    namespace: Namespace,
    name: &'static str,
    seq: u64,
    previous: [u8; 32],
    hash: [u8; 32],
    /// The device that signed it: the covered stream's own, or the signer of a fold.
    author: String,
    fold: bool,
    bytes: &'a [u8],
}
fn namespace_of(name: &str) -> Result<(Namespace, &'static str), Reject> {
    match name {
        "content" => Ok((Namespace::Content, "content")),
        "own" => Ok((Namespace::Own, "own")),
        _ => Err(Reject::Refused),
    }
}
impl<A: Admission> StoreInbound<A> {
    fn checkpoint_of<'a>(
        &self,
        page: &PageRef,
        stream: &str,
        bytes: &'a [u8],
    ) -> Result<Checkpoint<'a>, Reject> {
        let envelope = object::Envelope::from_json(bytes).map_err(|_| Reject::Refused)?;
        let header = object::Header::decode(envelope.header()).map_err(|_| Reject::Refused)?;
        let c = &header.context;
        let (namespace, name) = namespace_of(&c.namespace)?;
        let seq: u64 = c.stream_seq.parse().map_err(|_| Reject::Refused)?;
        let covered = header.covered_device.as_deref().unwrap_or(&c.author_device);
        if c.space != self.space
            || c.page != page.page
            || c.epoch != page.epoch.to_string()
            || covered != stream
            || !matches!(c.kind.as_str(), "checkpoint" | "fold")
            || seq == 0
        {
            return Err(Reject::Refused);
        }
        Ok(Checkpoint {
            namespace,
            name,
            seq,
            previous: c.prev_hash,
            hash: envelope.hash().map_err(|_| Reject::Refused)?,
            author: c.author_device.clone(),
            fold: header.covered_device.is_some(),
            bytes,
        })
    }
    /// The update a fold binds: of the covered stream, at the fold's sequence, and the one the fold
    /// names. Whether the covered device itself signed it is the fold's own check.
    fn anchor_of(
        &self,
        page: &PageRef,
        stream: &str,
        seq: u64,
        bound: [u8; 32],
        bytes: &[u8],
    ) -> Result<Bootstrapped, Reject> {
        let envelope = object::Envelope::from_json(bytes).map_err(|_| Reject::Refused)?;
        let header = object::Header::decode(envelope.header()).map_err(|_| Reject::Refused)?;
        let c = &header.context;
        let (namespace, _) = namespace_of(&c.namespace)?;
        let hash = envelope.hash().map_err(|_| Reject::Refused)?;
        if c.space != self.space
            || c.page != page.page
            || c.epoch != page.epoch.to_string()
            || c.kind != "update"
            || c.author_device != stream
            || c.stream_seq != seq.to_string()
            || hash != bound
        {
            return Err(Reject::Refused);
        }
        Ok(Bootstrapped {
            namespace,
            hash,
            previous: c.prev_hash,
            bytes: bytes.to_vec(),
            signer: None,
        })
    }
    pub(crate) fn new(layout: &Layout, server: Server<A>, space: String) -> crate::Result<Self> {
        Ok(Self {
            store: Store::open(layout)?,
            keyring: Keyring::read(layout)?,
            server,
            space,
            admission: None,
            chains: HashMap::new(),
            retained: HashSet::new(),
            swept: HashSet::new(),
            requests_due: false,
        })
    }
    /// Keep the chain of a writer this machine did not register, once an original of it has
    /// been admitted. A browser opening the page later is given the chain of every writer in
    /// the store, to verify what that writer wrote; without it the page could not be opened.
    /// A retained chain is not authority: roles and revocation still come from the owner log,
    /// and the device stays unregistered, so nothing of it is ever published from here.
    /// Whether the cloud's chain for `device` was issued by a link.
    fn is_link_device(&self, device: &str) -> bool {
        self.chains
            .get(device)
            .is_some_and(|bytes| link_issued(bytes))
    }
    fn retain(&mut self, device: &str) -> Result<(), Reject> {
        if self.retained.contains(device) {
            return Ok(());
        }
        let chain = self.chains.get(device).ok_or(Reject::Refused)?.clone();
        let owner = self.keyring.owner_public();
        self.store
            .device_transaction(&self.space, &owner, |tx| {
                if tx.device(device)?.is_none() {
                    tx.put_device(&Device {
                        chain,
                        revoked: false,
                    })?;
                }
                Ok(Vec::new())
            })
            .map_err(|error| {
                if store::contended(error.as_ref()) {
                    Reject::Retry
                } else {
                    Reject::Refused
                }
            })?;
        self.retained.insert(device.to_owned());
        Ok(())
    }
    fn compare(&self, cloud: &[Vec<u8>]) -> Result<(), Blocked> {
        let owner = self.keyring.owner_public();
        let mut ordered = cloud
            .iter()
            .map(|bytes| {
                let envelope = statement::Envelope::from_json(bytes)?;
                Ok((envelope.revision()?, envelope))
            })
            .collect::<Result<Vec<_>, tmt_colab_model::Invalid>>()
            .map_err(|_| Blocked::Stop(Stop::Invalid))?;
        ordered.sort_by_key(|(revision, _)| *revision);
        // Revisions are one through the head, once each, and a chain from the owner.
        let mut head = None;
        for (index, (revision, envelope)) in ordered.iter().enumerate() {
            if *revision != index as u64 + 1 {
                return Err(Blocked::Stop(Stop::Invalid));
            }
            head = Some(
                envelope
                    .verify_next(&self.space, &owner, head.as_ref())
                    .map_err(|_| Blocked::Stop(Stop::Invalid))?
                    .head,
            );
        }
        let local = self
            .store
            .owner_read(&self.space, &owner, |tx| tx.log())
            .map_err(|error| Blocked::Fault(read_fault(error.as_ref())))?;
        for ((_, cloud), local) in ordered.iter().zip(&local) {
            if cloud.hash().ok() != local.hash().ok() {
                return Err(Blocked::Stop(Stop::Fork));
            }
        }
        if ordered.len() > local.len() {
            return Err(Blocked::Stop(Stop::Ahead));
        }
        Ok(())
    }
}
enum Blocked {
    Stop(Stop),
    Fault(Fault),
}
impl<A: Admission> Inbound for StoreInbound<A> {
    fn membership(&mut self, _: &PageRef, cloud: &[Vec<u8>]) -> Result<(), Fault> {
        match self.compare(cloud) {
            Ok(()) => Ok(()),
            Err(Blocked::Stop(stop)) => Err(Fault::Stuck(stop.line())),
            Err(Blocked::Fault(fault)) => Err(fault),
        }
    }
    fn begin(&mut self, page: &PageRef, chains: &[Vec<u8>]) -> Result<Vec<String>, Fault> {
        self.requests_due |= self.swept.insert(page.clone());
        self.admission = None;
        self.chains.clear();
        let mut devices = Vec::new();
        let mut streams = Vec::new();
        for bytes in chains {
            let Ok(chain) = certificate::Chain::from_json(bytes) else {
                return Err(Fault::Stuck("A writer record in the cloud is not valid."));
            };
            let Ok(cert) = chain.certificate() else {
                return Err(Fault::Stuck("A writer record in the cloud is not valid."));
            };
            streams.push(cert.device_id.to_owned());
            self.chains.insert(cert.device_id.to_owned(), bytes.clone());
            devices.push(Device {
                chain: bytes.clone(),
                revoked: false,
            });
        }
        let owner = self.keyring.owner_public();
        let writer = self
            .keyring
            .local_writer()
            .map_err(|_| Fault::Stuck("This machine cannot name its own writer."))?
            .0;
        let (epoch, foreign) = self
            .store
            .owner_read(&self.space, &owner, |tx| {
                let mut foreign = Vec::new();
                for stream in &streams {
                    if !tx.is_local_device(&writer, stream)? {
                        foreign.push(stream.clone());
                    }
                }
                Ok((tx.page_epoch(&page.page)?, foreign))
            })
            .map_err(|error| read_fault(error.as_ref()))?;
        if epoch.as_deref() != Some(page.epoch.to_string().as_str()) {
            return Err(Fault::Moved);
        }
        self.admission = Some(
            Snapshot::admission(&self.store, &self.keyring, &page.page, devices).map_err(
                |error| {
                    if store::contended(error.as_ref()) {
                        Fault::Retry
                    } else {
                        Fault::Stuck("This machine cannot verify this page.")
                    }
                },
            )?,
        );
        Ok(foreign)
    }
    fn settle(&mut self, page: &PageRef) {
        if !std::mem::take(&mut self.requests_due) {
            return;
        }
        let scope = SyncScope {
            space: page.space.clone(),
            page: page.page.clone(),
            epoch: page.epoch.to_string(),
        };
        // An attempt that could not finish is made again with the next arrival or restart.
        if self.server.apply_edit_requests(&scope).is_err() {
            self.swept.remove(page);
        }
    }
    fn head(&mut self, page: &PageRef, stream: &str) -> Result<u64, Fault> {
        match self.store.stream_head(StreamScope {
            page: &page.page,
            epoch: page.epoch,
            stream,
        }) {
            Ok(head) => Ok(head.map_or(0, |(seq, _)| seq)),
            Err(store::Fault::StaleEpoch) => Err(Fault::Moved),
            Err(fault) if store::contended(&fault) => Err(Fault::Retry),
            Err(_) => Err(Fault::Stuck(
                "Local page state could not be read for cloud sync.",
            )),
        }
    }
    fn bootstrap(
        &mut self,
        page: &PageRef,
        stream: &str,
        anchor: Option<&[u8]>,
        pair: &[Vec<u8>],
    ) -> Result<Applied, Reject> {
        // The shape of the pair: one stream, one sequence, a namespace each, one head.
        let objects = pair
            .iter()
            .map(|bytes| self.checkpoint_of(page, stream, bytes))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(first) = objects.first() else {
            return Err(Reject::Refused);
        };
        let (seq, previous) = (first.seq, first.previous);
        let folds = objects.iter().any(|object| object.fold);
        if objects.len() != 2
            || objects[0].namespace == objects[1].namespace
            || objects
                .iter()
                .any(|object| object.seq != seq || object.previous != previous)
            || folds != anchor.is_some()
        {
            return Err(Reject::Refused);
        }
        let anchor = anchor
            .map(|bytes| self.anchor_of(page, stream, seq, previous, bytes))
            .transpose()?;
        // The same admission that reads a stored checkpoint or fold: signature, role, epoch, the
        // signed cut of a revoked device and, for a fold, its signer's list and its anchor.
        {
            let admission = self.admission.as_mut().ok_or(Reject::Retry)?;
            if let Some(anchor) = &anchor {
                admission.with_anchor(stream, seq, anchor.bytes.clone());
            }
            for object in &objects {
                let stored = store::owner::epoch::StoredObject {
                    seq,
                    hash: object.hash,
                    previous,
                    checkpoint: true,
                    bytes: object.bytes.to_vec(),
                };
                admission
                    .open_stream_object(&self.keyring, &page.page, stream, object.name, &stored)
                    .map_err(|_| Reject::Refused)?;
            }
        }
        for object in &objects {
            self.retain(&object.author)?;
            if object.fold {
                self.retain(stream)?;
            }
        }
        let installed: Vec<Bootstrapped> = objects
            .iter()
            .map(|object| Bootstrapped {
                namespace: object.namespace,
                hash: object.hash,
                previous,
                bytes: object.bytes.to_vec(),
                signer: object.fold.then(|| object.author.clone()),
            })
            .collect();
        let scope = SyncScope {
            space: self.space.clone(),
            page: page.page.clone(),
            epoch: page.epoch.to_string(),
        };
        match self
            .server
            .ingest_bootstrap(&scope, stream, seq, anchor.as_ref(), &installed)
        {
            Ok(store::Accepted::New) => Ok(Applied::New),
            Ok(store::Accepted::Replay) => Ok(Applied::Replay),
            Err(Code::StaleEpoch) => Err(Reject::Moved),
            Err(Code::Capacity) => Err(Reject::Full),
            Err(Code::Gap | Code::ResyncRequired) => Err(Reject::Retry),
            Err(_) => Err(Reject::Refused),
        }
    }
    fn admit(&mut self, page: &PageRef, original: &[u8]) -> Result<Applied, Reject> {
        let admission = self.admission.as_ref().ok_or(Reject::Retry)?;
        let envelope = object::Envelope::from_json(original).map_err(|_| Reject::Refused)?;
        let header = object::Header::decode(envelope.header()).map_err(|_| Reject::Refused)?;
        let c = &header.context;
        if c.space != self.space || c.page != page.page || c.epoch != page.epoch.to_string() {
            return Err(Reject::Refused);
        }
        let stored = store::owner::epoch::StoredObject {
            seq: c.stream_seq.parse().map_err(|_| Reject::Refused)?,
            hash: envelope.hash().map_err(|_| Reject::Refused)?,
            previous: c.prev_hash,
            checkpoint: false,
            bytes: original.to_vec(),
        };
        let opened = admission
            .open_stream_object(
                &self.keyring,
                &page.page,
                &c.author_device,
                &c.namespace,
                &stored,
            )
            .map_err(|_| Reject::Refused)?;
        self.retain(&c.author_device)?;
        let scope = SyncScope {
            space: self.space.clone(),
            page: page.page.clone(),
            epoch: page.epoch.to_string(),
        };
        match self.server.ingest(&scope, original, &opened.key_bytes) {
            Ok(store::Accepted::New) => {
                // A link device's own original may carry an edit request for this machine.
                self.requests_due |= c.namespace == "own" && self.is_link_device(&c.author_device);
                Ok(Applied::New)
            }
            Ok(store::Accepted::Replay) => Ok(Applied::Replay),
            Err(Code::StaleEpoch) => Err(Reject::Moved),
            Err(Code::Capacity) => Err(Reject::Full),
            Err(Code::Gap | Code::ResyncRequired) => Err(Reject::Retry),
            Err(_) => Err(Reject::Refused),
        }
    }
}
