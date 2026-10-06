//! Checklist owns every nested screen; no child menu lives in App.menu.
use super::*;
use crate::board::app::{App, Effect, Request as BoardRequest};
use crate::checklist::{
    Code,
    model::{Action, Confirmation, Mutation},
};
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use tmt_tui::components::{ListRow, PickerEvent, PickerField, PickerInput};

impl App {
    pub(in crate::board) fn checklist_shown(&self) -> bool {
        self.checklist.as_ref().is_some_and(|c| c.active)
    }
    pub(in crate::board) fn open_checklist(&mut self) -> Effect {
        if self.input.is_some()
            || self.settings.is_some()
            || self.help
            || self.view_picker.is_some()
            || self.theme_picker.is_some()
            || self.switcher.is_some()
            || self.cron_list.is_some()
        {
            return self.say("Close the current input or overlay before opening Checklist.");
        }
        self.menu = None;
        let room = self
            .view
            .as_ref()
            .filter(|v| {
                v.home.is_none()
                    && self
                        .shown_tab()
                        .is_some_and(|tab| !crate::tabs::aggregate(tab))
            })
            .and_then(|v| v.document["squad"]["roomId"].as_str())
            .and_then(|id| Id::parse(id).ok());
        let return_to = Return {
            tab: self.shown_tab().map(str::to_owned),
            room: self
                .view
                .as_ref()
                .and_then(|v| v.document["squad"]["roomId"].as_str())
                .map(str::to_owned),
            pane: self.focused(),
            jobs: self.jobs_focus,
            row: self.row_target(self.selected),
        };
        // A retained draft belongs to its original room even after the board navigates.
        if self.checklist.as_ref().is_none_or(|c| {
            c.room != room && c.draft.is_none() && c.submit.is_none() && c.unknown.is_empty()
        }) {
            self.checklist_generation += 1;
            self.checklist = Some(Controller::new(self.checklist_generation, room));
        }
        let controller = self.checklist.as_mut().unwrap();
        controller.active = true;
        controller.return_to = Some(return_to);
        controller.read()
    }
    pub(in crate::board) fn checklist_event(&mut self, event: &Event) -> Option<Effect> {
        let opener = self.view.as_ref().and_then(|v| v.opener.clone());
        let mut controller = self.checklist.take()?;
        let effect = controller.input(event, opener);
        if !controller.active
            && let Some(saved) = controller.return_to.take()
        {
            let room = self
                .view
                .as_ref()
                .and_then(|v| v.document["squad"]["roomId"].as_str());
            if self.shown_tab() == saved.tab.as_deref() && room == saved.room.as_deref() {
                if let Some(target) = saved.row
                    && let Some(index) = (0..self.rows().len())
                        .find(|index| self.row_target(*index).as_ref() == Some(&target))
                {
                    self.select(index);
                }
                if let Some(position) = self
                    .effective_board()
                    .and_then(|board| board.panes.iter().position(|pane| *pane == saved.pane))
                {
                    self.focus = position;
                }
                self.jobs_focus = saved.jobs && self.jobs_painted();
            }
        }
        self.checklist = Some(controller);
        Some(effect)
    }
    pub(in crate::board) fn finished_checklist(&mut self, completed: load::Completed) {
        if let Some(controller) = &mut self.checklist {
            controller.loaded(completed);
        }
    }
}
impl Controller {
    pub(super) fn loaded(&mut self, completed: load::Completed) {
        if self.pending.as_ref() != Some(&completed.key) {
            return;
        }
        self.pending = None;
        match completed.outcome {
            load::Outcome::Rooms(result) => match result {
                Ok(rooms) => {
                    self.rooms = rooms;
                    self.failure = None;
                }
                Err(error) => self.failure = Some(error),
            },
            load::Outcome::Read(result) => match result {
                Ok(preview) => {
                    self.preview = Some(preview);
                    self.failure = None;
                    if !self.unknown.is_empty() {
                        self.notice = Some(
                            "Current state refreshed; original update outcome remains unknown."
                                .into(),
                        );
                    }
                }
                Err(error) => self.failure = Some(error),
            },
            load::Outcome::Ids(result) => match result {
                Ok(ids) => {
                    if let Some(preview) = &self.preview {
                        self.draft = Some(forms::Draft::create(preview, ids));
                        self.screen = Screen::Form;
                        self.reset_panel(Some("title"));
                    }
                }
                Err(error) => self.failure = Some(error),
            },
            load::Outcome::Apply(result) => match result {
                Ok(applied) => {
                    let action = self.submit.as_ref().map(action_name).unwrap_or("update");
                    let hidden = applied
                        .item_id
                        .as_ref()
                        .and_then(|id| applied.current.items.iter().find(|v| &v.item.id == id))
                        .is_some_and(|v| !self.matches(v));
                    self.notice = Some(format!(
                        "✓ Checklist {action} acknowledged{}{}.{}",
                        if applied.changed {
                            ""
                        } else {
                            "; nothing changed"
                        },
                        if hidden {
                            "; item now hidden by filters"
                        } else {
                            ""
                        },
                        if self.unknown.is_empty() {
                            ""
                        } else {
                            " Original update outcome remains unknown."
                        }
                    ));
                    if let Some(preview) = &mut self.preview {
                        preview.current = applied.current;
                    }
                    self.draft = None;
                    self.submit = None;
                    self.screen = Screen::List;
                    self.focus = 0;
                    self.failure = None;
                }
                Err(error) => {
                    self.reviewed = false;
                    if let Some(draft) = &mut self.draft {
                        draft.fresh = false;
                    }
                    if error.code == Code::OutcomeUnknown {
                        self.unknown.push(self.target_text());
                    }
                    // Authorized conflict current is separate from the retained intent/draft.
                    if let Some(current) = &error.current
                        && let Some(preview) = &mut self.preview
                    {
                        preview.current = *current.clone();
                    }
                    self.failure = Some(error);
                    self.reset_panel(Some("cancel"));
                }
            },
        }
        self.reconcile();
        self.invalidate();
    }
    fn reset_panel(&self, selected: Option<&str>) {
        *self.panel.borrow_mut() = picker_surface::State::new(None, vec![], None);
        self.readable.set(false);
        *self.reading.borrow_mut() = picker_surface::State::new(None, vec![], None);
        if let Some(id) = selected {
            let rows = self.choice_rows();
            let mut panel = self.panel.borrow_mut();
            panel.reconcile(
                rows.iter()
                    .map(|r| ListRow {
                        id: r.id.clone(),
                        disabled: r.disabled,
                    })
                    .collect(),
            );
            panel.select(id);
        }
    }
    fn reconcile(&self) {
        let rows = self.choice_rows();
        let state = if self.screen == Screen::List {
            &self.list
        } else {
            &self.panel
        };
        state.borrow_mut().reconcile(
            rows.into_iter()
                .map(|row| ListRow {
                    id: row.id,
                    disabled: row.disabled,
                })
                .collect(),
        );
    }
    fn screen(&mut self, screen: Screen, selected: Option<&str>) -> Effect {
        self.screen = screen;
        self.focus = 0;
        self.reset_panel(selected);
        self.reconcile();
        Effect::None
    }
    fn problem(&mut self, message: impl Into<String>) -> Effect {
        self.notice = Some(message.into());
        Effect::None
    }
    pub(super) fn input(&mut self, event: &Event, opener: Option<Vec<String>>) -> Effect {
        if let Event::Resize(..) = event {
            self.invalidate();
            return Effect::None;
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return Effect::None;
            }
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
                && !(self.screen == Screen::Form && key.code == KeyCode::Char('q'))
            {
                if self.pending.as_ref().is_some_and(|_| self.submit.is_some()) {
                    return self.problem("Update in progress; its outcome will be retained.");
                }
                if self.screen == Screen::List || self.screen == Screen::Rooms {
                    self.active = false;
                    return Effect::None;
                }
                return self.screen(Screen::List, None);
            }
            if self.pending.is_some() {
                return Effect::None;
            }
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                if matches!(self.screen, Screen::Details(_)) {
                    self.focus = (self.focus + 1) % 2;
                    return Effect::None;
                }
                if self.screen == Screen::List {
                    let controls = self.controls.get();
                    let available: Vec<_> = std::iter::once(0)
                        .chain(
                            controls
                                .iter()
                                .enumerate()
                                .filter(|(_, rect)| rect.width > 0)
                                .map(|(index, _)| index + 1),
                        )
                        .collect();
                    let index = available
                        .iter()
                        .position(|focus| *focus == self.focus)
                        .unwrap_or(0);
                    let reverse =
                        key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                    let next = if reverse {
                        (index + available.len() - 1) % available.len()
                    } else {
                        (index + 1) % available.len()
                    };
                    self.focus = available[next];
                    return Effect::None;
                }
                let code = if key.code == KeyCode::BackTab
                    || key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                };
                let mut key = *key;
                key.code = code;
                return self.input(&Event::Key(key), opener);
            }
            if self.screen == Screen::List && self.focus > 0 {
                if key.code == KeyCode::Enter && self.controls.get()[self.focus - 1].width > 0 {
                    return self.screen(
                        if self.focus == 1 {
                            Screen::Filters
                        } else {
                            Screen::Actions
                        },
                        None,
                    );
                }
                return Effect::None;
            }
            if self.screen == Screen::Form {
                let field = self
                    .panel
                    .borrow()
                    .picker
                    .list
                    .selected()
                    .map(str::to_owned);
                if let Some(field) = field
                    && let Some(text) = self.draft.as_mut().and_then(|d| d.text_mut(&field))
                {
                    let limit = match field.as_str() {
                        "body" => 65536,
                        "title" => 1024,
                        _ => 4096,
                    };
                    match key.code {
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            text.clear()
                        }
                        KeyCode::Char(ch)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                && !ch.is_control() =>
                        {
                            if text.len() + ch.len_utf8() <= limit {
                                text.push(ch);
                            }
                        }
                        KeyCode::Backspace => {
                            text.pop();
                        }
                        KeyCode::Enter if field == "body" && text.len() < limit => {
                            text.push('\n');
                        }
                        _ => {}
                    }
                    if matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace)
                        || key.code == KeyCode::Enter && field == "body"
                    {
                        self.invalidate();
                        return Effect::None;
                    }
                }
            }
        }
        if let Event::Paste(value) = event
            && self.screen == Screen::Form
        {
            let field = self
                .panel
                .borrow()
                .picker
                .list
                .selected()
                .map(str::to_owned);
            if let Some(field) = field
                && let Some(text) = self.draft.as_mut().and_then(|d| d.text_mut(&field))
            {
                let limit = match field.as_str() {
                    "body" => 65536,
                    "title" => 1024,
                    _ => 4096,
                };
                if text.len() + value.len() <= limit
                    && value.chars().all(|ch| {
                        !ch.is_control() || field == "body" && matches!(ch, '\n' | '\r' | '\t')
                    })
                {
                    text.push_str(value);
                } else {
                    return self.problem(
                        "Pasted text exceeds the field limit or contains unsupported controls.",
                    );
                }
                self.invalidate();
            }
            return Effect::None;
        }
        if self.screen == Screen::Unknown
            || matches!(self.screen, Screen::Details(_)) && self.focus == 0
        {
            if matches!(event,Event::Key(key) if key.code==KeyCode::Enter)
                && self.screen != Screen::Unknown
            {
                self.focus = 1;
                return Effect::None;
            }
            self.reading.borrow_mut().picker.list.scroll.input(event);
            return Effect::None;
        }
        if let Event::Mouse(mouse) = event
            && self.screen == Screen::List
            && mouse.kind
                == ratatui::crossterm::event::MouseEventKind::Down(
                    ratatui::crossterm::event::MouseButton::Left,
                )
        {
            let control = self
                .controls
                .get()
                .iter()
                .position(|rect| rect.contains((mouse.column, mouse.row).into()));
            if let Some(control) = control {
                return self.screen(
                    if control == 0 {
                        Screen::Filters
                    } else {
                        Screen::Actions
                    },
                    None,
                );
            }
        }
        self.reconcile();
        let input = if self.screen == Screen::List {
            self.list.borrow_mut().input(event, PickerField::List)
        } else {
            self.panel.borrow_mut().input(event, PickerField::List)
        };
        match input {
            Some(PickerInput::Event(PickerEvent::Confirm(id))) => self.choose(&id, opener),
            _ => Effect::None,
        }
    }
    fn choose(&mut self, id: &str, opener: Option<Vec<String>>) -> Effect {
        match self.screen.clone() {
            Screen::Unknown => Effect::None,
            Screen::Rooms => {
                if id == "refresh" {
                    self.read()
                } else if let Ok(room) = Id::parse(id) {
                    self.room = Some(room);
                    self.screen = Screen::List;
                    self.read()
                } else {
                    Effect::None
                }
            }
            Screen::List => {
                if let Ok(item) = Id::parse(id) {
                    self.screen(Screen::Details(item), None)
                } else {
                    Effect::None
                }
            }
            Screen::Actions => match id {
                "refresh" => self.read(),
                "create" if self.writable() => self.task(load::Job::Ids),
                "unknown" => self.screen(Screen::Unknown, None),
                "resume" => self.screen(Screen::Form, None),
                "resume-operation" => self.screen(Screen::Confirm, Some("cancel")),
                "reorder" if self.manager() && self.writable() => {
                    self.target_label = None;
                    let current = self.current().unwrap();
                    self.submit = Some(Request {
                        room_id: current.room.id.clone(),
                        checklist_id: current.checklist_id.clone().unwrap(),
                        action: Action::Reorder {
                            inventory_revision: current.inventory_revision.unwrap(),
                            order: current.items.iter().map(|v| v.item.id.clone()).collect(),
                        },
                    });
                    self.reviewed = true;
                    self.screen(Screen::Reorder, None)
                }
                _ => Effect::None,
            },
            Screen::Filters => {
                match id {
                    "open" => self.completion = Some(Completion::Open),
                    "complete" => self.completion = Some(Completion::Complete),
                    "all" => self.completion = None,
                    "squad" => self.assignment = Assignment::All,
                    "unassigned" => self.assignment = Assignment::Unassigned,
                    "member" => return self.screen(Screen::Members(None), None),
                    "archive" => self.archived = !self.archived,
                    _ => {}
                }
                self.screen(Screen::List, None)
            }
            Screen::Members(assign) => {
                if let Ok(member) = Id::parse(id) {
                    if assign.is_some() {
                        return self.mutation(Mutation::Assign(member));
                    }
                    self.assignment = Assignment::Member(member);
                }
                self.screen(Screen::List, None)
            }
            Screen::Details(item) => match id {
                "refresh" => self.read(),
                "edit" if self.writable() => {
                    self.draft = self
                        .preview
                        .as_ref()
                        .and_then(|p| forms::Draft::edit(p, &item));
                    self.screen(Screen::Form, Some("title"))
                }
                "assign" => self.screen(Screen::Members(Some(item.clone())), None),
                "reference" => self
                    .item(&item)
                    .and_then(|v| v.item.content.reference.clone())
                    .map_or(Effect::None, |link| {
                        Effect::Act(BoardRequest::Open { link, opener })
                    }),
                "complete" => self.mutation(Mutation::Complete),
                "reopen" => self.mutation(Mutation::Reopen),
                "unassign" => self.mutation(Mutation::Unassign),
                "archive" => self.mutation(Mutation::Archive),
                "restore" => self.mutation(Mutation::Restore),
                "delete" => {
                    let current = self.current().unwrap();
                    let revision = self.item(&item).unwrap().item.revision;
                    self.mutation(Mutation::Delete {
                        inventory_revision: current.inventory_revision.unwrap(),
                        confirmation: Confirmation {
                            item_id: item,
                            item_revision: revision,
                        },
                    })
                }
                _ => Effect::None,
            },
            Screen::Form => match id {
                "cancel" => {
                    self.draft = None;
                    self.submit = None;
                    self.screen(Screen::List, None)
                }
                "review" => self.review(),
                "refresh" => self.read(),
                "apply" => {
                    let result = self.draft.as_ref().unwrap().submit();
                    match result {
                        Ok(request) => {
                            self.target_label = Some(self.draft.as_ref().unwrap().title.clone());
                            self.submit = Some(request);
                            self.reviewed = true;
                            self.screen(Screen::Confirm, Some("cancel"))
                        }
                        Err(error) => self.problem(error.message),
                    }
                }
                _ => Effect::None,
            },
            Screen::Confirm => match id {
                "cancel" => {
                    self.submit = None;
                    if self.draft.is_some() {
                        self.screen(Screen::Form, None)
                    } else {
                        self.screen(Screen::List, None)
                    }
                }
                "refresh" => self.read(),
                "review" => self.review(),
                "confirm" => self.apply(),
                _ => Effect::None,
            },
            Screen::Reorder => match id {
                "cancel" => {
                    self.submit = None;
                    self.screen(Screen::List, None)
                }
                "apply" => self.screen(Screen::Confirm, Some("cancel")),
                _ => Id::parse(id).map_or(Effect::None, |item| {
                    self.screen(Screen::OrderActions(item), None)
                }),
            },
            Screen::OrderActions(item) => {
                if let Some(Request {
                    action: Action::Reorder { order, .. },
                    ..
                }) = &mut self.submit
                    && let Some(index) = order.iter().position(|i| i == &item)
                {
                    let next = if id == "up" {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(order.len().saturating_sub(1))
                    };
                    order.swap(index, next);
                }
                self.screen(Screen::Reorder, Some(item.as_str()))
            }
        }
    }
    fn mutation(&mut self, mutation: Mutation) -> Effect {
        if !self.writable() {
            return self.problem("Read current admitted state before submitting.");
        }
        let item_id = match &self.screen {
            Screen::Details(id) => Some(id.clone()),
            Screen::Members(Some(id)) => Some(id.clone()),
            _ => None,
        };
        let Some(item_id) = item_id else {
            return Effect::None;
        };
        let current = self.current().unwrap();
        let Some(item) = self.item(&item_id) else {
            return self.problem("The exact item is no longer available.");
        };
        let revision = item.item.revision;
        let title = item.item.content.title.clone();
        let room_id = current.room.id.clone();
        let checklist_id = current.checklist_id.clone().unwrap();
        self.target_label = Some(title);
        self.submit = Some(Request {
            room_id,
            checklist_id,
            action: Action::Item {
                item_id,
                revision,
                mutation,
            },
        });
        self.reviewed = true;
        self.screen(Screen::Confirm, Some("cancel"))
    }
    fn review(&mut self) -> Effect {
        if self.failure.is_some() {
            return self.problem("Refresh admitted current state first; draft remains retained.");
        }
        let Some(preview) = &self.preview else {
            return Effect::None;
        };
        let result = if let Some(draft) = &mut self.draft {
            draft.review(preview)
        } else if let Some(request) = &mut self.submit {
            forms::review(request, preview)
        } else {
            return Effect::None;
        };
        match result {
            Ok(()) => {
                if self.draft.is_some() && self.submit.is_some() {
                    match self.draft.as_ref().unwrap().submit() {
                        Ok(request) => self.submit = Some(request),
                        Err(error) => return self.problem(error.message),
                    }
                }
                self.reviewed = true;
                self.notice=Some("Current expectations reviewed; submit explicitly. Original Unknown stays unknown.".into());
                self.reset_panel(Some("cancel"));
                Effect::None
            }
            Err(error) => self.problem(error.message),
        }
    }
    fn apply(&mut self) -> Effect {
        let confirm_visible = self
            .panel
            .borrow()
            .frame
            .as_ref()
            .and_then(|map| map.list.as_ref())
            .is_some_and(|map| {
                map.geometry.iter().any(|row| {
                    row.id == "confirm" && row.visible.height > 0 && row.visible.width >= 14
                })
            });
        if !confirm_visible {
            return self.problem(
                "Confirm is not visible in the current frame; enlarge or scroll before submitting.",
            );
        }
        let destructive = self.submit.as_ref().is_some_and(|r| {
            matches!(
                r.action,
                Action::Item {
                    mutation: Mutation::Delete { .. },
                    ..
                }
            )
        });
        if destructive && !self.readable.get() {
            return self.problem(
                "Exact target and Cancel/Confirm do not fit readably; enlarge the terminal.",
            );
        }
        if !self.writable() {
            return self.problem("Refresh and review admitted current state before submitting.");
        }
        if !self.reviewed || self.draft.as_ref().is_some_and(|d| !d.fresh) {
            return self.problem("Review the retained draft against current state first.");
        }
        let actor = self.preview.as_ref().unwrap().actor_id.clone();
        let request = self.submit.clone().unwrap();
        self.task(load::Job::Apply {
            actor,
            request: Box::new(request),
        })
    }
}
pub(super) fn action_name(request: &Request) -> &'static str {
    match &request.action {
        Action::Create { .. } => "create",
        Action::Reorder { .. } => "reorder",
        Action::Item { mutation, .. } => match mutation {
            Mutation::Edit(_) => "edit",
            Mutation::Assign(_) => "assign",
            Mutation::Unassign => "unassign",
            Mutation::Complete => "complete",
            Mutation::Reopen => "reopen",
            Mutation::Archive => "archive",
            Mutation::Restore => "restore",
            Mutation::Delete { .. } => "delete",
        },
    }
}
