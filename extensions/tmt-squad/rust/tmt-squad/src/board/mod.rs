//! `tmt squad board`: the terminal board. It reads the same status document as
//! `tmt squad ls`, paints from it, and reloads in the background.

mod app;
mod changes;
mod derived;
mod markdown;
pub(crate) mod notes;
mod refresh;
mod scroll;
mod tabs;
mod terminal;
mod theme_picker;
mod view;

pub use tabs::{ALL, LEADS};

use crate::{
    back,
    config::Config,
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
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
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
pub(super) enum BoardEvent {
    Input(Event),
    InputClosed,
    Snapshot {
        cancellation: crate::runner::Cancellation,
        snapshot: Box<Snapshot>,
    },
    Attention {
        cancellation: crate::runner::Cancellation,
        attention: std::collections::BTreeMap<String, crate::attention::Attention>,
    },
}

fn spawn_input(sender: Sender<BoardEvent>) {
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if sender.send(BoardEvent::Input(event)).is_err() {
                break;
            }
        }
        let _ = sender.send(BoardEvent::InputClosed);
    });
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
        Request::Reorder(keys) => Config::load(core)
            .and_then(|mut config| config.set_tab_order(&keys))
            .map(|()| "Tab order saved.".to_owned())
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

fn reload_interval(app: &App) -> Option<Duration> {
    app.view
        .as_ref()
        .map_or(Some(crate::config::DEFAULT_REFRESH), |view| view.refresh)
}

