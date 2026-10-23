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
            Stage::Review => self.review.as_ref().map(|r| r.lines()).unwrap_or_default(),
            Stage::Saving => vec![
                "Saving launch template…".into(),
                "Member draft kept.".into(),
            ],
            Stage::Launching => self
                .review
                .as_ref()
                .map(|r| {
                    vec![format!(
                        "Starting {} in {} / {}…",
                        r.name, r.host, r.session
                    )]
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
                    "Will run · e edit template".to_owned(),
                    crate::launch::line(&review.display.command),
                ];
                if review.room.is_some() {
                    text.push(review.copy().split_once('\n').unwrap().1.into());
                }
                text.iter()
                    .flat_map(|line| super::super::notes::wrap(&safe(line), usize::from(width)))
                    .collect()
            })
            .unwrap_or_default()
    }
    fn content_height(&self, width: u16) -> usize {
        match self.stage {
            Stage::Form if self.loaded.is_some() => {
                let preview = self.preview_lines(width);
                if preview.is_empty() {
                    if self.draft.saved {
                        15
                    } else {
                        15 + super::super::notes::wrap(
                            "Temporary agents stay in Common pool; choose Saved to join a squad.",
                            usize::from(width),
                        )
                        .len()
                    }
                } else {
                    (if self.draft.saved { 16 } else { 18 }) + preview.len()
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
        let content = areas.content;
        let mut complete = content.width >= 34;
        let mut paint = |text: &str, row: usize, role: Role| {
            if row < usize::from(content.height) {
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect::new(content.x, content.y + row as u16, content.width, 1),
                    Line::styled(safe(text), look.role(role)),
                );
            }
        };
        match &self.stage {
            Stage::Form => {
                if self.loaded.is_none() {
                    paint(
                        if self.notice.is_some() {
                            "Host templates unavailable. Close and reopen to retry."
                        } else {
                            "Loading host templates…"
                        },
                        0,
                        Role::Muted,
                    );
                    complete = false;
                } else {
                    paint("Create an agent in a new host window.", 0, Role::Muted);
                    for (index, label) in model::LABELS.iter().enumerate() {
                        let y = 2 + index * 2;
                        paint(label, y, Role::Muted);
                    }
                    for index in 0..7 {
                        let y = 2 + index * 2;
                        if y >= usize::from(content.height) {
                            complete = false;
                            continue;
                        }
                        let area = Rect::new(
                            content.x + 12,
                            content.y + y as u16,
                            content.width.saturating_sub(12),
                            1,
                        );
                        if index == 3 && !self.draft.saved {
                            strip::paint_left(
                                frame.buffer_mut(),
                                area,
                                Line::styled("Common pool", look.role(Role::Muted)),
                            );
                            continue;
                        }
                        if index == 4 {
                            self.fields.borrow_mut().push((Field::Host, area));
                            strip::paint_left(
                                frame.buffer_mut(),
                                area,
                                Line::styled(
                                    format!(
                                        "{} ▾",
                                        if self.draft.fields[4].is_empty() {
                                            "choose host"
                                        } else {
                                            self.draft.fields[4].as_str()
                                        }
                                    ),
                                    if self.focus == 4 {
                                        look.selection()
                                    } else {
                                        look.role(Role::Text)
                                    },
                                ),
                            );
                            continue;
                        }
                        if index < 6 {
                            self.fields.borrow_mut().push((Field::Text(index), area));
                        } else {
                            let first = Rect::new(area.x, area.y, 7.min(area.width), 1);
                            if area.width >= 7 {
                                self.fields
                                    .borrow_mut()
                                    .push((Field::Identity(true), first));
                            }
                            if area.width >= 21 {
                                self.fields.borrow_mut().push((
                                    Field::Identity(false),
                                    Rect::new(area.x + 10, area.y, 11, 1),
                                ));
                            }
                        }
                        if index < 6 {
                            let editor = &self.draft.fields[index];
                            let layout = editor.layout(area.width);
                            let focus = self.focus == index;
                            let cursor = layout.paint(
                                frame.buffer_mut(),
                                area,
                                0,
                                editor.selection(),
                                look.role(if focus { Role::Accent } else { Role::Text }),
                                look.selection(),
                            );
                            if focus {
                                self.editor_width.set(area.width);
                                if let Some(cursor) = cursor {
                                    frame.set_cursor_position(cursor);
                                }
                            }
                            if editor.is_empty() {
                                let placeholder = match index {
                                    0 => "agent name",
                                    1 => "choose or type tool",
                                    2 => "provider default",
                                    3 => "Common pool",
                                    4 => "choose host",
                                    _ => "session",
                                };
                                strip::paint_left(
                                    frame.buffer_mut(),
                                    area,
                                    Line::styled(placeholder, look.role(Role::Muted)),
                                );
                            }
                        } else {
                            strip::paint_left(
                                frame.buffer_mut(),
                                area,
                                Line::styled(
                                    if self.draft.saved {
                                        "● Saved   ○ Temporary"
                                    } else {
                                        "○ Saved   ● Temporary"
                                    },
                                    if self.focus == index {
                                        look.selection()
                                    } else {
                                        look.role(Role::Text)
                                    },
                                ),
                            );
                        }
                    }
                    if !self.draft.saved {
                        let hint =
                            "Temporary agents stay in Common pool; choose Saved to join a squad.";
                        for (index, line) in
                            super::super::notes::wrap(hint, usize::from(content.width))
                                .iter()
                                .enumerate()
                        {
                            let y = 15 + index;
                            if y < usize::from(content.height) {
                                strip::paint_left(
                                    frame.buffer_mut(),
                                    Rect::new(content.x, content.y + y as u16, content.width, 1),
                                    Line::styled(line.clone(), look.role(Role::Muted)),
                                );
                            }
                        }
                    }
                    for (index, line) in self.preview_lines(content.width).iter().enumerate() {
                        let y = if self.draft.saved { 16 } else { 18 } + index;
                        if y < usize::from(content.height) {
                            strip::paint_left(
                                frame.buffer_mut(),
                                Rect::new(content.x, content.y + y as u16, content.width, 1),
                                Line::styled(line.clone(), look.role(Role::Muted)),
                            );
                        }
                    }
                }
            }
            Stage::Hosts => {
                if let Some(loaded) = &self.loaded {
                    paint(
                        "Host templates · Enter edit · Ctrl-R in the editor resets",
                        0,
                        Role::Muted,
                    );
                    for (index, host) in loaded.hosts.iter().enumerate() {
                        let selected = host.name == self.draft.fields[4].as_str();
                        paint(
                            &format!(
                                "{} {} · {}{}",
                                if selected { "›" } else { " " },
                                host.name,
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
                    &format!(
                        "{} · JSON argv; whole {{session}} / {{command}} slots",
                        self.draft.fields[4]
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
                    paint(line, row, Role::Text);
                }
                complete &= scroll.offset() + usize::from(content.height) >= lines.len();
                if scroll.limit() > 0 {
                    strip::paint_left(
                        frame.buffer_mut(),
                        areas.position,
                        Line::styled(
                            format!("Read · {}", scroll.position()),
                            look.role(Role::Muted),
                        ),
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
        let (cancel, apply, focus) = match self.stage {
            Stage::Form if unavailable => ("Cancel", "", 0),
            Stage::Form => ("Cancel", "Review", self.focus.saturating_sub(7)),
            Stage::Hosts if unavailable => ("Close", "", 0),
            Stage::Hosts => ("Close", "Edit template", 1),
            Stage::Review => (
                "Cancel",
                if self.review.as_ref().is_some_and(|r| r.outer.is_none()) {
                    "Copy commands"
                } else {
                    "Create"
                },
                self.focus,
            ),
            Stage::Template => ("Cancel", "Save template", self.focus.saturating_sub(1)),
            Stage::Launching | Stage::Saving => ("", "", 0),
            Stage::Result(_) => ("Close", "", 0),
        };
        complete &= body.height >= 10;
        if matches!(self.stage, Stage::Form | Stage::Hosts) {
            complete &= usize::from(content.height) >= demand;
        }
        let buttons = [
            super::super::buttons::Button::secondary(cancel)
                .focused(focus == 0 && (self.stage != Stage::Form || self.focus >= 7))
                .enabled(!cancel.is_empty()),
            super::super::buttons::Button::primary(apply)
                .focused(focus == 1)
                .enabled(complete && !apply.is_empty()),
        ];
        let buttons = &buttons[..if apply.is_empty() {
            usize::from(!cancel.is_empty())
        } else {
            2
        }];
        let hits = super::super::buttons::paint(
            app,
            frame.buffer_mut(),
            areas.footer,
            buttons,
            Align::Left,
            Some(super::super::app::Overlay::NewMember),
        );
        self.controls.set([
            hits.first().copied().flatten(),
            hits.get(1).copied().flatten().filter(|_| complete),
        ]);
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
            Stage::Form => "Tab next · Space choose · e template · Esc cancel",
            Stage::Template => "Ctrl-R reset · Esc back",
            Stage::Review => "c copy · e template · PgDn read · Esc back",
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
