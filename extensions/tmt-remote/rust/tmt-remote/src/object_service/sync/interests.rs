//! One wake flag and reserved terminal status per page interest, never documents.
use super::{SealedSyncPort, SyncError, client, failure, wire};
use crate::objects::IoBudget;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
struct Pending {
    changed: bool,
    terminal: Option<wire::Error>,
    sequence: u64,
    ready: bool,
    terminal_recorded: bool,
}
pub(super) struct InterestEntry {
    pending: Mutex<Pending>,
    stop: AtomicBool,
    job: Mutex<Option<JoinHandle<()>>>,
}
pub(super) struct Interests {
    entries: Mutex<BTreeMap<String, Arc<InterestEntry>>>,
    signal: Arc<Signal>,
    stopped: AtomicBool,
}
impl Interests {
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            signal: Arc::new(Signal::new()),
            stopped: AtomicBool::new(false),
        }
    }
    pub(super) fn reserve(&self, id: &str) -> Result<Reservation<'_>, SyncError> {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if self.stopped.load(Ordering::Acquire) {
            return Err(SyncError::Cancelled);
        }
        if entries.len() >= tmt_extension_objects::limits::SYNC_INTERESTS
            || entries.contains_key(id)
        {
            return Err(SyncError::Unavailable);
        }
        let entry = Arc::new(InterestEntry {
            pending: Mutex::new(Pending {
                changed: false,
                terminal: None,
                sequence: 0,
                ready: false,
                terminal_recorded: false,
            }),
            stop: AtomicBool::new(false),
            job: Mutex::new(None),
        });
        entries.insert(id.into(), entry.clone());
        Ok(Reservation {
            registry: self,
            id: id.into(),
            entry,
            committed: false,
        })
    }
    pub(super) fn with_signal(signal: Arc<Signal>) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            signal,
            stopped: AtomicBool::new(false),
        }
    }
    fn wake(&self) {
        self.signal.wake();
    }
    pub(super) fn enable(&self, id: &str) {
        if let Some(entry) = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
        {
            entry
                .pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .ready = true;
        }
        self.wake();
    }
    pub(super) fn changed(&self, entry: &InterestEntry) {
        let mut pending = entry.pending.lock().unwrap_or_else(|p| p.into_inner());
        if !entry.stop.load(Ordering::Acquire) && !pending.terminal_recorded {
            pending.changed = true
        }
        drop(pending);
        self.wake();
    }
    pub(super) fn terminal(&self, entry: &InterestEntry, error: SyncError) {
        if let wire::Outcome::Error { error } = failure(error) {
            let mut pending = entry.pending.lock().unwrap_or_else(|p| p.into_inner());
            if !entry.stop.load(Ordering::Acquire) && !pending.terminal_recorded {
                pending.terminal = Some(error);
                pending.terminal_recorded = true
            }
            drop(pending);
            self.wake();
        }
    }
    pub(super) fn take(&self) -> Option<(String, u64, wire::InterestNotice)> {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        for (id, entry) in entries.iter() {
            let mut pending = entry.pending.lock().unwrap_or_else(|p| p.into_inner());
            if !pending.ready || entry.stop.load(Ordering::Acquire) {
                continue;
            }
            let notice = if pending.changed {
                pending.changed = false;
                wire::InterestNotice::Changed
            } else if let Some(error) = pending.terminal.take() {
                wire::InterestNotice::Terminal { error }
            } else {
                continue;
            };
            pending.sequence = pending.sequence.checked_add(1)?;
            return Some((id.clone(), pending.sequence, notice));
        }
        None
    }
    pub(super) fn next(&self, deadline: Instant) -> Option<(String, u64, wire::InterestNotice)> {
        self.signal
            .wait(deadline, || {
                if self.stopped.load(Ordering::Acquire) {
                    Some(None)
                } else {
                    self.take().map(Some)
                }
            })
            .flatten()
    }
    pub(super) fn start(
        self: &Arc<Self>,
        entry: Arc<InterestEntry>,
        subscription: Box<dyn client::Subscription>,
        port: Arc<dyn SealedSyncPort>,
    ) -> Result<(), SyncError> {
        let subscription = OwnedSubscription(subscription);
        let mut job = entry.job.lock().unwrap_or_else(|p| p.into_inner());
        if entry.stop.load(Ordering::Acquire) {
            return Err(SyncError::Cancelled);
        }
        let interests = self.clone();
        let running = entry.clone();
        *job = Some(
            thread::Builder::new()
                .name("tmt-sync-interest".into())
                .spawn(move || {
                    let mut subscription = subscription;
                    while !running.stop.load(Ordering::Acquire) {
                        let io = IoBudget {
                            deadline: Instant::now() + Duration::from_secs(30),
                            cancelled: &running.stop,
                        };
                        let change = complete(subscription.0.next(&io), &io);
                        if running.stop.load(Ordering::Acquire) {
                            break;
                        }
                        if port.context().auth().identity() != *port.context().identity() {
                            interests.terminal(&running, SyncError::Denied);
                            break;
                        }
                        match change {
                            Ok(client::Change::Changed) => interests.changed(&running),
                            Ok(client::Change::CatchUpRequired) => {
                                interests.terminal(&running, SyncError::CatchUpRequired);
                                break;
                            }
                            // An idle receive deadline preserves interest and cadence.
                            // The provider owns probing; a closed probe returns
                            // Unavailable on the next await, which is surfaced below.
                            Err(SyncError::Deadline) => continue,
                            Err(error) => {
                                interests.terminal(&running, error);
                                break;
                            }
                        }
                    }
                })
                .map_err(|_| SyncError::Unavailable)?,
        );
        Ok(())
    }
    pub(super) fn cancel(&self, id: &str) -> Result<(), SyncError> {
        let entry = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id)
            .ok_or(SyncError::Invalid)?;
        entry.stop.store(true, Ordering::Release);
        entry.join();
        self.wake();
        Ok(())
    }
    pub(super) fn stop(&self) -> Vec<Arc<InterestEntry>> {
        self.stopped.store(true, Ordering::Release);
        let entries = std::mem::take(&mut *self.entries.lock().unwrap_or_else(|p| p.into_inner()));
        for entry in entries.values() {
            entry.stop.store(true, Ordering::Release)
        }
        self.wake();
        entries.into_values().collect()
    }
}
/// Dropping a pending subscribe future releases its reserved interest too.
/// The completed registration commits only after its owned awaiter starts.
pub(super) struct Reservation<'a> {
    registry: &'a Interests,
    id: String,
    pub(super) entry: Arc<InterestEntry>,
    committed: bool,
}
impl Reservation<'_> {
    pub(super) fn commit(mut self) {
        self.committed = true;
    }
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.registry.cancel(&self.id);
        }
    }
}
/// Shared across binding registries, with the predicate checked under the
/// same event lock used by wake: no scan-to-wait lost wakeup.
pub(super) struct Signal {
    event: Mutex<()>,
    changed: Condvar,
    #[cfg(test)]
    before_wake: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}
