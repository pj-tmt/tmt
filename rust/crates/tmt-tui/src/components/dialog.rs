//! One dialog for every overlay that asks, edits, chooses or shows: its kind,
//! its button row and the one key and mouse rule of
//! `design/cli-style.md#dialogs`. The application owns field contents, rows,
//! data and effects; the dialog owns focus, the button cursor and the close
//! decision. Tab is never a dialog key.
use super::{
    button::{Button, ButtonRow},
    modal::{Modal, ModalAreas, Placement},
};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    layout::{Position, Rect},
};
use tmt_cli_style::{Depth, Theme, grid::Align};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    /// Text and buttons.
    Confirm,
    /// Fields and buttons.
    Form,
    /// Choosable rows and optional buttons; Enter chooses the row.
    List,
    /// Scrolling text and no buttons. An outside press closes it and lands on
    /// the board underneath.
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// Typed text; printable keys and ←→ edit it.
    Text,
    /// One value of several; ←→ change it.
    Choice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Field(usize),
    Buttons(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogInput {
    /// An edit key for field `i`: printable text, Backspace, ←→ and the like.
    Field(usize, KeyEvent),
    /// A row or scroll key for the list or text owner.
    Content(KeyEvent),
    /// Button `i` was chosen with Enter or a press.
    Choose(usize),
    /// Esc, `q` outside a text field, or a press outside the dialog.
    Cancel,
    /// The focus or the button cursor moved; repaint.
    Moved,
    /// Not a dialog key (Tab among them); nothing happens.
    Ignored,
}

/// What a painted dialog occupies. The application adds `fields` after it
/// paints them in `content`, in field order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DialogHits {
    pub outer: Rect,
    pub content: Rect,
    pub buttons: Vec<Option<Rect>>,
    pub fields: Vec<Rect>,
}

#[derive(Debug, Clone)]
pub struct Dialog<'a> {
    pub title: String,
    pub kind: DialogKind,
    pub placement: Placement,
    pub buttons: Vec<Button<'a>>,
}

impl Dialog<'_> {
    fn modal(&self) -> Modal {
        Modal {
            title: self.title.clone(),
            placement: self.placement,
        }
    }

    /// The modal size rule: as wide as `demand`, at most nine tenths of the
    /// body and four fifths of its height, full width below `md`. A dialog
    /// with buttons reserves its last inside line for them.
    pub fn areas(&self, body: Rect, demand: [u16; 2], status: bool) -> ModalAreas {
        self.modal()
            .areas(body, demand, !self.buttons.is_empty(), status)
    }

    /// Clear and frame the dialog, then paint its buttons on the footer line,
    /// from the left as every board dialog does, with the cursor's button focused.
    pub fn paint(
        &self,
        areas: ModalAreas,
        state: &DialogState,
        buffer: &mut Buffer,
        theme: &Theme,
        depth: Depth,
        hovered: Option<Rect>,
    ) -> DialogHits {
        self.modal().paint(areas, buffer, theme, depth);
        let cursor = state.button(&self.buttons);
        let buttons: Vec<_> = self
            .buttons
            .iter()
            .enumerate()
            .map(|(index, button)| button.focused(Some(index) == cursor))
            .collect();
        let row = ButtonRow {
            theme,
            depth,
            align: Align::Left,
            active: true,
            hovered,
        };
        DialogHits {
            outer: areas.outer,
            content: areas.content,
            buttons: row.paint(buffer, areas.footer, &buttons),
            fields: Vec::new(),
        }
    }
}

/// Focus and the button cursor of one open dialog. A new dialog starts on its
/// first field, or on the button row when it has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogState {
    pub focus: Focus,
}

