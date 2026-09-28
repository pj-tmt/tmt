//! `tmt squad board`: the terminal board. It reads the same status document as
//! `tmt squad status`, paints from it, and reloads in the background.

mod app;
mod notes;
mod refresh;
mod terminal;
mod view;

use crate::core::{Core, SquadError};
use app::{App, Effect, Snapshot};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::event::{self, Event, KeyEventKind},
};
use std::{
    io,
    sync::{
        atomic::AtomicUsize,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

const REFRESH: Duration = Duration::from_secs(5);
const INPUT_WAIT: Duration = Duration::from_millis(200);
/// Reported when input ends without a signal: the terminal is gone.
const HANGUP: i32 = signal_hook::consts::SIGHUP;

fn failed(error: io::Error) -> SquadError {
    SquadError::new(
        "SQUAD_TERMINAL_FAILED",
        format!("The board could not use the terminal: {error}"),
    )
}

/// Conventional shell status for a signal-ended process, after the terminal
/// is restored: 143 for TERM, 129 for HUP; 0 when the user quit.
pub fn exit_status(signal: Option<i32>) -> u8 {
    signal.map_or(0, |signal| u8::try_from(128 + signal).unwrap_or(1))
}

/// Terminal input on its own thread. A blocking read can spin forever once the
/// terminal hangs up, so the board never waits on it directly; the thread ends
/// with the process, and a read error disconnects the channel.
fn spawn_input() -> Receiver<Event> {
    let (sender, events) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if sender.send(event).is_err() {
                break;
            }
        }
    });
    events
}

/// The board loop, independent of the real terminal. It always returns within
/// one input wait of a stop signal or a closed input, whatever the reader does.
fn session(
    app: &mut App,
    stop: &AtomicUsize,
    input: &Receiver<Event>,
    results: &Receiver<Snapshot>,
    request: impl Fn(Option<String>),
    mut draw: impl FnMut(&App) -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let mut refreshed = Instant::now();
    loop {
        while let Ok(snapshot) = results.try_recv() {
            app.apply(snapshot);
        }
        draw(app)?;
        if let Some(signal) = terminal::stop_signal(stop) {
            return Ok(Some(signal));
        }
        match input.recv_timeout(INPUT_WAIT) {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => match app.key(key) {
                Effect::Quit => return Ok(None),
                Effect::Load(squad) => {
                    request(Some(squad));
                    refreshed = Instant::now();
                }
                Effect::None => {}
            },
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Ok(Some(terminal::stop_signal(stop).unwrap_or(HANGUP)));
            }
        }
        if refreshed.elapsed() >= REFRESH {
            request(app.current.clone());
            refreshed = Instant::now();
        }
    }
}

/// Returns the signal that ended the board, if any.
pub fn run(core: Core, squad: Option<String>) -> Result<Option<i32>, SquadError> {
    terminal::restore_before_panic_reports();
    let stop = terminal::stop_requested().map_err(failed)?;
    let worker = refresh::Worker::spawn(core);
    worker.request(squad.clone());
    let mut app = App::new(squad);
    let mut guard = terminal::Guard::enter(terminal::Crossterm).map_err(failed)?;
    let mut screen = Terminal::new(CrosstermBackend::new(io::stdout())).map_err(failed)?;
    let input = spawn_input();
    let result = session(
        &mut app,
        &stop,
        &input,
        &worker.results,
        |squad| worker.request(squad),
        |app| screen.draw(|frame| view::render(frame, app)).map(|_| ()),
    );
    // Restore first, whatever happened; then report the session's outcome.
    let restored = guard.restore();
    let signal = result.map_err(failed)?;
    restored.map_err(failed)?;
    Ok(signal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::{
        atomic::Ordering,
        mpsc::{Sender, channel},
    };

    fn fixture() -> (
        App,
        AtomicUsize,
        Sender<Event>,
        Receiver<Event>,
        Receiver<Snapshot>,
    ) {
        let (keys, input) = channel();
        let (_results_sender, results) = channel();
        (App::new(None), AtomicUsize::new(0), keys, input, results)
    }

    #[test]
    fn quit_signal_and_lost_input_each_end_the_session() {
        let (mut app, stop, keys, input, results) = fixture();
        keys.send(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )))
        .unwrap();
        let outcome = session(&mut app, &stop, &input, &results, |_| {}, |_| Ok(()));
        assert_eq!(outcome.unwrap(), None);

        let (mut app, stop, _keys, input, results) = fixture();
        stop.store(signal_hook::consts::SIGTERM as usize, Ordering::Relaxed);
        let outcome = session(&mut app, &stop, &input, &results, |_| {}, |_| Ok(()));
        assert_eq!(outcome.unwrap(), Some(signal_hook::consts::SIGTERM));

        // A hung-up terminal can leave the reader spinning without events:
        // a disconnected input ends the session instead of waiting forever.
        let (mut app, stop, keys, input, results) = fixture();
        drop(keys);
        let started = Instant::now();
        let outcome = session(&mut app, &stop, &input, &results, |_| {}, |_| Ok(()));
        assert_eq!(outcome.unwrap(), Some(HANGUP));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_failed_redraw_ends_the_session_with_its_error() {
        let (mut app, stop, _keys, input, results) = fixture();
        let outcome = session(
            &mut app,
            &stop,
            &input,
            &results,
            |_| {},
            |_| Err(io::Error::other("terminal gone")),
        );
        assert_eq!(outcome.unwrap_err().to_string(), "terminal gone");
    }

    #[test]
    fn a_signal_is_seen_within_one_wait_even_while_input_is_silent() {
        let (mut app, stop, _keys, input, results) = fixture();
        let stop = std::sync::Arc::new(stop);
        let setter = std::sync::Arc::clone(&stop);
        let signaller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            setter.store(signal_hook::consts::SIGHUP as usize, Ordering::Relaxed);
        });
        let started = Instant::now();
        let outcome = session(&mut app, &stop, &input, &results, |_| {}, |_| Ok(()));
        signaller.join().unwrap();
        assert_eq!(outcome.unwrap(), Some(signal_hook::consts::SIGHUP));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
