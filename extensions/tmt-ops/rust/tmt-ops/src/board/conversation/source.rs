//! Inline exact-source selection. Markdown geometry only opens the source view;
//! byte selection belongs exclusively to a newly painted raw Editor layout.
use super::{App, ConversationKey, RowTarget, Session, State, Target};
use crate::{
    board::{
        app::Effect,
        home_leads::MessageKey,
        response::{Attachment, Quote, Selection},
    },
    config::Pane,
    look::Look,
};
use ratatui::{
    Frame,
    crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    layout::Rect,
    text::Line,
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

pub(in crate::board) mod task;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Origin {
    Response(MessageKey),
    Task(task::Key),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    Byte(usize),
    Bottom,
}

#[derive(Clone)]
pub(super) struct ReplyHit {
    pub area: Rect,
    pub target: RowTarget,
    pub key: MessageKey,
    pub sequence: Option<u64>,
    pub offset: usize,
}

pub(super) struct Source {
    target: Target,
    selection: Selection<Origin>,
    anchor: Anchor,
    // Text wrapping context, never pointer authority. Relative key bursts can
    // advance before another paint while painted hit geometry is already gone.
    layout_width: Option<u16>,
    painted: Option<(Rect, usize)>,
    quote_button: Option<(Rect, Quote<Origin>)>,
    copy_pending: bool,
}
impl Source {
    pub(super) fn desired_height(&self, width: u16) -> usize {
        // Raw text plus its existing selection footer; caller owns the header.
        self.selection
            .source
            .layout(width)
            .lines
            .len()
            .saturating_add(1)
    }

    pub(super) fn label(&self) -> &'static str {
        match self.selection.key {
            Origin::Response(_) => "Exact response source",
            Origin::Task(_) => "Exact task field",
        }
    }

    pub(super) fn scroll(&mut self, offset: usize, step: super::Step, visible: bool) {
        match step {
            super::Step::Top => self.anchor = Anchor::Byte(0),
            super::Step::Bottom => self.anchor = Anchor::Bottom,
            super::Step::Lines(0) | super::Step::Pages(0) => {}
            _ if visible => {
                if let Some(width) = self.layout_width {
                    let layout = self.selection.source.layout(width);
                    let mut byte = layout
                        .lines
                        .get(offset.min(layout.lines.len().saturating_sub(1)))
                        .map_or(0, |line| line.start)
                        .min(self.selection.source.len());
                    while !self.selection.source.is_char_boundary(byte) {
                        byte = byte.saturating_sub(1);
                    }
                    self.anchor = Anchor::Byte(byte);
                }
            }
            _ => {}
        }
        self.invalidate();
    }

    pub(super) fn invalidate(&mut self) {
        self.painted = None;
        self.quote_button = None;
    }
}

impl State {
    pub(in crate::board) fn invalidate_source_geometry(&mut self) {
        self.reply_hits.clear();
        self.response_area = None;
        self.source_drag = None;
        self.task_hits.clear();
        self.task_drag = None;
        for session in self.sessions.values_mut() {
            if let Some(source) = &mut session.exact_source {
                source.copy_pending = false;
                source.invalidate();
            }
        }
    }
}

