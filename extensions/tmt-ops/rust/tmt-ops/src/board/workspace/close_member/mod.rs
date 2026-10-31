//! Close/delete consumes Core's verified stop and guarded remove, never host state.
mod surface;
use super::manage::{Change, Draft, Kind, Stage};
use crate::{
    board::app::{App, Effect, Request as Action},
    core::{Core, SquadError},
};
use serde_json::{Value, json};
use std::cell::RefCell;
pub(super) use surface::render;
use tmt_tui::components::DialogHits;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    CloseAndDelete,
    DeleteOnly,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub identity_id: String,
    pub expected_name: String,
    pub mode: Mode,
    pub pane_reason: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Stop,
    Delete,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Removed {
        pane_reason: Option<String>,
    },
    Changed {
        pane_reason: Option<String>,
    },
    Refused {
        step: Step,
        error: SquadError,
        pane_reason: Option<String>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Completed {
    pub request: Request,
    pub outcome: Outcome,
}
pub(crate) struct Close {
    viewer: Option<String>,
    pub result: Option<Outcome>,
    pub hits: RefCell<DialogHits>,
}
impl Close {
    pub fn new(viewer: Option<String>) -> Self {
        Self {
            viewer,
            result: None,
            hits: RefCell::new(DialogHits::default()),
        }
    }
    pub fn invalidate(&self) {
        *self.hits.borrow_mut() = DialogHits::default();
    }
}
fn invalid() -> SquadError {
    SquadError::new(
        "SQUAD_CLOSE_INVALID",
        "Core did not return a matching lifecycle result; the outcome is unknown. Nothing is retried.",
    )
}
fn matching(value: &Value, request: &Request) -> Result<(), SquadError> {
    if value["identityId"].as_str() == Some(request.identity_id.as_str()) {
        Ok(())
    } else {
        Err(invalid())
    }
}
impl Request {
    pub fn apply(self, core: &Core) -> Completed {
        let outcome = self.run(|operation, input| core.api(operation, input));
        Completed {
            request: self,
            outcome,
        }
    }
    fn run(&self, mut api: impl FnMut(&str, Value) -> Result<Value, SquadError>) -> Outcome {
        let mut pane_reason = self.pane_reason.clone();
        if self.mode == Mode::CloseAndDelete {
            let result = api(
                "identity.stop",
                json!({"identityId":self.identity_id,"expectedName":self.expected_name,"close":true}),
            );
            let result = result.and_then(|value| {
                matching(&value, self)?;
                match value["outcome"].as_str() {
                    Some("conflict" | "not-found") => return Ok(None),
                    Some("stopped" | "already-ended" | "not-running") => {}
                    _ => return Err(invalid()),
                }
                if value["name"].as_str() != Some(self.expected_name.as_str()) {
                    return Err(invalid());
                }
                match value["pane"].as_str() {
                    Some("kept") => {
                        pane_reason = Some(
                            value["paneReason"]
                                .as_str()
                                .filter(|v| !v.is_empty())
                                .ok_or_else(invalid)?
                                .to_owned(),
                        );
                    }
                    Some("closed" | "gone") => {}
                    None if value["outcome"] != "stopped" && value.get("pane").is_none() => {}
                    _ => return Err(invalid()),
                }
                Ok(Some(()))
            });
            match result {
                Ok(Some(())) => {}
                Ok(None) => return Outcome::Changed { pane_reason },
                Err(error) => {
                    return Outcome::Refused {
                        step: Step::Stop,
                        error,
                        pane_reason,
                    };
                }
            }
        }
        let result = api(
            "identity.remove",
            json!({"identityId":self.identity_id,"expectedName":self.expected_name}),
        );
        match result.and_then(|value| {
            matching(&value, self)?;
            match value["outcome"].as_str() {
                Some("removed") if value["name"].as_str() == Some(self.expected_name.as_str()) => {
                    Ok(true)
                }
                Some("conflict" | "not-found") => Ok(false),
                _ => Err(invalid()),
            }
        }) {
            Ok(true) => Outcome::Removed { pane_reason },
            Ok(false) => Outcome::Changed { pane_reason },
            Err(error) => Outcome::Refused {
                step: Step::Delete,
                error,
                pane_reason,
            },
        }
    }
}
impl App {
    pub(in crate::board) fn close_member_submit(&mut self, delete_only: bool) -> Effect {
        let Some(draft) = self
            .workspace_form
            .as_ref()
            .filter(|d| d.kind == Kind::RemoveMember)
        else {
            return Effect::None;
        };
        let Some(close) = &draft.close else {
            return Effect::None;
        };
        let index = if delete_only { 2 } else { 1 };
        if close
            .hits
            .borrow()
            .buttons
            .get(index)
            .is_none_or(Option::is_none)
        {
            return Effect::None;
        }
        if close.viewer != self.view.as_ref().and_then(|v| v.me.clone()) {
            return self.say("The viewer changed; close and reopen this review.");
        }
        if (delete_only && !matches!(close.result, Some(Outcome::Refused { .. })))
            || (!delete_only && draft.stage != Stage::Review)
        {
            return Effect::None;
        }
        let Some(member) = &draft.member else {
            return Effect::None;
        };
        let pane_reason = match &close.result {
            Some(Outcome::Refused { pane_reason, .. }) => pane_reason.clone(),
            _ => None,
        };
        let request = Request {
            identity_id: member.id.clone(),
            expected_name: member.name.clone(),
            mode: if delete_only {
                Mode::DeleteOnly
            } else {
                Mode::CloseAndDelete
            },
            pane_reason,
        };
        let draft = self.workspace_form.as_mut().unwrap();
        draft.stage = Stage::Pending;
        draft.operation = Some(Change::CloseMember(request.clone()));
        draft.invalidate();
        Effect::Act(Action::WorkspaceOperation(Change::CloseMember(request)))
    }
    pub(in crate::board) fn finished_close_member(&mut self, completed: Completed) {
        let Some(draft) = self.workspace_form.as_mut().filter(|d| {
            d.kind == Kind::RemoveMember
                && d.stage == Stage::Pending
                && d.operation.as_ref() == Some(&Change::CloseMember(completed.request.clone()))
        }) else {
            return;
        };
        if let Outcome::Removed { pane_reason } = &completed.outcome {
            let mut message = format!("Closed and deleted {}", completed.request.expected_name);
            if completed.request.mode == Mode::DeleteOnly {
                message = format!("Deleted identity {}", completed.request.expected_name);
            }
            if let Some(reason) = pane_reason {
                message.push_str(&format!(" · Pane left open: {reason}"));
            }
            self.workspace_form = None;
            self.set_notice(Some(super::render::safe(&message)), false);
            return;
        }
        draft.stage = Stage::Result(String::new());
        draft.close.as_mut().unwrap().result = Some(completed.outcome);
        draft.review = super::manage::review_start(Kind::RemoveMember, false);
        draft.invalidate();
        *draft.scroll.borrow_mut() = Default::default();
    }
    pub(in crate::board) fn close_member_event(
        &mut self,
        event: &ratatui::crossterm::event::Event,
    ) -> Option<Effect> {
        use ratatui::crossterm::event::{Event, KeyCode};
        use tmt_tui::components::{Button, DialogInput, DialogKind};
        let draft = self.workspace_form.as_ref()?;
        let close = draft.close.as_ref()?;
        let rows = surface::button_labels(draft);
        let hits = close.hits.borrow().clone();
        let buttons: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(i, label)| {
                Button::secondary(*label).enabled(hits.buttons.get(i).is_some_and(Option::is_some))
            })
            .collect();
        let jump = matches!(event, Event::Key(key) if key.code == KeyCode::Char('j') && key.modifiers.is_empty())
            && matches!(close.result, Some(Outcome::Refused { .. }))
            && buttons.get(1).is_some_and(Button::is_enabled);
        let draft = self.workspace_form.as_mut()?;
        let input = if jump {
            DialogInput::Choose(1)
        } else {
            match event {
                Event::Key(key) => draft.review.key(DialogKind::Confirm, key, &[], &buttons),
                Event::Mouse(mouse) => draft.review.mouse(mouse, &hits, &buttons),
                _ => DialogInput::Ignored,
            }
        };
        match input {
            DialogInput::Cancel | DialogInput::Choose(0) => {
                self.workspace_close();
                Some(Effect::None)
            }
            DialogInput::Choose(index)
                if matches!(draft.close.as_ref()?.result, Some(Outcome::Refused { .. })) =>
            {
                if index == 1 {
                    let id = draft.member.as_ref()?.id.clone();
                    Some(Effect::Act(Action::Jump(id)))
                } else {
                    Some(self.close_member_submit(true))
                }
            }
            DialogInput::Choose(1) => Some(self.close_member_submit(false)),
            DialogInput::Moved => Some(Effect::None),
            _ => {
                draft.scroll.borrow_mut().input(event);
                Some(Effect::None)
            }
        }
    }
}

#[cfg(test)]
mod tests;