impl Signal {
    pub(super) fn new() -> Self {
        Self {
            event: Mutex::new(()),
            changed: Condvar::new(),
            #[cfg(test)]
            before_wake: Mutex::new(None),
        }
    }
    pub(super) fn wake(&self) {
        #[cfg(test)]
        if let Some(observer) = self.before_wake.lock().unwrap().as_ref() {
            let _ = observer.send(());
        }
        let _event = self.event.lock().unwrap_or_else(|p| p.into_inner());
        self.changed.notify_all();
    }
    #[cfg(test)]
    pub(super) fn observe_wake(&self, observer: std::sync::mpsc::Sender<()>) {
        *self.before_wake.lock().unwrap() = Some(observer);
    }
    #[cfg(test)]
    pub(super) fn hold_event(&self) -> std::sync::MutexGuard<'_, ()> {
        self.event.lock().unwrap()
    }
    pub(super) fn wait<T>(
        &self,
        deadline: Instant,
        mut predicate: impl FnMut() -> Option<T>,
    ) -> Option<T> {
        let mut event = self.event.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if let Some(value) = predicate() {
                return Some(value);
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())?;
            event = self
                .changed
                .wait_timeout(event, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}
impl InterestEntry {
    pub(super) fn join(&self) {
        let job = self.job.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(job) = job {
            job.thread().unpark();
            let _ = job.join();
        }
    }
}
struct OwnedSubscription(Box<dyn client::Subscription>);
impl Drop for OwnedSubscription {
    fn drop(&mut self) {
        self.0.close();
    }
}
struct TaskWake(thread::Thread);
impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark()
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark()
    }
}
/// Drive the injected port on its owned worker; cancellation wakes a parked future.
pub(in crate::object_service) fn complete<T>(
    future: impl Future<Output = Result<T, SyncError>>,
    io: &IoBudget<'_>,
) -> Result<T, SyncError> {
    let waker = Waker::from(Arc::new(TaskWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => {}
        }
        if io.cancelled.load(Ordering::Acquire) {
            return Err(SyncError::Cancelled);
        }
        let left = io
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or(SyncError::Deadline)?;
        thread::park_timeout(left.min(Duration::from_millis(100)));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_an_incomplete_registration_returns_its_reserved_slot() {
        let interests = Interests::new();
        let pending = interests.reserve("1").unwrap();
        assert_eq!(interests.entries.lock().unwrap().len(), 1);
        drop(pending);
        assert!(interests.entries.lock().unwrap().is_empty());
        let _successor = interests
            .reserve("2")
            .expect("no orphan capacity after future cancellation");
    }
    #[test]
    fn coalescing_keeps_a_racing_change_and_reserves_terminal_capacity() {
        let interests = Interests::new();
        let entry = interests.reserve("page").unwrap();
        interests.changed(&entry.entry);
        interests.changed(&entry.entry);
        assert!(
            interests.take().is_none(),
            "no notice before register result"
        );
        interests.enable("page");
        assert_eq!(
            interests.take(),
            Some(("page".into(), 1, wire::InterestNotice::Changed))
        );
        interests.changed(&entry.entry); // Query/send can race another provider wake.
        interests.terminal(&entry.entry, SyncError::CatchUpRequired);
        assert_eq!(
            interests.take(),
            Some(("page".into(), 2, wire::InterestNotice::Changed))
        );
        let (_, sequence, notice) = interests.take().expect("terminal has its own capacity");
        assert_eq!(sequence, 3);
        assert!(
            matches!(notice,wire::InterestNotice::Terminal{error} if error.state==Some(wire::State::CatchUpRequired))
        );
        assert!(interests.take().is_none());
    }
}
