//! Terminal lifecycle: the board owns raw mode, mouse capture and the
//! alternate screen only between `Guard::enter` and restore, which also runs
//! on error, panic and TERM/HUP. A terminal left in raw mode is a defect, not a cosmetic issue.

pub(super) mod background;

use ratatui::crossterm::{
    cursor::{Hide, Show},
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use std::{
    io,
    sync::{
        Arc, Once,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// The terminal state the board changes. A trait so restore is testable.
pub trait Screen {
    fn enter(&mut self) -> io::Result<()>;
    fn leave(&mut self) -> io::Result<()>;
    fn enable_keyboard_enhancement(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn query_background(&mut self) -> io::Result<tmt_cli_style::theme::background::QueryReply> {
        Ok(tmt_cli_style::theme::background::QueryReply {
            background: None,
            received: Vec::new(),
            reply: None,
        })
    }
}

/// Asks again for the mouse reports (1000/1002/1003/1015/1006). Setting a mode
/// that is already set changes nothing, so it is safe at any time; it repairs a
/// request a multiplexer, a nested client or an emulator reset has dropped, which
/// would otherwise leave the wheel to the surrounding tmux pane (copy-mode).
pub fn reassert_mouse(output: &mut impl io::Write) -> io::Result<()> {
    execute!(output, EnableMouseCapture)
}

/// Only the terminal owner changes this flag; the panic hook needs the same
/// ownership evidence as ordinary cleanup so it never pops an unowned stack.
static KEYBOARD_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Same ownership rule for the modifyOtherKeys request tmux needs to tell
/// Shift+Enter from Enter.
static MODIFY_OTHER_KEYS_ACTIVE: AtomicBool = AtomicBool::new(false);

/// `CSI > 4 ; 1 m` asks for keys without a legacy encoding only (Shift+Enter,
/// Ctrl+Enter, Shift+Tab); Alt+Enter and Ctrl+letters keep their old bytes.
/// `CSI > 4 ; 0 m` releases the request.
fn write_modify_other_keys(output: &mut impl io::Write, on: bool) -> io::Result<()> {
    output.write_all(if on { b"\x1b[>4;1m" } else { b"\x1b[>4;0m" })?;
    output.flush()
}

/// Inside tmux only: one read of the server's options at start. A failure or
/// any other setting means no request.
fn tmux_shift_enter() -> bool {
    let Some(socket) = crate::effects::tmux_socket() else {
        return false;
    };
    let option = |name| crate::effects::tmux_option(&socket, name).unwrap_or_default();
    crate::effects::tmux_reports_shift_enter(
        &option("extended-keys"),
        &option("extended-keys-format"),
    )
}

#[derive(Default)]
pub struct Crossterm {
    enhanced: bool,
    /// tmux was asked for Shift+Enter (see `tmux_shift_enter`).
    shift_enter: bool,
    output: Option<io::Stdout>,
}

fn push_keyboard(output: &mut io::Stdout) -> io::Result<()> {
    // Mark ownership before writing: partial output also needs cleanup.
    KEYBOARD_ACTIVE.store(true, Ordering::Relaxed);
    execute!(
        output,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )
}

fn request_shift_enter(output: &mut io::Stdout) -> io::Result<()> {
    // Mark ownership before writing: partial output also needs cleanup.
    MODIFY_OTHER_KEYS_ACTIVE.store(true, Ordering::Relaxed);
    write_modify_other_keys(output, true)
}

impl Screen for Crossterm {
    fn enable_keyboard_enhancement(&mut self) -> io::Result<bool> {
        // A missing/unsupported reply is a capability observation, not a failure.
        // This runs before the input reader; crossterm queries share its queue.
        self.enhanced = supports_keyboard_enhancement().unwrap_or(false);
        if self.enhanced {
            push_keyboard(self.output.as_mut().expect("terminal entered"))?;
        } else if tmux_shift_enter() {
            self.shift_enter = true;
            request_shift_enter(self.output.as_mut().expect("terminal entered"))?;
        }
        Ok(self.enhanced || self.shift_enter)
    }

    fn query_background(&mut self) -> io::Result<tmt_cli_style::theme::background::QueryReply> {
        background::query()
    }

    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let output = self.output.get_or_insert_with(io::stdout);
        execute!(
            output,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableFocusChange,
            EnableBracketedPaste,
            Hide
        )?;
        if self.enhanced {
            push_keyboard(output)?;
        } else if self.shift_enter {
            request_shift_enter(output)?;
        }
        Ok(())
    }

    fn leave(&mut self) -> io::Result<()> {
        // Try every cleanup group even if one fails; report the first failure.
        let keyboard = if KEYBOARD_ACTIVE.swap(false, Ordering::Relaxed) {
            execute!(io::stdout(), PopKeyboardEnhancementFlags)
        } else {
            Ok(())
        };
        let keyboard = if MODIFY_OTHER_KEYS_ACTIVE.swap(false, Ordering::Relaxed) {
            keyboard.and(write_modify_other_keys(&mut io::stdout(), false))
        } else {
            keyboard
        };
        let screen = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableFocusChange,
            DisableMouseCapture,
            LeaveAlternateScreen,
            Show
        );
        let raw = disable_raw_mode();
        keyboard.and(screen).and(raw)
    }
}

/// Restores the screen exactly once: on `restore`, or when dropped, which
/// includes unwinding from a panic.
pub struct Guard<S: Screen> {
    screen: S,
    active: bool,
}

impl<S: Screen> Guard<S> {
    pub fn enter(mut screen: S) -> io::Result<Self> {
        if let Err(error) = screen.enter() {
            // Undo a partial entry (for example raw mode without the screen).
            let _ = screen.leave();
            return Err(error);
        }
        Ok(Self {
            screen,
            active: true,
        })
    }

    pub fn enable_keyboard_enhancement(&mut self) -> io::Result<bool> {
        self.screen.enable_keyboard_enhancement()
    }

    /// Restore before stopping, then re-enter with the already observed
    /// capability. The input reader may still exist, so resumption never queries.
    pub fn suspend(&mut self, pause: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        self.restore()?;
        let paused = pause();
        if let Err(error) = self.screen.enter() {
            let _ = self.screen.leave();
            return Err(error);
        }
        self.active = true;
        paused
    }

    pub fn query_background(&mut self) -> io::Result<tmt_cli_style::theme::background::QueryReply> {
        self.screen.query_background()
    }

    pub fn restore(&mut self) -> io::Result<()> {
        if !std::mem::replace(&mut self.active, false) {
            return Ok(());
        }
        self.screen.leave()
    }
}

impl<S: Screen> Drop for Guard<S> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// The panic message would otherwise be printed into the alternate screen and
/// lost; restore first, then let the previous hook report it.
pub fn restore_before_panic_reports() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = Crossterm::default().leave();
            previous(info);
        }));
    });
}

