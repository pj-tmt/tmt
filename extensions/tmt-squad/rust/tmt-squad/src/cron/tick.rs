use super::{Error, Job};
use sha2::{Digest, Sha256};

/// Dispatch owns fresh authorization/membership and uncertain-outcome recovery.
/// It runs outside the jobs lock and retains this operation ID on every retry.
pub trait Dispatch {
    fn send(&mut self, job: &Job, operation_id: &str) -> Result<(), Error>;
    /// The lifecycle owner stops further work after cancellation or lease loss.
    fn stopped(&self) -> bool {
        false
    }
}

/// The operation excludes actor, clock and revision so changed slot intent conflicts.
pub fn operation_id(room_id: &str, job_id: &str, slot_ms: i64) -> String {
    let mut hash = Sha256::new();
    hash.update(b"tmt-squad-cron-slot-v1\0");
    for value in [room_id.as_bytes(), job_id.as_bytes()] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    hash.update(slot_ms.to_be_bytes());
    let digest = hash.finalize();
    let mut bytes: [u8; 16] = digest[..16].try_into().expect("fixed digest length");
    bytes[6] = (bytes[6] & 0x0f) | 0x80; // RFC 9562 custom version 8 UUID.
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[derive(Debug, Default)]
pub struct TickReport {
    pub sent: usize,
    pub failures: Vec<Error>,
}

/// Only this clock's previous tick is retained; restarts never read old slots.
pub struct Tick {
    previous_ms: i64,
}

impl Tick {
    pub fn new(now_ms: i64) -> Self {
        Self {
            previous_ms: now_ms,
        }
    }

    pub fn running(
        &mut self,
        now_ms: i64,
        jobs: &[Job],
        dispatch: &mut impl Dispatch,
    ) -> TickReport {
        let after_ms = if now_ms < self.previous_ms {
            now_ms
        } else {
            self.previous_ms.max(now_ms.saturating_sub(300_000))
        };
        self.previous_ms = now_ms;
        send_due(after_ms, now_ms, jobs, dispatch)
    }

    pub fn standalone(now_ms: i64, jobs: &[Job], dispatch: &mut impl Dispatch) -> TickReport {
        send_due(now_ms.saturating_sub(60_000), now_ms, jobs, dispatch)
    }
}

fn send_due(after_ms: i64, now_ms: i64, jobs: &[Job], dispatch: &mut impl Dispatch) -> TickReport {
    let mut report = TickReport::default();
    for job in jobs
        .iter()
        .filter(|job| job.pause.is_none() && job.owner_id.is_some())
    {
        let mut previous = after_ms;
        loop {
            if dispatch.stopped() {
                return report;
            }
            let slot = match job.schedule.next_after(previous) {
                Ok(Some(slot)) if slot <= now_ms => slot,
                Ok(_) => break,
                Err(error) => {
                    report.failures.push(error);
                    break;
                }
            };
            let id = operation_id(&job.room_id, &job.id(), slot);
            match dispatch.send(job, &id) {
                Ok(()) => report.sent += 1,
                Err(error) => report.failures.push(error),
            }
            previous = slot;
        }
    }
    report
}

#[cfg(test)]
mod tests;
