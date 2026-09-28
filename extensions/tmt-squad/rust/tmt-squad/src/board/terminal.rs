//! Terminal lifecycle: the board owns raw mode and the alternate screen only
//! between `Guard::enter` and restore, which also runs on error, panic and
//! TERM/HUP. A terminal left in raw mode is a defect, not a cosmetic issue.

use ratatui::crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::{
    io,
    sync::{
        Arc, Once,
        atomic::{AtomicUsize, Ordering},
    },
};

/// The terminal state the board changes. A trait so restore is testable.
pub trait Screen {
    fn enter(&mut self) -> io::Result<()>;
    fn leave(&mut self) -> io::Result<()>;
}

pub struct Crossterm;

impl Screen for Crossterm {
    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)
    }

    fn leave(&mut self) -> io::Result<()> {
        // Try every step even if one fails; report the first failure.
        let screen = execute!(io::stdout(), LeaveAlternateScreen, Show);
        let raw = disable_raw_mode();
        screen.and(raw)
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
            let _ = Crossterm.leave();
            previous(info);
        }));
    });
}

/// TERM and HUP record their signal number; the board loop then exits,
/// restores the terminal and reports it. Raw mode delivers Ctrl-C as a key.
pub fn stop_requested() -> io::Result<Arc<AtomicUsize>> {
    let flag = Arc::new(AtomicUsize::new(0));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
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
    }

    impl Screen for Fake {
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
