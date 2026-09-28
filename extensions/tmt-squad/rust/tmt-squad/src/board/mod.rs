//! `tmt squad board`: the terminal board. It reads the same status document as
//! `tmt squad status`, paints from it, and reloads in the background.

mod app;
mod refresh;
mod terminal;
mod view;

use crate::core::{Core, SquadError};
use app::{App, Effect};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::event::{self, Event, KeyEventKind},
};
use std::{
    io,
    time::{Duration, Instant},
};

const REFRESH: Duration = Duration::from_secs(5);
const INPUT_WAIT: Duration = Duration::from_millis(200);

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

/// Returns the signal that ended the board, if any.
pub fn run(core: Core, squad: Option<String>) -> Result<Option<i32>, SquadError> {
    terminal::restore_before_panic_reports();
    let stop = terminal::stop_requested().map_err(failed)?;
    let worker = refresh::Worker::spawn(core);
    worker.request(squad.clone());
    let mut app = App::new(squad);
    let mut guard = terminal::Guard::enter(terminal::Crossterm).map_err(failed)?;
    let mut screen = Terminal::new(CrosstermBackend::new(io::stdout())).map_err(failed)?;
    let mut refreshed = Instant::now();
    let result = (|| {
        loop {
            while let Ok(snapshot) = worker.results.try_recv() {
                app.apply(snapshot);
            }
            screen.draw(|frame| view::render(frame, &app))?;
            if let Some(signal) = terminal::stop_signal(&stop) {
                return Ok(Some(signal));
            }
            if event::poll(INPUT_WAIT)?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                match app.key(key) {
                    Effect::Quit => return Ok(None),
                    Effect::Load(squad) => {
                        worker.request(Some(squad));
                        refreshed = Instant::now();
                    }
                    Effect::None => {}
                }
            }
            if refreshed.elapsed() >= REFRESH {
                worker.request(app.current.clone());
                refreshed = Instant::now();
            }
        }
    })();
    // Restore first, whatever happened; then report the loop's outcome.
    let restored = guard.restore();
    let signal = result.map_err(failed)?;
    restored.map_err(failed)?;
    Ok(signal)
}
