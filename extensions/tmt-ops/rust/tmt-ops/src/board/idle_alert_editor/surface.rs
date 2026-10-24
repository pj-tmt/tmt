//! Content-fit shared modal and flat chips; publish only current clipped hits.
use super::*;
use crate::board::chip_group;
use ratatui::{Frame, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::{Modal, Placement, strip};
fn safe(text: &str) -> String {
    crate::board::notes::sanitize(&tmt_cli_style::table::escape(text))
}
impl Controller {
    pub fn render(&self, frame: &mut Frame, app: &App, body: Rect) {
        self.invalidate();
        let look = app.look();
        let modal = Modal {
            title: format!("Idle alert · {}", safe(&self.name)),
            placement: Placement::Center,
        };
        let available = self.config.is_some();
        let invalid = available && self.draft().is_err();
        let probe = modal.areas(body, [66, 18], true, false);
        let impact: Vec<_> = self
            .impact(app)
            .iter()
            .flat_map(|text| {
                crate::board::notes::wrap(&safe(text), usize::from(probe.content.width))
            })
            .collect();
        let demand = if available {
            5 + impact.len() + usize::from(invalid)
        } else {
            1
        };
        let mut areas = modal.areas(
            body,
            [66, (demand + 4 + usize::from(self.notice.is_some())) as u16],
            true,
            self.notice.is_some(),
        );
        // The same modal geometry, placed beside the list rather than over Talk.
        if let Some(list) = app
            .workspace_placement(body)
            .and_then(|p| p.list)
            .filter(|list| list.width >= areas.outer.width)
        {
            let x = list.x + (list.width - areas.outer.width) / 2;
            let row_end = app
                .hits
                .borrow()
                .iter()
                .filter(|hit| hit.row().is_some())
                .map(|hit| hit.y.saturating_add(1))
                .max()
                .unwrap_or(body.y);
            let y = areas
                .outer
                .y
                .max(row_end)
                .min(body.bottom().saturating_sub(areas.outer.height));
            let dx = i32::from(x) - i32::from(areas.outer.x);
            let dy = i32::from(y) - i32::from(areas.outer.y);
            for area in [
                &mut areas.outer,
                &mut areas.content,
                &mut areas.footer,
                &mut areas.position,
                &mut areas.status,
            ] {
                area.x = (i32::from(area.x) + dx).max(0) as u16;
                area.y = (i32::from(area.y) + dy).max(0) as u16;
            }
        }
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        app.input_band.set(Some(areas.outer));
        let content = areas.content;
        let complete = content.width >= 40 && usize::from(content.height) >= demand;
        let line = |row: u16| {
            Rect::new(
                content.x,
                content.y.saturating_add(row),
                content.width,
                u16::from(row < content.height),
            )
        };
        let paint = |frame: &mut Frame, area: Rect, text: &str, role| {
            strip::paint_left(
                frame.buffer_mut(),
                area,
                Line::styled(safe(text), look.role(role)),
            );
        };
        if available {
            paint(frame, line(0), "Alert", Role::Text);
            let row = line(0);
            let hits = chips(
                app,
                frame.buffer_mut(),
                Rect::new(row.x + 9, row.y, row.width.saturating_sub(9), row.height),
                &[
                    (
                        "● On",
                        self.enabled,
                        self.focus == 0 && self.enabled,
                        !self.pending,
                    ),
                    (
                        "○ Off",
                        !self.enabled,
                        self.focus == 0 && !self.enabled,
                        !self.pending,
                    ),
                ],
            );
            for (i, hit) in hits {
                self.hits.borrow_mut().push((hit, Control::On(i == 0)));
            }
            paint(frame, line(1), "After", Role::Text);
            let row = line(1);
            let editor = Rect::new(
                row.x + 9,
                row.y,
                12.min(row.width.saturating_sub(9)),
                row.height,
            );
            let field_style = crate::board::input_field::style(look, false);
            strip::paint_left(
                frame.buffer_mut(),
                editor,
                Line::styled(" ".repeat(usize::from(editor.width)), field_style),
            );
            let editor_inner = Rect::new(
                editor.x.saturating_add(1),
                editor.y,
                editor.width.saturating_sub(2),
                editor.height,
            );
            let layout = self.after.layout(editor_inner.width);
            let cursor = layout.paint(
                frame.buffer_mut(),
                editor_inner,
                0,
                self.after.selection(),
                field_style,
                look.selection(),
            );
            self.editor_width.set(editor_inner.width);
            if self.focus == 1
                && self.choice == 0
                && !self.pending
                && let Some(cursor) = cursor
            {
                frame.set_cursor_position(cursor);
            }
            if editor.height > 0 {
                self.hits.borrow_mut().push((editor, Control::After));
            }
            let after = self.draft().ok().map(|s| s.after.as_secs());
            let options: Vec<_> = PRESETS
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    (
                        *label,
                        after == Some([900, 1800, 3600, 7200][i]),
                        self.focus == 1 && self.choice == i + 1,
                        !self.pending,
                    )
                })
                .collect();
            for (i, hit) in chips(
                app,
                frame.buffer_mut(),
                Rect::new(
                    editor.right() + 1,
                    row.y,
                    row.right().saturating_sub(editor.right() + 1),
                    row.height,
                ),
                &options,
            ) {
                self.hits.borrow_mut().push((hit, Control::Preset(i)));
            }
            if invalid {
                paint(frame, line(2), INVALID, Role::Blocked);
            }
            for (i, text) in impact.iter().enumerate() {
                paint(
                    frame,
                    line((3 + usize::from(invalid) + i) as u16),
                    text,
                    Role::Muted,
                );
            }
        } else {
            paint(
                frame,
                line(0),
                if self.pending {
                    "◌ Loading idle alert…"
                } else {
                    "✗ Unavailable · close and reopen to retry."
                },
                Role::Muted,
            );
        }
        if let Some(notice) = &self.notice {
            paint(frame, areas.status, notice, Role::Muted);
        }
        if self.pending {
            paint(
                frame,
                areas.footer,
                if available {
                    "◌ Saving idle alert…"
                } else {
                    "◌ Loading…"
                },
                Role::Muted,
            );
        } else {
            let hits = chips(
                app,
                frame.buffer_mut(),
                line((demand - 1) as u16),
                &[
                    (
                        "Save",
                        false,
                        self.focus == 2 && self.choice == 0,
                        complete && available && !invalid,
                    ),
                    ("Cancel", false, self.focus == 2 && self.choice == 1, true),
                ],
            );
            for (i, hit) in &hits {
                self.hits.borrow_mut().push((
                    *hit,
                    if *i == 0 {
                        Control::Save
                    } else {
                        Control::Cancel
                    },
                ));
            }
            paint(
                frame,
                areas.footer,
                if available {
                    "↑↓ row · ←→ choose · type a time · Enter save · Esc cancel"
                } else {
                    "Esc close"
                },
                Role::Muted,
            );
        }
        self.readable.set(complete && available);
    }
}

fn chips(
    app: &App,
    buffer: &mut ratatui::buffer::Buffer,
    area: Rect,
    options: &[(&str, bool, bool, bool)],
) -> Vec<(usize, Rect)> {
    let groups = [chip_group::ChipGroup {
        label: String::new(),
        choices: options
            .iter()
            .enumerate()
            .map(
                |(i, (label, selected, cursor, enabled))| chip_group::Choice {
                    id: format!("choice-{i}"),
                    actionable: true,
                    text: format!(" {label} "),
                    gap: " ".into(),
                    selected: *selected,
                    disabled: !*enabled,
                    cursor: *cursor,
                },
            )
            .collect(),
    }];
    chip_group::paint(
        buffer,
        area,
        &groups,
        app,
        Some(crate::board::app::Overlay::IdleAlert),
    )
    .into_iter()
    .filter_map(|hit| Some((hit.id.strip_prefix("choice-")?.parse().ok()?, hit.area)))
    .collect()
}
