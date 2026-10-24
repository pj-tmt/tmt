//! Selection of already rendered text. A frozen passage is literal clipboard/chat
//! content, never evidence for a request, recipient or exact row-note attachment.
use super::{App, RowTarget, Session, State};
use crate::board::home_leads::MessageKey;
use crate::{
    board::app::{CopyFeedback, Effect, Request},
    config::Pane,
};
use anchored::Anchored;
use menu::{Entry, Menu};
use ratatui::{
    Frame,
    crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    layout::Rect,
};
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
    control: Option<Menu>,
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

/// What the user selected. A task field is frozen as the cells it displayed; the
/// transcript scrolls and changes under the pointer, so it is anchored to its text.
pub(super) enum Selection {
    Cells(Source),
    Text(Anchored),
}

impl Selection {
    #[cfg(test)]
    fn selected(&self) -> Option<String> {
        match self {
            Self::Cells(source) => source.selected(),
            Self::Text(anchored) => anchored.selected(),
        }
    }
    fn has_text(&self) -> bool {
        match self {
            Self::Cells(source) => source.selected().is_some(),
            Self::Text(anchored) => anchored.has_range(),
        }
    }
    fn target(&self) -> &RowTarget {
        match self {
            Self::Cells(source) => &source.surface.target,
            Self::Text(anchored) => &anchored.target,
        }
    }
    fn control(&self) -> Option<Menu> {
        match self {
            Self::Cells(source) => source.control,
            Self::Text(anchored) => anchored.control,
        }
    }
    fn clear_control(&mut self) {
        match self {
            Self::Cells(source) => source.control = None,
            Self::Text(anchored) => anchored.control = None,
        }
    }
    fn stop_drag(&mut self) {
        match self {
            Self::Cells(source) => source.dragging = false,
            Self::Text(anchored) => anchored.dragging = false,
        }
    }
    fn dragging(&self) -> bool {
        match self {
            Self::Cells(source) => source.dragging,
            Self::Text(anchored) => anchored.dragging,
        }
    }
}

impl State {
    /// The text a copy or a quote of `selection` carries: a transcript selection is the
    /// source its messages were written as (#4457), anything else the displayed text.
    fn passage(&self, selection: &Selection) -> Option<String> {
        match selection {
            Selection::Text(anchored) => self
                .sessions
                .get(&anchored.key)
                .and_then(|session| session.rendered.as_ref())
                .filter(|rendered| anchored.attached_to(rendered))
                .map_or_else(
                    || anchored.selected(),
                    |rendered| anchored.source(&rendered.copy),
                ),
            Selection::Cells(source) => source.selected(),
        }
    }
    pub(in crate::board) fn invalidate_source_geometry(&mut self) {
        self.source_surfaces.clear();
        self.source_areas.clear();
        self.response_area = None;
        if let Some(selection) = &mut self.selection {
            selection.clear_control();
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
    state.response_area = None;
    state.source_areas.clear();
    state.source_surfaces.clear();
    if let Some(selection) = &mut state.selection {
        selection.clear_control();
    }
}

/// The name the recipient goes by in the conversation.
fn recipient_name<'a>(app: &'a App, target: &RowTarget) -> &'a str {
    app.target_row(target)
        .and_then(|row| row["name"].as_str())
        .unwrap_or("agent")
}

