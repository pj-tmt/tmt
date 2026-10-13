//! Generation-scoped bindings; no cached provider rows or consumer positions.
use super::{Adapter, Registration, SyncError, failure, interests::Signal, wire};
use crate::{objects::IoBudget, store::uuid_v4};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tmt_extension_objects::Uuid4;
pub(in super::super) struct Bindings {
    registration: Arc<Registration>,
    generation: Uuid4,
    adapters: Mutex<BTreeMap<String, Arc<Adapter>>>,
    signal: Arc<Signal>,
    stopped: AtomicBool,
}
impl Bindings {
    pub(in super::super) fn new(registration: Arc<Registration>, generation: Uuid4) -> Self {
        Self {
            registration,
            generation,
            adapters: Mutex::new(BTreeMap::new()),
            signal: Arc::new(Signal::new()),
            stopped: AtomicBool::new(false),
        }
    }
    pub(in super::super) async fn execute(
        &self,
        request: &wire::Frame,
        io: &IoBudget<'_>,
    ) -> Result<wire::Frame, SyncError> {
        let wire::Frame::Request { call, .. } = request else {
            return Err(SyncError::Invalid);
        };
        let adapter = self.adapter(call);
        match adapter {
            Ok(adapter) => adapter.execute(request, io).await,
            Err(error) => Ok(wire::Frame::Result {
                version: 1,
                generation: self.generation.to_string(),
                request_id: request
                    .request_id()
                    .map_err(|_| SyncError::Invalid)?
                    .to_string(),
                method: call.method(),
                outcome: failure(error),
            }),
        }
    }
    fn adapter(&self, call: &wire::Call) -> Result<Arc<Adapter>, SyncError> {
        // Serialize this fence with close's drain: a waiter cannot construct a
        // new port after shutdown has already removed and closed the registry.
        let mut adapters = self.adapters.lock().unwrap_or_else(|p| p.into_inner());
        if self.stopped.load(Ordering::Acquire) {
            return Err(SyncError::Cancelled);
        }
        if let wire::Call::Bind {
            project,
            extension,
            declaration_digest,
            space,
        } = call
        {
            let registration = &self.registration;
            if registration
                .public_config
                .get("projectId")
                .and_then(serde_json::Value::as_str)
                != Some(project.as_str())
                || extension != &registration.extension
                || declaration_digest != &registration.digest
            {
                return Err(SyncError::Denied);
            }
            if let Some(adapter) = adapters
                .values()
                .find(|adapter| adapter.port.context().space() == space)
            {
                return Ok(adapter.clone());
            }
            if adapters.len() >= tmt_extension_objects::limits::SYNC_BINDINGS {
                return Err(SyncError::Unavailable);
            }
            // The factory is constructor-only, so the registry lock spans no I/O.
            let port = registration.bind(space)?;
            let id = Uuid4::parse(&uuid_v4().map_err(|_| SyncError::Unavailable)?)
                .map_err(|_| SyncError::Unavailable)?;
            let adapter = Arc::new(Adapter::with_signal(
                port,
                self.generation,
                id,
                self.signal.clone(),
            ));
            adapters.insert(id.to_string(), adapter.clone());
            Ok(adapter)
        } else {
            let binding = match call {
                wire::Call::Read { binding, .. }
                | wire::Call::Query { binding, .. }
                | wire::Call::Subscribe { binding, .. }
                | wire::Call::Cancel { binding, .. }
                | wire::Call::Publish { binding, .. } => binding,
                _ => unreachable!(),
            };
            adapters.get(binding).cloned().ok_or(SyncError::Denied)
        }
    }
    pub(in super::super) fn registered(&self, id: &str) {
        for adapter in self
            .adapters
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            adapter.registered(id)
        }
    }
    pub(in super::super) fn cancel_registration(&self, id: &str) {
        for adapter in self
            .adapters
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            adapter.cancel_registration(id)
        }
    }
    pub(in super::super) fn next_notice(&self, deadline: Instant) -> Option<wire::Frame> {
        self.signal
            .wait(deadline, || {
                if self.stopped.load(Ordering::Acquire) {
                    return Some(None);
                }
                self.adapters
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .values()
                    .find_map(|adapter| adapter.take_notice())
                    .map(Some)
            })
            .flatten()
    }
    pub(in super::super) fn close(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let adapters =
            std::mem::take(&mut *self.adapters.lock().unwrap_or_else(|p| p.into_inner()));
        for adapter in adapters.into_values() {
            adapter.close()
        }
        self.signal.wake();
    }
}
impl Drop for Bindings {
    fn drop(&mut self) {
        self.close();
    }
}