impl DialogState {
    pub fn new(fields: &[FieldKind], buttons: &[Button<'_>]) -> Self {
        Self {
            focus: if fields.is_empty() {
                Focus::Buttons(primary(buttons).unwrap_or(0))
            } else {
                Focus::Field(0)
            },
        }
    }

    /// The button Enter would choose now: the cursor's on the button row,
    /// else the enabled primary one.
    pub fn button(&self, buttons: &[Button<'_>]) -> Option<usize> {
        match self.focus {
            Focus::Buttons(index) => buttons.get(index).map(|_| index),
            Focus::Field(_) => primary(buttons),
        }
    }

    pub fn key(
        &mut self,
        kind: DialogKind,
        key: &KeyEvent,
        fields: &[FieldKind],
        buttons: &[Button<'_>],
    ) -> DialogInput {
        if key.kind == KeyEventKind::Release
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return DialogInput::Ignored;
        }
        let text = matches!(self.focus, Focus::Field(i) if fields.get(i) == Some(&FieldKind::Text));
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => DialogInput::Ignored,
            KeyCode::Esc => DialogInput::Cancel,
            KeyCode::Char('q') if !text => DialogInput::Cancel,
            _ if matches!(kind, DialogKind::List | DialogKind::ReadOnly) => {
                self.content_key(kind, key)
            }
            KeyCode::Up | KeyCode::Down => self.step(key.code == KeyCode::Up, fields, buttons),
            KeyCode::Enter => self
                .button(buttons)
                .filter(|index| buttons[*index].is_enabled())
                .map_or(DialogInput::Ignored, DialogInput::Choose),
            KeyCode::Left | KeyCode::Right => match self.focus {
                Focus::Field(index) => DialogInput::Field(index, *key),
                Focus::Buttons(index) => {
                    let next = enabled_step(buttons, index, key.code == KeyCode::Left);
                    self.moved(Focus::Buttons(next))
                }
            },
            _ if text => match self.focus {
                Focus::Field(index) => DialogInput::Field(index, *key),
                Focus::Buttons(_) => DialogInput::Ignored,
            },
            _ => DialogInput::Ignored,
        }
    }

    fn content_key(&self, kind: DialogKind, key: &KeyEvent) -> DialogInput {
        match key.code {
            KeyCode::Enter if kind == DialogKind::ReadOnly => DialogInput::Ignored,
            KeyCode::Char(_)
            | KeyCode::Enter
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End => DialogInput::Content(*key),
            _ => DialogInput::Ignored,
        }
    }

    /// ↑↓ walk the fields, then the button row as one stop.
    fn step(&mut self, up: bool, fields: &[FieldKind], buttons: &[Button<'_>]) -> DialogInput {
        let stops = fields.len() + usize::from(!buttons.is_empty());
        let at = match self.focus {
            Focus::Field(index) => index,
            Focus::Buttons(_) => fields.len(),
        };
        let next = if up {
            at.checked_sub(1)
        } else {
            Some(at + 1).filter(|next| *next < stops)
        };
        match next {
            Some(index) if index < fields.len() => self.moved(Focus::Field(index)),
            Some(_) => self.moved(Focus::Buttons(primary(buttons).unwrap_or(0))),
            None => DialogInput::Ignored,
        }
    }

    fn moved(&mut self, focus: Focus) -> DialogInput {
        if focus == self.focus {
            return DialogInput::Ignored;
        }
        self.focus = focus;
        DialogInput::Moved
    }

    /// A press on an enabled button chooses it, a press on a field focuses it
    /// and a press outside the dialog cancels it. Other events are the
    /// application's (wheel, rows, hover).
    pub fn mouse(
        &mut self,
        event: &MouseEvent,
        hits: &DialogHits,
        buttons: &[Button<'_>],
    ) -> DialogInput {
        if !matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
            return DialogInput::Ignored;
        }
        let point = Position::new(event.column, event.row);
        if !hits.outer.contains(point) {
            return DialogInput::Cancel;
        }
        if let Some(index) = hits
            .buttons
            .iter()
            .position(|rect| rect.is_some_and(|rect| rect.contains(point)))
        {
            return if buttons.get(index).is_some_and(Button::is_enabled) {
                self.focus = Focus::Buttons(index);
                DialogInput::Choose(index)
            } else {
                DialogInput::Ignored
            };
        }
        match hits.fields.iter().position(|rect| rect.contains(point)) {
            Some(index) => self.moved(Focus::Field(index)),
            None => DialogInput::Ignored,
        }
    }
}

fn primary(buttons: &[Button<'_>]) -> Option<usize> {
    buttons
        .iter()
        .position(|button| button.is_focused() && button.is_enabled())
}

/// The next enabled button to the left or right, staying put at either end.
fn enabled_step(buttons: &[Button<'_>], from: usize, left: bool) -> usize {
    let enabled = |index: &usize| buttons[*index].is_enabled();
    if left {
        (0..from).rev().find(enabled)
    } else {
        (from + 1..buttons.len()).find(enabled)
    }
    .unwrap_or(from)
}

#[cfg(test)]
mod tests;
