//! Cron controls and the staged forms behind them. A form is a typed draft whose
//! steps use the board's one-line input; nothing is written until the last step,
//! and the service revalidates actor, room, owner and revision when it applies.

use super::{CronRequest, Op, rows::key_of, rows::row_id};
use crate::{
    board::app::{App, Choice, Compose, Effect, Hint, Input, Menu, MenuEntry, Request},
    cron_service::{CronActor, JobKey, JobView, schedule_text},
};
use tmt_squad::cron::Schedule;

/// Longest message the one-line input can hold; longer ones are kept as stored.
const LINE_LIMIT: usize = 4000;
const FORMS: &str = "every 3h · daily 09:00 · weekdays 09:00 · mon,thu 10:00 · cron 0 */3 * * *";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    New,
    Edit,
    Reassign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Owner,
    Message,
    Schedule,
}

/// What was typed so far, plus everything the opening captured. `original`
/// holds the stored message and schedule text an edit compares against.
#[derive(Debug, Clone)]
pub struct Draft {
    kind: Kind,
    step: Step,
    actor: CronActor,
    squad: String,
    room_id: String,
    job: Option<(JobKey, u64)>,
    zone: String,
    owner: String,
    message: Option<String>,
    original: Option<(String, String)>,
}

impl Draft {
    fn prompt(&self) -> String {
        let step = match self.step {
            Step::Owner => "owner",
            Step::Message => "message",
            Step::Schedule => "schedule",
        };
        match (&self.job, self.kind) {
            (Some((key, _)), Kind::Reassign) => {
                format!("reassign {} {} · {step}", key.squad, key.id)
            }
            (Some((key, _)), _) => format!("edit {} {} · {step}", key.squad, key.id),
            (None, _) => format!("new job in {} · {step}", self.squad),
        }
    }
    fn hint(&self) -> &'static str {
        match self.step {
            Step::Owner => "a member of this squad",
            Step::Message => "sent as written, with no substitution",
            Step::Schedule => FORMS,
        }
    }
}

impl App {
    fn cron_actor(&self) -> Result<CronActor, String> {
        match self.cron.cron.as_ref().map(|cron| cron.actor.clone()) {
            Some(Ok(actor)) => Ok(actor),
            Some(Err(why)) => Err(format!("Cron changes need a known user: {why}")),
            None => Err("Cron jobs have not been read yet.".into()),
        }
    }

    fn cron_job(&self, id: &str) -> Option<JobView> {
        self.cron
            .cron
            .iter()
            .flat_map(|cron| &cron.jobs)
            .find(|view| row_id(&key_of(view)) == id)
            .cloned()
    }

    /// `n e p x o d` on the selected job. `row` is that job's row id, from the
    /// jobs half or the `c` list; `n` also works with none, in the shown squad.
    pub(in crate::board) fn cron_key(&mut self, key: char, row: Option<&str>) -> Effect {
        let job = row.and_then(|id| self.cron_job(id));
        if key == 'n' {
            return self.cron_new(job);
        }
        let Some(job) = job else {
            return self.say("Select a job first.");
        };
        let actor = match self.cron_actor() {
            Ok(actor) => actor,
            Err(why) => return self.say(why),
        };
        let job_key = key_of(&job);
        let revision = job.job.revision;
        let existing = |op| {
            Request::Cron(CronRequest::Existing {
                actor: actor.clone(),
                key: job_key.clone(),
                revision,
                op,
            })
        };
        match key {
            'p' => match job.job.state() {
                "on" => Effect::Act(existing(Op::Pause)),
                "paused" => Effect::Act(existing(Op::Resume)),
                _ => self.say("This job has no owner; o reassigns it."),
            },
            'x' => Effect::Act(existing(Op::Send)),
            'd' => {
                self.menu = Some(Menu {
                    row_send: None,
                    link: None,
                    prefill: String::new(),
                    title: format!("delete {} {}?", job_key.squad, job_key.id),
                    entries: vec![
                        MenuEntry {
                            key: "n".into(),
                            label: format!("keep {}", job_key.id),
                            choice: Choice::Dismiss,
                        },
                        MenuEntry {
                            key: "y".into(),
                            label: format!("delete {} for good", job_key.id),
                            choice: Choice::Cron(match existing(Op::Remove) {
                                Request::Cron(request) => request,
                                _ => unreachable!("a cron request"),
                            }),
                        },
                    ],
                    selected: 0,
                    surface: Default::default(),
                });
                Effect::None
            }
            'e' | 'o' => {
                let zone = job.job.schedule.document()["zone"]
                    .as_str()
                    .unwrap_or("UTC")
                    .to_owned();
                let message = job.job.message.clone();
                let editable =
                    !message.chars().any(char::is_control) && message.chars().count() <= LINE_LIMIT;
                let draft = Draft {
                    kind: if key == 'e' {
                        Kind::Edit
                    } else {
                        Kind::Reassign
                    },
                    step: if key == 'e' && editable {
                        Step::Message
                    } else if key == 'e' {
                        Step::Schedule
                    } else {
                        Step::Owner
                    },
                    actor,
                    squad: job_key.squad.clone(),
                    room_id: job_key.room_id.clone(),
                    job: Some((job_key, revision)),
                    zone,
                    owner: String::new(),
                    message: None,
                    original: Some((message, schedule_text(&job.job.schedule))),
                };
                let prefill = match draft.step {
                    Step::Owner => job.owner_name.clone().unwrap_or_default(),
                    Step::Message => draft.original.as_ref().expect("edit").0.clone(),
                    Step::Schedule => draft.original.as_ref().expect("edit").1.clone(),
                };
                let note = (key == 'e' && !editable).then(|| {
                    "the message has several lines or is long: kept as stored, use tmt sq cron edit --message".to_owned()
                });
                self.cron_ask(draft, prefill, note.map(|text| Hint { text, error: false }))
            }
            _ => Effect::None,
        }
    }

