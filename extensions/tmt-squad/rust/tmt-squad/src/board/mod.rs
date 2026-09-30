//! `tmt squad board`: the terminal board. It reads the same status document as
//! `tmt squad ls`, paints from it, and reloads in the background.

mod app;
mod markdown;
pub(crate) mod notes;
mod refresh;
mod scroll;
mod terminal;
mod view;

use crate::{
    back,
    core::{Core, SquadError},
    effects, send,
};
use app::{App, Effect, Request, Snapshot};
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

/// Carries out a resolved request. Nothing here reads the row or config.
/// Jump and back share the plain commands' path, including the per-client
/// back stack.
fn execute(core: &Core, request: Request) -> Result<String, String> {
    match request {
        Request::Jump(member) => back::jump(core, &member)
            .map(|(focus, warning)| match warning {
                None => format!("Showing {member} ({}).", focus.pane),
                Some(warning) => format!(
                    "Showing {member} ({}); back will not return here: {warning}",
                    focus.pane
                ),
            })
            .map_err(|error| error.message),
        Request::Back => match back::back(core) {
            Ok(Some(focus)) => Ok(format!("Back at {}.", focus.pane)),
            Ok(None) => Ok("Nothing to go back to.".into()),
            Err(error) => Err(error.message),
        },
        Request::Open { link, opener } => {
            effects::open(&link, opener.as_deref()).map(|()| format!("Opened {link}"))
        }
        Request::Copy { text, program } => {
            effects::copy(&text, program.as_deref(), effects::tmux_socket().as_deref())
                .map(|copied| copied.describe().to_owned())
        }
        Request::Run(argv) => effects::spawn(&argv).map(|()| format!("Started {}.", argv[0])),
        Request::Talk {
            me,
            squad,
            to,
            text,
        } => send::talk(core, &squad, &me, &to, &text)
            .map(|request| format!("Sent to {to} ({request})."))
            .map_err(|error| error.message),
        Request::Annotate {
            me,
            squad,
            to,
            row,
            text,
        } => send::annotate(core, &squad, &me, &to, &row, &text)
            .map(|request| format!("Note on {row} sent to {to} ({request})."))
            .map_err(|error| error.message),
        Request::Reply {
            me,
            request,
            from,
            text,
        } => send::answer(core, &me, &request, &from, &text)
            .map(|()| format!("Replied to {from}."))
            .map_err(|error| error.message),
    }
}

/// The board loop, independent of the real terminal. It always returns within
/// one input wait of a stop signal or a closed input, whatever the reader does.
fn session(
    app: &mut App,
    stop: &AtomicUsize,
    input: &Receiver<Event>,
    results: &Receiver<Snapshot>,
    request: impl Fn(Option<String>),
    mut act: impl FnMut(Request) -> Result<String, String>,
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
        let effect = match input.recv_timeout(INPUT_WAIT) {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => app.key(key),
            Ok(Event::Mouse(mouse)) => app.mouse(mouse, Instant::now()),
            Ok(_) | Err(RecvTimeoutError::Timeout) => Effect::None,
            Err(RecvTimeoutError::Disconnected) => {
                return Ok(Some(terminal::stop_signal(stop).unwrap_or(HANGUP)));
            }
        };
        match effect {
            Effect::Quit => return Ok(None),
            Effect::Load(squad) => {
                request(Some(squad));
                refreshed = Instant::now();
            }
            Effect::Refresh => {
                request(app.current.clone());
                refreshed = Instant::now();
            }
            Effect::Act(action) => {
                let sends = action.sends();
                let jump = matches!(action, Request::Jump(_));
                let outcome = act(action);
                let jumped = jump && outcome.is_ok();
                app.finished(outcome);
                // A popup has done its job once the user is at the member.
                if jumped && app.popup {
                    return Ok(None);
                }
                if sends {
                    request(app.current.clone());
                    refreshed = Instant::now();
                }
            }
            Effect::None => {}
        }
        // A squad that failed to load keeps retrying at the default.
        let interval = app
            .view
            .as_ref()
            .map_or(Some(crate::config::DEFAULT_REFRESH), |view| view.refresh);
        if interval.is_some_and(|interval| refreshed.elapsed() >= interval) {
            request(app.current.clone());
            refreshed = Instant::now();
        }
    }
}

