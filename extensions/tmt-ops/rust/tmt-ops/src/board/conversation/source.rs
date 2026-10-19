//! Selection of already rendered text. A frozen passage is literal clipboard/chat
//! content, never evidence for a request, recipient or exact row-note attachment.
use super::{App, RowTarget, Session};
use crate::board::home_leads::MessageKey;
use crate::{
    board::app::{CopyFeedback, Effect, Request},
    config::Pane,
};
use ratatui::{
    Frame,
    crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    layout::Rect,
};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::components::Editor;
use unicode_width::UnicodeWidthStr;

#[derive(Clone)]
pub(super) struct Surface {
    area: Rect,
    target: RowTarget,
    authors: Vec<String>,
    text: Editor,
}

impl Surface {
    #[cfg(test)]
    pub(super) fn text(&self) -> &str {
        self.text.as_str()
    }
}

pub(super) struct Source {
    surface: Surface,
    text: Editor,
    dragging: bool,
    control: Option<Rect>,
}
impl Source {
    fn author(&self) -> String {
        let range = self.text.selection();
        let layout = self.text.layout(self.surface.area.width);
        let names: std::collections::BTreeSet<_> = layout
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.start < range.end && line.end > range.start)
            .filter_map(|(index, _)| self.surface.authors.get(index))
            .collect();
        if names.len() == 1 {
            (*names.first().unwrap()).clone()
        } else {
            "selection".into()
        }
    }
    fn selected(&self) -> Option<String> {
        let range = self.text.selection();
        (!range.is_empty()).then(|| self.text.as_str()[range].to_owned())
    }
}

impl super::State {
    pub(in crate::board) fn invalidate_source_geometry(&mut self) {
        self.source_surfaces.clear();
        self.source_areas.clear();
        self.response_area = None;
        if let Some(source) = &mut self.selection {
            source.control = None;
        }
    }
}

fn snapshot(
    buffer: &ratatui::buffer::Buffer,
    area: Rect,
    target: RowTarget,
    authors: Vec<String>,
) -> Surface {
    let mut rows = Vec::new();
    for y in area.y..area.bottom() {
        let mut line = String::new();
        let mut x = area.x;
        while x < area.right() {
            let symbol = buffer[(x, y)].symbol();
            line.push_str(symbol);
            x = x.saturating_add(symbol.width().max(1) as u16);
        }
        rows.push(line.trim_end().to_owned());
    }
    let mut text = Editor::from(rows.join("\n"));
    text.place(area.width, 0, 0, false);
    Surface {
        area,
        target,
        authors,
        text,
    }
}

/// Publish only the painted field rectangle. Literal selection captures its
/// displayed cells after all ordinary renderers finish, not a hidden raw value.
pub(in crate::board) fn publish(app: &App, target: RowTarget, area: Rect) {
    if !area.is_empty() {
        app.conversation
            .borrow_mut()
            .source_areas
            .push((target, area));
    }
}

pub(in crate::board) fn begin_frame(app: &App) {
    let mut state = app.conversation.borrow_mut();
    state.source_areas.clear();
    state.source_surfaces.clear();
    if let Some(source) = &mut state.selection {
        source.control = None;
    }
}

