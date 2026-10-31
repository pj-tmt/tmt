//! Shared modal chrome, Editor cells and flat action chips; clipped actions refuse.
use super::*;
use crate::look::Look;
use ratatui::{Frame, text::Line};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::components::strip;
use tmt_tui::components::{Modal, Placement};
fn safe(text: &str) -> String {
    super::super::notes::sanitize(&tmt_cli_style::table::escape(text))
}
impl Controller {
    fn body_lines(&self) -> Vec<String> {
        match &self.stage {
            Stage::Saving => vec![
                "Saving launch template…".into(),
                "Member draft kept.".into(),
            ],
            Stage::Launching => self
                .review
                .as_ref()
                .map(|r| {
                    vec![if r.host == model::BUILT_IN_HOST {
                        format!("Starting {} in the built-in terminal…", r.name)
                    } else {
                        format!(
                            "Starting {} in {} / {}…",
                            r.name,
                            model::host_label(&r.host),
                            r.session
                        )
                    }]
                })
                .unwrap_or_default(),
            Stage::Result(text) => text.lines().map(str::to_owned).collect(),
            _ => vec![],
        }
    }
    fn preview_lines(&self, width: u16) -> Vec<String> {
        self.loaded
            .as_ref()
            .and_then(|loaded| self.draft.review(loaded).ok())
            .map(|review| {
                let mut text = vec![
                    if review.outer.is_some() {
                        "Runs · e edit template"
                    } else {
                        "Copy only · this host has no launch template"
                    }
                    .to_owned(),
                    crate::launch::line(&review.display.command),
                ];
                // The follow-up commands, keep then squad, one row each.
                text.extend(review.copy().lines().skip(1).map(str::to_owned));
                text.iter()
                    .flat_map(|line| super::super::notes::wrap(&safe(line), usize::from(width)))
                    .collect()
            })
            .unwrap_or_default()
    }
    fn temporary_hint(&self, width: u16) -> Vec<String> {
        if self.draft.saved {
            return vec![];
        }
        super::super::notes::wrap(
            "Temporary agents stay in Common pool; choose Saved to join a squad.",
            usize::from(width),
        )
    }
    /// Form rows: the intro, one row per shown field with a gap, the Temporary hint, then the
    /// command preview after one blank row.
    fn form_height(&self, width: u16) -> (usize, usize) {
        let fields = 2 * self.draft.visible(self.loaded.as_ref()).len() + 1;
        let hint = self.temporary_hint(width).len();
        (fields + hint, fields + hint + 1)
    }
    fn content_height(&self, width: u16) -> usize {
        match self.stage {
            Stage::Form if self.list.is_some() => 3 + self.rows().len(),
            Stage::Form if self.loaded.is_some() => {
                let (rows, preview_at) = self.form_height(width);
                let preview = self.preview_lines(width);
                if preview.is_empty() {
                    rows
                } else {
                    preview_at + preview.len()
                }
            }
            Stage::Form | Stage::Hosts if self.loaded.is_none() => 1,
            Stage::Hosts => 2 + self.loaded.as_ref().map_or(0, |l| l.hosts.len()),
            Stage::Template => 2 + self.template.layout(width).lines.len(),
            _ => self
                .body_lines()
                .iter()
                .map(|line| super::super::notes::wrap(&safe(line), usize::from(width)).len())
                .sum::<usize>()
                .max(1),
        }
    }
    pub fn render(&self, frame: &mut Frame, app: &App, look: Look, body: Rect) {
        if self.edit.is_some() {
            self.render_edit(frame, app, look, body);
            return;
        }
        self.fields.borrow_mut().clear();
        self.controls.set([None, None]);
        self.readable.set(false);
        let title = match self.stage {
            Stage::Template => "Launch template",
            Stage::Hosts => "Launch templates",
            _ => "New member",
        };
        let modal = Modal {
            title: title.into(),
            placement: Placement::Center,
        };
        let unavailable = self.loaded.is_none() && matches!(self.stage, Stage::Form | Stage::Hosts);
        let status = self.notice.is_some() && !unavailable;
        let probe = modal.areas(body, [100, 29], true, status);
        let demand = self.content_height(probe.content.width);
        let areas = modal.areas(
            body,
            [
                100,
                demand
                    .saturating_add(4 + usize::from(status))
                    .min(usize::from(u16::MAX)) as u16,
            ],
            true,
            status,
        );
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        app.input_band.set(Some(areas.outer));
        app.peek_areas
            .set(crate::board::app::Overlay::NewMember, Some(areas.outer));
        let content = areas.content;
        let mut complete = content.width >= 34;
        let paint = |frame: &mut Frame, text: &str, row: usize, role: Role| {
            if row < usize::from(content.height) {
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect::new(content.x, content.y + row as u16, content.width, 1),
                    Line::styled(safe(text), look.role(role)),
                );
            }
        };
        match &self.stage {
            Stage::Form if self.loaded.is_none() => {
                paint(
                    frame,
                    if self.notice.is_some() {
                        "Host templates unavailable. Close and reopen to retry."
                    } else {
                        "Loading host templates…"
                    },
                    0,
                    Role::Muted,
                );
                complete = false;
            }
            Stage::Form if self.list.is_some() => {
                let list = self.list.as_ref().unwrap();
                paint(
                    frame,
                    &format!("{} · type to filter", list.field.label()),
                    0,
                    Role::Muted,
                );
                let area = Rect::new(
                    content.x,
                    content.y + 1,
                    content.width,
                    1.min(content.height.saturating_sub(1)),
                );
                let layout = list.filter.layout(area.width);
                let cursor = layout.paint(
                    frame.buffer_mut(),
                    area,
                    0,
                    list.filter.selection(),
                    look.role(Role::Accent),
                    look.selection(),
                );
                self.editor_width.set(area.width);
                if let Some(cursor) = cursor {
                    frame.set_cursor_position(cursor);
                }
                if list.filter.is_empty() && area.height > 0 {
                    strip::paint_left(
                        frame.buffer_mut(),
                        area,
                        Line::styled("filter", look.role(Role::Muted)),
                    );
                }
                for (index, choice) in self.rows().iter().enumerate() {
                    let y = 3 + index;
                    if y >= usize::from(content.height) {
                        complete = false;
                        continue;
                    }
                    let selected = index == list.row;
                    paint(
                        frame,
                        &format!("{} {}", if selected { "›" } else { " " }, choice.label),
                        y,
                        if selected { Role::Accent } else { Role::Text },
                    );
                    self.fields.borrow_mut().push((
                        Field::Choice(index),
                        Rect::new(content.x, content.y + y as u16, content.width, 1),
                    ));
                }
            }
            Stage::Form => {
                paint(
                    frame,
                    "Create an agent and start it on a host.",
                    0,
                    Role::Muted,
                );
                let slots = self.draft.visible(self.loaded.as_ref());
                for (index, slot) in slots.iter().copied().enumerate() {
                    let y = 2 + index * 2;
                    if y >= usize::from(content.height) {
                        complete = false;
                        continue;
                    }
                    paint(frame, slot.label(), y, Role::Muted);
                    let area = Rect::new(
                        content.x + 12,
                        content.y + y as u16,
                        content.width.saturating_sub(12),
                        1,
                    );
                    let focus = self.at == Focus::Slot(slot);
                    if slot == model::Slot::Keep {
                        if area.width >= 7 {
                            self.fields
                                .borrow_mut()
                                .push((Field::Keep(true), Rect::new(area.x, area.y, 7, 1)));
                        }
                        if area.width >= 21 {
                            self.fields
                                .borrow_mut()
                                .push((Field::Keep(false), Rect::new(area.x + 10, area.y, 11, 1)));
                        }
                        strip::paint_left(
                            frame.buffer_mut(),
                            area,
                            Line::styled(
                                if self.draft.saved {
                                    "● Saved   ○ Temporary"
                                } else {
                                    "○ Saved   ● Temporary"
                                },
                                if focus {
                                    look.selection()
                                } else {
                                    look.role(Role::Text)
                                },
                            ),
                        );
                        continue;
                    }
                    if slot == model::Slot::Squad && !self.draft.saved {
                        strip::paint_left(
                            frame.buffer_mut(),
                            area,
                            Line::styled("Common pool", look.role(Role::Muted)),
                        );
                        continue;
                    }
                    self.fields.borrow_mut().push((Field::Slot(slot), area));
                    // A listed field keeps its last column for ▾.
                    let text_area = if slot.listed() {
                        Rect::new(area.x, area.y, area.width.saturating_sub(2), 1)
                    } else {
                        area
                    };
                    if slot.listed() && area.width >= 2 {
                        strip::paint_left(
                            frame.buffer_mut(),
                            Rect::new(area.right() - 1, area.y, 1, 1),
                            Line::styled("▾", look.role(Role::Muted)),
                        );
                    }
                    if slot.typed() {
                        let editor = &self.draft.fields[slot.index()];
                        let layout = editor.layout(text_area.width);
                        let cursor = layout.paint(
                            frame.buffer_mut(),
                            text_area,
                            0,
                            editor.selection(),
                            look.role(if focus { Role::Accent } else { Role::Text }),
                            look.selection(),
                        );
                        if focus {
                            self.editor_width.set(text_area.width);
                            if let Some(cursor) = cursor {
                                frame.set_cursor_position(cursor);
                            }
                        }
                        if editor.is_empty() {
                            let placeholder = match slot {
                                model::Slot::Name => "agent name".into(),
                                model::Slot::Tool => "choose or type a tool".into(),
                                model::Slot::Model => "(provider default)".into(),
                                model::Slot::Squad => "Common pool".into(),
                                // Blank runs the board's shell, so the placeholder names it.
                                model::Slot::Command => {
                                    format!("{} (your shell)", self.draft.shell)
                                }
                                _ => "session".into(),
                            };
                            strip::paint_left(
                                frame.buffer_mut(),
                                text_area,
                                Line::styled(placeholder, look.role(Role::Muted)),
                            );
                        }
                    } else {
                        let shown = self.draft.shown(slot);
                        strip::paint_left(
                            frame.buffer_mut(),
                            text_area,
                            Line::styled(
                                shown.unwrap_or_else(|| match slot {
                                    model::Slot::Host => "choose host".into(),
                                    _ => "(provider default)".into(),
                                }),
                                if focus {
                                    look.selection()
                                } else {
                                    look.role(Role::Text)
                                },
                            ),
                        );
                    }
                }
                let (rows, preview_at) = self.form_height(content.width);
                for (index, line) in self.temporary_hint(content.width).iter().enumerate() {
                    paint(
                        frame,
                        line,
                        rows - self.temporary_hint(content.width).len() + index,
                        Role::Muted,
                    );
                }
                for (index, line) in self.preview_lines(content.width).iter().enumerate() {
                    paint(frame, line, preview_at + index, Role::Muted);
                }
            }
            Stage::Hosts => {
                if let Some(loaded) = &self.loaded {
                    paint(
                        frame,
                        "Host templates · Enter edit · Ctrl-R in the editor resets",
                        0,
                        Role::Muted,
                    );
                    for (index, host) in loaded.hosts.iter().enumerate() {
                        let selected = host.name == self.draft.fields[4].as_str();
                        paint(
                            frame,
                            &format!(
                                "{} {} · {}{}",
                                if selected { "›" } else { " " },
                                model::host_label(&host.name),
                                host.state,
                                if host.argv.is_none() {
                                    " · copy only"
                                } else {
                                    ""
                                }
                            ),
                            index + 2,
                            if selected { Role::Accent } else { Role::Text },
                        );
                    }
                } else {
                    paint(
                        frame,
                        if self.notice.is_some() {
                            "Host templates unavailable. Close and reopen to retry."
                        } else {
                            "Loading host templates…"
                        },
                        0,
                        Role::Muted,
                    );
                    complete = false;
                }
            }
            Stage::Template => {
                paint(
                    frame,
                    &format!(
                        "{} · JSON argv; whole {{session}} / {{command}} slots",
                        model::host_label(self.draft.text(model::Slot::Host))
                    ),
                    0,
                    Role::Muted,
                );
                let area = Rect::new(
                    content.x,
                    content.y.saturating_add(2),
                    content.width,
                    content.height.saturating_sub(2),
                );
                let layout = self.template.layout(area.width);
                let cursor = layout.paint(
                    frame.buffer_mut(),
                    area,
                    0,
                    self.template.selection(),
                    look.role(Role::Text),
                    look.selection(),
                );
                complete &= layout.lines.len() <= usize::from(area.height);
                self.editor_width.set(area.width);
                if self.focus == 0
                    && let Some(cursor) = cursor
                {
                    frame.set_cursor_position(cursor);
                }
            }
            _ => {
                let text = self.body_lines();
                let lines: Vec<_> = text
                    .iter()
                    .flat_map(|line| {
                        super::super::notes::wrap(&safe(line), usize::from(content.width))
                    })
                    .collect();
                let mut scroll = self.scroll.borrow_mut();
                scroll.update(content, lines.len(), None);
                for (row, line) in lines
                    .iter()
                    .skip(scroll.offset())
                    .take(usize::from(content.height))
                    .enumerate()
                {
                    paint(frame, line, row, Role::Text);
                }
                complete &= scroll.offset() + usize::from(content.height) >= lines.len();
                if let Some(position) = scroll.overflow_position() {
                    strip::paint_left(
                        frame.buffer_mut(),
                        areas.position,
                        Line::styled(format!("Read · {position}"), look.role(Role::Muted)),
                    );
                }
            }
        }
        if let Some(notice) = &self.notice
            && !unavailable
        {
            strip::paint_left(
                frame.buffer_mut(),
                areas.status,
                Line::styled(safe(notice), look.role(Role::Waiting)),
            );
        }
        let copy_only = self
            .loaded
            .as_ref()
            .and_then(|l| self.draft.review(l).ok())
            .is_some_and(|r| r.outer.is_none());
        use super::Control;
        let (cancel, apply, focus) = match self.stage {
            Stage::Form if unavailable => ("Cancel", "", None),
            Stage::Form if self.list.is_some() => ("Back", "Choose", Some(Control::Apply)),
            Stage::Form => (
                "Cancel",
                if copy_only { "Copy commands" } else { "Create" },
                match self.at {
                    Focus::Cancel => Some(Control::First),
                    Focus::Discard => Some(Control::Discard),
                    Focus::Create => Some(Control::Apply),
                    Focus::Slot(_) => None,
                },
            ),
            Stage::Hosts if unavailable => ("Close", "", None),
            Stage::Hosts => ("Close", "Edit template", Some(Control::Apply)),
            // Back steps to the form or host list; the editor holds focus at 0.
            Stage::Template => (
                "Back",
                "Save template",
                self.focus
                    .checked_sub(1)
                    .and_then(|at| self.template_controls().get(at).copied()),
            ),
            Stage::Launching | Stage::Saving => ("", "", None),
            Stage::Result(_) => ("Close", "", Some(Control::First)),
        };
        complete &= body.height >= 10;
        if matches!(self.stage, Stage::Form | Stage::Hosts) {
            complete &= usize::from(content.height) >= demand;
        }
        let discard = self.discards() && !unavailable;
        let mut controls = Vec::new();
        let mut buttons = Vec::new();
        if !cancel.is_empty() {
            controls.push(Control::First);
            buttons.push(
                super::super::buttons::Button::secondary(cancel)
                    .focused(focus == Some(Control::First)),
            );
        }
        if discard {
            controls.push(Control::Discard);
            buttons.push(
                super::super::buttons::Button::secondary("Discard")
                    .focused(focus == Some(Control::Discard)),
            );
        }
        if !apply.is_empty() {
            controls.push(Control::Apply);
            buttons.push(
                super::super::buttons::Button::primary(apply)
                    .focused(focus == Some(Control::Apply))
                    .enabled(complete),
            );
        }
        let hits = super::super::buttons::paint(
            app,
            frame.buffer_mut(),
            areas.footer,
            &buttons,
            Align::Left,
            Some(super::super::app::Overlay::NewMember),
        );
        let hit = |control| {
            controls
                .iter()
                .position(|c| *c == control)
                .and_then(|i| hits.get(i).copied().flatten())
        };
        self.controls.set([
            hit(Control::First),
            hit(Control::Apply).filter(|_| complete),
        ]);
        self.discard.set(hit(Control::Discard));
        self.readable.set(complete);
        let start = hits
            .iter()
            .flatten()
            .map(|r| r.right())
            .max()
            .unwrap_or(areas.footer.x)
            .saturating_add(1)
            .min(areas.footer.right());
        let hint = match self.stage {
            Stage::Form | Stage::Hosts if unavailable => "Esc close",
            Stage::Form if self.list.is_some() => "↑↓ choose · type to filter · Esc back",
            Stage::Form => "↑↓ move · ←→ change · Enter list · e template",
            Stage::Template => "Ctrl-R reset · Esc close",
            _ => "Esc close",
        };
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(
                start,
                areas.footer.y,
                areas.footer.right() - start,
                areas.footer.height,
            ),
            Line::styled(hint, look.role(Role::Muted)),
        );
    }
}