impl Session {
    pub(super) fn response_key_at(&self, line: usize) -> Option<(&MessageKey, Option<u64>)> {
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
    pub(in crate::board) fn selected_response_quote(&self) -> Option<(Target, Quote)> {
        let (key, _) = self.receiving_conversation()?;
        let state = self.conversation.borrow();
        let source = state.sessions.get(&key)?.exact_source.as_ref()?;
        let quote = source.selection.quote()?;
        let Origin::Response(key) = quote.key else {
            return None;
        };
        Some((
            source.target.clone(),
            Quote {
                key,
                range: quote.range,
                text: quote.text,
            },
        ))
    }

    fn source_quote_current(&self, target: &Target, quote: &Quote<Origin>) -> bool {
        match &quote.key {
            Origin::Response(key) => {
                self.response_source_current(target, key, &quote.range, &quote.text)
            }
            Origin::Task(_) => self.task_quote_current(target, quote),
        }
    }

    fn receiving_conversation(&self) -> Option<(ConversationKey, RowTarget)> {
        let target = self.conversation_target()?;
        let id = self.target_row(&target)?["id"].as_str()?.to_owned();
        Some(((self.view.as_ref()?.me_id.clone(), id), target))
    }

    /// This port owns only one Esc level, after all higher input/overlay owners.
    pub(in crate::board) fn conversation_source_escape(&mut self) -> bool {
        if self.focused() != Pane::Replies {
            return false;
        }
        let Some((key, _)) = self.receiving_conversation() else {
            return false;
        };
        let mut state = self.conversation.borrow_mut();
        let Some(session) = state.sessions.get_mut(&key) else {
            return false;
        };
        if session.exact_source.take().is_none() {
            return false;
        }
        // The normal response painter restores its retained source anchor.
        session.rendered = None;
        drop(state);
        self.invalidate_frame_geometry();
        true
    }

    fn insert_selected_source(
        &mut self,
        key: ConversationKey,
        target: Target,
        quote: Quote<Origin>,
    ) -> Effect {
        let result = match quote.key {
            Origin::Response(key) => self.insert_response_quote(
                target,
                Quote {
                    key,
                    range: quote.range,
                    text: quote.text,
                },
            ),
            Origin::Task(_) => {
                if !self.task_quote_current(&target, &quote) {
                    Attachment::Refused("The task field changed; quote not inserted.")
                } else if let Target::Row(target) = &target {
                    self.insert_chat_quote(target, &quote.text)
                } else {
                    Attachment::Unavailable
                }
            }
        };
        if matches!(result, Attachment::Composer(_))
            && let Some(session) = self.conversation.borrow_mut().sessions.get_mut(&key)
        {
            session.exact_source = None;
            session.rendered = None;
        }
        self.invalidate_frame_geometry();
        result.finish(self)
    }

    /// Read-only source keys run after editor/menu/member-link owners. Relative
    /// keys reuse text layout; they grant no authority to stale pointer cells.
    pub(in crate::board) fn conversation_source_key(&mut self, event: KeyEvent) -> Option<Effect> {
        if self.focused() != Pane::Replies
            || self.searching
            || self.workspace_snapshot().is_some() && !self.workspace_selection_visible()
        {
            return None;
        }
        let receiving = self.receiving_conversation();
        let mut state = self.conversation.borrow_mut();
        // An actor/target change invalidates the old surface. Do not replay its
        // Enter as the new actor's ordinary Talk action before a fresh paint.
        let retained_source = state
            .active
            .as_ref()
            .and_then(|key| state.sessions.get(key))
            .is_some_and(|session| session.exact_source.is_some());
        let Some((key, target)) = receiving else {
            return retained_source.then_some(Effect::None);
        };
        if state.active.as_ref() != Some(&key) {
            return retained_source.then_some(Effect::None);
        }
        let height = state.pane.map_or(1, |area| area.height);
        let source = state.sessions.get_mut(&key)?.exact_source.as_mut()?;
        if source.target != Target::Row(target) {
            return None;
        }
        if event.code == KeyCode::Enter && event.modifiers.is_empty() {
            let quote = source.selection.quote()?;
            if event.kind != KeyEventKind::Press {
                return Some(Effect::None);
            }
            let target = source.target.clone();
            drop(state);
            return Some(self.insert_selected_source(key, target, quote));
        }
        let navigation = matches!(
            event.code,
            KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
        ) || event.code == KeyCode::Char('a')
            && event.modifiers == KeyModifiers::CONTROL;
        if !navigation {
            return None;
        }
        if event.kind == KeyEventKind::Release {
            return Some(Effect::None);
        }
        let width = source.layout_width?;
        let before = source.selection.source.selection();
        source.selection.source.key(event, width, usize::MAX);
        let after = source.selection.source.selection();
        let quote = (before != after)
            .then(|| source.selection.quote())
            .flatten();
        let target = source.target.clone();
        let layout = source.selection.source.layout(width);
        let offset = layout.reveal(self.scrolls.offset(Pane::Replies), height);
        source.anchor = Anchor::Byte(layout.lines[offset].start);
        source.copy_pending = false;
        source.invalidate();
        drop(state);
        Some(
            quote
                .filter(|quote| self.source_quote_current(&target, quote))
                .map_or(Effect::None, |quote| {
                    Effect::Act(crate::board::app::Request::Copy {
                        text: quote.text,
                        program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
                        feedback: crate::board::app::CopyFeedback::Selection,
                    })
                }),
        )
    }

    /// Dismiss selection without discarding the clicked outside control's own
    /// still-current hit map. Higher modal/menu owners have already consumed.
    pub(in crate::board) fn conversation_source_outside_press(&mut self, event: MouseEvent) {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        // A new press cancels any earlier pending field drag, even when the
        // new control consumes it before the field owner runs again.
        self.conversation.borrow_mut().task_drag = None;
        let Some((key, target)) = self.receiving_conversation() else {
            return;
        };
        let mut state = self.conversation.borrow_mut();
        if state.active.as_ref() != Some(&key) {
            return;
        }
        let Some(session) = state.sessions.get_mut(&key) else {
            return;
        };
        let Some(source) = &session.exact_source else {
            return;
        };
        let Some((area, offset)) = source.painted else {
            return;
        };
        let point = (event.column, event.row).into();
        if offset == self.scrolls.offset(Pane::Replies)
            && source.target == Target::Row(target)
            && !area.contains(point)
            && !source
                .quote_button
                .as_ref()
                .is_some_and(|(area, _)| area.contains(point))
        {
            session.exact_source = None;
            session.rendered = None;
        }
    }

    pub(in crate::board) fn conversation_source_mouse(
        &mut self,
        event: MouseEvent,
    ) -> Option<Effect> {
        if self.searching {
            return None;
        }
        let (key, target) = self.receiving_conversation()?;
        let point = (event.column, event.row).into();
        let mut state = self.conversation.borrow_mut();
        if state.active.as_ref() != Some(&key) || !self.scrolls.visible(Pane::Replies) {
            return None;
        }
        let pane = state.pane?;
        let offset = self.scrolls.offset(Pane::Replies);
        let session = state.sessions.get_mut(&key)?;
        if let Some(source) = &mut session.exact_source {
            let (area, painted_offset) = source.painted?;
            if painted_offset != offset
                || area.intersection(pane) != area
                || source.target != Target::Row(target)
            {
                return None;
            }
            if event.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some((button, quote)) = &source.quote_button
                && button.contains(point)
                && source.selection.quote().as_ref() == Some(quote)
            {
                let quote = quote.clone();
                let target = source.target.clone();
                drop(state);
                return Some(self.insert_selected_source(key, target, quote));
            }
            if event.kind == MouseEventKind::Up(MouseButton::Left) && source.copy_pending {
                source.copy_pending = false;
                let quote = source.selection.quote()?;
                let target = source.target.clone();
                drop(state);
                if !self.source_quote_current(&target, &quote) {
                    return Some(Effect::None);
                }
                return Some(Effect::Act(crate::board::app::Request::Copy {
                    text: quote.text,
                    program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
                    feedback: crate::board::app::CopyFeedback::Selection,
                }));
            }
            if area.contains(point)
                && matches!(
                    event.kind,
                    MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Drag(MouseButton::Left)
                )
            {
                source.selection.source.place(
                    area.width,
                    offset + usize::from(event.row - area.y),
                    usize::from(event.column - area.x),
                    matches!(event.kind, MouseEventKind::Drag(_))
                        || event
                            .modifiers
                            .contains(ratatui::crossterm::event::KeyModifiers::SHIFT),
                );
                source.quote_button = None;
                source.copy_pending = source.selection.quote().is_some();
                return Some(Effect::None);
            }
            return None;
        }
        let hit = state
            .reply_hits
            .iter()
            .find(|hit| hit.area.contains(point) && hit.target == target && hit.offset == offset)
            .cloned();
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                state.source_drag = hit;
                None
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let hit = hit?;
                let start = state.source_drag.take()?;
                if (!start.key.same_source(&hit.key) || start.sequence != hit.sequence)
                    || start.target != hit.target
                    || start.offset != offset
                {
                    return None;
                }
                let session = state.sessions.get_mut(&key)?;
                let reply = session.retained_response(&hit.key, hit.sequence)?;
                session.exact_source = Some(Source {
                    target: Target::Row(target),
                    selection: Selection::new(Origin::Response(hit.key), reply.body.clone()),
                    anchor: Anchor::Byte(0),
                    layout_width: None,
                    painted: None,
                    quote_button: None,
                    copy_pending: false,
                });
                session.follow = false;
                drop(state);
                self.focus = 1;
                self.invalidate_frame_geometry();
                self.scrolls.restore([(Pane::Replies, 0)]);
                Some(Effect::None)
            }
            _ => None,
        }
    }
}

