//! Body-sized unread list and selected detail. Only the detail publishes read evidence.
use super::{Answer, Built, Overlay, Text};
use crate::board::{
    app::{App, Overlay as OverlayId},
    exchange_view,
    recent_responses::Observation,
    unread::walk::Body,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::{Dialog, DialogKind, ListFrame, Modal, Placement, RowGeometry, strip};

impl Overlay {
    fn prompt<'a>(&'a self, actor: &str, answer: &Answer) -> Option<&'a str> {
        use crate::board::recent_responses::timeline::Kind;
        let Observation::Observed(page) = self.pages.get(&answer.member)? else {
            return None;
        };
        page.timeline
            .as_ref()?
            .entries
            .iter()
            .find_map(|entry| match &entry.kind {
                Kind::Input(input)
                    if input.request == answer.request
                        && input.sender.identity.as_deref() == Some(actor) =>
                {
                    input.body.as_ref().map(|body| body.text.as_str())
                }
                _ => None,
            })
    }

    fn build_detail(&self, app: &App, actor: &str, width: usize, look: crate::look::Look) -> Built {
        let mut lines = Vec::new();
        let now = crate::status::now_ms();
        let Some(answer) = self.selected_answer() else {
            return Built {
                width,
                look,
                version: self.version.get(),
                minute: now / 60_000,
                lines,
                body: 0..0,
            };
        };
        let name = app.member_label(&answer.member);
        let age = crate::board::conversation::exchange::age_stamp(&crate::requests::age(
            now,
            answer.answered_at_ms,
        ));
        let clock = i64::try_from(answer.answered_at_ms)
            .ok()
            .zip(i64::try_from(now).ok())
            .zip(app.local_zone.as_ref())
            .and_then(|((at, now), zone)| crate::board::local_time::time_in(at, now, zone));
        let stamp = clock.map_or_else(
            || format!("in turn · {age}"),
            |clock| format!("in turn · {clock} · {age}"),
        );
        lines.push(exchange_view::header(
            exchange_view::Sender::Agent(&name),
            "you",
            &stamp,
            false,
            width,
            look,
        ));
        let prompt = self
            .prompt(actor, answer)
            .map_or_else(|| "request text unavailable".to_owned(), first_line);
        lines.push(Line::styled(format!(" re: {prompt}"), look.role(Role::Dim)));
        lines.push(Line::default());
        let start = lines.len();
        let text = self.text(actor, answer);
        let evidence = matches!(text, Text::Found { .. } | Text::Missing | Text::Unavailable);
        match text {
            Text::Found { text, truncated } => {
                let text = crate::board::notes::sanitize(text);
                let mapped = crate::board::markdown::render_conversation_with_members(
                    &text,
                    width.saturating_sub(3).max(1),
                    look,
                    Some(&app.references),
                );
                lines.extend(
                    mapped
                        .lines
                        .into_iter()
                        .map(|line| exchange_view::body(line, look)),
                );
                if truncated {
                    lines.push(caption(
                        "◌ answer truncated · full text in the agent's pane",
                        look,
                    ));
                }
            }
            Text::Waiting => lines.push(caption("Loading answers…", look)),
            Text::Failed => lines.push(caption("✗ Answers unavailable · r retry", look)),
            Text::Missing => lines.push(caption("○ answer not loaded · open it in chat", look)),
            Text::Unavailable => lines.push(caption("○ reply text unavailable", look)),
        }
        let end = lines.len();
        Built {
            width,
            look,
            version: self.version.get(),
            minute: now / 60_000,
            lines,
            body: if evidence { start..end } else { 0..0 },
        }
    }

    fn render_list(&self, frame: &mut Frame, app: &App, actor: &str, area: Rect) {
        let look = app.look();
        let now = crate::status::now_ms();
        let ordered = self.ordered();
        let selected = self.selected_index();
        let mut current = None;
        let mut lines = Vec::new();
        let mut positions = Vec::new();
        for (index, answer) in ordered.iter().enumerate() {
            if current != Some(answer.member.as_str()) {
                if current.is_some() {
                    lines.push((None, Line::default()));
                }
                current = Some(answer.member.as_str());
                let count = ordered
                    .iter()
                    .filter(|other| other.member == answer.member)
                    .count();
                lines.push((
                    None,
                    Line::styled(
                        format!("── {} · {count}", app.member_label(&answer.member)),
                        look.role(Role::Muted),
                    ),
                ));
            }
            let preview = match self.text(actor, answer) {
                Text::Found { text, .. } => first_line(text),
                Text::Waiting => "Loading answers…".into(),
                Text::Failed => "✗ Answers unavailable · r retry".into(),
                Text::Missing => "○ answer not loaded".into(),
                Text::Unavailable => "○ reply text unavailable".into(),
            };
            let read = !app.answer_unread(actor, answer);
            let mark = if read { "○ Read" } else { "● Unread" };
            let age = crate::requests::age(now, answer.answered_at_ms);
            let chosen = selected == Some(index);
            let highlight = chosen || self.hover.get() == Some(index);
            let background = if highlight {
                look.selection()
            } else {
                look.role(Role::Text)
            };
            let row_background = Style {
                fg: None,
                ..background
            };
            let mut line = Line::from(vec![
                Span::styled(
                    if chosen { "▸ " } else { "  " },
                    look.role(Role::Accent).patch(row_background),
                ),
                Span::styled(
                    format!("{mark:<8} "),
                    look.role(if read { Role::Dim } else { Role::Unread })
                        .patch(row_background),
                ),
                Span::styled(
                    format!("{age} · "),
                    look.role(Role::Dim).patch(row_background),
                ),
            ]);
            let preview = tmt_cli_style::grid::fit(
                &preview,
                usize::from(area.width).saturating_sub(line.width()),
                tmt_cli_style::grid::Align::Left,
                tmt_cli_style::grid::Truncate::End,
            );
            line.spans.push(Span::styled(
                preview,
                look.role(Role::Text).patch(row_background),
            ));
            let pad = usize::from(area.width).saturating_sub(line.width());
            line.spans.push(Span::styled(" ".repeat(pad), background));
            positions.push((index, lines.len()));
            lines.push((Some(index), line));
        }
        if lines.is_empty() {
            let label = if self.page_failed {
                "✗ Older answers unavailable · r retry"
            } else if self.more {
                "Loading older answers…"
            } else {
                "No unread"
            };
            lines.push((None, Line::styled(label, look.role(Role::Dim))));
        }
        let selected_range = positions
            .iter()
            .find(|(index, _)| Some(*index) == selected)
            .map(|(_, at)| *at..*at + 1);
        let mut list = self.list.borrow_mut();
        list.scroll.update(area, lines.len(), selected_range);
        let offset = list.scroll.offset();
        for (row, (_, line)) in lines
            .into_iter()
            .skip(offset)
            .take(usize::from(area.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(area.x, area.y + row as u16, area.width, 1),
                line,
            );
        }
        let geometry = positions
            .into_iter()
            .map(|(index, at)| RowGeometry {
                id: format!("answer-{index}"),
                lines: at..at + 1,
                visible: if at >= offset && at < offset + usize::from(area.height) {
                    Rect::new(area.x, area.y + (at - offset) as u16, area.width, 1)
                } else {
                    Rect::default()
                },
            })
            .collect();
        *self.list_frame.borrow_mut() = Some(ListFrame {
            rows: list.rows().to_vec(),
            geometry,
            viewport: area,
            offset,
        });
    }

    fn render_detail(&self, frame: &mut Frame, app: &App, actor: &str, area: Rect) {
        let look = app.look();
        let width = usize::from(area.width);
        let stale = self.built.borrow().as_ref().is_none_or(|built| {
            built.width != width
                || built.look != look
                || built.version != self.version.get()
                || built.minute != crate::status::now_ms() / 60_000
        });
        if stale {
            *self.built.borrow_mut() = Some(self.build_detail(app, actor, width, look));
        }
        let built = self.built.borrow();
        let built = built.as_ref().expect("built above");
        let mut scroll = self.scroll.borrow_mut();
        scroll.update(area, built.lines.len(), None);
        let offset = scroll.offset();
        let end = offset + usize::from(area.height);
        for (row, line) in built
            .lines
            .iter()
            .skip(offset)
            .take(usize::from(area.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(area.x, area.y + row as u16, area.width, 1),
                line.clone(),
            );
        }
        if built.body.start < end
            && built.body.end > offset
            && !area.is_empty()
            && let Some(answer) = self.selected_answer()
            && let Some(kind) = answer.kind
        {
            self.painted.borrow_mut().push(Body {
                actor: actor.into(),
                member: answer.member.clone(),
                request: answer.request.clone(),
                kind,
                at_ms: Some(answer.answered_at_ms),
                revision: Some(answer.revision),
            });
        }
        if let Some(answer) = self.selected_answer() {
            self.visible.borrow_mut().insert(answer.member.clone());
        }
    }

    fn render_confirmation(&self, frame: &mut Frame, app: &App, body: Rect) {
        let Some(confirmation) = &self.confirmation else {
            return;
        };
        let look = app.look();
        let mut dialog = Dialog {
            title: "Mark all read".into(),
            kind: DialogKind::Confirm,
            placement: Placement::Center,
            buttons: super::confirm_buttons(true).into(),
        };
        let areas = dialog.areas(body, [70, 7], false);
        app.peek_areas.set(OverlayId::Unread, Some(areas.outer));
        let ready = areas.content.width >= 30 && areas.content.height >= 3;
        if !ready {
            dialog.buttons.clear();
        }
        let hits = crate::board::buttons::paint_dialog(
            app,
            frame.buffer_mut(),
            &dialog,
            areas,
            &self.confirm_state,
            OverlayId::Unread,
        );
        *self.confirm_hits.borrow_mut() = hits;
        if !ready {
            strip::paint_left(
                frame.buffer_mut(),
                areas.footer,
                Line::styled("Enlarge terminal · Esc cancels", look.role(Role::Dim)),
            );
            return;
        }
        let count = confirmation.members().len();
        for (row, text) in [
            "Mark all unread answers read?".to_owned(),
            format!("Scope: {count} agent{}", if count == 1 { "" } else { "s" }),
            "Includes older answers.".into(),
        ]
        .into_iter()
        .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + row as u16,
                    areas.content.width,
                    1,
                ),
                Line::styled(text, look.role(Role::Text)),
            );
        }
    }

    pub fn render(&self, frame: &mut Frame, app: &App, body: Rect) {
        self.painted.borrow_mut().clear();
        self.visible.borrow_mut().clear();
        self.list_frame.borrow_mut().take();
        let Some(actor) = app.view.as_ref().and_then(|view| view.me_id.as_deref()) else {
            return;
        };
        if self.actor.as_deref() != Some(actor) {
            return;
        }
        let look = app.look();
        let narrow = body.width < 100;
        self.narrow.set(narrow);
        let modal = Modal {
            title: self.title(&|member| app.member_label(member)),
            placement: Placement::Body,
        };
        let areas = modal.areas(body, [body.width, body.height], true, true);
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        app.peek_areas.set(OverlayId::Unread, Some(areas.outer));
        if narrow && self.detail_open {
            self.render_detail(frame, app, actor, areas.content);
        } else {
            let list_width = if narrow {
                areas.content.width
            } else {
                ((u32::from(body.width) * 2 / 5) as u16)
                    .min(58)
                    .min(areas.content.width)
            };
            let list = Rect {
                width: list_width,
                ..areas.content
            };
            self.render_list(frame, app, actor, list);
            if !narrow {
                let right = Rect::new(
                    list.right().saturating_add(1),
                    areas.content.y,
                    areas.content.width.saturating_sub(list_width + 1),
                    areas.content.height,
                );
                for row in 0..areas.content.height {
                    strip::paint_left(
                        frame.buffer_mut(),
                        Rect::new(list.right(), areas.content.y + row, 1, 1),
                        Line::styled("│", look.role(Role::Dim)),
                    );
                }
                self.render_detail(frame, app, actor, right);
            }
        }
        let unread = self
            .items
            .values()
            .filter(|answer| app.answer_unread(actor, answer))
            .count();
        strip::paint_left(
            frame.buffer_mut(),
            areas.position,
            Line::styled(
                format!("● {unread} unread · ○ {} read", self.items.len() - unread),
                look.role(Role::Dim),
            ),
        );
        if self.page_failed {
            strip::paint_left(
                frame.buffer_mut(),
                areas.status,
                Line::styled(
                    "✗ Older answers unavailable · r retry",
                    look.role(Role::Blocked),
                ),
            );
        } else if self.more {
            strip::paint_left(
                frame.buffer_mut(),
                areas.status,
                Line::styled(
                    "○ Older unread answers are not loaded · they appear as these are read",
                    look.role(Role::Dim),
                ),
            );
        }
        let footer = if narrow && !self.detail_open {
            "↑↓ choose · Enter read · r toggle read · a all read · Esc close"
        } else if narrow {
            "↑↓ PgUp PgDn scroll · Enter chat · r toggle read · a all read · Esc list"
        } else {
            "↑↓ choose · Enter chat · r toggle read · a all read · Tab scroll detail · Esc close"
        };
        strip::paint_left(
            frame.buffer_mut(),
            areas.footer,
            Line::styled(footer, look.role(Role::Dim)),
        );
        if self.confirmation.is_some() {
            self.painted.borrow_mut().clear();
            self.render_confirmation(frame, app, body);
        }
    }
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    crate::board::notes::sanitize(&line.chars().take(512).collect::<String>())
}

fn caption(text: &str, look: crate::look::Look) -> Line<'static> {
    exchange_view::feedback(Line::styled(text.to_owned(), look.role(Role::Dim)), look)
}
