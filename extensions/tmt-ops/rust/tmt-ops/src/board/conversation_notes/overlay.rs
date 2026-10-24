//! The Notes list of the selected conversation, as an overlay: its keys and its writes. The items
//! and the unsent form stay in [`State`]; this owns the cursor and the one-line notice.
use super::{
    Draft, Key, State, Write, Written,
    draft::{Aim, MAX_BYTES},
    rows::{Kind, Row, rows},
};
use crate::{
    board::{
        app::{App, Effect, Request},
        picker_surface, search_field,
    },
    checklist::{Code, model::Id},
};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::{cell::RefCell, time::Instant};
use tmt_tui::components::{ListRow, PickerEvent, PickerField, PickerInput};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    List,
    /// Asked to delete this item; Enter does it, Esc keeps it.
    Delete(Id),
}

pub(in crate::board) struct Controller {
    pub(super) key: Key,
    mode: Mode,
    /// One line for the last thing that went wrong, in Core's words where it has them.
    pub(super) notice: Option<String>,
    /// The item to put the cursor on once the list has it, as after an add.
    pub(super) reveal: RefCell<Option<Id>>,
    pub(super) ui: RefCell<picker_surface::State>,
    pub(super) template: RefCell<Option<(String, tmt_tui::components::surface::Template<()>)>>,
    closed: bool,
}

