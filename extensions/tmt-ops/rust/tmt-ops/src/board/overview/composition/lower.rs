//! Narrow Timers/Digest placement within the primary scroll owner's row stream.
use crate::{
    board::{
        app::{App, Hit, HitTarget},
        m3_section_rule, m9_timer_rows, m14_divider, m18_empty_state,
        overview::{digest, timers},
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
    pub fn new(app: &App, frame_width: u16, width: u16) -> Option<Self> {
        // S4 introduces the final wide right column directly as M19.
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
    pub fn paint(&self, frame: &mut Frame, app: &App, slot: Rect, row: usize) {
        let look = app.look();
        if self.split {
            let timer = Rect {
                width: TIMER_CELLS,
                ..slot
            };
            self.timer(frame, app, timer, row, look);
            m14_divider::paint(
                frame.buffer_mut(),
                Rect::new(slot.x + TIMER_CELLS, slot.y, 1, 1),
                row == 0,
                look,
            );
            let area = Rect::new(
                slot.x + TIMER_CELLS + 1,
                slot.y,
                slot.width.saturating_sub(TIMER_CELLS + 1),
                1,
            );
            self.digest(
                frame,
                area,
                row,
                app.focused() == crate::config::Pane::Rows,
                look,
            );
        } else if row < self.timers.height() {
            self.timer(frame, app, slot, row, look);
        } else if row > self.timers.height() {
            self.digest(
                frame,
                slot,
                row - self.timers.height() - 1,
                app.focused() == crate::config::Pane::Rows,
                look,
            );
        }
    }
    fn timer(&self, frame: &mut Frame, app: &App, slot: Rect, row: usize, look: Look) {
        if row == 0 {
            m3_section_rule::paint(
                frame.buffer_mut(),
                slot,
                self.timers.title,
                Line::styled(self.timers.note.clone(), look.role(Role::Muted)),
                app.focused() == crate::config::Pane::Rows,
                look,
            );
        } else if row <= self.timers.rows.len().max(1) {
            if let Some(timer) = self.timers.rows.get(row - 1) {
                m9_timer_rows::paint(frame.buffer_mut(), slot, timer, look);
            } else {
                m18_empty_state::paint(frame.buffer_mut(), slot, self.timers.empty, None, look);
            }
        } else if row == self.timers.height() - 1 {
            use crate::action::{OpsUiAction, Verb};
            let bindings = app.bindings();
            let key = bindings
                .get("c")
                .filter(|action| action.verb == Verb::Ui(OpsUiAction::AllTimers))
                .map(|_| "c")
                .or_else(|| {
                    bindings
                        .iter()
                        .find(|(_, action)| action.verb == Verb::Ui(OpsUiAction::AllTimers))
                        .map(|(key, _)| key.as_str())
                });
            let footer = m9_timer_rows::footer(
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
            m3_section_rule::paint(
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
