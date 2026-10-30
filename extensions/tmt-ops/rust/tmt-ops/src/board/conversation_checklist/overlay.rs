//! The selected conversation's checklist overlay: its keys and writes. The items
//! and the unsent form stay in [`State`]; this owns the cursor and the one-line notice.
use super::{
    Draft, Key, State, Write, Written,
    draft::{Aim, MAX_BYTES, NoteKind, Seed},
    rows::{Row, rows},
};
use crate::{
    board::{
        app::{App, Effect, Request},
        picker_surface, search_field,
    },
    checklist::{Code, model::Id},
};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::Rect;
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
    pub(super) kind_hits: RefCell<Vec<(ratatui::layout::Rect, NoteKind)>>,
    /// Where the last paint put each link's cells.
    pub(super) link_hits: RefCell<Vec<super::surface::LinkHit>>,
    /// The link under the pointer: its item and which of the item's links it is.
    pub(super) hovered: RefCell<Option<(String, usize)>>,
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
            kind_hits: RefCell::new(Vec::new()),
            link_hits: RefCell::new(Vec::new()),
            hovered: RefCell::new(None),
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

    /// Esc or an outside press: the unsent form's text is kept for the next open, except an
    /// empty form or an edit that changed nothing, which is not worth keeping.
    pub(in crate::board) fn close(&self, notes: &mut State) {
        if self.owns_typing(notes) && notes.draft(&self.key).is_some_and(Draft::untouched) {
            notes.drop_draft(&self.key);
        }
    }

    /// Where the sheet was last painted.
    pub(in crate::board) fn modal_outer(&self) -> Option<Rect> {
        self.ui.borrow().outer()
    }

    pub fn invalidate(&self) {
        self.kind_hits.borrow_mut().clear();
        self.link_hits.borrow_mut().clear();
        self.ui.borrow_mut().invalidate();
    }

    /// The painted link that holds this point.
    fn link_at(&self, at: ratatui::layout::Position) -> Option<super::surface::LinkHit> {
        self.link_hits
            .borrow()
            .iter()
            .find(|hit| hit.area.contains(at))
            .cloned()
    }

    /// Follow the pointer over the links; true when the highlighted link changed.
    pub(in crate::board) fn move_pointer(&self, at: ratatui::layout::Position) -> bool {
        let next = self.link_at(at).map(|hit| (hit.row, hit.link));
        let changed = *self.hovered.borrow() != next;
        if changed {
            *self.hovered.borrow_mut() = next;
            self.ui.borrow_mut().invalidate();
        }
        changed
    }

    /// The open items as the overlay lists them: one list, in the order they are stored.
    pub(super) fn items(&self, notes: &State) -> Vec<Row> {
        notes
            .preview(&self.key)
            .map(|preview| rows(preview, crate::status::now_ms()))
            .unwrap_or_default()
    }

    /// An item's row is its own UUID.
    pub(super) fn list_rows(items: &[Row]) -> Vec<ListRow> {
        items
            .iter()
            .map(|row| ListRow {
                id: row.id.as_str().into(),
                disabled: false,
            })
            .collect()
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
            None => self.say("The checklist has not been read yet."),
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
                KeyCode::Char(' ') if plain(key) => return self.tick_selected(app, &items),
                KeyCode::Char('o') if plain(key) => return self.open_selected(app, &items),
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
        // A press on a link opens it; the rest of the row still chooses the row.
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(hit) = self.link_at((mouse.column, mouse.row).into())
            && items.iter().any(|row| row.id.as_str() == hit.row)
            && !hit.url.is_empty()
        {
            self.ui.borrow_mut().select(&hit.row);
            return Self::open(app, &hit.url);
        }
        let input = self.ui.borrow_mut().input(event, PickerField::List);
        match input {
            Some(PickerInput::Event(PickerEvent::Confirm(id))) => self.activate(app, &items, &id),
            _ => Effect::None,
        }
    }

    /// Enter on an item: a link row does what `board.checklist_enter` says, open or tick; any
    /// other row completes a todo.
    fn activate(&mut self, app: &mut App, items: &[Row], id: &str) -> Effect {
        let Some(row) = items.iter().find(|row| row.id.as_str() == id) else {
            return Effect::None;
        };
        let enter = app.view.as_ref().map(|view| view.checklist_enter);
        match (&row.reference, enter) {
            (Some(_), Some(crate::config::ChecklistEnter::Tick)) => self.tick(app, row),
            (Some(link), _) => Self::open(app, link),
            (None, _) => self.tick(app, row),
        }
    }

    /// Space ticks the chosen row whatever Enter is set to do.
    fn tick_selected(&mut self, app: &mut App, items: &[Row]) -> Effect {
        match self.selected(items) {
            Some(row) => self.tick(app, &row),
            None => Effect::None,
        }
    }

    /// `o` opens the chosen row's link whatever Enter is set to do.
    fn open_selected(&mut self, app: &mut App, items: &[Row]) -> Effect {
        match self.selected(items).and_then(|row| row.reference) {
            Some(link) => Self::open(app, &link),
            None => self.say("This item has no link."),
        }
    }

    fn open(app: &App, link: &str) -> Effect {
        Effect::Act(Request::Open {
            link: link.to_owned(),
            opener: app.view.as_ref().and_then(|view| view.opener.clone()),
        })
    }

    fn tick(&mut self, app: &mut App, row: &Row) -> Effect {
        let Some(revision) = self.revision(&app.notes, &row.id) else {
            return Effect::None;
        };
        let write = Write::Toggle {
            item: row.id.clone(),
            revision,
            complete: true,
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

    /// Moves the item one place in the list, which swaps it with that neighbour in the stored
    /// order.
    fn reorder(&mut self, app: &mut App, items: &[Row], down: bool) -> Effect {
        let (Some(row), Some(inventory), Some(preview)) = (
            self.selected(items),
            self.inventory(&app.notes),
            app.notes.preview(&self.key),
        ) else {
            return Effect::None;
        };
        let at = items.iter().position(|other| other.id == row.id);
        let neighbour = at.and_then(|at| {
            if down {
                items.get(at + 1)
            } else {
                at.checked_sub(1).and_then(|before| items.get(before))
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
            return self.say("The checklist has not been read yet.");
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
                    self.close(&mut app.notes);
                    self.closed = true;
                    return Effect::None;
                }
                _ if sent => {}
                KeyCode::Enter if app.notes.draft(&key).is_some_and(Draft::on_discard) => {
                    app.notes.drop_draft(&key);
                    self.invalidate();
                    return Effect::None;
                }
                KeyCode::Enter => return self.submit(app),
                KeyCode::Up | KeyCode::Down => {
                    if let Some(draft) = app.notes.draft_mut(&key) {
                        let stops = draft.stops();
                        let back = press.code == KeyCode::Up;
                        draft.focus = (draft.focus + if back { stops - 1 } else { 1 }) % stops;
                    }
                }
                KeyCode::Left | KeyCode::Right
                    if app
                        .notes
                        .draft(&key)
                        .is_some_and(|draft| draft.seed.is_some() && draft.focus == 0) =>
                {
                    if let Some(seed) = app
                        .notes
                        .draft_mut(&key)
                        .and_then(|draft| draft.seed.as_mut())
                    {
                        seed.kind = match seed.kind {
                            NoteKind::Todo => NoteKind::Link,
                            NoteKind::Link => NoteKind::Todo,
                        };
                    }
                }
                _ => {
                    if let Some(draft) = app.notes.draft_mut(&key)
                        && let Some(field) = draft.field()
                    {
                        draft.fields[field].key(*press, u16::MAX, MAX_BYTES);
                    }
                }
            },
            Event::Mouse(mouse)
                if !sent && mouse.kind == MouseEventKind::Down(MouseButton::Left) =>
            {
                let kind = self
                    .kind_hits
                    .borrow()
                    .iter()
                    .find(|(area, _)| area.contains((mouse.column, mouse.row).into()))
                    .map(|(_, kind)| *kind);
                if let Some(kind) = kind
                    && let Some(draft) = app.notes.draft_mut(&key)
                    && let Some(seed) = &mut draft.seed
                {
                    seed.kind = kind;
                    draft.focus = 0;
                }
            }
            Event::Paste(text) if !sent => {
                if let Some(draft) = app.notes.draft_mut(&key)
                    && let Some(field) = draft.field()
                    && !search_field::paste(&mut draft.fields[field], text)
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
        if app.notes.preview(&self.key).is_none() {
            return self.say("The checklist has not been read yet; your draft is kept.");
        }
        let key = self.key.clone();
        let Some(draft) = app.notes.draft(&key) else {
            return Effect::None;
        };
        let content = match draft.content() {
            Ok(content) => content,
            Err(words) => return self.say(words),
        };
        if let Some(seed) = &draft.seed
            && !app.note_seed_current(seed)
        {
            return self.say("The conversation changed; your draft is kept.");
        }
        let write = match draft.aim.clone() {
            Aim::Add => Write::Add {
                body: draft.seed.as_ref().map(|seed| seed.source.body()),
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
                self.notice = written.item.as_ref().and_then(|id| {
                    notes
                        .preview(&self.key)?
                        .current
                        .items
                        .iter()
                        .find(|view| {
                            &view.item.id == id
                                && view.item.completion
                                    == crate::checklist::model::Completion::Complete
                        })
                        .map(|view| format!("✓ Completed {}", view.item.content.title))
                });
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
    pub(in crate::board) fn note_seed_current(&self, seed: &Seed) -> bool {
        self.conversation_checklist_key().as_ref() == Some(&seed.key)
            && self.view.as_ref().and_then(|view| view.me_id.as_ref()) == seed.actor.as_ref()
            && !self.loading()
    }

    pub(in crate::board) fn add_to_notes(&mut self, seed: Seed) -> Effect {
        if !self.note_seed_current(&seed) {
            return self.say("The conversation changed; open Add to checklist again.");
        }
        if self.input.is_some() || self.searching || self.modal_open() && self.menu.is_none() {
            return self.say("Close the current input or overlay before adding to the checklist.");
        }
        let key = seed.key.clone();
        if self.notes.draft(&key).is_none()
            && (seed.title.len() > MAX_BYTES || seed.reference.len() > MAX_BYTES)
        {
            return self.say(
                "That selection or link is too long for the checklist form; select a shorter title.",
            );
        }
        let mut controller = Controller::new(key.clone());
        if let Some(draft) = self.notes.draft_mut(&key) {
            draft.reopened = !draft.untouched();
            controller.notice =
                Some("Your existing checklist draft is kept; finish or discard it first.".into());
        } else {
            self.notes.keep(&key, Draft::seeded(seed));
        }
        self.menu = None;
        self.notes_overlay = Some(controller);
        Effect::None
    }

    pub(in crate::board) fn open_notes(&mut self) -> Effect {
        if self.input.is_some() || self.searching || self.modal_open() && self.menu.is_none() {
            return self.say("Close the current input or overlay before opening the checklist.");
        }
        let Some(key) = self.conversation_checklist_key() else {
            return self.say("Select a squad member before opening the checklist.");
        };
        self.menu = None;
        // A kept draft with text comes back with Discard.
        if let Some(draft) = self.notes.draft_mut(&key) {
            draft.reopened = !draft.untouched();
        }
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