fn plain(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

impl Controller {
    fn new(key: Key) -> Self {
        Self {
            key,
            mode: Mode::List,
            notice: None,
            reveal: RefCell::new(None),
            ui: RefCell::new(picker_surface::State::new(None, vec![], None)),
            template: RefCell::new(None),
            closed: false,
        }
    }

    pub(super) fn deleting(&self) -> Option<&Id> {
        match &self.mode {
            Mode::Delete(id) => Some(id),
            Mode::List => None,
        }
    }

    /// An unsent form takes every key, digits and letters included, until it is sent.
    pub(in crate::board) fn owns_typing(&self, notes: &State) -> bool {
        notes.draft(&self.key).is_some_and(|draft| !draft.sent())
    }

    pub fn invalidate(&self) {
        self.ui.borrow_mut().invalidate();
    }

    /// The items as the overlay lists them: links first, then todos open and done.
    pub(super) fn items(&self, notes: &State) -> Vec<Row> {
        let mut items = notes
            .preview(&self.key)
            .map(|preview| rows(preview, crate::status::now_ms()))
            .unwrap_or_default();
        items.sort_by_key(|row| row.kind != Kind::Link);
        items
    }

    /// Headings are rows the cursor skips; an item's row is its own UUID.
    pub(super) fn list_rows(items: &[Row]) -> Vec<ListRow> {
        let mut list = Vec::new();
        for (heading, links) in [("group:links", true), ("group:todo", false)] {
            let members: Vec<_> = items
                .iter()
                .filter(|row| (row.kind == Kind::Link) == links)
                .collect();
            if members.is_empty() {
                continue;
            }
            list.push(ListRow {
                id: heading.into(),
                disabled: true,
            });
            list.extend(members.into_iter().map(|row| ListRow {
                id: row.id.as_str().into(),
                disabled: false,
            }));
        }
        list
    }

    fn selected(&self, items: &[Row]) -> Option<Row> {
        let ui = self.ui.borrow();
        let id = ui.picker.list.selected()?;
        items.iter().find(|row| row.id.as_str() == id).cloned()
    }

    fn say(&mut self, words: impl Into<String>) -> Effect {
        self.notice = Some(words.into());
        self.invalidate();
        Effect::None
    }

    fn revision(&self, notes: &State, item: &Id) -> Option<u64> {
        notes
            .preview(&self.key)?
            .current
            .items
            .iter()
            .find(|view| &view.item.id == item)
            .map(|view| view.item.revision)
    }

    fn inventory(&self, notes: &State) -> Option<u64> {
        notes.preview(&self.key)?.current.inventory_revision
    }

    fn write(&mut self, notes: &mut State, write: Write) -> Effect {
        match notes.write(&self.key, write) {
            Some(task) => {
                self.notice = None;
                self.mode = Mode::List;
                self.invalidate();
                Effect::Notes(task)
            }
            None if notes.writing(&self.key) => self.say("Saving the last change…"),
            None => self.say("The Notes have not been read yet."),
        }
    }

    pub(in crate::board) fn event(&mut self, app: &mut App, event: &Event) -> Effect {
        if let Event::Resize(..) = event {
            self.invalidate();
            return Effect::None;
        }
        if app.notes.draft(&self.key).is_some() {
            return self.form(app, event);
        }
        let items = self.items(&app.notes);
        self.ui.borrow_mut().reconcile(Self::list_rows(&items));
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return Effect::None;
            }
            if let Mode::Delete(id) = self.mode.clone() {
                match key.code {
                    KeyCode::Enter => return self.delete(app, &id),
                    KeyCode::Esc => {
                        self.mode = Mode::List;
                        self.invalidate();
                    }
                    _ => {}
                }
                return Effect::None;
            }
            match key.code {
                KeyCode::Esc => {
                    self.closed = true;
                    return Effect::None;
                }
                KeyCode::Char('q') if plain(key) => {
                    self.closed = true;
                    return Effect::None;
                }
                KeyCode::Char('a') if plain(key) => return self.begin(app, Draft::add()),
                KeyCode::Char('e') if plain(key) => return self.edit(app, &items),
                KeyCode::Char('d') if plain(key) => {
                    if let Some(row) = self.selected(&items) {
                        self.notice = None;
                        self.mode = Mode::Delete(row.id);
                        self.invalidate();
                    }
                    return Effect::None;
                }
                KeyCode::Up | KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    return self.reorder(app, &items, key.code == KeyCode::Down);
                }
                _ => {}
            }
        }
        let input = self.ui.borrow_mut().input(event, PickerField::List);
        match input {
            Some(PickerInput::Event(PickerEvent::Confirm(id))) => self.activate(app, &items, &id),
            _ => Effect::None,
        }
    }

    /// Enter on an item: open a link, or tick a todo and untick a done one.
    fn activate(&mut self, app: &mut App, items: &[Row], id: &str) -> Effect {
        let Some(row) = items.iter().find(|row| row.id.as_str() == id) else {
            return Effect::None;
        };
        if let (Kind::Link, Some(link)) = (row.kind, &row.reference) {
            return Effect::Act(Request::Open {
                link: link.clone(),
                opener: app.view.as_ref().and_then(|view| view.opener.clone()),
            });
        }
        let Some(revision) = self.revision(&app.notes, &row.id) else {
            return Effect::None;
        };
        let write = Write::Toggle {
            item: row.id.clone(),
            revision,
            complete: row.kind == Kind::Todo,
        };
        self.write(&mut app.notes, write)
    }

    fn delete(&mut self, app: &mut App, item: &Id) -> Effect {
        let (Some(revision), Some(inventory)) =
            (self.revision(&app.notes, item), self.inventory(&app.notes))
        else {
            self.mode = Mode::List;
            return self.say("That note was removed elsewhere.");
        };
        let write = Write::Delete {
            item: item.clone(),
            revision,
            inventory,
        };
        self.write(&mut app.notes, write)
    }

    /// Moves the item one place among the items of its own group, which swaps it with that
    /// neighbour in the list's full order.
    fn reorder(&mut self, app: &mut App, items: &[Row], down: bool) -> Effect {
        let (Some(row), Some(inventory), Some(preview)) = (
            self.selected(items),
            self.inventory(&app.notes),
            app.notes.preview(&self.key),
        ) else {
            return Effect::None;
        };
        let group: Vec<_> = items
            .iter()
            .filter(|other| (other.kind == Kind::Link) == (row.kind == Kind::Link))
            .collect();
        let at = group.iter().position(|other| other.id == row.id);
        let neighbour = at.and_then(|at| {
            if down {
                group.get(at + 1)
            } else {
                at.checked_sub(1).and_then(|before| group.get(before))
            }
        });
        let Some(neighbour) = neighbour else {
            return Effect::None;
        };
        let mut order: Vec<Id> = preview
            .current
            .items
            .iter()
            .map(|view| view.item.id.clone())
            .collect();
        let (Some(a), Some(b)) = (
            order.iter().position(|id| *id == row.id),
            order.iter().position(|id| *id == neighbour.id),
        ) else {
            return Effect::None;
        };
        order.swap(a, b);
        self.write(&mut app.notes, Write::Reorder { inventory, order })
    }

    fn begin(&mut self, app: &mut App, draft: Draft) -> Effect {
        if app.notes.preview(&self.key).is_none() {
            return self.say("The Notes have not been read yet.");
        }
        app.notes.keep(&self.key, draft);
        self.notice = None;
        self.invalidate();
        Effect::None
    }

    fn edit(&mut self, app: &mut App, items: &[Row]) -> Effect {
        let Some(row) = self.selected(items) else {
            return Effect::None;
        };
        let Some(revision) = self.revision(&app.notes, &row.id) else {
            return Effect::None;
        };
        let draft = Draft::edit(row.id, revision, &row.title, row.reference.as_deref());
        self.begin(app, draft)
    }

    /// Keys of the unsent form: it takes every key but Esc, Enter and the field switches.
    fn form(&mut self, app: &mut App, event: &Event) -> Effect {
        let key = self.key.clone();
        let sent = app.notes.draft(&key).is_some_and(Draft::sent);
        match event {
            Event::Key(press) if press.kind != KeyEventKind::Release => match press.code {
                KeyCode::Esc => {
                    // An empty form, or an edit that changed nothing, is not worth keeping.
                    if !sent && app.notes.draft(&key).is_some_and(Draft::untouched) {
                        app.notes.drop_draft(&key);
                    }
                    self.closed = true;
                    return Effect::None;
                }
                _ if sent => {}
                KeyCode::Enter => return self.submit(app),
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                    if let Some(draft) = app.notes.draft_mut(&key) {
                        draft.focus ^= 1;
                    }
                }
                _ => {
                    if let Some(draft) = app.notes.draft_mut(&key) {
                        draft.fields[draft.focus].key(*press, u16::MAX, MAX_BYTES);
                    }
                }
            },
            Event::Paste(text) if !sent => {
                if let Some(draft) = app.notes.draft_mut(&key)
                    && !search_field::paste(&mut draft.fields[draft.focus], text)
                {
                    return self.say("That text is too long for one line.");
                }
            }
            _ => {}
        }
        self.notice = None;
        self.invalidate();
        Effect::None
    }

    fn submit(&mut self, app: &mut App) -> Effect {
        let key = self.key.clone();
        let Some(draft) = app.notes.draft(&key) else {
            return Effect::None;
        };
        let content = match draft.content() {
            Ok(content) => content,
            Err(words) => return self.say(words),
        };
        let write = match draft.aim.clone() {
            Aim::Add => Write::Add {
                title: content.title,
                reference: content.reference,
                inventory: self.inventory(&app.notes),
            },
            Aim::Edit { item, revision } => {
                if self.revision(&app.notes, &item).is_none() {
                    return self.say("That note was removed elsewhere; your draft is kept.");
                }
                Write::Edit {
                    item,
                    revision,
                    title: content.title,
                    reference: content.reference,
                }
            }
        };
        let effect = self.write(&mut app.notes, write);
        if matches!(effect, Effect::Notes(_))
            && let Some(draft) = app.notes.draft_mut(&key)
        {
            draft.send();
        }
        effect
    }

    /// How a write ended: the cursor goes to the item it made, or the notice says why not.
    pub(in crate::board) fn written(&mut self, notes: &State, written: &Written) {
        match &written.result {
            Ok(()) => {
                self.notice = None;
                *self.reveal.borrow_mut() = written.item.clone();
            }
            Err(error) => self.notice = Some(match error.code {
                Code::Conflict if notes.draft(&self.key).is_some() => {
                    "Changed elsewhere. Your draft is kept; Enter applies it to the new version."
                        .into()
                }
                Code::Conflict => "Changed elsewhere. The list is refreshed; try again.".into(),
                Code::Deleted => "That note was removed elsewhere.".into(),
                _ => error.message.clone(),
            }),
        }
        self.invalidate();
    }
}

impl App {
    pub(in crate::board) fn open_notes(&mut self) -> Effect {
        if self.input.is_some() || self.searching || self.modal_open() && self.menu.is_none() {
            return self.say("Close the current input or overlay before opening Notes.");
        }
        let Some(key) = self.conversation_notes_key() else {
            return self.say("Select a squad member before opening Notes.");
        };
        self.menu = None;
        self.notes_overlay = Some(Controller::new(key));
        Effect::None
    }

    pub(in crate::board) fn notes_event(&mut self, event: &Event) -> Option<Effect> {
        let mut controller = self.notes_overlay.take()?;
        let effect = controller.event(self, event);
        if !controller.closed {
            self.notes_overlay = Some(controller);
        }
        Some(effect)
    }

    pub(in crate::board) fn finished_notes(&mut self, completed: super::Completed, now: Instant) {
        if let Some(written) = self.notes.finished(completed, now)
            && let Some(controller) = &mut self.notes_overlay
            && controller.key == written.key
        {
            controller.written(&self.notes, &written);
        }
    }
}
