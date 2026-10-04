//! Board-only cron state: one projection of every squad's jobs plus the clock,
//! loaded on the refresh worker. Paint and input only read it; reads and writes
//! go through `cron_service`, never the store.

mod act;
mod controller;
mod forms;
mod half;
mod hints;
mod line;
mod list;
mod load;
mod place;
mod rows;
mod surface;

use crate::cron_service::{CronActor, JobView};
use tmt_squad::cron::ClockStatus;

pub(super) use act::{CronRequest, Op, act};
pub(super) use forms::Draft;
pub(super) use half::{render as render_half, wanted as half_wanted};
pub(super) use hints::{help as help_keys, jobs as jobs_hints};
pub(super) use line::line as home_line;
#[cfg(test)]
pub(super) use line::tests::{
    NOW as TEST_NOW, cron as test_cron, view as test_view, views as test_views,
};
pub(super) use list::{Input as ListInput, List};
pub(super) use load::{Fetch, fetch};
pub(super) use place::Places;
pub(super) use surface::Pane as JobsPane;

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
    /// Jobs of one squad room; a reused squad name never matches an old room.
    pub fn of_room<'a>(&'a self, room_id: &'a str) -> impl Iterator<Item = &'a JobView> {
        self.jobs
            .iter()
            .filter(move |view| view.job.room_id == room_id)
    }

    /// The earliest future slot among active jobs.
    pub fn next(&self) -> Option<(i64, &JobView)> {
        self.jobs
            .iter()
            .filter_map(|view| Some((*view.next_ms.first()?, view)))
            .min_by_key(|(at, _)| *at)
    }

    /// The owner's earliest active slot, matched by identity UUID.
    pub fn next_of(&self, owner_id: &str) -> Option<(i64, &JobView)> {
        self.jobs
            .iter()
            .filter(|view| view.job.owner_id.as_deref() == Some(owner_id))
            .filter_map(|view| Some((*view.next_ms.first()?, view)))
            .min_by_key(|(at, _)| *at)
    }
}

impl State {
    pub fn clock_note<'a>(&self, place: Option<&'a str>) -> line::ClockNote<'a> {
        line::ClockNote {
            checking: self.reads <= 1,
            place,
        }
    }

    /// The member's next job for the detail pane: when, which job, what it says.
    pub fn member_detail(&self, member_id: &str, now_ms: i64) -> Option<String> {
        let (at, view) = self.cron.as_ref()?.next_of(member_id)?;
        Some(format!(
            "{} · {} {}",
            line::time(at, now_ms, &line::zone(view))?,
            view.job.id(),
            line::first_line(&view.job.message)
        ))
    }

    /// The instant the shown text is relative to.
    pub fn now_ms(&self) -> i64 {
        self.cron.as_ref().map_or(0, |cron| cron.read_ms)
    }

    /// A member row's `cron <next>` label; none without an active job, so the
    /// label's presence is the job's.
    pub fn member_label(&self, member_id: &str, now_ms: i64) -> Option<String> {
        let (at, view) = self.cron.as_ref()?.next_of(member_id)?;
        Some(format!(
            "cron {}",
            line::short_time(at, now_ms, &line::zone(view))?
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use line::tests::{NOW, cron, view};

    #[test]
    fn member_labels_match_the_owner_uuid_and_pick_the_earliest_active_slot() {
        let mut soon = view("a", "x", Some(NOW + 1_800_000));
        soon.job.owner_id = Some("u1".into());
        let mut late = view("a", "y", Some(NOW + 7_200_000));
        late.job.owner_id = Some("u1".into());
        let mut paused = view("a", "z", None);
        paused.job.owner_id = Some("u2".into());
        let state = State {
            cron: Some(cron(vec![late, soon, paused], ClockStatus::NoClock)),
            failure: None,
            reads: 2,
        };
        assert_eq!(
            state.member_label("u1", NOW).as_deref(),
            Some("cron Mon 00:00")
        );
        assert_eq!(
            state.member_label("u2", NOW),
            None,
            "a paused job has no next run"
        );
        assert_eq!(state.member_label("someone-else", NOW), None);
        assert_eq!(State::default().member_label("u1", NOW), None);
    }
}
