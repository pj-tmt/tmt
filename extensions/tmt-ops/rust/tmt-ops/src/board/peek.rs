//! Light dismiss: a click outside a window that only peeks at the board closes it, and the same
//! click lands on what was under it. Every other centered overlay is a dialog: a click outside it
//! closes it the way Esc does, drops what it holds unsent, and does not land on what was under it.

use super::app::{App, Effect, Overlay};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent},
    layout::Rect,
};
use std::cell::Cell;

/// One slot per overlay, indexed by the overlay's position in its enum.
const OVERLAYS: usize = Overlay::PastePreview as usize + 1;
/// A dialog can hold a form over a list over a confirmation; this many Esc presses leave them all.
const MAX_ESC: usize = 4;

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
pub(super) struct PeekAreas(Cell<[Option<Rect>; 6]>, Cell<[Option<Rect>; OVERLAYS]>);

impl PeekAreas {
    pub fn clear(&self) {
        self.0.set([None; 6]);
        self.1.set([None; OVERLAYS]);
    }

    /// Records where a dialog overlay painted its sheet.
    pub fn set_dialog(&self, overlay: Overlay, area: Option<Rect>) {
        let mut areas = self.1.get();
        areas[overlay as usize] = area;
        self.1.set(areas);
    }

    #[cfg(test)]
    pub fn dialog(&self, overlay: Overlay) -> Option<Rect> {
        self.1.get()[overlay as usize]
    }

    fn dialog_outside(&self, overlay: Overlay, event: MouseEvent) -> bool {
        self.1.get()[overlay as usize]
            .is_some_and(|area| !area.contains((event.column, event.row).into()))
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
            Peek::Paste => {
                self.composer.paste_preview.get_mut().take();
            }
        }
        true
    }

    /// Closes the open dialog on an outside left press, as Esc would, and takes the press.
    /// Esc is pressed until the dialog is gone, so a delete question is dropped on the way; the
    /// Checklist form that Esc keeps is dropped first. None when the press is not an outside press on a painted dialog.
    pub(super) fn dialog_dismiss(&mut self, event: MouseEvent) -> Option<Effect> {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        if event.kind != MouseEventKind::Down(MouseButton::Left) || self.menu.is_some() {
            return None;
        }
        let overlay = self.overlay()?;
        if matches!(
            overlay,
            Overlay::PastePreview
                | Overlay::CronList
                | Overlay::Help
                | Overlay::QuotaDetails
                | Overlay::MonitorDetails
        ) || !self.peek_areas.dialog_outside(overlay, event)
        {
            return None;
        }
        if overlay == Overlay::Notes
            && let Some(notes) = &self.notes_overlay
        {
            notes.discard_unsent(&mut self.notes);
        }
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let mut effect = Effect::None;
        for _ in 0..MAX_ESC {
            let pressed = self.overlay_event(&esc).unwrap_or(Effect::None);
            if matches!(effect, Effect::None) {
                effect = pressed;
            }
            if self.overlay() != Some(overlay) {
                break;
            }
        }
        Some(effect)
    }
}
