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
    /// Wake and join can wait on the notice event lock. Its predicate scans this
    /// registry, so neither caller may retain the registry lock while doing so.
    fn adapters_snapshot(&self) -> Vec<Arc<Adapter>> {
        self.adapters
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }
    pub(in super::super) fn registered(&self, id: &str) {
        for adapter in self.adapters_snapshot() {
            adapter.registered(id)
        }
    }
    pub(in super::super) fn cancel_registration(&self, id: &str) {
        for adapter in self.adapters_snapshot() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sealed_sync::SealedSyncPort;
    use std::{sync::mpsc, thread, time::Duration};

    #[test]
    fn registration_and_cancel_release_the_registry_before_waiting_on_a_notice_wake() {
        for cancel in [false, true] {
            let fake = Arc::new(super::super::tests::Fake::new());
            let context = fake.context();
            let vectors: serde_json::Value = serde_json::from_str(include_str!(
                "../../../tests/fixtures/sealed_sync/codec-v1.json"
            ))
            .unwrap();
            let registration = Registration::new(
                context.config().clone(),
                vectors["declaration"].as_str().unwrap().as_bytes().to_vec(),
                context.declaration_digest().into(),
                Arc::new(super::super::tests::Auth),
                Arc::new({
                    let fake = fake.clone();
                    move |_| Ok(fake.clone() as Arc<dyn SealedSyncPort>)
                }),
            )
            .unwrap();
            let registry = Arc::new(Bindings::new(
                Arc::new(registration),
                Uuid4::parse("efdc7a11-b37e-4b86-8baf-c6f613b0df91").unwrap(),
            ));
            let adapter = registry
                .adapter(&wire::Call::Bind {
                    project: context.identity().project.clone(),
                    extension: context.extension().into(),
                    declaration_digest: context.declaration_digest().into(),
                    space: context.space().into(),
                })
                .unwrap();
            adapter.interests.reserve("1").unwrap().commit();
            let (before_wake, arrived) = mpsc::channel();
            registry.signal.observe_wake(before_wake);
            // next_notice holds this event lock while scanning the registry.
            let event = registry.signal.hold_event();
            let worker = {
                let registry = registry.clone();
                thread::spawn(move || {
                    if cancel {
                        registry.cancel_registration("1")
                    } else {
                        registry.registered("1")
                    }
                })
            };
            let observed = arrived.recv_timeout(Duration::from_secs(2));
            let unlocked = registry.adapters.try_lock().is_ok();
            drop(event); // Always release before assertion/join, including the red run.
            worker.join().unwrap();
            assert!(observed.is_ok(), "worker reached the wake boundary");
            assert!(
                unlocked,
                "registry must be available to the notice predicate while a worker waits to wake it"
            );
        }
    }
}