fn authors(app: &App, state: &super::State, target: &RowTarget, area: Rect) -> Vec<String> {
    let name = app
        .target_row(target)
        .and_then(|row| row["name"].as_str())
        .unwrap_or("agent");
    let session = state
        .active
        .as_ref()
        .and_then(|key| state.sessions.get(key));
    (0..usize::from(area.height))
        .map(|row| {
            let Some(session) = session.filter(|_| state.response_area == Some(area)) else {
                return name.to_owned();
            };
            let Some(rendered) = session.rendered.as_ref() else {
                return name.to_owned();
            };
            let Some(line) = rendered
                .mapped
                .sources
                .get(app.scrolls.offset(Pane::Replies) + row)
                .copied()
            else {
                return name.to_owned();
            };
            if let Some((key, _)) = session.response_key_at(line) {
                return app.quote_author(&key.lead).unwrap_or(name).to_owned();
            }
            let original = rendered
                .response_sources
                .as_ref()
                .map_or(Some(line), |sources| sources.get(line).copied().flatten());
            if original
                .and_then(|line| session.authored_lines.range(..=line).next_back())
                .is_some_and(|(_, kind)| matches!(kind, super::AuthoredLine::Outgoing { .. }))
            {
                app.view
                    .as_ref()
                    .and_then(|view| view.me.clone())
                    .unwrap_or_else(|| "you".into())
            } else {
                name.to_owned()
            }
        })
        .collect()
}

/// Paint follows the ordinary conversation, without reserving rows or replacing
/// its Markdown. Only frame geometry expires; the selected literal text survives.
pub(in crate::board) fn paint(frame: &mut Frame, app: &App) {
    let mut state = app.conversation.borrow_mut();
    state.source_surfaces.clear();
    if let Some(source) = &mut state.selection {
        source.control = None;
    }
    if !app.button_owner_is_active(None)
        || app
            .input
            .as_ref()
            .is_some_and(|input| input.draft_key.is_none())
    {
        return;
    }
    let mut areas = state.source_areas.clone();
    if let Some((target, area)) = app.conversation_target().zip(state.response_area) {
        areas.push((target, area));
    }
    let surfaces: Vec<_> = areas
        .into_iter()
        .filter_map(|(target, area)| {
            let area = area.intersection(frame.area());
            if area.is_empty() {
                return None;
            }
            let authors = authors(app, &state, &target, area);
            Some(snapshot(frame.buffer_mut(), area, target, authors))
        })
        .collect();
    if let Some(source) = &mut state.selection {
        let range = source.text.selection();
        if !range.is_empty()
            && surfaces.iter().any(|surface| {
                surface.target == source.surface.target && surface.area == source.surface.area
            })
        {
            let layout = source.text.layout(source.surface.area.width);
            let mut end = None;
            let mut matches = true;
            for (row, line) in layout
                .lines
                .iter()
                .enumerate()
                .take(usize::from(source.surface.area.height))
            {
                for cell in &line.cells {
                    if cell.source.start >= range.end || cell.source.end <= range.start {
                        continue;
                    }
                    let x = source.surface.area.x + cell.column as u16;
                    let y = source.surface.area.y + row as u16;
                    if x >= source.surface.area.right()
                        || frame.buffer_mut()[(x, y)].symbol() != cell.text
                    {
                        matches = false;
                        continue;
                    }
                    for column in x..x
                        .saturating_add(cell.width as u16)
                        .min(source.surface.area.right())
                    {
                        frame.buffer_mut()[(column, y)].set_style(app.look().role(Role::Selection));
                    }
                    end = Some((x + cell.width as u16, y));
                }
            }
            if matches && let Some((x, y)) = end {
                let area = source.surface.area;
                let (x, y) = if area.right().saturating_sub(x + 1) >= 9 {
                    (x + 1, y)
                } else {
                    (area.x, y + 1)
                };
                if y < area.bottom() {
                    source.control = crate::board::buttons::paint(
                        app,
                        frame.buffer_mut(),
                        Rect::new(x, y, area.right().saturating_sub(x), 1),
                        &[crate::board::buttons::Button::secondary("+ Quote")],
                        Align::Left,
                        None,
                    )[0];
                }
            }
        }
    }
    state.source_surfaces = surfaces;
}

