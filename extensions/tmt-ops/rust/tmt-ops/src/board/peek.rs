//! Light dismiss: a click outside a window that only peeks at the board closes it, and the same
//! click lands on what was under it. A window that holds a draft or a pending choice is a dialog
//! and is not listed here.

use super::app::App;
use ratatui::{crossterm::event::MouseEvent, layout::Rect};
use std::cell::Cell;

/// The windows that light-dismiss.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Peek {
    Menu,
    Timers,
    Help,
    Quota,
    Monitor,
    Paste,
}

/// Where each peek window was last painted. Cleared at the start of every paint, so a window that
/// has not been painted since a resize or a change never dismisses on a stale rectangle.
#[derive(Default)]
pub(super) struct PeekAreas(Cell<[Option<Rect>; 6]>);

impl PeekAreas {
    pub fn clear(&self) {
        self.0.set([None; 6]);
    }

    pub fn set(&self, peek: Peek, area: Rect) {
        let mut areas = self.0.get();
        areas[peek as usize] = Some(area);
        self.0.set(areas);
    }

    fn outside(&self, peek: Peek, event: MouseEvent) -> bool {
        self.0.get()[peek as usize]
            .is_some_and(|area| !area.contains((event.column, event.row).into()))
    }
}

impl App {
    /// Closes the open peek window on an outside left press. Only the read-only paste
    /// preview can dismiss while its underlying composer holds a draft.
    /// True means the window closed and the press goes on to what is under it.
    pub(super) fn light_dismiss(&mut self, event: MouseEvent) -> bool {
        use super::app::Overlay;
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return false;
        }
        let peek = match self.overlay() {
            Some(Overlay::PastePreview) => Peek::Paste,
            Some(Overlay::CronList) => Peek::Timers,
            Some(Overlay::Help) => Peek::Help,
            Some(Overlay::QuotaDetails) => Peek::Quota,
            Some(Overlay::MonitorDetails) => Peek::Monitor,
            Some(_) => return false,
            None if self.menu.is_some() => Peek::Menu,
            None => return false,
        };
        if self.input.is_some() && peek != Peek::Paste {
            return false;
        }
        if !self.peek_areas.outside(peek, event) {
            return false;
        }
        match peek {
            Peek::Menu => self.menu = None,
            Peek::Timers => self.cron_list = None,
            Peek::Help => self.help = false,
            Peek::Quota => self.quota_details = None,
            Peek::Monitor => self.monitor_details = None,
            Peek::Paste => self.composer.paste_preview = None,
        }
        true
    }
}