/// The board loop, independent of the real terminal. It always returns within
/// one input wait of a stop signal or a closed input, whatever the reader does.
fn session(
    app: &mut App,
    stop: &AtomicUsize,
    input: &Receiver<BoardEvent>,
    request: impl Fn(Option<String>, bool),
    mut act: impl FnMut(Request) -> Result<String, String>,
    mut theme_config: impl FnMut() -> Result<Config, String>,
    mut draw: impl FnMut(&mut App) -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let mut refreshed = Instant::now();
    let mut dirty = true;
    let mut marks = view::time_marks(app, crate::status::now_ms());
    let mut spinner = view::spinner_frame(app, Instant::now());
    loop {
        if let Some(signal) = terminal::stop_signal(stop) {
            return Ok(Some(signal));
        }
        let now = Instant::now();
        let next_spinner = view::spinner_frame(app, now);
        let next_marks = view::time_marks(app, crate::status::now_ms());
        dirty |= next_spinner != spinner || next_marks != marks;
        spinner = next_spinner;
        marks = next_marks;
        if dirty {
            draw(app)?;
            dirty = false;
        }
        // Timers still wake for stop signals and interval reloads, but a
        // silent wake never rebuilds an unchanged view.
        let interval = reload_interval(app);
        let mut wait =
            view::spinner_wait(app, Instant::now()).map_or(INPUT_WAIT, |wait| wait.min(INPUT_WAIT));
        if let Some(interval) = interval {
            wait = wait.min(interval.saturating_sub(refreshed.elapsed()));
        }
        let effect = match input.recv_timeout(wait) {
            Ok(BoardEvent::Snapshot {
                cancellation,
                snapshot,
            }) => {
                if !cancellation.cancelled() {
                    app.apply(*snapshot);
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::Attention {
                cancellation,
                attention,
            }) => {
                if !cancellation.cancelled() {
                    dirty |= app.attention != attention;
                    app.attention = attention;
                }
                Effect::None
            }
            Ok(BoardEvent::Input(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                dirty = true;
                app.key(key)
            }
            Ok(BoardEvent::Input(Event::Mouse(mouse))) => {
                dirty = true;
                app.mouse(mouse, Instant::now())
            }
            Ok(BoardEvent::Input(Event::Resize(_, _))) => {
                dirty = true;
                Effect::None
            }
            Ok(BoardEvent::InputClosed) | Err(RecvTimeoutError::Disconnected) => {
                return Ok(Some(terminal::stop_signal(stop).unwrap_or(HANGUP)));
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => Effect::None,
        };
        match effect {
            Effect::Quit => return Ok(None),
            Effect::Load(squad) => {
                request(Some(squad), true);
                refreshed = Instant::now();
            }
            Effect::Refresh => {
                request(app.current.clone(), false);
                refreshed = Instant::now();
            }
            Effect::PickTheme => {
                let squad = app.current.clone().filter(|name| !tabs::builtin(name));
                match theme_config().and_then(|config| {
                    theme_picker::Picker::open(config, squad).map_err(|error| error.message)
                }) {
                    Ok(picker) => {
                        app.theme_picker = Some(picker);
                        app.help = false;
                    }
                    Err(error) => app.finished(Err(error)),
                }
            }
            Effect::SaveTheme => {
                if let Some(picker) = &mut app.theme_picker {
                    match picker.save() {
                        Ok(changed) => {
                            let message = picker.saved_message(changed);
                            let depth = app
                                .view
                                .as_ref()
                                .map_or(tmt_cli_style::Depth::None, |view| view.look.depth);
                            let look = picker.preview(depth);
                            if let Some(view) = &mut app.view {
                                view.look = look;
                            }
                            app.theme_picker = None;
                            app.finished(Ok(message));
                            request(app.current.clone(), false);
                            refreshed = Instant::now();
                        }
                        Err(error) => picker.notice = Some(error.message),
                    }
                }
            }
            Effect::Act(action) => {
                let sends = action.sends();
                let jump = matches!(action, Request::Jump(_));
                let outcome = act(action);
                let jumped = jump && outcome.is_ok();
                app.finished(outcome);
                if jumped && app.popup {
                    return Ok(None);
                }
                if sends {
                    request(app.current.clone(), false);
                    refreshed = Instant::now();
                }
            }
            Effect::None => {}
        }
        // A snapshot may change the interval, including turning reload off.
        if reload_interval(app).is_some_and(|interval| refreshed.elapsed() >= interval) {
            request(app.current.clone(), false);
            refreshed = Instant::now();
        }
    }
}

/// Returns the signal that ended the board, if any.
/// `popup` closes the board after a successful jump, as a tmux popup should.
pub fn run(core: Core, squad: Option<String>, popup: bool) -> Result<Option<i32>, SquadError> {
    terminal::restore_before_panic_reports();
    let stop = terminal::stop_requested().map_err(failed)?;
    let (events, input) = mpsc::channel();
    let worker = refresh::Worker::spawn(
        core.clone(),
        effects::tmux_socket().is_some(),
        events.clone(),
    );
    worker.request(squad.clone(), false);
    let mut app = App::new(squad);
    app.popup = popup;
    let mut guard = terminal::Guard::enter(terminal::Crossterm).map_err(failed)?;
    let mut screen = Terminal::new(CrosstermBackend::new(io::stdout())).map_err(failed)?;
    spawn_input(events);
    let result = session(
        &mut app,
        &stop,
        &input,
        |squad, preempt| worker.request(squad, preempt),
        |request| execute(&core, request),
        || Config::load(&core).map_err(|error| error.message),
        |app| {
            screen
                .draw(|frame| {
                    // The vertical board bands, including its body, occupy the full width.
                    app.set_body_width(frame.area().width);
                    view::render(frame, app);
                })
                .map(|_| ())
        },
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
    use std::sync::{atomic::Ordering, mpsc::channel};

    #[test]
    fn theme_confirm_saves_once_then_restores_normal_row_actions() {
        let directory =
            std::env::temp_dir().join(format!("tmt-theme-session-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("squad.toml");
        std::fs::write(&path, "# kept\n[board.theme]\nbase = \"tmt\"\n").unwrap();
        let (events, input) = channel();
        let mut app = App::new(Some("product".into()));
        app.apply(app::tests::snapshot(
            "product",
            serde_json::json!([{"title":null,"rows":[{"name":"coder"}]}]),
        ));
        for code in [
            KeyCode::Char('T'),
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Enter,
            KeyCode::Char('q'),
        ] {
            events.send(key(code)).unwrap();
        }
        let mut reads = 0;
        let mut actions = 0;
        let reloads = std::cell::Cell::new(0);
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _| reloads.set(reloads.get() + 1),
            |request| {
                assert_eq!(request, Request::Jump("coder".into()));
                actions += 1;
                Ok("Jumped".into())
            },
            || {
                reads += 1;
                Config::read(path.clone()).map_err(|error| error.message)
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!((reads, actions, reloads.get()), (1, 1, 1));
        assert!(app.theme_picker.is_none());
        assert_eq!(app.look().theme.base, tmt_cli_style::Base::TmtLight);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# kept\n[board.theme]\nbase = \"tmt-light\"\n"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn theme_session_reports_stale_save_without_retry_and_escape_never_writes() {
        let directory =
            std::env::temp_dir().join(format!("tmt-theme-session-stale-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("squad.toml");
        std::fs::write(&path, "[board.theme]\nbase = \"tmt\"\n").unwrap();
        let (events, input) = channel();
        let mut app = App::new(Some("product".into()));
        app.apply(app::tests::snapshot("product", serde_json::json!([])));
        for code in [
            KeyCode::Char('T'),
            KeyCode::Down,
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Char('q'),
        ] {
            events.send(key(code)).unwrap();
        }
        let mut changed = false;
        let mut failure_shown = false;
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _| panic!("a failed save must not reload or retry"),
            no_actions,
            || Config::read(path.clone()).map_err(|error| error.message),
            |app| {
                if let Some(picker) = &app.theme_picker {
                    if !changed {
                        std::fs::write(&path, "# external edit\n").unwrap();
                        changed = true;
                    }
                    if let Some(notice) = &picker.notice {
                        assert!(notice.contains("changed") && notice.contains("retry"));
                        failure_shown = true;
                    }
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(failure_shown);
        assert!(app.theme_picker.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# external edit\n");
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn no_theme_config() -> Result<Config, String> {
        panic!("unexpected config read")
    }

    fn no_actions(request: Request) -> Result<String, String> {
        panic!("unexpected {request:?}")
    }

    /// A stand-in core that logs each call: `answer` succeeds, anything else
    /// fails, so the test proves the board needs no other core command.
    fn logging_core(name: &str) -> (Core, std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("squad-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls");
        let fake = dir.join("tmt");
        crate::test_support::write_executable(
            &fake,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n\
                 case \"$1\" in\n\
                 answer) echo '{{\"status\":\"submitted\",\"requestId\":\"q1\"}}' ;;\n\
                 *) echo '{{\"error\":{{\"code\":\"X_NOT_FOUND\",\"message\":\"Exchange was not found.\"}}}}'; exit 3 ;;\n\
                 esac\n",
                log.display()
            ),
        );
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

    fn snapshot_event(snapshot: Snapshot) -> BoardEvent {
        BoardEvent::Snapshot {
            cancellation: crate::runner::Cancellation::default(),
            snapshot: Box::new(snapshot),
        }
    }

    fn key(code: KeyCode) -> BoardEvent {
        BoardEvent::Input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    #[test]
    fn quit_signal_and_lost_input_each_end_the_session() {
        let run = |event, signal| {
            let (events, input) = channel();
            events.send(event).unwrap();
            let mut app = App::new(None);
            let stop = AtomicUsize::new(signal);
            session(
                &mut app,
                &stop,
                &input,
                |_, _| {},
                no_actions,
                no_theme_config,
                |_| Ok(()),
            )
            .unwrap()
        };
        assert_eq!(run(key(KeyCode::Char('q')), 0), None);
        assert_eq!(run(BoardEvent::InputClosed, 0), Some(HANGUP));
        assert_eq!(
            run(
                BoardEvent::InputClosed,
                signal_hook::consts::SIGTERM as usize
            ),
            Some(signal_hook::consts::SIGTERM)
        );
    }

    #[test]
    fn a_popup_closes_after_a_successful_jump_and_stays_otherwise() {
        let run = |popup: bool, outcome: Result<String, String>| {
            let (events, input) = channel();
            let mut app = App::new(Some("product".into()));
            app.popup = popup;
            events
                .send(snapshot_event(app::tests::snapshot(
                    "product",
                    serde_json::json!([{"title": null, "rows": [{"name": "auth-fix"}]}]),
                )))
                .unwrap();
            events.send(key(KeyCode::Enter)).unwrap();
            events.send(BoardEvent::InputClosed).unwrap();
            let mut jumps = 0;
            let ended = session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _| {},
                |request| {
                    assert_eq!(request, Request::Jump("auth-fix".into()));
                    jumps += 1;
                    outcome.clone()
                },
                no_theme_config,
                |_| Ok(()),
            )
            .unwrap();
            (ended, jumps)
        };
        assert_eq!(run(true, Ok("Showing auth-fix.".into())), (None, 1));
        assert_eq!(run(true, Err("tmt focus failed".into())), (Some(HANGUP), 1));
        assert_eq!(
            run(false, Ok("Showing auth-fix.".into())),
            (Some(HANGUP), 1)
        );
    }

    #[test]
    fn a_failed_redraw_ends_the_session_with_its_error() {
        let (_events, input) = channel();
        let error = session(
            &mut App::new(None),
            &AtomicUsize::new(0),
            &input,
            |_, _| {},
            no_actions,
            no_theme_config,
            |_| Err(io::Error::other("terminal gone")),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "terminal gone");
    }

    /// Each event must cause a frame even after a silent interval. Receiving
    /// the frame is the barrier; no sleep guesses when the session is waiting.
    #[test]
    fn data_input_and_resize_wake_frames_but_idle_does_not_redraw() {
        let (events, input) = channel();
        let (painted, frames) = channel();
        let session = std::thread::spawn(move || {
            let mut app = App::new(Some("product".into()));
            let mut snapshot = app::tests::snapshot("product", serde_json::json!([]));
            snapshot.view.as_mut().unwrap().refresh = None;
            app.apply(snapshot);
            super::session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _| {},
                no_actions,
                no_theme_config,
                |app| {
                    painted
                        .send((app.selected, app.view.as_ref().unwrap().document.clone()))
                        .unwrap();
                    Ok(())
                },
            )
            .unwrap()
        });
        frames.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            frames.recv_timeout(INPUT_WAIT * 2).is_err(),
            "idle never builds another frame"
        );
        let document =
            serde_json::json!([{"title": null, "rows": [{"name":"first"}, {"name":"second"}]}]);
        events
            .send(snapshot_event(app::tests::snapshot(
                "product",
                document.clone(),
            )))
            .unwrap();
        let (_, fresh) = frames.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            fresh["sections"], document,
            "data wake is painted without another key"
        );
        events.send(key(KeyCode::Down)).unwrap();
        assert_eq!(frames.recv_timeout(Duration::from_secs(2)).unwrap().0, 1);
        events
            .send(BoardEvent::Input(Event::Resize(80, 24)))
            .unwrap();
        frames.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(frames.recv_timeout(INPUT_WAIT * 2).is_err());
        events.send(key(KeyCode::Char('q'))).unwrap();
        assert_eq!(session.join().unwrap(), None);
    }

    #[test]
    fn interval_reloads_continue_without_idle_frames_and_off_never_reloads() {
        for interval in [Some(Duration::from_millis(10)), None] {
            let (events, input) = channel();
            let (requested, requests) = channel();
            let (painted, frames) = channel();
            let session = std::thread::spawn(move || {
                let mut app = App::new(Some("product".into()));
                let mut snapshot = app::tests::snapshot("product", serde_json::json!([]));
                snapshot.view.as_mut().unwrap().refresh = interval;
                app.apply(snapshot);
                super::session(
                    &mut app,
                    &AtomicUsize::new(0),
                    &input,
                    |squad, _| {
                        requested.send(squad).unwrap();
                    },
                    no_actions,
                    no_theme_config,
                    |_| {
                        painted.send(()).unwrap();
                        Ok(())
                    },
                )
                .unwrap()
            });
            frames.recv_timeout(Duration::from_secs(2)).unwrap();
            if interval.is_some() {
                for _ in 0..3 {
                    assert_eq!(
                        requests.recv_timeout(Duration::from_secs(2)).unwrap(),
                        Some("product".into())
                    );
                }
            } else {
                assert!(requests.recv_timeout(INPUT_WAIT * 2).is_err());
            }
            assert!(
                frames.try_recv().is_err(),
                "reload requests are not paint changes"
            );
            events.send(key(KeyCode::Char('q'))).unwrap();
            assert_eq!(session.join().unwrap(), None);
        }
    }

    #[test]
    fn a_signal_is_seen_within_one_wait_even_while_input_is_silent() {
        let (_events, input) = channel();
        let (painted, frames) = channel();
        let stop = std::sync::Arc::new(AtomicUsize::new(0));
        let read = std::sync::Arc::clone(&stop);
        let session = std::thread::spawn(move || {
            super::session(
                &mut App::new(None),
                &read,
                &input,
                |_, _| {},
                no_actions,
                no_theme_config,
                |_| {
                    painted.send(()).unwrap();
                    Ok(())
                },
            )
            .unwrap()
        });
        frames.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        stop.store(signal_hook::consts::SIGHUP as usize, Ordering::Relaxed);
        assert_eq!(session.join().unwrap(), Some(signal_hook::consts::SIGHUP));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn already_queued_cancelled_snapshots_and_attention_cannot_replace_current_data() {
        let generation = crate::runner::Cancellation::default();
        generation.cancel();
        let (events, input) = channel();
        let mut app = App::new(Some("product".into()));
        let mut current = app::tests::snapshot("product", serde_json::json!([]));
        current.view.as_mut().unwrap().refresh = None;
        app.apply(current);
        events
            .send(BoardEvent::Snapshot {
                cancellation: generation.clone(),
                snapshot: Box::new(app::tests::snapshot(
                    "product",
                    serde_json::json!([{ "title": null, "rows": [{"name": "stale"}] }]),
                )),
            })
            .unwrap();
        events
            .send(BoardEvent::Attention {
                cancellation: generation,
                attention: std::collections::BTreeMap::from([(
                    "product".into(),
                    crate::attention::Attention {
                        waiting: 99,
                        blocked: 99,
                    },
                )]),
            })
            .unwrap();
        events.send(key(KeyCode::Char('q'))).unwrap();
        let mut frames = 0;
        assert_eq!(
            session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _| {},
                no_actions,
                no_theme_config,
                |_| {
                    frames += 1;
                    Ok(())
                }
            )
            .unwrap(),
            None
        );
        assert_eq!(frames, 1, "discarded events do not trigger a redraw");
        assert_eq!(
            app.view.as_ref().unwrap().document["sections"],
            serde_json::json!([])
        );
        assert!(
            app.attention
                .values()
                .all(|value| *value == crate::attention::Attention::default())
        );
    }
    #[test]
    fn a_snapshot_turning_refresh_off_does_not_request_the_expired_previous_interval() {
        let (events, input) = channel();
        let mut app = App::new(Some("product".into()));
        let mut previous = app::tests::snapshot("product", serde_json::json!([]));
        // An expired interval models a snapshot arriving after a long frame.
        previous.view.as_mut().unwrap().refresh = Some(Duration::ZERO);
        app.apply(previous);
        let mut updated = app::tests::snapshot("product", serde_json::json!([]));
        updated.view.as_mut().unwrap().refresh = None;
        events.send(snapshot_event(updated)).unwrap();
        events.send(key(KeyCode::Char('q'))).unwrap();
        assert_eq!(
            session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _| panic!("automatic refresh was turned off by the snapshot"),
                no_actions,
                no_theme_config,
                |_| Ok(())
            )
            .unwrap(),
            None
        );
    }
}
