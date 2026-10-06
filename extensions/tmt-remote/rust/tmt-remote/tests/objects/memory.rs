//! A second, independent adapter: an in-memory model of the backend contract.
//! It shares no code with `LocalFs` (not its ledger, tree or accounting), so the
//! same conformance scenarios passing for both shows the contract, not one
//! implementation, is what a consumer sees. It also serves as the executable
//! reference of the charge formula.
use crate::conformance::{Adapter, Session};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tmt_remote::{
    limits,
    objects::{
        BackendCaps, BackendError, BackendResult, BeginResult, BeginSpec, BlobKey, IntentId,
        IoBudget, Limit, NamespaceId, ObjectBackend, OriginalIntent, Progress, Quotas, ReadPart,
        Receipt, Transfer, TransferState, Usage,
    },
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Staging,
    Committed,
    Discarded,
    Expired,
    Deleted,
}
struct Row {
    spec: BeginSpec,
    adopted: u64,
    expires: u64,
    state: State,
    body: Vec<u8>,
}
impl Row {
    fn holds_payload(&self) -> bool {
        matches!(self.state, State::Staging | State::Committed)
    }
    fn live(&self) -> bool {
        self.holds_payload()
    }
}
struct Model {
    quotas: Quotas,
    now: u64,
    rows: HashMap<(String, [u8; 32]), Row>,
    /// `(extension, namespace) -> open`
    namespaces: HashMap<(String, [u8; 32]), bool>,
}
#[derive(Default, Clone, Copy)]
struct Totals {
    bytes: u64,
    entries: u32,
    active: u32,
    retained: u32,
}
impl Model {
    fn totals(&self, extension: Option<&str>, namespace: Option<[u8; 32]>) -> Totals {
        let mut totals = Totals::default();
        let selected = |e: &str, n: &[u8; 32]| {
            extension.is_none_or(|x| x == e) && namespace.is_none_or(|x| x == *n)
        };
        for ((e, _), row) in &self.rows {
            if !selected(e, &row.spec.key.namespace.0) {
                continue;
            }
            totals.retained += 1;
            totals.bytes += limits::OBJECT_RECORD_BYTES;
            if row.holds_payload() {
                totals.entries += 1;
                totals.bytes += row.spec.payload_bytes.div_ceil(4096) * 4096;
            }
            if row.state == State::Staging {
                totals.active += 1;
            }
        }
        totals.bytes += self
            .namespaces
            .keys()
            .filter(|(e, n)| selected(e, n))
            .count() as u64
            * limits::OBJECT_FENCE_BYTES;
        // Each extension with any namespace owns a fixed payload tree.
        if namespace.is_none() {
            let trees: std::collections::HashSet<_> = self
                .namespaces
                .keys()
                .filter(|(e, _)| extension.is_none_or(|x| x == e))
                .map(|(e, _)| e.clone())
                .collect();
            totals.bytes += trees.len() as u64 * limits::OBJECT_TREE_BASE_BYTES;
        }
        if extension.is_none() {
            totals.bytes += limits::OBJECT_LEDGER_BASE_BYTES;
        }
        totals
    }
    fn expire(&mut self, extension: &str, intent: &IntentId) {
        let now = self.now;
        if let Some(row) = self.rows.get_mut(&(extension.to_owned(), intent.0))
            && row.state == State::Staging
            && now >= row.expires
        {
            row.state = State::Expired;
            row.body.clear();
        }
    }
}
fn progress(row: &Row) -> Progress {
    Progress {
        intent: row.spec.intent,
        next_index: (row.body.len() as u64).div_ceil(32768) as u32,
        received: row.body.len() as u64,
    }
}
fn receipt(spec: &BeginSpec) -> Receipt {
    Receipt {
        intent: spec.intent,
        key: spec.key,
        payload_sha256: spec.payload_sha256,
        payload_bytes: spec.payload_bytes,
        binding: spec.binding.clone(),
    }
}

