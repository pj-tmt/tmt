//! The Timers module of the Measure column: the soonest timers and the link to all of them.
use crate::{
    board::{
        app::{App, Hit, HitTarget},
        empty_state,
        overview::timers,
        section_rule, timer_rows,
    },
    look::Look,
};
use ratatui::{Frame, layout::Rect, text::Line};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

pub(in crate::board) struct Band {
    timers: timers::Model,
    pub height: usize,
}
impl Band {
    pub(super) fn timers(app: &App) -> Self {
        let timers = timers::project(app);
        let height = timers.height();
        Self { timers, height }
    }
    /// How many timers the band's scope holds.
    pub fn total(&self) -> usize {
        self.timers.total
    }
    pub fn paint(&self, frame: &mut Frame, app: &App, slot: Rect, row: usize, _height: u16) {
        self.timer(frame, app, slot, row, app.look());
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
}
