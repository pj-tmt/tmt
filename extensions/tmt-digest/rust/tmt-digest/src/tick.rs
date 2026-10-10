//! One bounded minute of scheduling; Core alone admits and settles delivery.

use crate::{core::Error, settings::Mode};
use nix::fcntl::{Flock, FlockArg};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod process;

const MINUTE_MS: u64 = 60_000;
// Refresh arrivals and settings between deadlines; this is not a delivery retry lease.
const OBSERVATION_MS: u64 = 1000;

pub fn now_ms() -> Result<u64, Error> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::new("DIGEST_CLOCK_INVALID", error.to_string()))?
        .as_millis();
    u64::try_from(value).map_err(|_| Error::new("DIGEST_CLOCK_INVALID", "Clock overflow"))
}
struct MinuteClock(Instant);
impl TickClock for MinuteClock {
    fn now_ms(&self) -> Result<u64, Error> {
        now_ms()
    }
    fn elapsed_ms(&self) -> u64 {
        self.0.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
    }
    fn sleep_ms(&mut self, milliseconds: u64) {
        std::thread::sleep(Duration::from_millis(milliseconds));
    }
}

pub fn execute() -> Result<(), Error> {
    let core = crate::core::Core::discover()?;
    let shown = core.api("storage.root", serde_json::json!({}))?;
    let root = shown["dataRoot"]
        .as_str()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            Error::new(
                "CORE_RESPONSE_INVALID",
                "Core reported no absolute data root",
            )
        })?;
    let directory = root.join("digest");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join("tick.lock"))?;
    let _guard = match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(guard) => guard,
        Err((_, error)) if error == nix::errno::Errno::EWOULDBLOCK => return Ok(()),
        Err((_, error)) => return Err(Error::new("DIGEST_TICK_LOCK", error.to_string())),
    };
    let mut clock = MinuteClock(Instant::now());
    let entered = now_ms()?;
    let remaining = MINUTE_MS - entered % MINUTE_MS;
    let boundary = clock.0 + Duration::from_millis(remaining);
    let boundary_ms = entered
        .checked_add(remaining)
        .ok_or_else(|| Error::new("DIGEST_CLOCK_INVALID", "Minute boundary overflow"))?;
    if Instant::now() >= boundary || now_ms()? >= boundary_ms {
        return Ok(());
    }
    let core = core.until(boundary + Duration::from_secs(15));
    let path = core.settings_path()?;
    let mut port = process::TickProcess::new(core, path, boundary_ms, boundary);
    let outcome = run_until(&mut clock, &mut port, boundary_ms, remaining);
    match (outcome, port.finish()) {
        (Ok(()), result) => result,
        (Err(error), Err(partial)) => Err(Error::new(
            &error.code,
            format!("{}; {}", error.message, partial.message),
        )),
        (result @ Err(_), Ok(())) => result,
    }
}

pub struct TickObservation {
    pub identity_id: String,
    pub mode: Mode,
    pub flush_count: u64,
    pub held_count: u64,
    pub oldest_held_at_ms: Option<u64>,
}

pub trait TickClock {
    fn now_ms(&self) -> Result<u64, Error>;
    fn elapsed_ms(&self) -> u64;
    fn sleep_ms(&mut self, milliseconds: u64);
}

pub trait TickPort {
    /// Re-read configuration and batch Core observations; reconcile hold policies.
    fn observe(&mut self) -> Result<Vec<TickObservation>, Error>;
    /// Re-read this member's current setting before adding eligibility or checking.
    fn deliver_if_current(&mut self, identity_id: &str) -> Result<(), Error>;
}

impl TickObservation {
    fn deadline(&self, now: u64) -> Option<u64> {
        let Mode::Interval { milliseconds, .. } = self.mode else {
            return None;
        };
        if self.held_count == 0 {
            return None;
        }
        if self.held_count >= self.flush_count {
            return Some(now);
        }
        self.oldest_held_at_ms
            .map(|oldest| oldest.saturating_add(milliseconds))
    }
}

/// The minute is fixed at entry. A wall-clock rollback cannot extend the lifetime.
#[cfg(test)]
pub fn run(clock: &mut impl TickClock, port: &mut impl TickPort) -> Result<(), Error> {
    let started = clock.now_ms()?;
    let boundary = started
        .checked_div(MINUTE_MS)
        .and_then(|minute| minute.checked_add(1))
        .and_then(|minute| minute.checked_mul(MINUTE_MS))
        .ok_or_else(|| Error::new("DIGEST_CLOCK_INVALID", "Minute boundary overflow"))?;
    let budget = boundary - started;
    let end_elapsed = clock.elapsed_ms().saturating_add(budget);
    run_until(clock, port, boundary, end_elapsed)
}
fn run_until(
    clock: &mut impl TickClock,
    port: &mut impl TickPort,
    boundary: u64,
    end_elapsed: u64,
) -> Result<(), Error> {
    loop {
        let now = clock.now_ms()?;
        if now >= boundary || clock.elapsed_ms() >= end_elapsed {
            return Ok(());
        }
        let rows = port.observe()?;
        let mut next = boundary.min(now.saturating_add(OBSERVATION_MS));
        for row in rows {
            let now = clock.now_ms()?;
            if now >= boundary || clock.elapsed_ms() >= end_elapsed {
                return Ok(());
            }
            if let Some(deadline) = row.deadline(now) {
                if deadline <= now {
                    port.deliver_if_current(&row.identity_id)?;
                } else {
                    next = next.min(deadline);
                }
            }
        }
        let now = clock.now_ms()?;
        let remaining = end_elapsed.saturating_sub(clock.elapsed_ms());
        let delay = next.saturating_sub(now).min(remaining);
        if delay > 0 {
            clock.sleep_ms(delay);
        }
    }
}

#[cfg(test)]
#[path = "tick/tests.rs"]
mod tests;
