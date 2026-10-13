//! A pure cadence policy. The eventual subscription owns its one in-flight
//! request and cancellation-aware wait; this value spawns no thread or timer.
use crate::limits;
#[derive(Default)]
pub struct PollCadence {
    last_activity_ms: Option<u64>,
    failures: u32,
}
impl PollCadence {
    pub fn activity(&mut self, now_monotonic_ms: u64) {
        self.last_activity_ms = Some(now_monotonic_ms);
    }
    pub fn failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }
    pub fn succeeded(&mut self) {
        self.failures = 0;
    }
    pub fn delay_ms(&self, now_monotonic_ms: u64, mut jitter: impl FnMut(u64) -> u64) -> u64 {
        let base = match self.last_activity_ms {
            None => limits::NATIVE_POLL_IDLE_MS,
            Some(last) => {
                let age = now_monotonic_ms.saturating_sub(last);
                let steps = age
                    .saturating_sub(limits::NATIVE_POLL_ACTIVITY_MS)
                    .div_ceil(limits::NATIVE_POLL_DECAY_MS)
                    .min(4);
                (limits::NATIVE_POLL_FLOOR_MS << steps).min(limits::NATIVE_POLL_IDLE_MS)
            }
        };
        let backoff = base
            .saturating_mul(1_u64 << self.failures.min(8))
            .min(limits::NATIVE_POLL_BACKOFF_MS);
        let maximum_jitter = backoff / 10;
        backoff
            .saturating_add(jitter(maximum_jitter).min(maximum_jitter))
            .min(limits::NATIVE_POLL_BACKOFF_MS)
    }
}
