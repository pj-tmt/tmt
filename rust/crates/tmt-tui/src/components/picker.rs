//! Query editing and neutral picker outcomes. Filtering, preview, save and
//! cancellation rollback are application-owned; Ctrl-C stays in app::route.
use super::{ListEvent, ListFrame, ListRow, ListState};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use tmt_cli_style::table::escape;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const MAX_QUERY_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerEvent {
    QueryChanged(String),
    Changed(String),
    Confirm(String),
    Cancel,
}

/// A handled input need not produce an application effect. Cursor motion,
/// navigation boundaries and bounded edits still consume their key in a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerInput {
    Event(PickerEvent),
    Captured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerField {
    Query,
    List,
}

#[derive(Debug, Default)]
pub struct Picker {
    pub list: ListState,
    query: Option<String>,
    cursor: usize,
}
impl Picker {
    /// None makes a selection-only picker; Some enables an editable query.
    pub fn new(query: Option<String>) -> Result<Self, String> {
        if query
            .as_ref()
            .is_some_and(|query| query.len() > MAX_QUERY_BYTES)
        {
            return Err("picker query exceeds 4096 bytes".into());
        }
        Ok(Self {
            cursor: query.as_ref().map_or(0, String::len),
            query,
            list: ListState::default(),
        })
    }
    /// After QueryChanged, the consumer filters its own projected data and
    /// reconciles synchronously. This reports the new preview identity without
    /// acquiring data or applying a preview in the component.
    pub fn reconcile(&mut self, rows: Vec<ListRow>) -> Result<Option<PickerEvent>, String> {
        let previous = self.list.selected().map(str::to_owned);
        self.list.reconcile(rows)?;
        Ok(self
            .list
            .selected()
            .filter(|id| Some(*id) != previous.as_deref())
            .map(|id| PickerEvent::Changed(id.into())))
    }
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }
    /// Bind this display value to the query slot. Escaping and grapheme fitting
    /// still belong to the existing text owner, including the cursor mark.
    pub fn query_display(&self) -> Option<String> {
        self.query
            .as_ref()
            .map(|query| format!("{}▏{}", &query[..self.cursor], &query[self.cursor..]))
    }

    /// Bind a horizontal query window when the input is narrower than its text.
    /// The cursor always remains visible; data is returned raw for text::lines
    /// to escape once. Escaped display widths determine the window boundaries.
    pub fn query_visible(&self, width: u16) -> Option<String> {
        let query = self.query.as_ref()?;
        if width == 0 {
            return Some(String::new());
        }
        let mut budget = usize::from(width).saturating_sub(1);
        let mut start = self.cursor;
        for (index, part) in query[..self.cursor].grapheme_indices(true).rev() {
            let cells = escape(part).width();
            if cells > budget {
                break;
            }
            start = index;
            budget -= cells;
        }
        let mut end = self.cursor;
        for part in query[self.cursor..].graphemes(true) {
            let cells = escape(part).width();
            if cells > budget {
                break;
            }
            end += part.len();
            budget -= cells;
        }
        Some(format!(
            "{}▏{}",
            &query[start..self.cursor],
            &query[self.cursor..end]
        ))
    }
    /// Convenience for a filter picker whose query remains focused. Consumers
    /// with Tab-separated fields use input_field and their shared FocusStack.
    pub fn input(&mut self, event: &Event, frame: &ListFrame) -> Option<PickerInput> {
        let field = if self.query.is_some() {
            PickerField::Query
        } else {
            PickerField::List
        };
        self.input_field(event, frame, field)
    }
    pub fn input_field(
        &mut self,
        event: &Event,
        frame: &ListFrame,
        field: PickerField,
    ) -> Option<PickerInput> {
        let accepted = match event {
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                matches!(
                    key.code,
                    KeyCode::Esc
                        | KeyCode::Enter
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                        | KeyCode::Home
                        | KeyCode::End
                ) || if self.query.is_some() && field == PickerField::Query {
                    matches!(
                        key.code,
                        KeyCode::Char(_)
                            | KeyCode::Backspace
                            | KeyCode::Delete
                            | KeyCode::Left
                            | KeyCode::Right
                    )
                } else {
                    matches!(key.code, KeyCode::Char('q' | 'j' | 'k' | 'g' | 'G'))
                }
            }
            Event::Paste(_) => self.query.is_some() && field == PickerField::Query,
            Event::Mouse(mouse) => {
                frame.viewport.contains((mouse.column, mouse.row).into())
                    && matches!(
                        mouse.kind,
                        MouseEventKind::Down(MouseButton::Left)
                            | MouseEventKind::ScrollUp
                            | MouseEventKind::ScrollDown
                    )
            }
            _ => false,
        };
        if !accepted {
            return None;
        }
        Some(
            self.event_field(event, frame, field)
                .map_or(PickerInput::Captured, PickerInput::Event),
        )
    }
    fn event_field(
        &mut self,
        event: &Event,
        frame: &ListFrame,
        field: PickerField,
    ) -> Option<PickerEvent> {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release
                || key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                return None;
            }
            if key.code == KeyCode::Esc
                || ((self.query.is_none() || field == PickerField::List)
                    && key.code == KeyCode::Char('q'))
            {
                return Some(PickerEvent::Cancel);
            }
            if self.query.is_some() && field == PickerField::Query {
                match key.code {
                    KeyCode::Char(ch) => return self.edit(&ch.to_string()),
                    KeyCode::Backspace
                    | KeyCode::Delete
                    | KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Home
                    | KeyCode::End => {
                        let query = self.query.as_mut().expect("editable query");
                        let previous = query[..self.cursor]
                            .grapheme_indices(true)
                            .next_back()
                            .map_or(0, |(index, _)| index);
                        let next = query[self.cursor..]
                            .graphemes(true)
                            .next()
                            .map_or(self.cursor, |part| self.cursor + part.len());
                        match key.code {
                            KeyCode::Left => self.cursor = previous,
                            KeyCode::Right => self.cursor = next,
                            KeyCode::Home => self.cursor = 0,
                            KeyCode::End => self.cursor = query.len(),
                            KeyCode::Backspace if previous < self.cursor => {
                                query.replace_range(previous..self.cursor, "");
                                self.cursor = cursor_boundary(query, previous);
                                return Some(PickerEvent::QueryChanged(query.clone()));
                            }
                            KeyCode::Delete if next > self.cursor => {
                                query.replace_range(self.cursor..next, "");
                                self.cursor = cursor_boundary(query, self.cursor);
                                return Some(PickerEvent::QueryChanged(query.clone()));
                            }
                            _ => {}
                        }
                        return None;
                    }
                    _ => {}
                }
            }
        }
        if let Event::Paste(value) = event {
            return if field == PickerField::Query {
                self.edit(value)
            } else {
                None
            };
        }
        self.list.input(event, frame).map(|event| match event {
            ListEvent::Changed(id) => PickerEvent::Changed(id),
            ListEvent::Confirm(id) => PickerEvent::Confirm(id),
        })
    }
    fn edit(&mut self, value: &str) -> Option<PickerEvent> {
        let query = self.query.as_mut()?;
        if value.is_empty() || query.len().saturating_add(value.len()) > MAX_QUERY_BYTES {
            return None;
        }
        query.insert_str(self.cursor, value);
        self.cursor += value.len();
        // Combining input can join adjacent graphemes. Keep the edit cursor at
        // the next valid boundary instead of splitting a grapheme on deletion.
        self.cursor = cursor_boundary(query, self.cursor);
        Some(PickerEvent::QueryChanged(query.clone()))
    }
}

fn cursor_boundary(query: &str, cursor: usize) -> usize {
    query
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .chain(std::iter::once(query.len()))
        .find(|index| *index >= cursor)
        .unwrap_or(query.len())
}

#[cfg(test)]
mod tests;