    fn cron_new(&mut self, near: Option<JobView>) -> Effect {
        let actor = match self.cron_actor() {
            Ok(actor) => actor,
            Err(why) => return self.say(why),
        };
        // In the jobs half this is the shown squad; in the `c` list, the squad of
        // the selected job. Neither exists on an empty list of another tab.
        let target = match near {
            Some(job) => Some((job.job.squad.clone(), job.job.room_id.clone())),
            None => super::half::room(self)
                .zip(self.current.clone())
                .map(|(room, name)| (name, room.to_owned())),
        };
        let Some((squad, room_id)) = target else {
            return self.say("Open a squad tab, then n adds a job to that squad.");
        };
        let zone = match Schedule::local_zone() {
            Ok(zone) => zone,
            Err(error) => return self.say(error.message),
        };
        let owner = self
            .selected_row()
            .filter(|_| self.current.as_deref() == Some(&squad))
            .and_then(|row| row["name"].as_str())
            .map(str::to_owned)
            .unwrap_or_default();
        let draft = Draft {
            kind: Kind::New,
            step: Step::Owner,
            actor,
            squad,
            room_id,
            job: None,
            zone,
            owner: String::new(),
            message: None,
            original: None,
        };
        self.cron_ask(draft, owner, None)
    }

    fn cron_ask(&mut self, draft: Draft, text: String, hint: Option<Hint>) -> Effect {
        self.input = Some(Input {
            row_send: None,
            alternative: None,
            quote: None,
            link: None,
            prompt: draft.prompt(),
            text,
            compose: Compose::Cron,
            squad: draft.squad.clone(),
            hint: hint.or_else(|| {
                Some(Hint {
                    text: draft.hint().into(),
                    error: false,
                })
            }),
        });
        self.cron_draft = Some(draft);
        Effect::None
    }

    fn cron_retry(&mut self, draft: Draft, text: String, why: String) -> Effect {
        self.input = Some(Input {
            row_send: None,
            alternative: None,
            quote: None,
            link: None,
            prompt: draft.prompt(),
            text,
            compose: Compose::Cron,
            squad: draft.squad.clone(),
            hint: Some(Hint {
                text: why,
                error: true,
            }),
        });
        self.cron_draft = Some(draft);
        Effect::None
    }

    /// Enter on a form step. The text arrives exactly as typed.
    pub(in crate::board) fn cron_submit(&mut self, text: String) -> Effect {
        let Some(mut draft) = self.cron_draft.take() else {
            return Effect::None;
        };
        match draft.step {
            Step::Owner => {
                let owner = text.trim();
                if owner.is_empty() {
                    return self.cron_retry(draft, text, "Name a member of this squad.".into());
                }
                draft.owner = owner.to_owned();
                if draft.kind == Kind::Reassign {
                    return self.cron_finish(draft, None);
                }
                draft.step = Step::Message;
                self.cron_ask(draft, String::new(), None)
            }
            Step::Message => {
                if draft.kind == Kind::New && text.trim().is_empty() {
                    return self.cron_retry(draft, text, "The message cannot be empty.".into());
                }
                let unchanged = draft
                    .original
                    .as_ref()
                    .is_some_and(|(original, _)| *original == text);
                draft.message = (!unchanged).then_some(text);
                draft.step = Step::Schedule;
                let prefill = draft
                    .original
                    .as_ref()
                    .map(|(_, schedule)| schedule.clone())
                    .unwrap_or_default();
                self.cron_ask(draft, prefill, None)
            }
            Step::Schedule => {
                let schedule = text.trim().to_owned();
                if schedule.is_empty() {
                    return self.cron_retry(draft, text, "Give a schedule.".into());
                }
                let same = draft
                    .original
                    .as_ref()
                    .is_some_and(|(_, original)| *original == schedule);
                if !same {
                    let now = jiff::Timestamp::now().as_millisecond();
                    if let Err(error) =
                        crate::cron_service::parse_schedule_text(&schedule, &draft.zone, now)
                    {
                        return self.cron_retry(draft, text, error.message);
                    }
                }
                self.cron_finish(draft, (!same).then_some(schedule))
            }
        }
    }

    fn cron_finish(&mut self, draft: Draft, schedule: Option<String>) -> Effect {
        let request = match (draft.kind, draft.job) {
            (Kind::New, _) => CronRequest::Add {
                actor: draft.actor,
                squad: draft.squad,
                room_id: draft.room_id,
                owner: draft.owner,
                message: draft.message.unwrap_or_default(),
                schedule: schedule.unwrap_or_default(),
                zone: draft.zone,
            },
            (Kind::Edit, Some((key, revision))) => {
                if draft.message.is_none() && schedule.is_none() {
                    return self.say("Nothing changed; nothing written.");
                }
                CronRequest::Existing {
                    actor: draft.actor,
                    key,
                    revision,
                    op: Op::Edit {
                        message: draft.message,
                        schedule,
                        zone: draft.zone,
                    },
                }
            }
            (Kind::Reassign, Some((key, revision))) => CronRequest::Existing {
                actor: draft.actor,
                key,
                revision,
                op: Op::Reassign { owner: draft.owner },
            },
            (_, None) => return self.say("The job is no longer selected; nothing written."),
        };
        Effect::Act(Request::Cron(request))
    }
}
