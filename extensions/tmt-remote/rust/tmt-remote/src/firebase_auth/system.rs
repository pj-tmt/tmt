//! Invocation-owned time and entropy. No timer thread survives the owner.
use super::{Cancellation, Clock, Jitter, Time};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct SystemClock {
    origin: Instant,
}
impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}
impl Clock for SystemClock {
    fn now(&self) -> Time {
        Time {
            wall_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            monotonic_ms: self
                .origin
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        }
    }
    fn wait_until(&self, deadline_ms: u64, cancel: &dyn Cancellation) -> bool {
        while !cancel.cancelled() {
            let remaining = deadline_ms.saturating_sub(self.now().monotonic_ms);
            if remaining == 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(remaining.min(10)));
        }
        false
    }
}
pub struct RandomJitter;
impl Jitter for RandomJitter {
    fn milliseconds(&self, maximum_inclusive: u64) -> u64 {
        // Positive jitter is enforced by each schedule's owner. Entropy failure
        // uses the upper bound, never a retrying entropy loop or a shorter wait.
        getrandom::u64()
            .map(|value| {
                if maximum_inclusive == u64::MAX {
                    value
                } else {
                    value % (maximum_inclusive + 1)
                }
            })
            .unwrap_or(maximum_inclusive)
    }
}
