//! New-member form and one confirmed host-template launch on the action lane.
mod input;
mod load;
mod model;
mod surface;
#[cfg(test)]
pub(in crate::board) mod tests;

use super::{
    app::{App, CopyFeedback, Effect, Request},
    workspace::OpsTarget,
};
pub(in crate::board) use load::{Completed, Task, complete};
use load::{Loaded, Outcome, Work};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    layout::Rect,
};
use std::cell::{Cell, RefCell};
use tmt_tui::components::Editor;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Stage {
    Form,
    Hosts,
    Review,
    Template,
    Launching,
    Saving,
    Result(String),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Destination {
    Member,
    Settings,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Text(usize),
    Host,
    Identity(bool),
}
pub(in crate::board) struct Controller {
    owner: u64,
    destination: Destination,
    draft: model::Draft,
    loaded: Option<Loaded>,
    stage: Stage,
    review: Option<model::Review>,
    focus: usize,
    template: Editor,
    notice: Option<String>,
    readable: Cell<bool>,
    fields: RefCell<Vec<(Field, Rect)>>,
    controls: Cell<[Option<Rect>; 2]>,
    editor_width: Cell<u16>,
    scroll: RefCell<tmt_tui::components::ScrollState>,
}
impl Controller {
    pub fn invalidate(&self) {
        self.readable.set(false);
        self.fields.borrow_mut().clear();
        self.controls.set([None, None]);
    }
    fn task(&self, work: Work) -> Task {
        Task {
            owner: self.owner,
            work,
            config: None,
        }
    }
    fn cycle(&mut self, back: bool) {
        let index = self.focus;
        if index == 3 && !self.draft.saved {
            return;
        }
        if index == 6 {
            self.draft.saved = !self.draft.saved;
            return;
        }
        let choices = match index {
            1 => self.draft.tools.clone(),
            2 => std::iter::once(String::new())
                .chain(
                    self.draft
                        .models
                        .get(self.draft.fields[1].as_str())
                        .into_iter()
                        .flatten()
                        .cloned(),
                )
                .collect(),
            3 => std::iter::once(String::new())
                .chain(self.draft.rooms.iter().map(|r| r.name.clone()))
                .collect(),
            4 => self
                .loaded
                .as_ref()
                .map(|l| {
                    l.hosts
                        .iter()
                        .filter(|h| matches!(h.state.as_str(), "builtin" | "ok"))
                        .map(|h| h.name.clone())
                        .collect()
                })
                .unwrap_or_default(),
            _ => return,
        };
        if choices.is_empty() {
            return;
        }
        let current = choices
            .iter()
            .position(|v| v == self.draft.fields[index].as_str());
        let next = current.map_or(0, |i| {
            (i + if back { choices.len() - 1 } else { 1 }) % choices.len()
        });
        self.draft.fields[index] = Editor::from(choices[next].clone());
        if index == 1 {
            self.draft.fields[2].clear();
        }
    }
    fn begin_template(&mut self) {
        if let Some(loaded) = &self.loaded {
            let host = self.draft.fields[4].as_str();
            let argv = loaded
                .config
                .launch_override(host)
                .ok()
                .flatten()
                .or_else(|| {
                    loaded
                        .hosts
                        .iter()
                        .find(|h| h.name == host)
                        .and_then(|h| h.argv.clone())
                });
            self.template = Editor::from(serde_json::to_string(&argv.unwrap_or_default()).unwrap());
            self.stage = Stage::Template;
            self.focus = 0;
            self.notice = None;
            self.invalidate();
        }
    }
}
impl App {
    pub(in crate::board) fn open_new_member(&mut self) -> Effect {
        self.open_member_form(Destination::Member)
    }
    pub(in crate::board) fn open_launch_templates(&mut self) -> Effect {
        self.open_member_form(Destination::Settings)
    }
    fn open_member_form(&mut self, destination: Destination) -> Effect {
        if destination == Destination::Member && self.shell.tab != super::shell::OpsTab::Agents
            || self.input.is_some()
            || self.searching
            || self.modal_open() && self.menu.is_none()
        {
            return self.say("Close the current input or overlay before opening this form.");
        }
        self.new_member_generation += 1;
        let c = Controller {
            owner: self.new_member_generation,
            destination,
            draft: model::Draft::from_app(self),
            loaded: None,
            stage: if destination == Destination::Settings {
                Stage::Hosts
            } else {
                Stage::Form
            },
            review: None,
            focus: 0,
            template: Editor::default(),
            notice: None,
            readable: Cell::new(false),
            fields: RefCell::new(Vec::new()),
            controls: Cell::new([None, None]),
            editor_width: Cell::new(40),
            scroll: RefCell::new(Default::default()),
        };
        let task = c.task(Work::Load);
        self.new_member = Some(c);
        self.menu = None;
        Effect::NewMember(task)
    }
    pub(in crate::board) fn finished_new_member(&mut self, completed: Completed) -> Effect {
        let created = if let Ok(Outcome::Created { id, room, .. }) = &completed.result {
            Some(OpsTarget::Member {
                room_id: room.clone(),
                identity_id: id.clone(),
            })
        } else {
            None
        };
        if let Some(c) = &mut self.new_member
            && c.owner == completed.task.owner
        {
            match completed.result {
                Ok(Outcome::Loaded(loaded)) => {
                    if matches!(completed.task.work, Work::Load)
                        && c.draft.fields[4].is_empty()
                        && let Some(host) = loaded.hosts.iter().find(|h| h.eligible())
                    {
                        c.draft.fields[4] = Editor::from(host.name.clone());
                    }
                    c.loaded = Some(*loaded);
                    c.stage = if c.destination == Destination::Settings {
                        Stage::Hosts
                    } else {
                        Stage::Form
                    };
                    if matches!(completed.task.work, Work::Save { .. }) {
                        c.notice = Some(
                            if matches!(completed.task.work, Work::Save { argv: None, .. }) {
                                "Launch template reset to Core default. Member draft kept.".into()
                            } else {
                                "Launch template saved. Member draft kept.".into()
                            },
                        );
                    }
                }
                Ok(Outcome::Created { message, .. }) => {
                    c.stage = Stage::Result(message);
                }
                Err(error) => {
                    c.notice = Some(error.message);
                    c.stage = if matches!(completed.task.work, Work::Save { .. }) {
                        Stage::Template
                    } else if c.destination == Destination::Settings {
                        Stage::Hosts
                    } else {
                        Stage::Form
                    };
                }
            }
            c.invalidate();
        }
        if let Some(target) = created {
            if let OpsTarget::Member { identity_id, .. } = &target {
                self.discovered_uuid = Some(identity_id.clone());
            }
            self.workspace.selected = Some(target);
            return Effect::Refresh;
        }
        Effect::None
    }
}
