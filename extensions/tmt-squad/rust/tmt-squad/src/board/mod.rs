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

pub fn run(core: Core, squad: Option<String>) -> Result<(), SquadError> {
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
            if terminal::stopped(&stop) {
                return Ok(());
            }
            if event::poll(INPUT_WAIT)?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                match app.key(key) {
                    Effect::Quit => return Ok(()),
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
    let restored = guard.restore();
    result.and(restored).map_err(failed)
}