impl Session {
    pub(super) fn response_key_at(&self, line: usize) -> Option<(&MessageKey, Option<u64>)> {
        if self
            .rendered
            .as_ref()
            .is_some_and(|rendered| rendered.fold_controls.contains_key(&line))
        {
            return None;
        }
        if let Some((_, index)) = self.rendered.as_ref().and_then(|rendered| {
            let original = *rendered.anchor_sources.get(line)?;
            rendered
                .final_spans
                .iter()
                .find(|(range, _)| range.contains(&original))
        }) {
            return self
                .finals
                .get(*index)
                .filter(|reply| reply.acquired && reply.source)?
                .key
                .as_ref()
                .map(|key| (key, None));
        }
        let line = if let Some(sources) = self
            .rendered
            .as_ref()
            .and_then(|rendered| rendered.response_sources.as_ref())
        {
            sources.get(line).copied().flatten()?
        } else {
            line
        };
        if self.outgoing.is_empty() {
            self.reply
                .as_ref()
                .filter(|reply| reply.acquired && reply.source)?
                .key
                .as_ref()
                .map(|key| (key, None))
        } else {
            self.response_spans
                .iter()
                .find(|(_, _, range)| range.contains(&line))
                .map(|(sequence, key, _)| (key, *sequence))
        }
    }
}

