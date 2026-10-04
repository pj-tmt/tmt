//! Board-only cron state: one projection of every squad's jobs plus the clock,
//! loaded on the refresh worker. Paint and input only read it; reads and writes
//! go through `cron_service`, never the store.

mod load;

use crate::cron_service::{CronActor, JobView};
use tmt_squad::cron::ClockStatus;

pub(super) use load::{Fetch, fetch};

/// One successful read. The same instant produced every field.
pub struct Cron {
    /// Stable order: squad name, then numeric job id.
    pub jobs: Vec<JobView>,
    pub clock: ClockStatus,
    /// Resolved once per read so input never touches core; `apply` revalidates it.
    pub actor: Result<CronActor, String>,
}

/// The latest read and whether the newest attempt failed. A failed refresh keeps
/// the previous jobs on screen next to its reason.
#[derive(Default)]
pub struct State {
    pub cron: Option<Cron>,
    pub failure: Option<String>,
}

impl State {
    pub fn replace(&mut self, read: Result<Cron, String>) {
        match read {
            Ok(cron) => {
                self.cron = Some(cron);
                self.failure = None;
            }
            Err(message) => self.failure = Some(message),
        }
    }
}

impl Cron {
    /// The earliest future slot among active jobs.
    pub fn next(&self) -> Option<(i64, &JobView)> {
        self.jobs
            .iter()
            .filter_map(|view| Some((*view.next_ms.first()?, view)))
            .min_by_key(|(at, _)| *at)
    }

    /// The owner's earliest active slot, matched by identity UUID.
    pub fn next_of(&self, owner_id: &str) -> Option<i64> {
        self.jobs
            .iter()
            .filter(|view| view.job.owner_id.as_deref() == Some(owner_id))
            .filter_map(|view| view.next_ms.first().copied())
            .min()
    }
}
