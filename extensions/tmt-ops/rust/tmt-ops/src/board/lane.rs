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
    HostAction(super::settings::host_action::Selection),
    /// The theme picker, for this squad tab (none on the aggregate tabs).
    ThemePicker(Option<String>),
}

#[derive(Debug)]
pub(super) enum Job {
    Act(Request),
    Jump {
        member: String,
        seen: Option<super::unread::publish::Seen>,
        publisher: super::unread::publish::Writer,
    },
    Send {
        request: Request,
        ticket: super::conversation::outgoing::Ticket,
    },
    Recovery(super::recovery::load::Task),
    Discovery(super::discovery::load::Task),
    Open(Opening),
    /// Save the header's token-rate window with no settings overlay open.
    TokenWindow(String),
    ViewState(super::view_state::Writer),
    ReadMarkers(super::unread::publish::Writer),
}

impl Job {
    /// What the board shows until the job's event arrives.
    pub fn progress(&self) -> &'static str {
        match self {
            Self::Act(request) | Self::Send { request, .. } => request.progress(),
            Self::Jump { .. } => "Jumping…",
            Self::ReadMarkers(_) => "Saving read markers…",
            Self::Recovery(_) => "Reading or applying the selected workspace snapshot…",
            Self::Discovery(_) => "Scanning or binding the selected agent…",
            Self::Open(Opening::Settings | Opening::HostAction(_)) => "Loading settings…",
            Self::Open(Opening::ThemePicker(_)) => "Loading themes…",
            Self::TokenWindow(_) => "Saving the token window…",
            Self::ViewState(_) => "Saving the board view…",
        }
    }
}

/// The result of one job.
pub(super) fn run(core: &Core, labels: Option<&Refresh>, job: Job) -> BoardEvent {
    match job {
        Job::ReadMarkers(writer) => BoardEvent::ReadMarkersPublished(writer.publish(core)),
        Job::Jump {
            member,
            seen,
            publisher,
        } => {
            let seen = seen.filter(|seen| seen.matches_member(&member));
            let outcome = execute(core, Request::Jump(member));
            let outcome = match (outcome, seen) {
                (Ok(super::ActionOutcome::Message(message)), Some(seen)) => {
                    Ok(super::ActionOutcome::JumpRead {
                        message,
                        markers: publisher.publish_seen(&seen, core),
                    })
                }
                (outcome, _) => outcome,
            };
            BoardEvent::Acted {
                jump: true,
                sends: false,
                outcome,
            }
        }
        Job::ViewState(writer) => {
            writer.publish(std::time::Instant::now(), false);
            BoardEvent::ViewStatePublished
        }
        Job::Recovery(task) => BoardEvent::Recovery(super::recovery::load::complete(core, task)),
        Job::Discovery(task) => BoardEvent::Discovery(super::discovery::load::complete(core, task)),
        Job::Send { request, ticket } => {
            if let Request::Reply {
                me,
                squad,
                request,
                from,
                text,
            } = request
            {
                let outcome =
                    match crate::send::answer_or_talk(core, &squad, &me, &request, &from, &text) {
                        crate::send::AnswerOrTalk::Talk(outcome) => super::ActionOutcome::Send(
                            Box::new(super::conversation::outgoing::Completed {
                                ticket: Some(ticket),
                                outcome,
                                what: "Message".into(),
                                to: from,
                            }),
                        ),
                        crate::send::AnswerOrTalk::Answer(outcome) => super::ActionOutcome::Answer(
                            Box::new(super::conversation::outgoing::Answered {
                                ticket,
                                outcome,
                                to: from,
                            }),
                        ),
                    };
                return BoardEvent::Acted {
                    jump: false,
                    sends: true,
                    outcome: Ok(outcome),
                };
            }
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
