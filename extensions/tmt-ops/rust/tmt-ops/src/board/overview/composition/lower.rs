//! Narrow Timers/Digest band below the primary list viewport.
use crate::{
    board::{
        app::{App, Hit, HitTarget},
        divider, empty_state,
        overview::{digest, timers},
        section_rule, timer_rows,
    },
    look::Look,
};
use ratatui::{Frame, layout::Rect, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

const TIMER_CELLS: u16 = 62;
const DIGEST_MIN: u16 = 32;

pub(in crate::board) struct Band {
    timers: timers::Model,
    digest: Option<[digest::Metric; 5]>,
    split: bool,
    pub height: usize,
}
impl Band {
    pub(super) fn timers(app: &App) -> Self {
        let timers = timers::project(app);
        let height = timers.height();
        Self {
            timers,
            digest: None,
            split: false,
            height,
        }
    }
    pub fn new(app: &App, frame_width: u16, width: u16) -> Option<Self> {
        // S4 introduces the final wide right column with the Digest matrix.
        if frame_width >= 140 || width == 0 {
            return None;
        }
        let timers = timers::project(app);
        let digest = app
            .overview_panels()
            .shown(crate::config::OverviewPanel::Digest)
            .then(|| {
                let scene = super::super::monitor::scene(
                    app,
                    app.workspace_snapshot().expect("admitted workspace"),
                );
                digest::rows(app, &scene.summary)
            });
        let split = digest.is_some() && width >= TIMER_CELLS + 1 + DIGEST_MIN;
        let height = if digest.is_none() {
            timers.height()
        } else if split {
            timers.height().max(6)
        } else {
            timers.height() + 1 + 6
        };
        Some(Self {
            timers,
            digest,
            split,
            height,
        })
    }
    fn timer_height(&self, app: &App, height: u16) -> usize {
        if self.digest.is_none() {
            return self.timers.height();
        }
        app.pane_resize.size(
            crate::action::ResizeBoundary::OverviewLowerRows,
            height.saturating_sub(1),
            self.timers.height() as u16,
            self.timers.height() as u16,
            6,
        ) as usize
    }
    pub fn register(&self, app: &App, area: Rect, clip: Rect) {
        if self.split {
            let total = area.width.saturating_sub(1);
            let before = app.pane_resize.size(
                crate::action::ResizeBoundary::OverviewLowerColumns,
                total,
                TIMER_CELLS,
                32,
                DIGEST_MIN,
            );
            app.pane_resize.record(
                crate::board::pane_resize::Boundary {
                    id: crate::action::ResizeBoundary::OverviewLowerColumns,
                    hit: Rect::new(area.x + before, area.y, 1, area.height),
                    total,
                    before,
                    min: 32,
                    max: total.saturating_sub(DIGEST_MIN),
                },
                clip,
            );
        } else if self.digest.is_some() {
            let total = area.height.saturating_sub(1);
            let before = self.timer_height(app, area.height) as u16;
            app.pane_resize.record(
                crate::board::pane_resize::Boundary {
                    id: crate::action::ResizeBoundary::OverviewLowerRows,
                    hit: Rect::new(area.x, area.y + before + 1, area.width, 1),
                    total,
                    before,
                    min: self.timers.height() as u16,
                    max: total.saturating_sub(6),
                },
                clip,
            );
        }
    }
    pub fn paint(&self, frame: &mut Frame, app: &App, slot: Rect, row: usize, height: u16) {
        let look = app.look();
        if self.split {
            let timer_cells = app.pane_resize.size(
                crate::action::ResizeBoundary::OverviewLowerColumns,
                slot.width.saturating_sub(1),
                TIMER_CELLS,
                32,
                DIGEST_MIN,
            );
            let timer = Rect {
                width: timer_cells,
                ..slot
            };
            self.timer(frame, app, timer, row, look);
            divider::paint(
                frame.buffer_mut(),
                Rect::new(slot.x + timer_cells, slot.y, 1, 1),
                row == 0,
                look,
            );
            let area = Rect::new(
                slot.x + timer_cells + 1,
                slot.y,
                slot.width.saturating_sub(timer_cells + 1),
                1,
            );
            self.digest(
                frame,
                area,
                row,
                app.focused() == crate::config::Pane::Rows,
                look,
            );
        } else {
            let timer_height = self.timer_height(app, height);
            if row < timer_height {
                self.timer(frame, app, slot, row, look);
            } else if row > timer_height {
                self.digest(
                    frame,
                    slot,
                    row - timer_height - 1,
                    app.focused() == crate::config::Pane::Rows,
                    look,
                );
            }
        }
    }
    fn timer(&self, frame: &mut Frame, app: &App, slot: Rect, row: usize, look: Look) {
        if row == 0 {
            section_rule::paint(
                frame.buffer_mut(),
                slot,
                self.timers.title,
                Line::styled(self.timers.note.clone(), look.role(Role::Muted)),
                app.focused() == crate::config::Pane::Rows,
                look,
            );
        } else if row <= self.timers.rows.len().max(1) {
            if let Some(timer) = self.timers.rows.get(row - 1) {
                strip::paint_left(
                    frame.buffer_mut(),
                    slot,
                    crate::board::search_field::highlight_line(
                        timer_rows::line(timer, usize::from(slot.width), look),
                        &app.workspace.query,
                    ),
                );
            } else {
                empty_state::paint(
                    frame.buffer_mut(),
                    Rect::new(
                        slot.x.saturating_add(1),
                        slot.y,
                        slot.width.saturating_sub(1),
                        1,
                    ),
                    self.timers.empty,
                    None,
                    look,
                );
            }
        } else if row == self.timers.height() - 1 {
            use crate::action::{OpsUiAction, Verb};
            let bindings = app.bindings();
            let key = if app.input.is_none() && !app.modal_open() && !app.loading() {
                bindings
                    .get("c")
                    .filter(|action| action.verb == Verb::Ui(OpsUiAction::AllTimers))
                    .map(|_| "c")
                    .or_else(|| {
                        bindings
                            .iter()
                            .find(|(_, action)| action.verb == Verb::Ui(OpsUiAction::AllTimers))
                            .map(|(key, _)| key.as_str())
                    })
            } else {
                None
            };
            let footer = timer_rows::footer(
                self.timers.total.saturating_sub(self.timers.rows.len()),
                key,
                usize::from(slot.width),
                look,
            );
            let width = footer.width().min(usize::from(slot.width)) as u16;
            strip::paint_left(frame.buffer_mut(), slot, footer);
            if width > 1 && app.cron.cron.is_some() && !app.modal_open() && !app.loading() {
                app.hits.borrow_mut().push(Hit {
                    x: slot.x + 1,
                    y: slot.y,
                    width: width - 1,
                    target: HitTarget::AllRoomTimers,
                });
            }
        }
    }
    fn digest(&self, frame: &mut Frame, slot: Rect, row: usize, focused: bool, look: Look) {
        let Some(metrics) = &self.digest else {
            return;
        };
        if row == 0 {
            section_rule::paint(
                frame.buffer_mut(),
                slot,
                "Digest",
                Line::default(),
                focused,
                look,
            );
        } else if let Some(metric) = metrics.get(row - 1) {
            metric.paint(frame.buffer_mut(), slot, look);
        }
    }
}

#[cfg(test)]
mod tests;