/// Who wrote transcript line `line`; `None` is a surface that is not the transcript.
fn author_at(app: &App, state: &State, name: &str, line: Option<usize>) -> String {
    let session = state
        .active
        .as_ref()
        .and_then(|key| state.sessions.get(key));
    let Some((session, rendered, line)) = session
        .zip(line)
        .and_then(|(session, line)| Some((session, session.rendered.as_ref()?, line)))
    else {
        return name.to_owned();
    };
    let Some(line) = rendered.mapped.sources.get(line).copied() else {
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
}

fn authors(app: &App, state: &State, target: &RowTarget, area: Rect) -> Vec<String> {
    let offset = app.scrolls.offset(Pane::Replies);
    let name = recipient_name(app, target);
    (0..usize::from(area.height))
        .map(|row| {
            author_at(
                app,
                state,
                name,
                (state.response_area == Some(area)).then_some(offset + row),
            )
        })
        .collect()
}

/// What the frame displays on the transcript lines from `offset`: the rows above
/// the scroll indicator, and no more than the transcript holds.
fn displayed(surface: &Surface, content: usize, offset: usize) -> impl Iterator<Item = &str> {
    let rows = crate::board::scroll::Scrolls::viewport(surface.area, content);
    surface
        .text
        .as_str()
        .split('\n')
        .take(rows.min(content.saturating_sub(offset)))
}

/// Highlight a frozen task-field selection while the cells still show its text.
fn paint_cells(frame: &mut Frame, app: &App, source: &mut Source, surfaces: &[Surface]) {
    let range = source.text.selection();
    if range.is_empty()
        || !surfaces.iter().any(|surface| {
            surface.target == source.surface.target && surface.area == source.surface.area
        })
    {
        return;
    }
    let layout = source.text.layout(source.surface.area.width);
    let mut end = None;
    let mut first = None;
    let mut matches = true;
    let mut highlights = Vec::new();
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
            if x >= source.surface.area.right() || frame.buffer_mut()[(x, y)].symbol() != cell.text
            {
                matches = false;
                continue;
            }
            highlights.push((x, y, cell.width as u16));
            first.get_or_insert(y);
            end = Some((x + cell.width as u16, y));
        }
    }
    if matches && let Some((end, first)) = end.zip(first) {
        let style = app.look().selection();
        for (x, y, width) in highlights {
            for column in x..x.saturating_add(width).min(source.surface.area.right()) {
                frame.buffer_mut()[(column, y)].set_style(style);
            }
        }
        source.control = menu::paint(frame, app, source.surface.area, end, first);
    }
}

/// Highlight an anchored selection on the transcript lines in view, and record
/// what they display. `None` ends the selection: its text changed, was re-wrapped
/// or is gone, so it could no longer be seen.
fn paint_text(
    frame: &mut Frame,
    app: &App,
    anchored: &mut Anchored,
    surface: &Surface,
    rendered: &super::Rendered,
    offset: usize,
) -> Option<()> {
    if !anchored.resolve(rendered) {
        return None;
    }
    let area = surface.area;
    let style = app.look().selection();
    let (first, last) = anchored.lines();
    let mut end = None;
    let mut first_row = None;
    for (row, text) in displayed(surface, rendered.mapped.lines.len(), offset).enumerate() {
        let line = offset + row;
        if line < first || line > last {
            continue;
        }
        anchored.capture(line, text);
        for (column, width) in anchored.highlights(line, text) {
            let y = area.y + row as u16;
            let x = area.x + column as u16;
            for column in x..x.saturating_add(width as u16).min(area.right()) {
                frame.buffer_mut()[(column, y)].set_style(style);
            }
            first_row.get_or_insert(y);
            end = Some((x + width as u16, y));
        }
    }
    if let Some((end, first)) = end.zip(first_row) {
        anchored.control = menu::paint(frame, app, area, end, first);
    }
    Some(())
}

