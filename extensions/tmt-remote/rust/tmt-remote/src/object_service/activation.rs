//! A mount demand joins one bounded setup, never a retry loop or another service owner.
use super::{ActivateError, ObjectService, State};
use crate::{
    limits,
    mount::{ActivationSink, IdleClock, Mounts},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

struct ActivationAttempt {
    deadline: Instant,
    done: Mutex<bool>,
    changed: Condvar,
}
impl ActivationAttempt {
    fn finish(&self) {
        *self.done.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.changed.notify_all();
    }
    fn wait(&self, deadline: Instant) {
        let mut done = self.done.lock().unwrap_or_else(|p| p.into_inner());
        while !*done {
            let Some(left) = deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
            else {
                break;
            };
            done = self
                .changed
                .wait_timeout(done, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}
struct ActivationDemand {
    name: &'static str,
    flight: Option<Arc<ActivationAttempt>>,
    retry_at: Option<Instant>,
}
struct ActivationQueue {
    closed: bool,
    slots: Vec<ActivationDemand>,
    pending: VecDeque<(&'static str, Arc<ActivationAttempt>)>,
}
/// A static mount hook owns only setup requests and a status view, not the lease/backend.
/// Serve joins the scoped borrower before dropping ObjectService.
pub struct Reactivation {
    state: Arc<Mutex<State>>,
    queue: Mutex<ActivationQueue>,
    changed: Condvar,
    stop: Arc<AtomicBool>,
    clock: IdleClock,
    capacity: usize,
}
impl Reactivation {
    pub(super) fn new(
        service: &ObjectService<'_>,
        stop: Arc<AtomicBool>,
        clock: IdleClock,
    ) -> Arc<Self> {
        let slots = service
            .locked()
            .slots
            .iter()
            .map(|s| ActivationDemand {
                name: s.name,
                flight: None,
                retry_at: None,
            })
            .collect();
        Arc::new(Self {
            state: Arc::clone(&service.state),
            queue: Mutex::new(ActivationQueue {
                closed: false,
                slots,
                pending: VecDeque::new(),
            }),
            changed: Condvar::new(),
            stop,
            clock,
            capacity: service.bounds.buses,
        })
    }
    fn healthy(&self, name: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .slots
            .iter()
            .any(|s| s.name == name && s.running())
    }
    fn next(&self) -> Option<(&'static str, Arc<ActivationAttempt>)> {
        let mut queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if queue.closed || self.stop.load(Ordering::Acquire) {
                return None;
            }
            if let Some(next) = queue.pending.pop_front() {
                return Some(next);
            }
            queue = self.changed.wait(queue).unwrap_or_else(|p| p.into_inner());
        }
    }
    fn run(&self, service: &ObjectService<'_>, mounts: &Mounts) {
        while let Some((name, attempt)) = self.next() {
            let outcome = if self.stop.load(Ordering::Acquire) {
                Err(ActivateError::Stopped)
            } else if self.healthy(name) {
                Ok(())
            } else {
                service
                    .activate_until(mounts, name, attempt.deadline)
                    .map(|_| ())
            };
            let failed = outcome.is_err() || !self.healthy(name);
            let mut queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            let slot = queue
                .slots
                .iter_mut()
                .find(|s| s.name == name)
                .expect("declared demand");
            if failed {
                slot.retry_at = Some((self.clock)() + limits::OBJECT_REACTIVATION_COOLDOWN);
            }
            // Publish completion before making a new flight possible.
            attempt.finish();
            slot.flight = None;
        }
    }
    #[cfg(test)]
    pub(super) fn queued(&self) -> usize {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pending
            .len()
    }
    #[cfg(test)]
    pub(super) fn idle(&self) -> bool {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .slots
            .iter()
            .all(|s| s.flight.is_none())
    }
    fn disable(&self) {
        let mut queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        queue.closed = true;
        queue.pending.clear();
        for slot in &queue.slots {
            if let Some(attempt) = &slot.flight {
                attempt.finish();
            }
        }
        self.changed.notify_all();
    }
}
impl ActivationSink for Reactivation {
    fn prepare(&self, name: &str, deadline: Instant) -> bool {
        if Instant::now() >= deadline || self.stop.load(Ordering::Acquire) {
            return false;
        }
        if self.healthy(name) {
            return true;
        }
        let mut queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        if queue.closed {
            return false;
        }
        let Some(index) = queue.slots.iter().position(|s| s.name == name) else {
            return false;
        };
        if queue.slots[index]
            .retry_at
            .is_some_and(|t| (self.clock)() < t)
        {
            return false;
        }
        let attempt = if let Some(attempt) = &queue.slots[index].flight {
            Arc::clone(attempt)
        } else {
            if queue.slots.iter().filter(|s| s.flight.is_some()).count() >= self.capacity {
                return false;
            }
            let attempt = Arc::new(ActivationAttempt {
                deadline: deadline.min(Instant::now() + limits::OBJECT_REACTIVATION),
                done: Mutex::new(false),
                changed: Condvar::new(),
            });
            queue.slots[index].flight = Some(Arc::clone(&attempt));
            let name = queue.slots[index].name;
            queue.pending.push_back((name, Arc::clone(&attempt)));
            self.changed.notify_one();
            attempt
        };
        drop(queue);
        attempt.wait(deadline.min(attempt.deadline));
        // An unfinished candidate may already have registered its generation. Only an
        // installed live channel can supply a ticket; false never refuses forwarding.
        !self.stop.load(Ordering::Acquire) && self.healthy(name)
    }
    fn close(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).stopped = true;
        self.disable();
    }
}
struct ScopedReactivation<'a>(&'a Reactivation);
impl Drop for ScopedReactivation<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}
impl ObjectService<'_> {
    pub fn readiness(&self) -> super::ObjectReadiness {
        super::ObjectReadiness::live(Arc::clone(&self.state))
    }
    pub fn reactivation(&self, stop: Arc<AtomicBool>) -> Arc<Reactivation> {
        Reactivation::new(self, stop, Arc::new(Instant::now))
    }
    /// Mounts is captured by static HTTP workers and cannot hold this lease borrow.
    /// One scoped worker handles setup; closing the hook wakes it on every exit path.
    pub fn with_reactivation<T>(
        &self,
        hook: &Reactivation,
        mounts: &Mounts,
        serve: impl FnOnce() -> T,
    ) -> T {
        std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name("remote-object-setup".into())
                .spawn_scoped(scope, || hook.run(self, mounts));
            if worker.is_err() {
                hook.disable();
            }
            let guard = ScopedReactivation(hook);
            let result = serve();
            drop(guard);
            if let Ok(worker) = worker {
                worker.join().expect("object setup worker panicked");
            }
            result
        })
    }
}