/// Returns the signal that ended the board, if any.
/// `popup` closes the board after a successful jump, as a tmux popup should.
pub fn run(core: Core, squad: Option<String>, popup: bool) -> Result<Option<i32>, SquadError> {
    terminal::restore_before_panic_reports();
    let stop = terminal::stop_requested().map_err(failed)?;
    let worker = refresh::Worker::spawn(core.clone(), effects::tmux_socket().is_some());
    worker.request(squad.clone());
    let mut app = App::new(squad);
    app.popup = popup;
    let mut guard = terminal::Guard::enter(terminal::Crossterm).map_err(failed)?;
    let mut screen = Terminal::new(CrosstermBackend::new(io::stdout())).map_err(failed)?;
    let input = spawn_input();
    let result = session(
        &mut app,
        &stop,
        &input,
        &worker.results,
        |squad| worker.request(squad),
        |request| execute(&core, request),
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

    fn no_actions(request: Request) -> Result<String, String> {
        panic!("unexpected {request:?}")
    }

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

    /// A stand-in core that logs each call: `answer` succeeds, anything else
    /// fails, so the test proves the board needs no other core command.
    fn logging_core(name: &str) -> (Core, std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("squad-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls");
        let fake = dir.join("tmt");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n\
                 case \"$1\" in\n\
                 answer) echo '{{\"status\":\"submitted\",\"requestId\":\"q1\"}}' ;;\n\
                 *) echo '{{\"error\":{{\"code\":\"X_NOT_FOUND\",\"message\":\"Exchange was not found.\"}}}}'; exit 3 ;;\n\
                 esac\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        (Core::at(fake), log, dir)
    }

    /// The board's `r` is one `tmt answer` call: core selects and proves the
    /// request, so no receipt passes through Squad (#512).
    #[test]
    fn the_board_answers_through_tmt_answer_without_a_receipt() {
        let (core, log, dir) = logging_core("answer");
        let reply = |text: &str| {
            execute(
                &core,
                Request::Reply {
                    me: "ben".into(),
                    request: "q1".into(),
                    from: "alice".into(),
                    text: text.into(),
                },
            )
        };
        assert_eq!(reply("-use postgres"), Ok("Replied to alice.".into()));
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "answer --identity ben --request q1 --json -- alice -use postgres\n",
            "one call, no x show and no receipt"
        );
        assert_eq!(reply("  "), Err("Nothing to send.".into()));
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn quit_signal_and_lost_input_each_end_the_session() {
        let (mut app, stop, keys, input, results) = fixture();
        keys.send(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )))
        .unwrap();
        let outcome = session(
            &mut app,
            &stop,
            &input,
            &results,
            |_| {},
            no_actions,
            |_| Ok(()),
        );
        assert_eq!(outcome.unwrap(), None);

        let (mut app, stop, _keys, input, results) = fixture();
        stop.store(signal_hook::consts::SIGTERM as usize, Ordering::Relaxed);
        let outcome = session(
            &mut app,
            &stop,
            &input,
            &results,
            |_| {},
            no_actions,
            |_| Ok(()),
        );
        assert_eq!(outcome.unwrap(), Some(signal_hook::consts::SIGTERM));

        // A hung-up terminal can leave the reader spinning without events:
        // a disconnected input ends the session instead of waiting forever.
        let (mut app, stop, keys, input, results) = fixture();
        drop(keys);
        let started = Instant::now();
        let outcome = session(
            &mut app,
            &stop,
            &input,
            &results,
            |_| {},
            no_actions,
            |_| Ok(()),
        );
        assert_eq!(outcome.unwrap(), Some(HANGUP));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_popup_closes_after_a_successful_jump_and_stays_otherwise() {
        let enter = || Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let run = |popup: bool, outcome: Result<String, String>| {
            let (keys, input) = channel();
            let (snapshots, results) = channel();
            let mut app = App::new(Some("product".into()));
            app.popup = popup;
            snapshots
                .send(app::tests::snapshot(
                    "product",
                    serde_json::json!([{"title": null, "rows": [{"name": "auth-fix"}]}]),
                ))
                .unwrap();
            keys.send(enter()).unwrap();
            drop(keys);
            let stop = AtomicUsize::new(0);
            let mut jumps = 0;
            let ended = session(
                &mut app,
                &stop,
                &input,
                &results,
                |_| {},
                |request| {
                    assert_eq!(request, Request::Jump("auth-fix".into()));
                    jumps += 1;
                    outcome.clone()
                },
                |_| Ok(()),
            )
            .unwrap();
            (ended, jumps)
        };
        assert_eq!(
            run(true, Ok("Showing auth-fix.".into())),
            (None, 1),
            "closed"
        );
        // The input then disconnects, which ends the session as a hangup.
        assert_eq!(
            run(true, Err("tmt focus failed".into())),
            (Some(HANGUP), 1),
            "a failed jump keeps the popup open"
        );
        assert_eq!(
            run(false, Ok("Showing auth-fix.".into())),
            (Some(HANGUP), 1),
            "the pane form stays open"
        );
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
            no_actions,
            |_| Err(io::Error::other("terminal gone")),
        );
        assert_eq!(outcome.unwrap_err().to_string(), "terminal gone");
    }

    /// The configured interval drives the automatic reload; "off" never
    /// reloads on its own.
    #[test]
    fn the_board_reloads_at_its_configured_interval_or_not_at_all() {
        let reloads = |refresh: Option<Duration>| {
            let (mut app, stop, _keys, input, results) = fixture();
            app.apply(crate::board::app::tests::snapshot(
                "product",
                serde_json::json!([]),
            ));
            app.view.as_mut().unwrap().refresh = refresh;
            let requests = std::cell::Cell::new(0);
            let frames = std::cell::Cell::new(0);
            let outcome = session(
                &mut app,
                &stop,
                &input,
                &results,
                |_| requests.set(requests.get() + 1),
                no_actions,
                |_| {
                    // Each silent frame waits one input interval.
                    frames.set(frames.get() + 1);
                    if frames.get() == 4 {
                        stop.store(signal_hook::consts::SIGTERM as usize, Ordering::Relaxed);
                    }
                    Ok(())
                },
            );
            assert_eq!(outcome.unwrap(), Some(signal_hook::consts::SIGTERM));
            requests.get()
        };
        assert_eq!(
            reloads(Some(Duration::from_millis(1))),
            3,
            "one per silent wait"
        );
        assert_eq!(reloads(Some(Duration::from_secs(3600))), 0);
        assert_eq!(reloads(None), 0, "off reloads only on F5 and actions");
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
        let outcome = session(
            &mut app,
            &stop,
            &input,
            &results,
            |_| {},
            no_actions,
            |_| Ok(()),
        );
        signaller.join().unwrap();
        assert_eq!(outcome.unwrap(), Some(signal_hook::consts::SIGHUP));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
