//! Content-fit shared modal, Editor fields and whole buttons with current hits.
use super::*;
use ratatui::{Frame, text::Line};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::components::{Modal, Placement, strip};
impl Controller {
    fn lines(&self) -> Vec<String> {
        match &self.stage {
            Stage::Form if self.loaded.is_none()=>vec!["◌ Loading · host templates…".into()],
            Stage::Form=>vec![format!("Switch {} from {}",self.source.as_ref().map_or("selected agent",|s|s.name.as_str()),self.source.as_ref().map_or("unknown",Source::driver)),
                String::new(),"AI tool".into(),String::new(),String::new(),"Model".into(),String::new(),String::new(),
                "↑↓ choose an observed tool/model, or type a value. Empty model uses the selected tool's policy.".into()],
            Stage::Review=>self.review.as_ref().map_or_else(Vec::new,Review::lines),
            Stage::Pending=>vec![format!("◌ Switching · {} to {} in {} / {}…",self.source.as_ref().map_or("selected agent",|s|s.name.as_str()),self.fields[0],self.source.as_ref().map_or("—",Source::host),self.source.as_ref().map_or("—",Source::session)),
                "Waiting for the same UUID to report the requested running tool.".into(), "Esc hides · the switch continues.".into()],
            Stage::Result(text)=>text.lines().map(str::to_owned).collect(),
        }
    }
    pub fn render(&self, frame: &mut Frame, app: &App, look: crate::look::Look, body: Rect) {
        self.invalidate();
        let modal = Modal {
            title: "Switch AI tool".into(),
            placement: Placement::Center,
        };
        let status = self.notice.is_some();
        let probe = modal.areas(body, [100, 29], true, status);
        let wrap = |width: u16| {
            self.lines()
                .iter()
                .flat_map(|l| {
                    super::super::notes::wrap(
                        &super::super::notes::sanitize(&tmt_cli_style::table::escape(l)),
                        usize::from(width.max(1)),
                    )
                })
                .collect::<Vec<_>>()
        };
        let demand = wrap(probe.content.width).len();
        let areas = modal.areas(
            body,
            [100, (demand + 4 + usize::from(status)).min(29) as u16],
            true,
            status,
        );
        modal.paint(areas, frame.buffer_mut(), &look.theme, look.depth);
        app.input_band.set(Some(areas.outer));
        let lines = wrap(areas.content.width);
        let mut scroll = self.scroll.borrow_mut();
        scroll.update(areas.content, lines.len(), None);
        for (i, line) in lines
            .iter()
            .skip(scroll.offset())
            .take(usize::from(areas.content.height))
            .enumerate()
        {
            strip::paint_left(
                frame.buffer_mut(),
                Rect::new(
                    areas.content.x,
                    areas.content.y + i as u16,
                    areas.content.width,
                    1,
                ),
                Line::styled(line.clone(), look.role(Role::Text)),
            );
        }
        let loaded_form = self.stage == Stage::Form && self.loaded.is_some();
        let mut readable = body.height >= 10
            && areas.content.width >= 34
            && scroll.offset() == 0
            && lines.len() <= usize::from(areas.content.height);
        if loaded_form {
            let mut fields = [None, None];
            let intro =
                super::super::notes::wrap(&self.lines()[0], usize::from(areas.content.width)).len();
            for (index, field) in self.fields.iter().enumerate() {
                let y = intro + 1 + index * 3;
                if y >= usize::from(areas.content.height) {
                    readable = false;
                    continue;
                }
                let rect = Rect::new(
                    areas.content.x + 12,
                    areas.content.y + y as u16,
                    areas.content.width.saturating_sub(12),
                    1,
                );
                fields[index] = Some(rect);
                let layout = field.layout(rect.width);
                let cursor = layout.paint(
                    frame.buffer_mut(),
                    rect,
                    0,
                    field.selection(),
                    look.role(if self.focus == index {
                        Role::Accent
                    } else {
                        Role::Text
                    }),
                    look.selection(),
                );
                if self.focus == index {
                    self.editor_width.set(rect.width);
                    if let Some(cursor) = cursor {
                        frame.set_cursor_position(cursor);
                    }
                }
                if field.is_empty() {
                    strip::paint_left(
                        frame.buffer_mut(),
                        rect,
                        Line::styled(
                            if index == 0 {
                                "choose or type tool"
                            } else {
                                "remembered / default"
                            },
                            look.role(Role::Muted),
                        ),
                    );
                }
            }
            self.fields_hit.set(fields);
            use super::super::chip_group::{self, ChipGroup, Choice};
            for (index, field) in fields.iter().enumerate() {
                let Some(field) = field else { continue };
                if field.y + 1 >= areas.content.bottom() {
                    continue;
                }
                let values = if index == 0 {
                    self.tools.clone()
                } else {
                    std::iter::once(String::new())
                        .chain(
                            self.models
                                .get(self.fields[0].as_str())
                                .into_iter()
                                .flatten()
                                .cloned(),
                        )
                        .collect::<Vec<_>>()
                };
                let groups = [ChipGroup {
                    label: String::new(),
                    choices: values
                        .iter()
                        .enumerate()
                        .map(|(n, value)| Choice {
                            id: format!("choice-{n}"),
                            text: if value.is_empty() {
                                " default ".into()
                            } else if index == 0
                                && self.source.as_ref().is_some_and(|s| s.driver() == value)
                            {
                                format!(" {value} · current ")
                            } else {
                                format!(" {value} ")
                            },
                            gap: " ".into(),
                            actionable: true,
                            selected: value == self.fields[index].as_str(),
                            disabled: index == 0
                                && self.source.as_ref().is_some_and(|s| s.driver() == value),
                            cursor: false,
                        })
                        .collect(),
                }];
                let hits = chip_group::paint(
                    frame.buffer_mut(),
                    Rect::new(field.x, field.y + 1, field.width, 1),
                    &groups,
                    app,
                    Some(super::super::app::Overlay::SwitchTool),
                );
                for hit in hits {
                    if let Some(n) = hit
                        .id
                        .strip_prefix("choice-")
                        .and_then(|n| n.parse::<usize>().ok())
                    {
                        self.choices
                            .borrow_mut()
                            .push((index, values[n].clone(), hit.area));
                    }
                }
            }
        }
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
        if let Some(notice) = &self.notice {
            strip::paint_left(
                frame.buffer_mut(),
                areas.status,
                Line::styled(
                    super::super::notes::single_line(notice),
                    look.role(Role::Waiting),
                ),
            );
        }
        let (cancel, apply, focus) = match self.stage {
            Stage::Form => (
                "Cancel",
                if loaded_form { "Review" } else { "" },
                self.focus.saturating_sub(2),
            ),
            Stage::Review => (
                "Cancel",
                if self.review.as_ref().is_some_and(|r| r.outer.is_none()) {
                    "Copy command"
                } else {
                    "Switch"
                },
                self.focus,
            ),
            Stage::Pending => ("", "", 0),
            Stage::Result(_) => ("Close", "", 0),
        };
        let can_apply = readable
            && !apply.is_empty()
            && (self.stage != Stage::Form || self.review_from_fields().is_ok());
        let buttons = [
            super::super::buttons::Button::secondary(cancel)
                .focused(focus == 0 && (self.stage != Stage::Form || self.focus >= 2))
                .enabled(!cancel.is_empty()),
            super::super::buttons::Button::primary(apply)
                .focused(focus == 1)
                .enabled(can_apply),
        ];
        let n = if apply.is_empty() {
            usize::from(!cancel.is_empty())
        } else {
            2
        };
        let hits = super::super::buttons::paint(
            app,
            frame.buffer_mut(),
            areas.footer,
            &buttons[..n],
            Align::Left,
            Some(super::super::app::Overlay::SwitchTool),
        );
        self.controls.set([
            hits.first().copied().flatten(),
            hits.get(1).copied().flatten().filter(|_| can_apply),
        ]);
        if self.stage == Stage::Review && self.review.as_ref().is_some_and(|r| r.outer.is_some()) {
            let x = hits
                .iter()
                .flatten()
                .map(|r| r.right())
                .max()
                .unwrap_or(areas.footer.x)
                .saturating_add(1);
            if areas.footer.right().saturating_sub(x) >= 14 {
                strip::paint_left(
                    frame.buffer_mut(),
                    Rect::new(x, areas.footer.y, areas.footer.right() - x, 1),
                    Line::styled("c copy command", look.role(Role::Muted)),
                );
            }
        }
        if self.stage == Stage::Pending {
            strip::paint_left(
                frame.buffer_mut(),
                areas.footer,
                Line::styled(app.switch_tool_reopen_hint(), look.role(Role::Muted)),
            );
        }
        self.readable.set(readable);
    }
}
