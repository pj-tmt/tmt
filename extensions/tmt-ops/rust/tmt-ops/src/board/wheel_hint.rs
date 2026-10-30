//! A root `WheelUpPane` binding that enters copy-mode without asking whether the pane
//! wants the mouse sends wheel-up to tmux's scrollback instead of the board; wheel-down
//! has no root binding, so it still reaches the board. Inside tmux the board reads that
//! binding once at start, read only, and says so in one notice line. The fix is the
//! user's own tmux.conf edit, described in the tmt-ops skill; Ops never changes tmux.

use std::path::Path;

/// The notice line; the tmt-ops skill's "tmux wheel" section names the fix.
pub(super) const HINT: &str =
    "Wheel-up may scroll tmux, not the board · see tmt-ops skill: tmux wheel";

/// Whether a `list-keys -T root WheelUpPane` listing can send wheel-up to copy-mode
/// over the board: it names `copy-mode` but tests neither `alternate_on` (the board's
/// screen) nor `mouse_any_flag` (its mouse request). tmux's own defaults test one.
pub(super) fn escapes(listing: &str) -> bool {
    listing.lines().any(|line| {
        line.contains("WheelUpPane")
            && line.contains("copy-mode")
            && !line.contains("alternate_on")
            && !line.contains("mouse_any_flag")
    })
}

/// Reads the binding on its own thread, so a slow or failed read never delays a frame
/// and stays silent; `report` runs only for a binding that escapes.
pub(super) fn spawn(socket: String, report: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        if crate::effects::tmux_root_binding(Path::new("tmux"), &socket, "WheelUpPane")
            .is_some_and(|listing| escapes(&listing))
        {
            report();
        }
    });
}

impl super::App {
    /// The read found an escaping binding: the hint takes the notice line once, and
    /// only while no other notice holds it, so it never replaces a migration, reload
    /// or action notice.
    pub(super) fn wheel_up_escapes(&mut self) {
        if self.notice.is_none() && self.migration_notice.is_none() {
            self.set_notice(Some(HINT.into()), false);
        }
    }
}

#[cfg(test)]
mod tests;
