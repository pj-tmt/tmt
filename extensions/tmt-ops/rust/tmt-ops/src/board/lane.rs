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
pub(super) enum Opening {
    Settings,
    /// The settings overlay for this squad, opened from the Workspace settings menu.
    SquadSettings(String),
    HostAction(Box<super::settings::host_action::Selection>),
    /// The theme picker, for this squad tab (none on the aggregate tabs).
    ThemePicker(Option<String>),
}

#[derive(Debug)]
pub(super) enum Job {
    Act(Request),
    Jump {
        member: String,
    },
    Send {
        request: Request,
        ticket: super::conversation::outgoing::Ticket,
    },
    Recovery(super::recovery::load::Task),
    Discovery(super::discovery::load::Task),
    MemberSession(super::member_session::Task),
    NewMember(super::new_member::Task),
    IdleAlert(super::idle_alert_editor::Task),
    Open(Opening),
    /// Save the header's token-rate window with no settings overlay open.
    TokenWindow(String),
    ViewState(super::view_state::Writer),
    AcknowledgeAnswers(super::unread::publish::Writer),
    /// Read the checklist summary of the conversation on screen.
    Notes(super::conversation_checklist::Task),
    /// Read the running versions once, at start.
    Versions,
}

impl Job {
    /// Whether the user is waiting on this job. Quiet work, and a choice saved with no
    /// overlay open, shows no progress; its failure still shows.
    pub fn shows_progress(&self) -> bool {
        !matches!(
            self,
            Self::Recovery(_)
                | Self::TokenWindow(_)
                | Self::ViewState(_)
                | Self::AcknowledgeAnswers(_)
                | Self::Notes(_)
                | Self::Versions
        )
    }

    /// What the board shows until the job's event arrives.
    pub fn progress(&self) -> &'static str {
        match self {
            Self::Act(request) | Self::Send { request, .. } => request.progress(),
            Self::Jump { .. } => "Jumping…",
            Self::AcknowledgeAnswers(_) => "Acknowledging answers…",
            Self::Notes(_) => "Reading the checklist…",
            Self::Versions => "Reading the versions…",
            Self::Recovery(_) => "Reading or applying the selected workspace snapshot…",
            Self::MemberSession(_) => "Loading session…",
            Self::IdleAlert(_) => "Reading or saving idle alert…",
            Self::NewMember(_) => "Preparing or creating member…",
            Self::Discovery(_) => "Scanning or binding the selected agent…",
            Self::Open(Opening::Settings | Opening::SquadSettings(_) | Opening::HostAction(_)) => {
                "Loading settings…"
            }
            Self::Open(Opening::ThemePicker(_)) => "Loading themes…",
            Self::TokenWindow(_) => "Saving the token window…",
            Self::ViewState(_) => "Saving the board view…",
        }
    }
}

/// The result of one job.
pub(super) fn run(core: &Core, labels: Option<&Refresh>, job: Job) -> BoardEvent {
    match job {
        Job::AcknowledgeAnswers(writer) => BoardEvent::AnswersAcknowledged(writer.publish(core)),
        Job::Notes(task) => {
            BoardEvent::NotesRead(super::conversation_checklist::complete(core, task))
        }
        Job::Versions => BoardEvent::VersionsRead(super::versions::Versions::read(core)),
        Job::Jump { member } => BoardEvent::Acted {
            jump: true,
            sends: false,
            outcome: execute(core, Request::Jump(member)),
        },
        Job::ViewState(writer) => {
            writer.publish(std::time::Instant::now(), false);
            BoardEvent::ViewStatePublished
        }
        Job::Recovery(task) => BoardEvent::Recovery(super::recovery::load::complete(core, task)),
        Job::IdleAlert(task) => {
            BoardEvent::IdleAlert(super::idle_alert_editor::complete(core, task))
        }
        Job::NewMember(task) => BoardEvent::NewMember(super::new_member::complete(core, task)),
        Job::MemberSession(task) => {
            BoardEvent::MemberSession(super::member_session::complete(core, task))
        }
        Job::Discovery(task) => BoardEvent::Discovery(super::discovery::load::complete(core, task)),
        Job::Send { request, ticket } => {
            let mut outcome = execute(core, request);
            if let Ok(super::ActionOutcome::Send(completed)) = &mut outcome {
                completed.ticket = Some(ticket);
            }
            BoardEvent::Acted {
                jump: false,
                sends: true,
                outcome,
            }
        }
        Job::Act(request) => {
            let jump = matches!(request, Request::Jump(_));
            let sends = request.sends();
            let digest = matches!(request, Request::Digest(_) | Request::DigestSwitch(_));
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
        Job::Open(opening) => {
            let board_pane = matches!(&opening, Opening::HostAction(_))
                .then(|| {
                    core.json(&["whoami"]).ok().and_then(|caller| {
                        caller["pane"]
                            .as_str()
                            .filter(|pane| !pane.is_empty())
                            .map(str::to_owned)
                    })
                })
                .flatten();
            BoardEvent::Opened {
                opening,
                config: Config::load(core).map_err(|error| error.message),
                board_pane,
            }
        }
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn only_jobs_the_user_waits_on_show_progress() {
        assert!(Job::Act(Request::Back).shows_progress());
        assert!(Job::Open(Opening::Settings).shows_progress());
        assert!(!Job::TokenWindow("1h".into()).shows_progress());
    }
}