struct Memory {
    model: Arc<Mutex<Model>>,
    extension: String,
}
impl ObjectBackend for Memory {
    fn id(&self) -> &'static str {
        "memory-model"
    }
    fn capabilities(&self) -> BackendCaps {
        BackendCaps {
            max_payload_bytes: limits::OBJECT_PAYLOAD_BYTES,
            chunk_bytes: limits::OBJECT_CHUNK_BYTES,
        }
    }
    fn usage(&self, namespace: Option<NamespaceId>, io: &IoBudget<'_>) -> BackendResult<Usage> {
        io.check()?;
        let model = self.model.lock().unwrap();
        let totals = model.totals(Some(&self.extension), namespace.map(|n| n.0));
        io.check()?;
        Ok(Usage {
            charged_bytes: totals.bytes,
            entries: totals.entries,
            active_uploads: totals.active,
            retained_identities: totals.retained,
        })
    }
    fn begin(&self, spec: &BeginSpec, io: &IoBudget<'_>) -> BackendResult<BeginResult> {
        io.check()?;
        if spec.payload_bytes > limits::OBJECT_PAYLOAD_BYTES
            || spec.binding.len() > limits::OBJECT_BINDING_BYTES
        {
            return Err(BackendError::Invalid);
        }
        let mut model = self.model.lock().unwrap();
        let id = (self.extension.clone(), spec.intent.0);
        model.expire(&self.extension, &spec.intent);
        if let Some(row) = model.rows.get(&id) {
            if row.spec != *spec {
                return Err(BackendError::Conflict);
            }
            let state = match row.state {
                State::Staging => return Ok(BeginResult::Pending(progress(row))),
                State::Committed => return Ok(BeginResult::Committed(receipt(&row.spec))),
                State::Discarded => TransferState::Discarded,
                State::Expired => TransferState::Expired,
                State::Deleted => TransferState::Unavailable,
            };
            return Ok(BeginResult::Terminal(state));
        }
        let namespace = (self.extension.clone(), spec.key.namespace.0);
        let open = model.namespaces.get(&namespace).copied();
        if open == Some(false) {
            return Err(BackendError::Conflict);
        }
        if model
            .rows
            .iter()
            .any(|((e, _), row)| *e == self.extension && row.live() && row.spec.key == spec.key)
        {
            return Err(BackendError::Conflict);
        }
        let added = spec.payload_bytes.div_ceil(4096) * 4096
            + limits::OBJECT_RECORD_BYTES
            + if open.is_none() {
                limits::OBJECT_FENCE_BYTES
            } else {
                0
            };
        let first = !model.namespaces.keys().any(|(e, _)| *e == self.extension);
        let shared = added
            + if first {
                limits::OBJECT_TREE_BASE_BYTES
            } else {
                0
            };
        let q = model.quotas;
        let installation = model.totals(None, None);
        let extension = model.totals(Some(&self.extension), None);
        let scoped = model.totals(Some(&self.extension), Some(spec.key.namespace.0));
        let checks = [
            (
                installation.active >= q.active_intents,
                Limit::ActiveIntents,
            ),
            (
                extension.retained >= q.retained_extension,
                Limit::RetainedExtension,
            ),
            (
                installation.retained >= q.retained_installation,
                Limit::RetainedInstallation,
            ),
            (
                scoped.entries >= q.namespace_entries,
                Limit::NamespaceEntries,
            ),
            (
                extension.entries >= q.extension_entries,
                Limit::ExtensionEntries,
            ),
            (
                installation.entries >= q.installation_entries,
                Limit::InstallationEntries,
            ),
            (
                scoped.bytes + added > q.namespace_bytes,
                Limit::NamespaceBytes,
            ),
            (
                extension.bytes + shared > q.extension_bytes,
                Limit::ExtensionBytes,
            ),
            (
                installation.bytes + shared > q.installation_bytes,
                Limit::InstallationBytes,
            ),
        ];
        if let Some((_, limit)) = checks.into_iter().find(|(refused, _)| *refused) {
            return Err(BackendError::Capacity(limit));
        }
        model.namespaces.insert(namespace, true);
        let (adopted, expires) = (model.now, model.now + 24 * 60 * 60 * 1000);
        model.rows.insert(
            id,
            Row {
                spec: spec.clone(),
                adopted,
                expires,
                state: State::Staging,
                body: Vec::new(),
            },
        );
        Ok(BeginResult::Pending(Progress {
            intent: spec.intent,
            next_index: 0,
            received: 0,
        }))
    }
    fn append(
        &self,
        intent: IntentId,
        index: u32,
        bytes: &[u8],
        io: &IoBudget<'_>,
    ) -> BackendResult<Progress> {
        io.check()?;
        let mut model = self.model.lock().unwrap();
        model.expire(&self.extension, &intent);
        let namespace_open = |model: &Model, ns: [u8; 32]| {
            model.namespaces.get(&(self.extension.clone(), ns)) == Some(&true)
        };
        let row = model
            .rows
            .get(&(self.extension.clone(), intent.0))
            .ok_or(BackendError::Missing)?;
        if row.state != State::Staging || !namespace_open(&model, row.spec.key.namespace.0) {
            return Err(BackendError::Conflict);
        }
        let chunk = 32768usize;
        let (total, received) = (row.spec.payload_bytes as usize, row.body.len());
        let next = received.div_ceil(chunk) as u32;
        if index < next {
            let offset = index as usize * chunk;
            let length = chunk.min(total - offset);
            return if bytes.len() == length && row.body[offset..offset + length] == *bytes {
                Ok(progress(row))
            } else {
                Err(BackendError::Conflict)
            };
        }
        if index > next || received == total {
            return Err(BackendError::Conflict);
        }
        if bytes.len() != chunk.min(total - received) {
            return Err(BackendError::Invalid);
        }
        let row = model
            .rows
            .get_mut(&(self.extension.clone(), intent.0))
            .unwrap();
        row.body.extend_from_slice(bytes);
        Ok(progress(row))
    }
    fn commit(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<Receipt> {
        io.check()?;
        let mut model = self.model.lock().unwrap();
        model.expire(&self.extension, &intent);
        let open = |model: &Model, ns: [u8; 32]| {
            model.namespaces.get(&(self.extension.clone(), ns)) == Some(&true)
        };
        let row = model
            .rows
            .get(&(self.extension.clone(), intent.0))
            .ok_or(BackendError::Missing)?;
        match row.state {
            State::Committed => return Ok(receipt(&row.spec)),
            State::Staging => {}
            _ => return Err(BackendError::Conflict),
        }
        if !open(&model, row.spec.key.namespace.0)
            || row.body.len() as u64 != row.spec.payload_bytes
        {
            return Err(BackendError::Conflict);
        }
        if Sha256::digest(&row.body).as_slice() != row.spec.payload_sha256 {
            return Err(BackendError::Invalid);
        }
        let row = model
            .rows
            .get_mut(&(self.extension.clone(), intent.0))
            .unwrap();
        row.state = State::Committed;
        Ok(receipt(&row.spec))
    }
    fn status(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<Transfer> {
        io.check()?;
        let model = self.model.lock().unwrap();
        let Some(row) = model.rows.get(&(self.extension.clone(), intent.0)) else {
            return Ok(Transfer {
                state: TransferState::NotObserved,
                original: None,
            });
        };
        let state = match row.state {
            State::Staging if model.now >= row.expires => TransferState::Expired,
            State::Staging => TransferState::Pending(progress(row)),
            State::Committed => TransferState::Committed(receipt(&row.spec)),
            State::Discarded => TransferState::Discarded,
            State::Expired => TransferState::Expired,
            State::Deleted => TransferState::Unavailable,
        };
        Ok(Transfer {
            state,
            original: Some(OriginalIntent {
                spec: row.spec.clone(),
                adopted_at_ms: row.adopted,
                expires_at_ms: row.expires,
            }),
        })
    }
    fn stat(&self, key: BlobKey, io: &IoBudget<'_>) -> BackendResult<Receipt> {
        io.check()?;
        let model = self.model.lock().unwrap();
        model
            .rows
            .iter()
            .find(|((e, _), row)| {
                *e == self.extension && row.state == State::Committed && row.spec.key == key
            })
            .map(|(_, row)| receipt(&row.spec))
            .ok_or(BackendError::Missing)
    }
    fn read(
        &self,
        key: BlobKey,
        offset: u64,
        count: u32,
        io: &IoBudget<'_>,
    ) -> BackendResult<ReadPart> {
        io.check()?;
        if count == 0 || count > limits::OBJECT_CHUNK_BYTES {
            return Err(BackendError::Invalid);
        }
        let model = self.model.lock().unwrap();
        let row = model
            .rows
            .iter()
            .find(|((e, _), row)| {
                *e == self.extension && row.state == State::Committed && row.spec.key == key
            })
            .map(|(_, row)| row)
            .ok_or(BackendError::Missing)?;
        let total = row.spec.payload_bytes;
        if offset > total || (offset == total && total != 0) {
            return Err(BackendError::Invalid);
        }
        let end = (offset + u64::from(count)).min(total);
        io.check()?;
        Ok(ReadPart {
            offset,
            total_bytes: total,
            bytes: row.body[offset as usize..end as usize].to_vec(),
        })
    }
    fn discard(&self, intent: IntentId, io: &IoBudget<'_>) -> BackendResult<()> {
        io.check()?;
        let mut model = self.model.lock().unwrap();
        model.expire(&self.extension, &intent);
        let row = model
            .rows
            .get_mut(&(self.extension.clone(), intent.0))
            .ok_or(BackendError::Missing)?;
        match row.state {
            State::Staging => {
                row.state = State::Discarded;
                row.body.clear();
                Ok(())
            }
            State::Discarded | State::Expired => Ok(()),
            _ => Err(BackendError::Conflict),
        }
    }
    fn remove_namespace(&self, namespace: NamespaceId, io: &IoBudget<'_>) -> BackendResult<()> {
        io.check()?;
        let mut model = self.model.lock().unwrap();
        let id = (self.extension.clone(), namespace.0);
        if model.namespaces.get(&id) == Some(&false) {
            return Ok(());
        }
        if !model.namespaces.contains_key(&id) {
            let q = model.quotas;
            let first = !model.namespaces.keys().any(|(e, _)| *e == self.extension);
            let added = limits::OBJECT_FENCE_BYTES;
            let shared = added
                + if first {
                    limits::OBJECT_TREE_BASE_BYTES
                } else {
                    0
                };
            let scoped = model.totals(Some(&self.extension), Some(namespace.0));
            let extension = model.totals(Some(&self.extension), None);
            let installation = model.totals(None, None);
            if scoped.bytes + added > q.namespace_bytes {
                return Err(BackendError::Capacity(Limit::NamespaceBytes));
            }
            if extension.bytes + shared > q.extension_bytes {
                return Err(BackendError::Capacity(Limit::ExtensionBytes));
            }
            if installation.bytes + shared > q.installation_bytes {
                return Err(BackendError::Capacity(Limit::InstallationBytes));
            }
        }
        model.namespaces.insert(id, false);
        for ((e, _), row) in model.rows.iter_mut() {
            if *e == self.extension
                && row.spec.key.namespace == namespace
                && !matches!(
                    row.state,
                    State::Discarded | State::Expired | State::Deleted
                )
            {
                row.state = State::Deleted;
                row.body.clear();
            }
        }
        Ok(())
    }
}

struct MemorySession {
    model: Arc<Mutex<Model>>,
}
impl Session for MemorySession {
    fn backend(&self, extension: &str) -> Box<dyn ObjectBackend + '_> {
        Box::new(Memory {
            model: self.model.clone(),
            extension: extension.to_owned(),
        })
    }
    fn advance(&self, milliseconds: u64) {
        self.model.lock().unwrap().now += milliseconds;
    }
    fn restart(&mut self) {}
    fn installation(&self) -> Usage {
        let totals = self.model.lock().unwrap().totals(None, None);
        Usage {
            charged_bytes: totals.bytes,
            entries: totals.entries,
            active_uploads: totals.active,
            retained_identities: totals.retained,
        }
    }
}
pub struct MemoryAdapter;
impl Adapter for MemoryAdapter {
    fn run(&self, quotas: Quotas, scenario: &mut dyn FnMut(&mut dyn Session)) {
        let mut session = MemorySession {
            model: Arc::new(Mutex::new(Model {
                quotas,
                now: 1_000_000_000,
                rows: HashMap::new(),
                namespaces: HashMap::new(),
            })),
        };
        scenario(&mut session);
    }
}

mod suite {
    use super::MemoryAdapter;
    crate::conformance!(MemoryAdapter);
}
