//! Board-only cron state: one projection of every squad's jobs plus the clock,
//! loaded on the refresh worker. Paint and input only read it; reads and writes
//! go through `cron_service`, never the store.

mod act;
mod controller;
mod forms;
mod hints;
mod line;
mod list;
mod load;
mod place;
mod rows;
mod surface;

use crate::cron_service::{CronActor, JobView};
use tmt_ops::cron::ClockStatus;

pub(super) use act::{CronRequest, Op, act};
pub(super) use forms::Draft;
#[cfg(test)]
pub(super) use line::tests::{
    NOW as TEST_NOW, cron as test_cron, view as test_view, views as test_views,
};
pub(super) use list::{Input as ListInput, List};
pub(super) use load::{Fetch, fetch};
pub(super) use place::Places;
pub(super) use rows::{detail as job_detail, detail_id};

/// One successful read. The same instant produced every field.
pub struct Cron {
    /// Stable order: squad name, then numeric job id.
    pub jobs: Vec<JobView>,
    pub clock: ClockStatus,
    /// Resolved once per read so input never touches core; `apply` revalidates it.
    pub actor: Result<CronActor, String>,
    /// The clock holder's `session:window`, when tmux could say; else the pane id shows.
    pub place: Option<String>,
    /// When the read happened. Every time shown derives from it, so the text is
    /// one coherent snapshot of the read rather than of a moving clock.
    pub read_ms: i64,
}

/// The latest read and whether the newest attempt failed. A failed refresh keeps
/// the previous jobs on screen next to its reason.
#[derive(Default)]
pub struct State {
    pub cron: Option<Cron>,
    pub failure: Option<String>,
    /// Successful reads so far; the first one may precede the board's own clock.
    pub reads: u32,
}

impl State {
    pub fn replace(&mut self, read: Result<Cron, String>) {
        match read {
            Ok(cron) => {
                self.cron = Some(cron);
                self.failure = None;
                self.reads = self.reads.saturating_add(1);
            }
            Err(message) => self.failure = Some(message),
        }
    }
}

impl Cron {
    /// The same room-UUID admission for picker identities and painted jobs.
    /// `None` is the explicit all-rooms context, never a squad-name fallback.
    pub fn jobs_in<'a>(&'a self, room_id: Option<&'a str>) -> impl Iterator<Item = &'a JobView> {
        self.jobs
            .iter()
            .filter(move |view| room_id.is_none_or(|room| view.job.room_id == room))
    }
}

impl State {
    pub fn clock_note<'a>(&self, place: Option<&'a str>) -> line::ClockNote<'a> {
        line::ClockNote {
            checking: self.reads <= 1,
            place,
        }
    }

    /// The instant the shown text is relative to.
    pub fn now_ms(&self) -> i64 {
        self.cron.as_ref().map_or(0, |cron| cron.read_ms)
    }
}
