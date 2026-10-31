//! Agent and squad Edit use the New member field owner and the existing action lane.
#[cfg(test)]
pub(super) mod captures;
pub(super) mod load;
pub(super) mod model;
mod surface;
#[cfg(test)]
mod tests;
use super::*;
use crate::config::OpsPreferences;
use model::Target;
use tmt_tui::components::{Button, DialogHits, DialogInput, DialogKind, DialogState, FieldKind};

pub(in crate::board::new_member) struct Edit {
    target: Target,
    intent: Intent,
    actor: Option<String>,
    initial: [String; super::model::FIELDS],
    preset: Result<model::Preset, String>,
    preferences: OpsPreferences,
    enabled: bool,
    pub(super) after: Editor,
    hidden: bool,
    dialog: DialogState,
    pub(super) hits: RefCell<DialogHits>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Edit,
    Rename,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Member(super::model::Slot),
    Alert,
    After,
    Hidden,
}
impl Field {
    #[cfg(test)]
    pub(in crate::board::new_member) fn debug_name(self) -> String {
        match self {
            Self::Member(slot) => format!("{slot:?}"),
            other => format!("{other:?}"),
        }
    }

    fn kind(self) -> FieldKind {
        match self {
            Self::Member(super::model::Slot::Name) | Self::After => FieldKind::Text,
            _ => FieldKind::Choice,
        }
    }
    pub(in crate::board::new_member) fn label(self) -> &'static str {
        match self {
            Self::Member(s) => s.label(),
            Self::Alert => "Idle alert",
            Self::After => "After",
            Self::Hidden => "Hidden",
        }
    }
}
impl Edit {
    fn fields(&self, draft: &super::model::Draft, loaded: Option<&Loaded>) -> Vec<Field> {
        use super::model::Slot;
        match self.target {
            Target::Squad(_) if loaded.is_none() => vec![Field::Member(Slot::Name)],
            Target::Squad(_) => vec![
                Field::Member(Slot::Name),
                Field::Alert,
                Field::After,
                Field::Hidden,
            ],
            Target::Member { .. } => [
                Slot::Name,
                Slot::Tool,
                Slot::Model,
                Slot::Effort,
                Slot::Squad,
            ]
            .into_iter()
            .filter(|s| *s != Slot::Effort || loaded.is_some_and(|l| !draft.efforts(l).is_empty()))
            .map(Field::Member)
            .collect(),
        }
    }
    fn rename_on_enter(&self, c: &Controller, current: bool) -> bool {
        current
            && c.stage == Stage::Form
            && self.intent == Intent::Rename
            && self.dialog.focus == tmt_tui::components::Focus::Field(0)
            && c.loaded
                .as_ref()
                .is_some_and(|l| self.plan(&c.draft, l).is_ok_and(|p| p.only_rename()))
    }
    fn enter_hint(&self, c: &Controller, current: bool) -> &'static str {
        if self.rename_on_enter(c, current) {
            "Enter rename"
        } else if self.intent == Intent::Rename
            && self.dialog.focus == tmt_tui::components::Focus::Field(0)
        {
            "Enter next"
        } else {
            "Enter choose"
        }
    }
    fn buttons(&self, c: &Controller, current: bool) -> Vec<Button<'static>> {
        if matches!(c.stage, Stage::Result(_)) {
            return vec![Button::primary("Close")];
        }

        let plan = c.loaded.as_ref().and_then(|l| self.plan(&c.draft, l).ok());
        let copy_only = plan
            .as_ref()
            .and_then(|p| p.runtime.as_ref())
            .is_some_and(|r| r.outer.is_none());
        let enabled = current && plan.is_some_and(|p| !p.empty()) && c.stage == Stage::Form;
        let mut buttons = vec![Button::primary("Cancel")];
        if (c.reopened || !current) && c.stage == Stage::Form {
            buttons.push(Button::secondary("Discard"));
        }
        if current {
            buttons.push(
                Button::secondary(if copy_only { "Copy command" } else { "Save" }).enabled(enabled),
            );
        }
        buttons
    }
    fn current_error(&self, app: &App) -> Option<String> {
        let changed =
            || Some("The target changed. Draft kept; choose Discard to start again.".into());
        if app.view.as_ref().and_then(|v| v.me_id.as_ref()) != self.actor.as_ref() {
            return changed();
        }
        if app.workspace.selected.as_ref() != Some(&self.target.selected()) {
            return changed();
        }
        let Some(snapshot) = app.workspace_snapshot() else {
            return changed();
        };
        let room_current = |room: &crate::membership::workspace::RoomTarget| {
            snapshot.roster.squads.iter().any(|s| {
                s.room_id == room.id
                    && s.name == room.name
                    && s.observation == crate::board::workspace::OpsObservation::Complete
            }) && snapshot
                .rows
                .get(&OpsTarget::Squad(room.id.clone()))
                .is_some_and(|row| row["revision"] == room.revision)
        };
        match &self.target {
            Target::Member {
                id,
                name,
                observation,
                source,
                ..
            } => {
                let Some(agent) = app
                    .references
                    .agents
                    .get(id)
                    .filter(|_| app.references.error.is_none())
                else {
                    return changed();
                };
                let held = crate::board::switch_tool::Source::frozen(
                    self.target.selected(),
                    name.clone(),
                    observation,
                );
                if agent.name != *name
                    || !held.same_runtime_observation(&agent.parameters)
                    || source.as_ref().is_some_and(|r| !room_current(r))
                {
                    return changed();
                }
            }
            Target::Squad(room) if !room_current(room) => return changed(),
            _ => {}
        }
        None
    }
    fn title(&self) -> String {
        format!(
            "Edit {} · {}",
            if matches!(self.target, Target::Squad(_)) {
                "squad"
            } else {
                "member"
            },
            self.target.name()
        )
    }
    fn choices(&self, c: &Controller, slot: super::model::Slot) -> Vec<super::model::Choice> {
        let Some(loaded) = &c.loaded else {
            return vec![];
        };
        let mut choices = c.draft.choices(slot, loaded);
        use super::model::Slot;
        // Edit keeps an agent an agent; a plain shell is made from New member only.
        choices.retain(|choice| choice.value.as_deref() != Some(super::model::PLAIN_TOOL));
        if slot == Slot::Squad {
            let common =
                matches!(&self.target,Target::Member{memberships,..} if memberships.len()<=1);
            choices.retain(|choice| {
                choice
                    .value
                    .as_ref()
                    .is_some_and(|v| !v.is_empty() || common)
            });
            if let Target::Member {
                source: None,
                memberships,
                ..
            } = &self.target
            {
                choices.retain(|choice| {
                    choice.value.as_ref().is_none_or(|name| {
                        !c.draft
                            .rooms
                            .iter()
                            .any(|room| &room.name == name && memberships.contains(&room.id))
                    })
                });
            }
        }
        if matches!(slot, Slot::Model | Slot::Effort)
            && c.draft.text(Slot::Tool) == self.initial[Slot::Tool.index()]
        {
            for choice in &mut choices {
                if choice.value.as_deref() == Some("") {
                    choice.label = "keep current".into();
                    choice.value = Some(self.initial[slot.index()].clone());
                }
            }
        }
        if matches!(slot, Slot::Model | Slot::Effort) {
            let same_tool = c.draft.text(Slot::Tool) == self.initial[Slot::Tool.index()];
            if !same_tool {
                for choice in &mut choices {
                    if choice.value.as_deref() == Some("") {
                        choice.label = "remembered / default".into();
                    }
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            choices.retain(|choice| {
                choice
                    .value
                    .as_ref()
                    .is_none_or(|value| seen.insert(value.clone()))
            });
        }
        choices
    }
}
impl Controller {
    pub(super) fn edit_choices(
        &self,
        slot: super::model::Slot,
    ) -> Option<Vec<super::model::Choice>> {
        self.edit.as_ref().map(|e| e.choices(self, slot))
    }
    pub(super) fn load_edit_preset(&mut self, preset: Result<model::Preset, String>) {
        if let Some(edit) = &mut self.edit {
            match &preset {
                Ok(preset) if preset.associated => {
                    let recorded = match &edit.target {
                        Target::Member { observation, .. } => observation["resume"]["model"]
                            .as_str()
                            .or(preset.model.as_deref()),
                        _ => None,
                    };
                    self.draft
                        .set(super::model::Slot::Model, recorded.unwrap_or(""));
                    edit.initial[super::model::Slot::Model.index()] =
                        self.draft.text(super::model::Slot::Model).into();

                    self.draft.set(
                        super::model::Slot::Effort,
                        preset.effort.as_deref().unwrap_or(""),
                    );
                    edit.initial[super::model::Slot::Effort.index()] =
                        self.draft.text(super::model::Slot::Effort).into();
                }
                Err(error) => {
                    self.notice = Some(format!(
                        "Recorded runtime settings unavailable: {error}. Name and Squad remain editable."
                    ))
                }
                _ => {}
            }
            edit.preset = preset;
        }
    }
    pub(super) fn load_edit_defaults(&mut self) {
        let Some(edit) = &mut self.edit else { return };
        if let Target::Squad(room) = &edit.target
            && let Some(loaded) = &self.loaded
            && let Ok(alert) = loaded.config.idle_alert(&room.name)
        {
            if let Ok(preferences) = loaded.config.workspace_preferences() {
                edit.hidden = preferences.hidden.contains(&room.id);
                edit.preferences = preferences;
            }
            edit.enabled = alert.enabled;
            edit.after = Editor::from(crate::idle_alert::threshold(alert.after));
        }
    }
}
impl App {
    pub(in crate::board) fn open_member_edit(&mut self) -> Effect {
        if let Some(c) = &mut self.new_member
            && let Some(edit) = &mut c.edit
            && !c.shown
        {
            edit.intent = Intent::Edit;
            if self.workspace.selected.as_ref() != Some(&edit.target.selected()) {
                c.shown = true;
                c.reopened = c.stage == Stage::Form;
                c.notice=Some("Kept Edit for the original target. Discard it before editing the newly selected row.".into());
                c.invalidate();
                self.menu = None;
                return Effect::None;
            }
            c.shown = true;
            c.reopened = c.stage == Stage::Form;
            c.invalidate();
            self.menu = None;
            return Effect::None;
        }
        if self.shell.tab != super::super::shell::OpsTab::Agents
            || !self.workspace_selection_visible()
        {
            return self.say("Select a visible agent or squad to edit.");
        }
        if self
            .new_member
            .as_ref()
            .is_some_and(|c| !c.shown && c.edit.is_none())
        {
            return self.say("A New member draft is kept. Reopen it with n and discard it before editing another target.");
        }
        let mut draft = super::model::Draft::from_app(self);
        let Some(snapshot) = self.workspace_snapshot() else {
            return Effect::None;
        };
        let preferences = snapshot.preferences.clone();
        let target = match self.workspace.selected.as_ref() {
            Some(OpsTarget::Member {
                identity_id,
                room_id,
            }) => {
                let Some(agent) = self
                    .references
                    .agents
                    .get(identity_id)
                    .filter(|_| self.references.error.is_none())
                else {
                    return self.say("Agent observation unavailable; refresh before editing.");
                };
                let memberships = snapshot
                    .roster
                    .squads
                    .iter()
                    .filter(|s| s.members.contains(identity_id))
                    .map(|s| s.room_id.clone())
                    .collect::<Vec<_>>();
                let source_id = room_id
                    .as_ref()
                    .or_else(|| (memberships.len() == 1).then(|| &memberships[0]));
                let source = source_id
                    .and_then(|id| draft.rooms.iter().find(|r| &r.id == id))
                    .cloned();
                if source_id.is_some() && source.is_none() {
                    return self.say("Squad observation unavailable; refresh before editing.");
                }
                draft.set(super::model::Slot::Name, &agent.name);
                draft.set(
                    super::model::Slot::Tool,
                    agent.runtime.as_deref().unwrap_or(""),
                );
                draft.set(
                    super::model::Slot::Model,
                    agent.parameters["resume"]["model"].as_str().unwrap_or(""),
                );
                draft.set(
                    super::model::Slot::Host,
                    agent.parameters["location"]["host"].as_str().unwrap_or(""),
                );
                draft.set(
                    super::model::Slot::Session,
                    agent.parameters["location"]["session"]
                        .as_str()
                        .unwrap_or(""),
                );
                draft.set(
                    super::model::Slot::Squad,
                    source.as_ref().map_or("", |r| r.name.as_str()),
                );
                Target::Member {
                    selection: self.workspace.selected.clone().unwrap(),
                    id: identity_id.clone(),
                    name: agent.name.clone(),
                    observation: agent.parameters.clone(),
                    source,
                    memberships,
                }
            }
            Some(OpsTarget::Squad(id)) => {
                let Some(room) = draft.rooms.iter().find(|r| &r.id == id).cloned() else {
                    return self.say("Squad observation unavailable; refresh before editing.");
                };
                draft.set(super::model::Slot::Name, &room.name);
                Target::Squad(room)
            }
            _ => {
                return self.say("Common pool is not an editable squad. Select an agent or squad.");
            }
        };
        let fields = [FieldKind::Text];
        let edit = Edit {
            intent: Intent::Edit,
            initial: draft.fields.each_ref().map(|e| e.as_str().to_owned()),
            actor: self.view.as_ref().and_then(|v| v.me_id.clone()),
            preset: Err("Loading recorded launch settings…".into()),
            hidden: matches!(&target,Target::Squad(room) if preferences.hidden.contains(&room.id)),
            target,
            preferences,
            enabled: false,
            after: Editor::from("30m"),
            dialog: DialogState::new(&fields, &[Button::primary("Cancel")]),
            hits: RefCell::new(DialogHits::default()),
        };
        let mut effect = self.open_member_form(Destination::Member);
        if matches!(effect, Effect::NewMember(_))
            && let Some(c) = &mut self.new_member
        {
            c.draft = draft;
            if let (Target::Member { id, .. }, Effect::NewMember(task)) =
                (&edit.target, &mut effect)
            {
                task.work = Work::LoadEdit(id.clone());
            }
            c.edit = Some(edit);
        }
        effect
    }
    pub(in crate::board) fn open_member_rename(&mut self) -> Effect {
        let effect = self.open_member_edit();
        if let Some(c) = self
            .new_member
            .as_mut()
            .filter(|c| c.shown && c.stage == Stage::Form)
            && let Some(edit) = c.edit.as_mut()
        {
            edit.intent = Intent::Rename;
            edit.dialog.focus = tmt_tui::components::Focus::Field(0);
            c.draft.fields[0].key(
                ratatui::crossterm::event::KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL),
                c.editor_width.get().max(1),
                1024,
            );
            c.invalidate();
        }
        effect
    }
    pub(super) fn member_edit_event(&mut self, event: &Event) -> Option<Effect> {
        let target_error = self
            .new_member
            .as_ref()
            .and_then(|c| c.edit.as_ref())
            .and_then(|edit| edit.current_error(self));
        let c = self
            .new_member
            .as_mut()
            .filter(|c| c.shown && c.edit.is_some())?;
        if matches!(event, Event::Resize(..)) {
            c.invalidate();
            *c.edit.as_ref()?.hits.borrow_mut() = DialogHits::default();
            return Some(Effect::None);
        }
        // Field-choice content is shared with New member; only its parent form differs.
        if c.list.is_some() {
            return self.member_edit_list_event(event);
        }
        let fields = if c.stage == Stage::Form {
            c.edit.as_ref()?.fields(&c.draft, c.loaded.as_ref())
        } else {
            Vec::new()
        };
        let kinds = fields.iter().map(|f| f.kind()).collect::<Vec<_>>();
        let buttons = c.edit.as_ref()?.buttons(c, target_error.is_none());
        let prior = c.edit.as_ref()?.dialog.focus;
        let name_enter = matches!(event, Event::Key(k) if k.code == KeyCode::Enter && k.kind != KeyEventKind::Release && k.modifiers.is_empty())
            && prior == tmt_tui::components::Focus::Field(0)
            && c.edit.as_ref()?.intent == Intent::Rename;
        let rename_enter =
            name_enter && c.edit.as_ref()?.rename_on_enter(c, target_error.is_none());
        // One Save path below serves both the button and the name-only shortcut.
        // A kept multi-field plan always goes to its Save row for explicit review.
        if name_enter
            && !rename_enter
            && c.loaded.as_ref().is_some_and(|l| {
                c.edit
                    .as_ref()
                    .unwrap()
                    .plan(&c.draft, l)
                    .is_ok_and(|p| !p.empty())
            })
            && let Some(index) = buttons
                .iter()
                .position(|b| b.label == "Save" || b.label == "Copy command")
        {
            c.edit.as_mut()?.dialog.focus = tmt_tui::components::Focus::Buttons(index);
            c.invalidate();
            return Some(Effect::None);
        }
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && let tmt_tui::components::Focus::Field(i) = prior
            && let Some(field) = fields.get(i)
        {
            match field {
                Field::Member(super::model::Slot::Name) => {
                    c.draft.fields[0].key(*key, c.editor_width.get(), 1024);
                }
                Field::After => {
                    c.edit.as_mut()?.after.key(*key, c.editor_width.get(), 64);
                }
                _ => {}
            }
            c.invalidate();
            return Some(Effect::None);
        }
        let input = if rename_enter {
            match buttons.iter().position(|b| b.label == "Save") {
                Some(index) => DialogInput::Choose(index),
                None => DialogInput::Ignored,
            }
        } else {
            match event {
                Event::Key(key) => {
                    c.edit
                        .as_mut()?
                        .dialog
                        .key(DialogKind::Form, key, &kinds, &buttons)
                }
                Event::Mouse(mouse) => {
                    let edit = c.edit.as_mut()?;
                    let hits = edit.hits.borrow().clone();
                    if hits.outer.is_empty() {
                        DialogInput::Ignored
                    } else {
                        edit.dialog.mouse(mouse, &hits, &buttons)
                    }
                }
                Event::Paste(text) => {
                    if !text.chars().any(char::is_control)
                        && let tmt_tui::components::Focus::Field(i) = prior
                        && let Some(field) = fields.get(i)
                    {
                        match field {
                            Field::Member(super::model::Slot::Name) => {
                                c.draft.fields[0].insert(text, 1024);
                            }
                            Field::After => {
                                c.edit.as_mut()?.after.insert(text, 64);
                            }
                            _ => {}
                        }
                    }
                    c.invalidate();
                    return Some(Effect::None);
                }
                _ => DialogInput::Ignored,
            }
        };
        if let DialogInput::Choose(index) = input
            && (!matches!(prior, tmt_tui::components::Focus::Field(_)) || rename_enter)
            && c.edit
                .as_ref()?
                .hits
                .borrow()
                .buttons
                .get(index)
                .copied()
                .flatten()
                .is_none()
        {
            return Some(Effect::None);
        }
        if input == DialogInput::Cancel {
            return Some(self.new_member_close());
        }
        if c.stage != Stage::Form {
            if matches!(c.stage, Stage::Result(_)) {
                if let DialogInput::Choose(0) = input {
                    return Some(self.new_member_close());
                }
                c.scroll.borrow_mut().input(event);
            }
            return Some(Effect::None);
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let point = ratatui::layout::Position::new(mouse.column, mouse.row);
            let at = c
                .edit
                .as_ref()?
                .hits
                .borrow()
                .fields
                .iter()
                .position(|area| area.contains(point));
            if let Some(i) = at
                && let Some(field) = fields.get(i)
            {
                match field {
                    Field::Member(slot) if slot.listed() => c.open_list(*slot),
                    Field::Member(slot) => {
                        let area = c.edit.as_ref()?.hits.borrow().fields[i];
                        c.draft.fields[slot.index()].place(
                            area.width,
                            0,
                            usize::from(mouse.column.saturating_sub(area.x)),
                            false,
                        );
                    }
                    Field::After => {
                        let area = c.edit.as_ref()?.hits.borrow().fields[i];
                        if mouse.column == area.right().saturating_sub(1) {
                            c.open_field_list(Field::After);
                        } else {
                            c.edit.as_mut()?.after.place(
                                area.width.saturating_sub(2),
                                0,
                                usize::from(mouse.column.saturating_sub(area.x)),
                                false,
                            );
                        }
                    }
                    Field::Alert => {
                        let e = c.edit.as_mut()?;
                        e.enabled = !e.enabled;
                    }
                    Field::Hidden => {
                        let e = c.edit.as_mut()?;
                        e.hidden = !e.hidden;
                    }
                }
                c.invalidate();
                return Some(Effect::None);
            }
        }
        let entering = matches!(event,Event::Key(k) if k.code==KeyCode::Enter);
        let field = match input {
            DialogInput::Field(i, _) => fields.get(i).copied(),
            // Enter on a field chooses or advances that field; it never invokes Save.
            DialogInput::Choose(_) if entering && !rename_enter => match prior {
                tmt_tui::components::Focus::Field(i) => fields.get(i).copied(),
                _ => None,
            },
            _ => None,
        };
        if let Some(field) = field {
            if let Event::Key(key) = event {
                match field {
                    Field::Member(slot) if slot.listed() => {
                        if entering {
                            c.open_list(slot)
                        } else if matches!(key.code, KeyCode::Left | KeyCode::Right) {
                            c.cycle(slot, key.code == KeyCode::Left)
                        }
                    }
                    Field::Member(slot) => {
                        if entering {
                            c.edit.as_mut()?.dialog.key(
                                DialogKind::Form,
                                &ratatui::crossterm::event::KeyEvent::new(
                                    KeyCode::Down,
                                    KeyModifiers::NONE,
                                ),
                                &kinds,
                                &buttons,
                            );
                        }
                        if !entering {
                            c.draft.fields[slot.index()].key(*key, c.editor_width.get(), 1024);
                        }
                    }
                    Field::After => {
                        if entering {
                            c.open_field_list(Field::After);
                        } else {
                            c.edit.as_mut()?.after.key(*key, c.editor_width.get(), 64);
                        }
                    }
                    Field::Alert
                        if matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Enter) =>
                    {
                        let e = c.edit.as_mut()?;
                        e.enabled = !e.enabled;
                    }
                    Field::Hidden
                        if matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Enter) =>
                    {
                        let e = c.edit.as_mut()?;
                        e.hidden = !e.hidden;
                    }
                    _ => {}
                }
            }
        } else if let DialogInput::Choose(index) = input {
            if index == 0 {
                return Some(self.new_member_close());
            }
            if buttons.get(index).is_some_and(|b| b.label == "Discard") {
                self.new_member = None;
                return Some(Effect::None);
            }
            if c.readable.get()
                && let Some(loaded) = &c.loaded
            {
                match c.edit.as_ref()?.plan(&c.draft, loaded) {
                    Ok(plan) if !plan.empty() => {
                        if self.workspace.selected.as_ref() != Some(&plan.target.selected()) {
                            c.notice = Some(
                                "The selected target changed. Reopen and review before saving."
                                    .into(),
                            );
                            c.invalidate();
                            return Some(Effect::None);
                        }
                        if let Some(runtime) = plan.runtime.as_ref().filter(|r| r.outer.is_none()) {
                            let text = runtime.copy();
                            c.notice=Some("Copy-only runtime command. Nothing changed; other edits remain unsaved.".into());
                            c.invalidate();
                            return Some(Effect::Act(Request::Copy {
                                text,
                                program: self.view.as_ref().and_then(|v| v.clipboard.clone()),
                                feedback: CopyFeedback::Destination,
                            }));
                        }
                        let mut task = c.task(Work::Edit(Box::new(plan)));
                        task.config = Some(Box::new(loaded.config.clone()));
                        c.stage = Stage::Launching;
                        c.invalidate();
                        return Some(Effect::NewMember(task));
                    }
                    Err(error) => c.notice = Some(error),
                    _ => {}
                }
            }
        }
        c.invalidate();
        Some(Effect::None)
    }
    fn member_edit_list_event(&mut self, event: &Event) -> Option<Effect> {
        let c = self.new_member.as_mut()?;
        let count = c.rows().len().max(1);
        let buttons = [Button::primary("Back")];
        let hits = c.edit.as_ref()?.hits.borrow().clone();
        let input = match event {
            Event::Key(key) => {
                c.list
                    .as_mut()?
                    .dialog
                    .key(DialogKind::List, key, &[FieldKind::Text], &buttons)
            }
            Event::Mouse(mouse) if !hits.outer.is_empty() => {
                c.list.as_mut()?.dialog.mouse(mouse, &hits, &buttons)
            }
            Event::Paste(text) if !text.chars().any(char::is_control) => {
                let list = c.list.as_mut()?;
                list.filter.insert(text, 1024);
                list.row = 0;
                DialogInput::Moved
            }
            _ => DialogInput::Ignored,
        };
        match input {
            DialogInput::Cancel | DialogInput::Choose(0) => c.list = None,
            DialogInput::Content(key) => match key.code {
                KeyCode::Up => {
                    let list = c.list.as_mut()?;
                    list.row = list.row.saturating_sub(1);
                }
                KeyCode::Down => {
                    let list = c.list.as_mut()?;
                    list.row = (list.row + 1).min(count - 1);
                }
                KeyCode::Enter => {
                    let row = c.list.as_ref()?.row;
                    c.choose(row);
                }
                _ => {
                    let list = c.list.as_mut()?;
                    list.filter.key(key, c.editor_width.get(), 1024);
                    list.row = 0;
                }
            },
            _ => {}
        }
        if c.list.is_some()
            && let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let point = ratatui::layout::Position::new(mouse.column, mouse.row);
            let row = c.fields.borrow().iter().find_map(|(field, area)| {
                if area.contains(point) {
                    if let super::Field::Choice(i) = field {
                        Some(*i)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
            if let Some(row) = row {
                c.choose(row);
            }
        }
        c.invalidate();
        Some(Effect::None)
    }
}
