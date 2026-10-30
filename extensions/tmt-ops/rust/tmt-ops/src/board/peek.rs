//! One close rule for every overlay (`design/cli-style.md#dialogs`): Esc or a press outside the
//! open overlay closes it through [`App::close_overlay`]. A peek (a window that only looks at the
//! board) lets the outside press land on what was under it; any other dialog takes the press. A
//! dialog whose operation is still running hides instead, and the operation keeps going.

use super::app::{App, Effect, Overlay};
use crate::action::{OpsUiAction, Verb};
use ratatui::{
    crossterm::event::{MouseButton, MouseEvent, MouseEventKind},
    layout::Rect,
};
use std::cell::Cell;

/// One slot per overlay, indexed by the overlay's position in its enum.
const OVERLAYS: usize = Overlay::PastePreview as usize + 1;

/// Where each overlay and the menu were last painted. Cleared at the start of every paint, so a
/// window that has not been painted since a resize or a change never closes on a stale rectangle.
#[derive(Default)]
pub(super) struct PeekAreas {
    overlays: Cell<[Option<Rect>; OVERLAYS]>,
    menu: Cell<Option<Rect>>,
}

impl PeekAreas {
    pub fn clear(&self) {
        self.overlays.set([None; OVERLAYS]);
        self.menu.set(None);
    }

    /// Records where an overlay painted its sheet.
    pub fn set(&self, overlay: Overlay, area: Option<Rect>) {
        let mut areas = self.overlays.get();
        areas[overlay as usize] = area;
        self.overlays.set(areas);
    }

    pub fn set_menu(&self, area: Rect) {
        self.menu.set(Some(area));
    }

    #[cfg(test)]
    pub fn area(&self, overlay: Overlay) -> Option<Rect> {
        self.overlays.get()[overlay as usize]
    }

    fn outside(area: Option<Rect>, event: MouseEvent) -> bool {
        area.is_some_and(|area| !area.contains((event.column, event.row).into()))
    }
}

impl Overlay {
    /// A peek only looks at the board, so a press outside it also reaches the board.
    pub(super) fn peeks(self) -> bool {
        matches!(
            self,
            Self::PastePreview
                | Self::CronList
                | Self::Help
                | Self::QuotaDetails
                | Self::EstimateBasis
                | Self::MonitorDetails
        )
    }
}

impl App {
    /// Closes the menu or the open overlay on a left press outside where it was painted, also
    /// while a composer is open. `Some` means the press was taken; `None` lets it go on, to what
    /// is under a closed peek or menu.
    pub(super) fn outside_press(&mut self, event: MouseEvent) -> Option<Effect> {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        if self.menu.is_some() {
            if PeekAreas::outside(self.peek_areas.menu.get(), event) {
                self.menu = None;
            }
            return None;
        }
        let overlay = self.overlay()?;
        if !PeekAreas::outside(self.peek_areas.overlays.get()[overlay as usize], event) {
            return None;
        }
        let effect = self.close_overlay(overlay);
        // Under an open composer a peek's press is taken, so it never lands on a row and drops the
        // draft. Only a press on the composer itself goes on, as from its paste preview.
        let on_composer = self
            .input_band
            .get()
            .is_some_and(|band| band.contains((event.column, event.row).into()));
        let lands = overlay.peeks() && (self.input.is_none() || on_composer);
        (!lands).then_some(effect)
    }

    /// Closes `overlay` without doing what it was asked to do. A dialog whose operation is
    /// running hides and says so on the status line; reopening it shows the operation's result.
    pub(super) fn close_overlay(&mut self, overlay: Overlay) -> Effect {
        match overlay {
            Overlay::PastePreview => {
                self.composer.paste_preview.get_mut().take();
            }
            Overlay::MonitorDetails => self.monitor_details = None,
            Overlay::CronReview => self.cron_review = None,
            Overlay::Confirm => self.confirm = None,
            Overlay::Workspace => {
                self.workspace_close();
                self.input = None;
            }
            Overlay::Recovery => {
                if let Some(recovery) = &mut self.recovery {
                    recovery.close();
                }
            }
            Overlay::ReplyReader => {
                self.row_details.reader = None;
                self.reconcile_row_details();
            }
            Overlay::Checklist => return self.checklist_close(),
            Overlay::UsageControls => {
                self.usage_controls = false;
                self.usage_chart.get_mut().release_focus();
                self.invalidate_frame_geometry();
            }
            Overlay::QuotaDetails => self.quota_details = None,
            Overlay::EstimateBasis => {
                self.estimate_basis = None;
                self.invalidate_frame_geometry();
            }
            Overlay::Unread => {
                self.unread_overlay = None;
                self.unread.displayed.clear();
            }
            Overlay::SwitchTool => return self.switch_tool_close(),
            Overlay::IdleAlert => return self.idle_alert_close(),
            Overlay::NewMember => return self.new_member_close(),
            Overlay::MemberSession => self.member_session = None,
            Overlay::Notes => {
                if let Some(notes) = self.notes_overlay.take() {
                    notes.close(&mut self.notes);
                }
            }
            Overlay::Discovery => {
                if let Some(discovery) = &mut self.discovery {
                    discovery.close();
                }
            }
            Overlay::Help => self.help = false,
            Overlay::Settings => {
                self.settings_preview();
                self.settings = None;
                return Effect::CancelSettings;
            }
            Overlay::Panels => {
                self.panels_picker = None;
                self.invalidate_frame_geometry();
            }
            Overlay::Theme => self.theme_picker = None,
            Overlay::CronList => self.cron_list = None,
        }
        Effect::None
    }

    /// `"<key> reopens <what> or its result."` for the key bound to `action`, preferring
    /// `default`, or `"<reach> reopens …"` when no key is bound.
    pub(super) fn reopen_hint(
        &self,
        action: OpsUiAction,
        default: &str,
        what: &str,
        reach: &str,
    ) -> String {
        let bindings = self.bindings();
        let action = Verb::Ui(action);
        let key = bindings
            .get(default)
            .filter(|a| a.verb == action)
            .map(|_| default)
            .or_else(|| {
                bindings
                    .iter()
                    .find(|(_, a)| a.verb == action)
                    .map(|(key, _)| key.as_str())
            });
        match key {
            Some(key) => format!(
                "{} reopens {what} or its result.",
                super::view::footer::key_label(key)
            ),
            None => format!("{reach} reopens {what} or its result."),
        }
    }
}
