//! Drive an injected async port on its invocation-owned thread, with bounded
//! parking and the caller's cancellation/deadline. No worker or timer is created.
use crate::{objects::IoBudget, sealed_sync::SyncError};
use std::{
    future::Future,
    sync::{Arc, atomic::Ordering},
    task::{Context, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant},
};
struct TaskWake(thread::Thread);
impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark()
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark()
    }
}
pub(crate) fn complete<T>(
    future: impl Future<Output = T>,
    io: &IoBudget<'_>,
) -> Result<T, SyncError> {
    let waker = Waker::from(Arc::new(TaskWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return Ok(value);
        }
        if io.cancelled.load(Ordering::Acquire) {
            return Err(SyncError::Cancelled);
        }
        let left = io
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|n| !n.is_zero())
            .ok_or(SyncError::Deadline)?;
        thread::park_timeout(left.min(Duration::from_millis(100)));
    }
}