pub(super) fn paint_with_interactivity(
    frame: &mut Frame,
    app: &App,
    source: &mut Source,
    target: &RowTarget,
    body: Rect,
    look: Look,
    interactive: bool,
) {
    source.invalidate();
    let footer = (body.height > 1).then(|| Rect::new(body.x, body.bottom() - 1, body.width, 1));
    let area = Rect {
        height: body.height - u16::from(footer.is_some()),
        ..body
    };
    let layout = source.selection.source.layout(area.width);
    source.layout_width = Some(area.width);
    let offset = match source.anchor {
        Anchor::Bottom => layout.lines.len(),
        Anchor::Byte(byte) => layout
            .lines
            .iter()
            .rposition(|line| line.start <= byte)
            .unwrap_or_default(),
    };
    app.scrolls.restore([(Pane::Replies, offset)]);
    app.scrolls.show_paint(
        frame,
        Pane::Replies,
        area,
        layout.lines.len(),
        look,
        |frame, viewport, offset| {
            layout.paint(
                frame.buffer_mut(),
                viewport,
                offset,
                source.selection.source.selection(),
                look.role(Role::Text),
                look.selection(),
            );
            if interactive {
                source.painted = Some((viewport, offset));
            }
        },
    );
    if let Some(quote) = source.selection.quote().filter(|quote| {
        interactive
            && source.target == Target::Row(target.clone())
            && app.source_quote_current(&source.target, quote)
    }) && let Some((viewport, offset)) = source.painted
        && let Some(control) = super::super::response::quote_control(
            app,
            frame.buffer_mut(),
            &layout,
            quote.range.end,
            viewport,
            offset,
            None,
        )
    {
        source.quote_button = Some((control, quote));
    }
    if let Some(footer) = footer {
        strip::paint_left(
            frame.buffer_mut(),
            footer,
            Line::styled(
                if interactive {
                    "Select text to copy · Enter quotes in chat · Esc clears"
                } else {
                    "Clear the filter to quote this source."
                },
                look.role(Role::Dim),
            ),
        );
    }
}

#[cfg(test)]
mod tests;