impl App {
    pub(in crate::board::conversation) fn receiving_conversation(
        &self,
    ) -> Option<(super::ConversationKey, RowTarget)> {
        let target = self.conversation_target()?;
        let id = self.target_row(&target)?["id"].as_str()?.to_owned();
        Some(((self.view.as_ref()?.me_id.clone(), id), target))
    }
    pub(in crate::board) fn rendered_selection_at(&self, point: ratatui::layout::Position) -> bool {
        !self.modal_open()
            && self
                .conversation
                .borrow()
                .source_surfaces
                .iter()
                .any(|surface| surface.area.contains(point))
    }
    pub(in crate::board) fn rendered_selection_hint(&self) -> Option<&'static str> {
        if self.focused() != Pane::Replies
            || self.input.is_some()
            || self.modal_open()
            || self.searching
        {
            return None;
        }
        let state = self.conversation.borrow();
        if state.source_areas.is_empty() && state.response_area.is_none_or(|area| area.is_empty()) {
            return None;
        }
        Some(
            if state
                .selection
                .as_ref()
                .and_then(Source::selected)
                .is_some()
            {
                "select text to copy · y copy · Enter + Quote · Esc clear"
            } else {
                "select text to copy · Shift+arrows select · Esc back"
            },
        )
    }
    pub(in crate::board) fn conversation_source_escape(&mut self) -> bool {
        self.conversation.borrow_mut().selection.take().is_some()
    }
    pub(in crate::board) fn conversation_source_outside_press(&mut self, event: MouseEvent) {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let point = (event.column, event.row).into();
        let mut state = self.conversation.borrow_mut();
        if state.selection.as_ref().is_some_and(|source| {
            !source.surface.area.contains(point)
                && !source.control.is_some_and(|area| area.contains(point))
        }) {
            state.selection = None;
        }
    }
    fn copy_rendered_selection(&self, text: String) -> Effect {
        Effect::Act(Request::Copy {
            text,
            program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
            feedback: CopyFeedback::Selection,
        })
    }
    pub(in crate::board) fn conversation_source_key(&mut self, event: KeyEvent) -> Option<Effect> {
        if self.focused() != Pane::Replies
            || self.searching
            || self.input.is_some()
            || event.kind == KeyEventKind::Release
        {
            return None;
        }
        let target = self.conversation_target();
        let mut state = self.conversation.borrow_mut();
        if event.code == KeyCode::Char('@') && event.modifiers.is_empty() {
            state.selection = None;
            return None;
        }
        let movement = matches!(
            event.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
        );
        if movement && event.modifiers.contains(KeyModifiers::SHIFT) {
            if state.selection.is_none() {
                let surface = state
                    .source_surfaces
                    .iter()
                    .find(|surface| Some(&surface.target) == target.as_ref())?
                    .clone();
                state.selection = Some(Source {
                    text: surface.text.clone(),
                    surface,
                    dragging: false,
                    control: None,
                });
            }
            let source = state.selection.as_mut().unwrap();
            source
                .text
                .key(event, source.surface.area.width, usize::MAX);
            source.control = None;
            return Some(Effect::None);
        }
        let source = state.selection.as_ref()?;
        if event.code == KeyCode::Esc {
            state.selection = None;
            return Some(Effect::None);
        }
        let passage = source.selected()?;
        match event.code {
            KeyCode::Char('y') if event.modifiers.is_empty() => {
                drop(state);
                Some(self.copy_rendered_selection(passage))
            }
            KeyCode::Enter if event.modifiers.is_empty() => {
                let author = source.author();
                let target = source.surface.target.clone();
                state.selection = None;
                drop(state);
                Some(self.quote_rendered(target, author, passage))
            }
            _ => None,
        }
    }
    pub(in crate::board) fn conversation_quote_mouse(
        &mut self,
        event: MouseEvent,
    ) -> Option<Effect> {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        let point = (event.column, event.row).into();
        let mut state = self.conversation.borrow_mut();
        let source = state
            .selection
            .as_ref()
            .filter(|source| source.control.is_some_and(|area| area.contains(point)))?;
        let passage = source.selected()?;
        let author = source.author();
        let target = source.surface.target.clone();
        state.selection = None;
        drop(state);
        Some(self.quote_rendered(target, author, passage))
    }
    fn quote_rendered(&mut self, target: RowTarget, author: String, passage: String) -> Effect {
        if self.conversation_target().as_ref() != Some(&target)
            && let RowTarget::Workspace(target) = &target
        {
            self.choose(crate::board::app::Choice::WorkspaceTarget(target.clone()));
        }
        if self.conversation_target().as_ref() != Some(&target) {
            return self.say("This member is no longer available; draft retained.");
        }
        self.add_chat_quote(author, passage)
    }

    pub(in crate::board) fn conversation_source_mouse(
        &mut self,
        event: MouseEvent,
    ) -> Option<Effect> {
        if self.searching {
            return None;
        }
        let point = (event.column, event.row).into();
        let mut state = self.conversation.borrow_mut();
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let surface = state
                    .source_surfaces
                    .iter()
                    .find(|surface| surface.area.contains(point))?
                    .clone();
                let mut text = surface.text.clone();
                text.place(
                    surface.area.width,
                    usize::from(event.row - surface.area.y),
                    usize::from(event.column - surface.area.x),
                    false,
                );
                state.selection = Some(Source {
                    surface,
                    text,
                    dragging: true,
                    control: None,
                });
                if let Some(active) = state.active.clone()
                    && let Some(session) = state.sessions.get_mut(&active)
                {
                    session.follow = false;
                    session.folds.focused = None;
                }
                drop(state);
                self.focus = 1;
                Some(Effect::None)
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let fresh = state.selection.as_ref().is_some_and(|source| {
                    state.source_surfaces.iter().any(|surface| {
                        surface.area == source.surface.area
                            && surface.target == source.surface.target
                            && surface.text.as_str() == source.surface.text.as_str()
                    })
                });
                if !fresh {
                    if let Some(source) = &mut state.selection {
                        source.dragging = false;
                    }
                    return Some(Effect::None);
                }
                let source = state.selection.as_mut().filter(|source| source.dragging)?;
                let area = source.surface.area;
                // A drag may end just beyond a field's final cell. Clamp its
                // caret to the frozen surface rather than losing that last glyph.
                source.text.place(
                    area.width,
                    usize::from(event.row.saturating_sub(area.y).min(area.height - 1)),
                    usize::from(event.column.saturating_sub(area.x).min(area.width)),
                    true,
                );
                source.control = None;
                Some(Effect::None)
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let source = state.selection.as_mut().filter(|source| source.dragging)?;
                source.dragging = false;
                let passage = source.selected();
                drop(state);
                Some(passage.map_or(Effect::None, |text| self.copy_rendered_selection(text)))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