/// Paint follows the ordinary conversation, without reserving rows or replacing
/// its Markdown. Only frame geometry expires; the selected literal text survives.
pub(in crate::board) fn paint(frame: &mut Frame, app: &App) {
    let mut state = app.conversation.borrow_mut();
    state.source_surfaces.clear();
    if let Some(selection) = &mut state.selection {
        selection.clear_control();
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
    let state_ref = &mut *state;
    let mut ended = false;
    match &mut state_ref.selection {
        Some(Selection::Cells(source)) => paint_cells(frame, app, source, &surfaces),
        Some(Selection::Text(anchored)) => {
            let surface = surfaces.iter().find(|surface| {
                surface.target == anchored.target && Some(surface.area) == state_ref.response_area
            });
            if let Some(surface) = surface {
                let rendered = state_ref
                    .active
                    .as_ref()
                    .filter(|key| **key == anchored.key)
                    .and_then(|key| state_ref.sessions.get(key))
                    .and_then(|session| session.rendered.as_ref());
                ended = rendered.is_none_or(|rendered| {
                    paint_text(
                        frame,
                        app,
                        anchored,
                        surface,
                        rendered,
                        app.scrolls.offset(Pane::Replies),
                    )
                    .is_none()
                });
            }
        }
        None => {}
    }
    if ended {
        state.selection = None;
    }
    state.source_surfaces = surfaces;
}

impl Session {
    pub(super) fn response_key_at(&self, line: usize) -> Option<(&MessageKey, Option<u64>)> {
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
    pub(in crate::board) fn rendered_selection_at(&self, point: ratatui::layout::Position) -> bool {
        !self.modal_open()
            && self
                .conversation
                .borrow()
                .source_surfaces
                .iter()
                .any(|surface| surface.area.contains(point))
    }
    pub(in crate::board) fn rendered_selection_hint(&self, width: usize) -> Option<&'static str> {
        if self.focused() != Pane::Replies
            || self.input.is_some()
            || self.modal_open()
            || self.searching
        {
            return None;
        }
        let state = self.conversation.borrow();
        if state.screen_mode {
            return Some("l chat · ↑↓ scroll · Esc back");
        }
        if state.source_areas.is_empty() && state.response_area.is_none_or(|area| area.is_empty()) {
            return None;
        }
        Some(
            if state.selection.as_ref().is_some_and(Selection::has_text) {
                "select text to copy · y copy · q quote · Esc clear"
            } else {
                // The way to the live screen joins the guidance only when all of it fits.
                const SELECT: &str = "select text to copy · Shift+arrows select · Esc back";
                const WITH_SCREEN: &str =
                    "select text to copy · Shift+arrows select · l live screen · Esc back";
                if WITH_SCREEN.chars().count() <= width {
                    WITH_SCREEN
                } else {
                    SELECT
                }
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
        let inside = match &state.selection {
            Some(Selection::Cells(source)) => source.surface.area.contains(point),
            Some(Selection::Text(_)) => {
                state.response_area.is_some_and(|area| area.contains(point))
            }
            None => return,
        };
        if !inside
            && !state
                .selection
                .as_ref()
                .and_then(Selection::control)
                .is_some_and(|menu| menu.bounds().contains(point))
        {
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
    /// The author a quote credits: the single writer of the selected lines, else
    /// "selection".
    fn selection_author(&self, state: &State, selection: &Selection) -> String {
        let anchored = match selection {
            Selection::Cells(source) => return source.author(),
            Selection::Text(anchored) => anchored,
        };
        let (first, last) = anchored.lines();
        let name = recipient_name(self, &anchored.target);
        let names: std::collections::BTreeSet<_> = (first..=last)
            .map(|line| author_at(self, state, name, Some(line)))
            .collect();
        if names.len() == 1 {
            names.into_iter().next().unwrap()
        } else {
            "selection".into()
        }
    }
    pub(in crate::board) fn conversation_source_key(&mut self, event: KeyEvent) -> Option<Effect> {
        if self.focused() != Pane::Replies
            || self.searching
            || self.input.is_some()
            || event.kind == KeyEventKind::Release
        {
            return None;
        }
        if event.code == KeyCode::Char('@') && event.modifiers.is_empty() {
            self.conversation.borrow_mut().selection = None;
            return None;
        }
        // `l` swaps the chat and the agent's live screen, unless the user bound `l`.
        // (`v` is the factory `view-reply`.)
        if event.code == KeyCode::Char('l')
            && event.modifiers.is_empty()
            && !self.bindings().contains_key("l")
            && self.conversation_target().is_some()
        {
            return Some(self.toggle_screen());
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
        // Resolving the recipient scans the roster, so only a key that can start
        // a selection pays for it; plain scrolling keys stay O(1) here.
        let target = (movement && event.modifiers.contains(KeyModifiers::SHIFT))
            .then(|| self.conversation_target())
            .flatten();
        let mut state = self.conversation.borrow_mut();
        if movement && event.modifiers.contains(KeyModifiers::SHIFT) {
            if state.selection.is_none() {
                let surface = state
                    .source_surfaces
                    .iter()
                    .find(|surface| Some(&surface.target) == target.as_ref())?
                    .clone();
                state.selection = Some(if Some(surface.area) == state.response_area {
                    let key = state.active.clone()?;
                    let rendered = state.sessions.get(&key)?.rendered.as_ref()?;
                    let offset = self.scrolls.offset(Pane::Replies);
                    let row = surface.text.as_str().split('\n').next()?;
                    Selection::Text(Anchored::start(
                        surface.target.clone(),
                        key,
                        surface.area.width,
                        rendered,
                        (offset, 0),
                        row,
                        false,
                    ))
                } else {
                    Selection::Cells(Source {
                        text: surface.text.clone(),
                        surface,
                        dragging: false,
                        control: None,
                    })
                });
            }
            let state = &mut *state;
            match state.selection.as_mut().unwrap() {
                Selection::Cells(source) => {
                    source
                        .text
                        .key(event, source.surface.area.width, usize::MAX);
                    source.control = None;
                }
                Selection::Text(anchored) => {
                    let rendered = state
                        .active
                        .as_ref()
                        .filter(|key| **key == anchored.key)
                        .and_then(|key| state.sessions.get(key))
                        .and_then(|session| session.rendered.as_ref());
                    let rendered = rendered?;
                    let surface = state
                        .source_surfaces
                        .iter()
                        .find(|surface| Some(surface.area) == state.response_area)?;
                    let offset = self.scrolls.offset(Pane::Replies);
                    let painted: Vec<_> =
                        displayed(surface, rendered.mapped.lines.len(), offset).collect();
                    anchored.key(event, rendered, offset, &painted);
                }
            }
            return Some(Effect::None);
        }
        let selection = state.selection.as_ref()?;
        if event.code == KeyCode::Esc {
            state.selection = None;
            return Some(Effect::None);
        }
        let passage = state.passage(selection)?;
        let entry = match event.code {
            KeyCode::Char('y') if event.modifiers.is_empty() => Entry::Copy,
            KeyCode::Char('n') if event.modifiers.is_empty() => Entry::Note,
            KeyCode::Char('q') | KeyCode::Enter if event.modifiers.is_empty() => Entry::Quote,
            _ => return None,
        };
        drop(state);
        Some(self.run_selection_entry(entry, passage))
    }
    /// A press on an entry of the Selection menu runs it, as its key does.
    pub(in crate::board) fn conversation_menu_mouse(
        &mut self,
        event: MouseEvent,
    ) -> Option<Effect> {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        let point = (event.column, event.row).into();
        let state = self.conversation.borrow();
        let selection = state.selection.as_ref()?;
        let menu = selection.control()?;
        // The border and the hint belong to the box: they neither act nor start a selection.
        let Some(entry) = menu.entry_at(point) else {
            return menu.bounds().contains(point).then_some(Effect::None);
        };
        let passage = state.passage(selection)?;
        drop(state);
        Some(self.run_selection_entry(entry, passage))
    }
    /// Quote, copy or add the selection to Notes: keys and pointer share these entries.
    fn run_selection_entry(&mut self, entry: Entry, passage: String) -> Effect {
        match entry {
            Entry::Note => {
                let state = self.conversation.borrow();
                let Some(selection) = state.selection.as_ref() else {
                    return Effect::None;
                };
                if self.conversation_target().as_ref() != Some(selection.target()) {
                    drop(state);
                    return self.say("The conversation changed; select the text again.");
                }
                let Some(key) = self.conversation_notes_key() else {
                    drop(state);
                    return self.say("Select a squad member to add to Notes.");
                };
                let source = match selection {
                    Selection::Cells(_) => super::note_source::Source::Unavailable,
                    Selection::Text(anchored) => state
                        .sessions
                        .get(&anchored.key)
                        .and_then(|session| session.rendered.as_ref())
                        .filter(|rendered| anchored.attached_to(rendered))
                        .map_or(super::note_source::Source::Unavailable, |rendered| {
                            let (first, last) = anchored.lines();
                            rendered
                                .anchor_sources
                                .get(first..=last)
                                .map_or(super::note_source::Source::Unavailable, |lines| {
                                    rendered.note_sources.selected(lines.iter().copied())
                                })
                        }),
                };
                let seed = crate::board::conversation_notes::Seed {
                    key,
                    actor: self.view.as_ref().and_then(|view| view.me_id.clone()),
                    kind: crate::board::conversation_notes::NoteKind::Todo,
                    title: crate::board::notes::single_line(&passage),
                    reference: String::new(),
                    source,
                };
                drop(state);
                let effect = self.add_to_notes(seed);
                if self.notes_overlay.is_some() {
                    self.conversation.borrow_mut().selection = None;
                }
                effect
            }
            Entry::Copy => self.copy_rendered_selection(passage),
            Entry::Quote => {
                let mut state = self.conversation.borrow_mut();
                let Some(selection) = state.selection.as_ref() else {
                    return Effect::None;
                };
                let author = self.selection_author(&state, selection);
                let target = selection.target().clone();
                state.selection = None;
                drop(state);
                self.quote_rendered(target, author, passage)
            }
        }
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
                let selection = if Some(surface.area) == state.response_area {
                    let key = state.active.clone()?;
                    let rendered = state.sessions.get(&key)?.rendered.as_ref()?;
                    let area = surface.area;
                    let row = usize::from(event.row - area.y);
                    let offset = self.scrolls.offset(Pane::Replies);
                    let line = offset + row;
                    let text = displayed(&surface, rendered.mapped.lines.len(), offset).nth(row)?;
                    Selection::Text(Anchored::start(
                        surface.target.clone(),
                        key,
                        area.width,
                        rendered,
                        (line, usize::from(event.column - area.x)),
                        text,
                        true,
                    ))
                } else {
                    let mut text = surface.text.clone();
                    text.place(
                        surface.area.width,
                        usize::from(event.row - surface.area.y),
                        usize::from(event.column - surface.area.x),
                        false,
                    );
                    Selection::Cells(Source {
                        surface,
                        text,
                        dragging: true,
                        control: None,
                    })
                };
                state.selection = Some(selection);
                if let Some(active) = state.active.clone()
                    && let Some(session) = state.sessions.get_mut(&active)
                {
                    session.follow = false;
                }
                drop(state);
                self.focus = 1;
                Some(Effect::None)
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if matches!(&state.selection, Some(Selection::Text(_))) {
                    drop(state);
                    return self.extend_text_selection(event);
                }
                let fresh = matches!(&state.selection, Some(Selection::Cells(source))
                if state.source_surfaces.iter().any(|surface| {
                    surface.area == source.surface.area
                        && surface.target == source.surface.target
                        && surface.text.as_str() == source.surface.text.as_str()
                }));
                if !fresh {
                    if let Some(selection) = &mut state.selection {
                        selection.stop_drag();
                    }
                    return Some(Effect::None);
                }
                let Some(Selection::Cells(source)) = &mut state.selection else {
                    return None;
                };
                if !source.dragging {
                    return None;
                }
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
                state
                    .selection
                    .as_mut()
                    .filter(|selection| selection.dragging())?
                    .stop_drag();
                let passage = state
                    .selection
                    .as_ref()
                    .and_then(|selection| state.passage(selection));
                drop(state);
                Some(passage.map_or(Effect::None, |text| self.copy_rendered_selection(text)))
            }
            _ => None,
        }
    }

    /// A drag of an anchored selection reads the pointer against the transcript
    /// as it is now, so it keeps working while history updates. At the first or
    /// last row of the pane it scrolls one line toward the pointer.
    fn extend_text_selection(&mut self, event: MouseEvent) -> Option<Effect> {
        let (area, content, rows) = {
            let mut state = self.conversation.borrow_mut();
            let state = &mut *state;
            let Some(Selection::Text(anchored)) = &mut state.selection else {
                return None;
            };
            let rendered = state
                .active
                .as_ref()
                .filter(|key| **key == anchored.key)
                .and_then(|key| state.sessions.get(key))
                .and_then(|session| session.rendered.as_ref());
            let area = state.response_area;
            let (Some(area), Some(rendered), true) = (area, rendered, anchored.dragging) else {
                anchored.dragging = false;
                return Some(Effect::None);
            };
            if !anchored.attached_to(rendered) {
                anchored.dragging = false;
                return Some(Effect::None);
            }
            let content = rendered.mapped.lines.len();
            (
                area,
                content,
                crate::board::scroll::Scrolls::viewport(area, content),
            )
        };
        let offset = self.scrolls.offset(Pane::Replies);
        if event.row <= area.y && offset > 0 {
            self.scroll_pane(Pane::Replies, crate::board::scroll::Step::Lines(-1));
        } else if usize::from(event.row.saturating_sub(area.y)) + 1 >= rows
            && offset + rows < content
        {
            self.scroll_pane(Pane::Replies, crate::board::scroll::Step::Lines(1));
        }
        let offset = self.scrolls.offset(Pane::Replies);
        let row = usize::from(event.row.saturating_sub(area.y)).min(rows.saturating_sub(1));
        let (line, column) = if offset + row < content {
            (
                offset + row,
                usize::from(event.column.saturating_sub(area.x).min(area.width)),
            )
        } else {
            (content.saturating_sub(1), usize::from(area.width))
        };
        let mut state = self.conversation.borrow_mut();
        let state = &mut *state;
        if let Some(Selection::Text(anchored)) = &mut state.selection
            && let Some(rendered) = state
                .active
                .as_ref()
                .filter(|key| **key == anchored.key)
                .and_then(|key| state.sessions.get(key))
                .and_then(|session| session.rendered.as_ref())
        {
            anchored.extend(rendered, line, column);
        }
        Some(Effect::None)
    }
}

mod anchored;
mod menu;
#[cfg(test)]
pub(in crate::board) mod scenes;
#[cfg(test)]
mod tests;