/// Record signals for the board loop: TERM/HUP restore and exit; TSTP restores,
/// pauses and resumes the same board. Raw mode delivers Ctrl-C as a key.
pub fn stop_requested() -> io::Result<Arc<AtomicUsize>> {
    let flag = Arc::new(AtomicUsize::new(0));
    for signal in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGTSTP,
    ] {
        signal_hook::flag::register_usize(signal, Arc::clone(&flag), signal as usize)?;
    }
    Ok(flag)
}

pub fn stop_signal(flag: &AtomicUsize) -> Option<i32> {
    match flag.load(Ordering::Relaxed) {
        0 => None,
        signal => i32::try_from(signal).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Clone, Default)]
    struct Fake {
        log: Rc<RefCell<Vec<&'static str>>>,
        fail_enter: bool,
        fail_query: bool,
        fail_keyboard: bool,
    }

    impl Screen for Fake {
        fn enable_keyboard_enhancement(&mut self) -> io::Result<bool> {
            self.log.borrow_mut().push("keyboard");
            if self.fail_keyboard {
                Err(io::Error::other("keyboard failed"))
            } else {
                Ok(true)
            }
        }
        fn query_background(&mut self) -> io::Result<tmt_cli_style::theme::background::QueryReply> {
            self.log.borrow_mut().push("query");
            if self.fail_query {
                return Err(io::Error::other("query failed"));
            }
            Ok(tmt_cli_style::theme::background::QueryReply {
                background: None,
                received: Vec::new(),
                reply: None,
            })
        }
        fn enter(&mut self) -> io::Result<()> {
            self.log.borrow_mut().push("enter");
            if self.fail_enter {
                return Err(io::Error::other("no terminal"));
            }
            Ok(())
        }
        fn leave(&mut self) -> io::Result<()> {
            self.log.borrow_mut().push("leave");
            Ok(())
        }
    }

    #[test]
    fn the_modify_other_keys_request_is_mode_one_and_its_reset_is_mode_zero() {
        let mut on = Vec::new();
        write_modify_other_keys(&mut on, true).unwrap();
        assert_eq!(on, b"\x1b[>4;1m");
        let mut off = Vec::new();
        write_modify_other_keys(&mut off, false).unwrap();
        assert_eq!(off, b"\x1b[>4;0m");
    }

    #[test]
    fn reasserting_the_mouse_sends_the_same_request_as_entering() {
        let mut sent = Vec::new();
        reassert_mouse(&mut sent).unwrap();
        let mut entered = Vec::new();
        execute!(&mut entered, EnableMouseCapture).unwrap();
        assert_eq!(sent, entered);
        let text = String::from_utf8(sent).unwrap();
        // Button and drag reports plus the SGR encoding the wheel arrives in.
        for mode in ["1000", "1002", "1006"] {
            assert!(text.contains(&format!("\x1b[?{mode}h")), "{text:?}");
        }
    }

    #[test]
    fn restore_runs_once_on_return_drop_and_panic() {
        let fake = Fake::default();
        let mut guard = Guard::enter(fake.clone()).unwrap();
        guard.restore().unwrap();
        drop(guard);
        assert_eq!(*fake.log.borrow(), ["enter", "leave"]);

        let fake = Fake::default();
        let inside = fake.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = Guard::enter(inside).unwrap();
            panic!("board failure");
        }));
        assert!(result.is_err());
        assert_eq!(*fake.log.borrow(), ["enter", "leave"], "unwinding restores");
    }

    #[test]
    fn query_runs_inside_the_guard_and_failure_still_restores() {
        for fail_query in [false, true] {
            let fake = Fake {
                fail_query,
                ..Fake::default()
            };
            let mut guard = Guard::enter(fake.clone()).unwrap();
            assert_eq!(guard.query_background().is_err(), fail_query);
            drop(guard);
            assert_eq!(*fake.log.borrow(), ["enter", "query", "leave"]);
        }
    }

    #[test]
    fn suspend_restores_then_reenters_without_repeating_capability_queries() {
        for pause_failed in [false, true] {
            let fake = Fake::default();
            let log = fake.log.clone();
            let mut guard = Guard::enter(fake.clone()).unwrap();
            assert!(guard.enable_keyboard_enhancement().unwrap());
            let result = guard.suspend(|| {
                log.borrow_mut().push("pause");
                if pause_failed {
                    Err(io::Error::other("stop failed"))
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.is_err(), pause_failed);
            drop(guard);
            assert_eq!(
                *fake.log.borrow(),
                ["enter", "keyboard", "leave", "pause", "enter", "leave"]
            );
        }
    }

    #[test]
    fn failed_keyboard_setup_and_resume_both_restore_partial_terminal_state() {
        let fake = Fake {
            fail_keyboard: true,
            ..Fake::default()
        };
        let mut guard = Guard::enter(fake.clone()).unwrap();
        assert!(guard.enable_keyboard_enhancement().is_err());
        drop(guard);
        assert_eq!(*fake.log.borrow(), ["enter", "keyboard", "leave"]);

        let fake = Fake::default();
        let mut guard = Guard::enter(fake.clone()).unwrap();
        guard.screen.fail_enter = true;
        assert!(guard.suspend(|| Ok(())).is_err());
        drop(guard);
        assert_eq!(*fake.log.borrow(), ["enter", "leave", "enter", "leave"]);
    }

    #[test]
    fn a_failed_entry_is_undone_and_reported() {
        let fake = Fake {
            fail_enter: true,
            ..Fake::default()
        };
        assert!(Guard::enter(fake.clone()).is_err());
        assert_eq!(*fake.log.borrow(), ["enter", "leave"]);
    }

    #[test]
    fn term_and_hup_are_recorded_for_the_exit_status() {
        let flag = stop_requested().unwrap();
        assert_eq!(stop_signal(&flag), None);
        signal_hook::low_level::raise(signal_hook::consts::SIGHUP).unwrap();
        assert_eq!(stop_signal(&flag), Some(signal_hook::consts::SIGHUP));
        signal_hook::low_level::raise(signal_hook::consts::SIGTERM).unwrap();
        assert_eq!(stop_signal(&flag), Some(signal_hook::consts::SIGTERM));
        assert_eq!(super::super::exit_status(stop_signal(&flag)), 143);
        assert_eq!(
            super::super::exit_status(Some(signal_hook::consts::SIGHUP)),
            129
        );
        assert_eq!(super::super::exit_status(None), 0);
    }
}
