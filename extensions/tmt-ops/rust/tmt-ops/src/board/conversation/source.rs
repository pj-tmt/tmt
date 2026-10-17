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
    crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
    text::Line,
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

const ANNOTATE: &str = "Annotate selection";

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
    selection: Selection,
    anchor: Anchor,
    // Text wrapping context, never pointer authority. Relative key bursts can
    // advance before another paint while painted hit geometry is already gone.
    layout_width: Option<u16>,
    painted: Option<(Rect, usize)>,
    annotate: Option<(Rect, Quote)>,
}
impl Source {
    pub(super) fn desired_height(&self, width: u16) -> usize {
        self.selection
            .source
            .layout(width)
            .lines
            .len()
            .saturating_add(1)
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
        self.annotate = None;
    }
}

impl State {
    pub(in crate::board) fn invalidate_source_geometry(&mut self) {
        self.reply_hits.clear();
        self.response_area = None;
        self.source_drag = None;
        for session in self.sessions.values_mut() {
            if let Some(source) = &mut session.exact_source {
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
                && let Some((button, quote)) = &source.annotate
                && button.contains(point)
                && source.selection.quote().as_ref() == Some(quote)
            {
                let quote = quote.clone();
                let target = source.target.clone();
                drop(state);
                let result = self.attach_response_quote(target, quote);
                if matches!(result, Attachment::Composer(_) | Attachment::Opened { .. })
                    && let Some(session) = self.conversation.borrow_mut().sessions.get_mut(&key)
                {
                    session.exact_source = None;
                    session.rendered = None;
                }
                self.invalidate_frame_geometry();
                return Some(result.finish(self));
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
                source.annotate = None;
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
                    selection: Selection::new(hit.key, reply.body.clone()),
                    anchor: Anchor::Byte(0),
                    layout_width: None,
                    painted: None,
                    annotate: None,
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
                look.role(Role::Text)
                    .add_modifier(ratatui::style::Modifier::REVERSED),
            );
            if interactive {
                source.painted = Some((viewport, offset));
            }
        },
    );
    if let Some(footer) = footer {
        let quote = source.selection.quote().filter(|quote| {
            source.target == Target::Row(target.clone())
                && app.response_quote_current(&source.target, quote)
        });
        let control = if interactive && quote.is_some() {
            super::super::buttons::paint(
                app,
                frame.buffer_mut(),
                footer,
                &[super::super::buttons::Button::secondary(ANNOTATE)],
                tmt_cli_style::grid::Align::Left,
                None,
            )[0]
        } else {
            None
        };
        if let Some(control) = control.zip(quote) {
            source.annotate = Some(control);
        } else {
            strip::paint_left(
                frame.buffer_mut(),
                footer,
                Line::styled(
                    if interactive {
                        "Drag exact source · View reply for keyboard selection"
                    } else {
                        "Clear the filter to annotate this source."
                    },
                    look.role(Role::Dim),
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests;
