//! The thread behind keys and clicks. A user action (send, jump, open, copy,
//! save a tab order) and the config read behind a picker both start commands or
//! take locks, so the session loop never runs them: it submits a [`Job`], paints
//! a placeholder notice, and applies the [`BoardEvent`] that comes back. Jobs run
//! in the order submitted.

use super::{BoardEvent, Request, execute};
use crate::{config::Config, core::Core, labels::Refresh};
use std::{
    sync::mpsc::{Sender, channel},
    thread::JoinHandle,
};

/// The overlay a config read is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Opening {
    ViewPicker,
    Settings,
    /// The theme picker, for this squad tab (none on the aggregate tabs).
    ThemePicker(Option<String>),
}

#[derive(Debug)]
pub(super) enum Job {
    Act(Request),
    Open(Opening),
    /// Save the header's token-rate window with no settings overlay open.
    TokenWindow(String),
}

impl Job {
    /// What the board shows until the job's event arrives.
    pub fn progress(&self) -> &'static str {
        match self {
            Self::Act(request) => request.progress(),
            Self::Open(Opening::ViewPicker) => "Loading views…",
            Self::Open(Opening::Settings) => "Loading settings…",
            Self::Open(Opening::ThemePicker(_)) => "Loading themes…",
            Self::TokenWindow(_) => "Saving the token window…",
        }
    }
}

/// The result of one job.
pub(super) fn run(core: &Core, labels: Option<&Refresh>, job: Job) -> BoardEvent {
    match job {
        Job::Act(request) => {
            let jump = matches!(request, Request::Jump(_));
            let sends = request.sends();
            let digest = matches!(request, Request::Digest(_));
            let outcome = execute(core, request);
            // What the extension supplies changed: read it now, not at the next tick.
            if let (true, Ok(_), Some(labels)) = (digest, &outcome, labels) {
                labels.now();
            }
            BoardEvent::Acted {
                jump,
                sends,
                outcome,
            }
        }
        Job::Open(opening) => BoardEvent::Opened {
            opening,
            config: Config::load(core).map_err(|error| error.message),
        },
        Job::TokenWindow(window) => BoardEvent::TokenWindowSaved(
            Config::load(core)
                .and_then(|mut config| {
                    config.set_setting(None, "board.token_rate.window", &window)?;
                    Ok(config)
                })
                .map_err(|error| error.message),
        ),
    }
}

pub(super) struct Lane {
    jobs: Option<Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Lane {
    pub fn spawn(core: Core, labels: Option<Refresh>, events: Sender<BoardEvent>) -> Self {
        let (jobs, queue) = channel();
        let thread = std::thread::spawn(move || {
            for job in queue {
                if events.send(run(&core, labels.as_ref(), job)).is_err() {
                    break;
                }
            }
        });
        Self {
            jobs: Some(jobs),
            thread: Some(thread),
        }
    }

    pub fn submit(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
    }

    /// Ends the lane once its queued jobs finish; each command has its own deadline.
    pub fn finish(mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
