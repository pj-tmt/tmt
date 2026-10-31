//! New-member form and one confirmed host-template launch on the action lane.
mod edit;
mod input;
mod load;
pub(in crate::board) mod model;
mod surface;
#[cfg(test)]
pub(in crate::board) mod tests;

use super::host_launch::Loaded;
use super::{
    app::{App, CopyFeedback, Effect, Request},
    workspace::OpsTarget,
};
pub(in crate::board) use load::{Completed, Task, complete};
use load::{Outcome, Work};
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
/// A painted hit: a field, one Keep choice, or a row of the open list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Slot(model::Slot),
    Keep(bool),
    Choice(usize),
}
/// Where the form's keys go: a field, or a footer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Slot(model::Slot),
    Cancel,
    Discard,
    Create,
}
/// A footer button: the first one (Cancel, Back or Close), Discard on a reopened draft, or the
/// stage's action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    First,
    Discard,
    Apply,
}
/// A field's open list: typing filters it, and its free-form row takes the filter text.
#[derive(Clone, Debug)]
struct List {
    dialog: tmt_tui::components::DialogState,
    field: edit::Field,
    filter: Editor,
    row: usize,
}
pub(in crate::board) struct Controller {
    owner: u64,
    edit: Option<edit::Edit>,
    destination: Destination,
    draft: model::Draft,
    loaded: Option<Loaded>,
    stage: Stage,
    /// False while the form is hidden: a kept draft, or a launch or save running behind it.
    shown: bool,
    /// Opened again from a kept draft: Discard drops it.
    reopened: bool,
    review: Option<model::Review>,
    /// The Hosts and Template stages' focus; the form uses `at`.
    focus: usize,
    at: Focus,
    list: Option<List>,
    template: Editor,
    notice: Option<String>,
    readable: Cell<bool>,
    fields: RefCell<Vec<(Field, Rect)>>,
    controls: Cell<[Option<Rect>; 2]>,
    discard: Cell<Option<Rect>>,
    editor_width: Cell<u16>,
    scroll: RefCell<tmt_tui::components::ScrollState>,
}
impl Controller {
    pub fn invalidate(&self) {
        if let Some(edit) = &self.edit {
            *edit.hits.borrow_mut() = Default::default();
        }
        self.readable.set(false);
        self.fields.borrow_mut().clear();
        self.controls.set([None, None]);
        self.discard.set(None);
    }
    /// Discard shows on a reopened draft, beside the form's or the template's first button.
    fn discards(&self) -> bool {
        self.reopened
            && (self.stage == Stage::Template || self.stage == Stage::Form && self.list.is_none())
    }
    /// The Template stage's buttons after its editor (`focus` 1, 2, …).
    fn template_controls(&self) -> Vec<Control> {
        [Control::First]
            .into_iter()
            .chain(self.discards().then_some(Control::Discard))
            .chain([Control::Apply])
            .collect()
    }
    fn task(&self, work: Work) -> Task {
        Task {
            owner: self.owner,
            work,
            config: None,
        }
    }
    /// The form's stops, in order: the shown fields (Squad only for a saved member), then
    /// the buttons.
    fn stops(&self) -> Vec<Focus> {
        self.draft
            .visible(self.loaded.as_ref())
            .into_iter()
            .filter(|slot| *slot != model::Slot::Squad || self.draft.saved)
            .map(Focus::Slot)
            .chain([Focus::Cancel])
            .chain(self.discards().then_some(Focus::Discard))
            .chain([Focus::Create])
            .collect()
    }
    /// ←→ on the button row move between its buttons only.
    fn step_button(&mut self, back: bool) {
        let buttons: Vec<_> = self
            .stops()
            .into_iter()
            .filter(|stop| !matches!(stop, Focus::Slot(_)))
            .collect();
        let at = buttons.iter().position(|b| *b == self.at).unwrap_or(0);
        let n = buttons.len();
        self.at = buttons[(at + if back { n - 1 } else { 1 }) % n];
    }
    fn step_focus(&mut self, back: bool) {
        let stops = self.stops();
        let at = stops.iter().position(|s| *s == self.at).unwrap_or(0);
        self.at = stops[(at + if back { stops.len() - 1 } else { 1 }) % stops.len()];
    }
    fn list_choices(&self, field: edit::Field) -> Vec<model::Choice> {
        let Some(loaded) = &self.loaded else {
            return vec![];
        };
        match field {
            edit::Field::Member(slot) => self
                .edit_choices(slot)
                .unwrap_or_else(|| self.draft.choices(slot, loaded)),
            edit::Field::After => ["15m", "30m", "1h", "2h"]
                .into_iter()
                .map(|value| model::Choice {
                    label: value.into(),
                    value: Some(value.into()),
                })
                .chain([model::Choice {
                    label: "Type a duration…".into(),
                    value: None,
                }])
                .collect(),
            _ => vec![],
        }
    }
    /// One choice-list owner serves New member fields and Edit's After presets.
    fn rows(&self) -> Vec<model::Choice> {
        let Some(list) = &self.list else {
            return vec![];
        };
        let filter = list.filter.as_str().to_lowercase();
        self.list_choices(list.field)
            .into_iter()
            .filter(|c| c.value.is_none() || c.label.to_lowercase().contains(&filter))
            .map(|mut c| {
                if c.value.is_none() && !filter.is_empty() {
                    c.label = format!("Use “{}”", list.filter.as_str());
                }
                c
            })
            .collect()
    }
    fn open_list(&mut self, slot: model::Slot) {
        self.open_field_list(edit::Field::Member(slot));
    }
    fn open_field_list(&mut self, field: edit::Field) {
        if self.loaded.is_none() {
            return;
        }
        let current = match field {
            edit::Field::Member(slot) => self.draft.text(slot),
            edit::Field::After => self.edit.as_ref().map_or("", |e| e.after.as_str()),
            _ => "",
        };
        let row = self
            .list_choices(field)
            .iter()
            .position(|c| c.value.as_deref() == Some(current))
            .unwrap_or(0);
        self.list = Some(List {
            dialog: tmt_tui::components::DialogState::new(
                &[tmt_tui::components::FieldKind::Text],
                &[tmt_tui::components::Button::primary("Back")],
            ),
            field,
            filter: Editor::default(),
            row,
        });
    }
    fn choose(&mut self, row: usize) {
        let rows = self.rows();
        let Some(list) = self.list.take() else { return };
        let Some(choice) = rows.get(row) else {
            self.list = Some(list);
            return;
        };
        let value = choice
            .value
            .as_deref()
            .or_else(|| (!list.filter.is_empty()).then(|| list.filter.as_str()));
        match list.field {
            edit::Field::Member(slot) => {
                self.draft.set(slot, value.unwrap_or(""));
                self.at = Focus::Slot(slot);
            }
            edit::Field::After => {
                if let (Some(edit), Some(value)) = (&mut self.edit, value) {
                    edit.after = Editor::from(value);
                }
            }
            _ => {}
        }
    }
    /// ←→ on a listed field: the next or previous fixed choice.
    fn cycle(&mut self, slot: model::Slot, back: bool) {
        let Some(loaded) = &self.loaded else { return };
        let choices: Vec<_> = self
            .edit_choices(slot)
            .unwrap_or_else(|| self.draft.choices(slot, loaded))
            .into_iter()
            .filter_map(|c| c.value)
            .collect();
        if choices.is_empty() {
            return;
        }
        let current = choices.iter().position(|v| v == self.draft.text(slot));
        let next = current.map_or(0, |i| {
            (i + if back { choices.len() - 1 } else { 1 }) % choices.len()
        });
        self.draft.set(slot, &choices[next].clone());
    }
    fn begin_template(&mut self) {
        if let Some(loaded) = &self.loaded {
            let host = self.draft.text(model::Slot::Host);
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
    pub(in crate::board) fn new_member_shown(&self) -> bool {
        self.new_member.as_ref().is_some_and(|c| c.shown)
    }
    fn new_member_reopen_hint(&self) -> String {
        self.reopen_hint(
            crate::action::OpsUiAction::NewMember,
            "n",
            "this new member",
            "New member on the Agents tab",
        )
    }
    /// Esc, Cancel or an outside press. A loaded form or template hides and keeps its draft for
    /// the same key; a running launch or template save hides and keeps going; a load, the host
    /// list or a result closes.
    pub(in crate::board) fn new_member_close(&mut self) -> Effect {
        let reopen = if self.new_member.as_ref().is_some_and(|c| c.edit.is_some()) {
            "Press e on this target to reopen Edit.".into()
        } else {
            self.new_member_reopen_hint()
        };
        let Some(c) = &mut self.new_member else {
            return Effect::None;
        };
        let running = match c.stage {
            Stage::Launching if c.edit.is_some() => "Saving changes",
            Stage::Launching => "Launching",
            Stage::Saving => "Saving the launch template",
            Stage::Form | Stage::Template if c.loaded.is_some() => {
                c.shown = false;
                c.list = None;
                c.invalidate();
                return Effect::None;
            }
            _ => {
                self.new_member = None;
                return Effect::None;
            }
        };
        c.shown = false;
        c.invalidate();
        let name = c.draft.text(model::Slot::Name).to_owned();
        let text = if name.is_empty() {
            format!("◌ {running}. {reopen}")
        } else {
            format!("◌ {running} · {name}. {reopen}")
        };
        self.say(&text)
    }
    fn open_member_form(&mut self, destination: Destination) -> Effect {
        if self
            .new_member
            .as_ref()
            .is_some_and(|c| !c.shown && c.edit.is_some())
        {
            return self.say("An Edit draft is kept. Reopen its target with e and discard it before creating a member.");
        }

        if let Some(c) = &mut self.new_member
            && !c.shown
            && c.edit.is_none()
        {
            c.shown = true;
            // Discard joins the row; the focused button stays the same button.
            let control = (c.stage == Stage::Template)
                .then(|| c.focus.checked_sub(1))
                .flatten()
                .and_then(|at| c.template_controls().get(at).copied());
            c.reopened = matches!(c.stage, Stage::Form | Stage::Template);
            if let Some(control) = control
                && let Some(at) = c.template_controls().iter().position(|b| *b == control)
            {
                c.focus = at + 1;
            }
            c.invalidate();
            self.menu = None;
            return Effect::None;
        }
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
            edit: None,
            destination,
            draft: model::Draft::from_app(self),
            loaded: None,
            stage: if destination == Destination::Settings {
                Stage::Hosts
            } else {
                Stage::Form
            },
            shown: true,
            reopened: false,
            review: None,
            focus: 0,
            at: Focus::Slot(model::Slot::Name),
            list: None,
            template: Editor::default(),
            notice: None,
            readable: Cell::new(false),
            fields: RefCell::new(Vec::new()),
            controls: Cell::new([None, None]),
            discard: Cell::new(None),
            editor_width: Cell::new(40),
            scroll: RefCell::new(Default::default()),
        };
        let task = c.task(Work::Load);
        self.new_member = Some(c);
        self.menu = None;
        Effect::NewMember(task)
    }
    pub(in crate::board) fn finished_new_member(&mut self, completed: Completed) -> Effect {
        let effect = self.finish_new_member(completed);
        if let Some(c) = &self.new_member
            && !c.shown
        {
            let line = match &c.stage {
                Stage::Result(text) => Some(text.lines().next().unwrap_or(text).to_owned()),
                _ => c.notice.clone(),
            };
            if let Some(line) = line {
                let text = format!("{line} · {}", self.new_member_reopen_hint());
                self.set_notice(Some(text), false);
            }
        }
        effect
    }
    fn finish_new_member(&mut self, mut completed: Completed) -> Effect {
        completed.result = match completed.result {
            Ok(Outcome::EditLoaded { loaded, preset }) => {
                if let Some(c) = self
                    .new_member
                    .as_mut()
                    .filter(|c| c.owner == completed.task.owner && c.edit.is_some())
                {
                    c.load_edit_preset(preset);
                }
                Ok(Outcome::Loaded(loaded))
            }
            other => other,
        };
        let owned = self
            .new_member
            .as_ref()
            .is_some_and(|c| c.owner == completed.task.owner);
        let edited = matches!(&completed.result, Ok(Outcome::Edited { .. }));
        let preferences = if owned {
            match &completed.result {
                Ok(Outcome::Edited { preferences, .. }) => preferences.clone(),
                _ => None,
            }
        } else {
            None
        };
        let created = if owned && let Ok(Outcome::Created { id, room, .. }) = &completed.result {
            Some(OpsTarget::Member {
                room_id: room.clone(),
                identity_id: id.clone(),
            })
        } else {
            None
        };
        // A clean start on the built-in host closes the form onto the member's terminal.
        let mut attach = None;
        if let Some(c) = &mut self.new_member
            && c.owner == completed.task.owner
        {
            match completed.result {
                Ok(Outcome::EditLoaded { .. }) => {
                    unreachable!("Edit settings normalized before dispatch")
                }
                Ok(Outcome::Loaded(loaded)) => {
                    if matches!(completed.task.work, Work::Load)
                        && c.edit.is_none()
                        && c.draft.text(model::Slot::Host).is_empty()
                        && let Some(host) = model::default_host(&loaded)
                    {
                        c.draft.set(model::Slot::Host, &host.name);
                    }
                    c.loaded = Some(*loaded);
                    c.load_edit_defaults();
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
                Ok(Outcome::Edited { message, .. }) => {
                    c.stage = Stage::Result(message);
                }
                Ok(Outcome::Created { message, .. }) => {
                    if matches!(&completed.task.work, Work::Launch(review) if review.host == model::BUILT_IN_HOST)
                        && !message.contains('\n')
                    {
                        attach = Some(message);
                    } else {
                        c.stage = Stage::Result(message);
                    }
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
        if let Some(preferences) = preferences {
            self.finished_workspace_preferences(preferences, "✓ Applied · Squad visibility".into());
        }
        if let Some(target) = created {
            if let OpsTarget::Member { identity_id, .. } = &target {
                self.discovered_uuid = Some(identity_id.clone());
            }
            let typing_into = match &target {
                OpsTarget::Member { identity_id, .. } => Some(identity_id.clone()),
                _ => None,
            };
            self.workspace.selected = Some(target);
            if let Some(message) = attach {
                self.new_member = None;
                self.right_tab = super::shell::RightTab::Talk;
                // Only a member has a terminal to show; any other target opens on its chat.
                if let Some(identity) = typing_into {
                    self.terminal.show_typing(&identity);
                }
                self.say(message);
            }
            return Effect::Refresh;
        }
        if edited {
            Effect::Refresh
        } else {
            Effect::None
        }
    }
}
