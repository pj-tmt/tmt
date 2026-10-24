//! `tmt ops ui`: the terminal board. It reads the same status document as
//! `tmt ops squad ls`, paints from it, and reloads in the background.

pub(crate) mod activity;
mod agents_focus;
mod app;
mod buttons;
mod changes;
mod checklist;
mod chip_group;
mod command_line;
mod composer;
mod conversation;
mod cronboard;
mod derived;
mod digest;
mod digest_dashboard;
mod digest_matrix;
mod discovery;
mod divider;
mod empty_state;
mod exchange_matrix;
mod exchange_view;
mod filter_bar;
#[cfg(test)]
pub(crate) mod glyph_guard;
mod header_sort;
mod help;
mod home;
mod home_leads;
mod hyperlinks;
mod identity_line;
mod input_field;
mod inventory;
mod key_hints;
mod key_value;
mod keycap;
mod lane;
mod limits;
mod member_session;
mod new_member;
pub(crate) use limits::history::document as limits_history;
mod conversation_notes;
mod idle_alert_editor;
mod markdown;
mod member_links;
mod mentions;
mod menu_surface;
mod meter;
pub(crate) mod notes;
mod notice;
mod overview;
mod pane_chrome;
mod pane_resize;
mod panels_picker;
mod peek;
mod picker_surface;
mod rate;
mod recent_responses;
mod recovery;
mod refresh;
mod reload;
mod response;
mod resume;
mod row_chips;
mod row_detail;
mod savings;
mod screen_view;
mod scroll;
mod search_field;
mod section_rule;
mod selected_conversation;
mod settings;
mod spark_row;
mod stat_tiles;
mod state_cell;
mod status_update;
mod tab_strip;
mod talk_measure;
mod team_strip;
mod timer_rows;
mod top_bar;
mod tree_row;
mod unread;
mod versions;
pub(crate) use crate::tabs;
mod shell;
mod terminal;
mod theme_picker;
mod timing;
mod usage_report;
mod view;
mod view_state;
pub(crate) use view_state::Layer as ViewLayer;
mod workspace;

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
    crossterm::event::{self, Event, KeyEventKind, MouseEventKind},
};
use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
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
pub(in crate::board) enum BoardEvent {
    Migration(Option<String>),
    /// The host could not say which agent runs the board, once at startup; the
    /// text is core's own error, shown instead of a bare sender prompt.
    CallerFailed(String),
    Recovery(recovery::load::Completed),
    Checklist(checklist::load::Completed),
    Discovery(discovery::load::Completed),
    MemberSession(member_session::Completed),
    NewMember(new_member::Completed),
    IdleAlert(idle_alert_editor::Completed),
    Input(Event),
    InputClosed,
    Snapshot {
        cancellation: crate::runner::Cancellation,
        snapshot: Box<Snapshot>,
    },
    /// The shown squad's deferred focus and reply-body reads.
    Enriched {
        cancellation: crate::runner::Cancellation,
        squad: String,
        enrichment: refresh::Enrichment,
    },
    History {
        cancellation: crate::runner::Cancellation,
        read: refresh::HistoryRead,
    },
    Usage {
        cancellation: crate::runner::Cancellation,
        room: String,
        input: Result<rate::Input, ()>,
    },
    HomeLeads {
        cancellation: crate::runner::Cancellation,
        read: home_leads::Read,
    },
    /// The labels the configured sources now supply; no generation, it is not a load.
    Labels(crate::labels::Supplied),
    /// What the worker read from a dropped file's first bytes; no generation, it is not a load.
    FileInfo(composer::files::Delivery),
    /// A finished Usage report read; no generation, it is not a load.
    UsageReport(usage_report::Delivery),
    HomeUsage {
        cancellation: crate::runner::Cancellation,
        input: Result<std::collections::BTreeMap<String, serde_json::Value>, ()>,
        /// Absent when the listing failed; the shown limits then keep aging.
        limits: Option<limits::Snapshot>,
    },
    Conversation {
        cancellation: crate::runner::Cancellation,
        plan: selected_conversation::Plan,
        revision: u64,
        projection: selected_conversation::Projection,
    },
    Message {
        cancellation: crate::runner::Cancellation,
        key: home_leads::MessageKey,
        revision: u64,
        body: Result<String, String>,
    },
    PaneOutput {
        cancellation: crate::runner::Cancellation,
        key: conversation::ConversationKey,
        revision: u64,
        observation: recent_responses::Observation,
    },
    PaneScreen {
        cancellation: crate::runner::Cancellation,
        key: conversation::ConversationKey,
        round: u64,
        revision: u64,
        observation: screen_view::Observation,
    },
    Status {
        cancellation: crate::runner::Cancellation,
        target: crate::membership::status_update::Target,
        revision: u64,
        result: Result<crate::membership::status_update::Preview, String>,
    },
    Attention {
        cancellation: crate::runner::Cancellation,
        attention: std::collections::BTreeMap<String, crate::attention::Attention>,
    },
    Cron {
        cancellation: crate::runner::Cancellation,
        read: Result<cronboard::Cron, String>,
    },
    /// A user action finished on the lane.
    Acted {
        jump: bool,
        sends: bool,
        outcome: Result<ActionOutcome, String>,
    },
    /// The config an overlay opens over was read on the lane.
    Opened {
        opening: lane::Opening,
        config: Result<Config, String>,
        board_pane: Option<String>,
    },
    TokenWindowSaved(Result<Config, String>),
    /// A quiet cache publication; it owns no user action or progress notice.
    ViewStatePublished,
    ReadMarkersPublished(Result<crate::config::ReadMarkers, String>),
    /// A conversation's Notes summary was read; it owns no user action or progress notice.
    NotesRead(conversation_notes::Completed),
    /// The running versions, read once at start; they own no user action or progress notice.
    VersionsRead(Option<versions::Versions>),
}

fn spawn_input(sender: Sender<BoardEvent>, mut filter: Option<terminal::background::ReplyFilter>) {
    std::thread::spawn(move || {
        loop {
            match event::poll(INPUT_WAIT) {
                Ok(true) => {
                    let Ok(event) = event::read() else {
                        break;
                    };
                    let events = match &mut filter {
                        Some(filter) => filter.push(event, Instant::now()),
                        None => vec![event],
                    };
                    for event in events {
                        if sender.send(BoardEvent::Input(event)).is_err() {
                            return;
                        }
                    }
                }
                Ok(false) => {
                    if let Some(filter) = &mut filter {
                        for event in filter.expire(Instant::now()) {
                            if sender.send(BoardEvent::Input(event)).is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let _ = sender.send(BoardEvent::InputClosed);
    });
}

/// Carries out a resolved request. Nothing here reads the row or config.
/// Jump and back share the plain commands' path, including the per-client
/// back stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ActionOutcome {
    CronPreview(
        Box<(
            cronboard::preview::Task,
            Result<cronboard::preview::Preview, String>,
        )>,
    ),
    Workspace(Box<Result<crate::membership::Outcome, crate::core::SquadError>>),
    RenameMember(Box<workspace::rename_member::Completed>),
    Message(String),
    JumpRead {
        message: String,
        markers: Result<crate::config::ReadMarkers, String>,
    },
    WorkspacePreferences(Box<(crate::config::OpsPreferences, String)>),
    Send(Box<conversation::outgoing::Completed>),
    SendNow(Box<conversation::SendNowDone>),
    Status(Box<crate::membership::status_update::Outcome>),
    /// Core agreed that the person at the board may change a member's digest.
    Digest(Box<digest::Granted>),
}
impl From<String> for ActionOutcome {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}
impl From<&str> for ActionOutcome {
    fn from(message: &str) -> Self {
        Self::Message(message.into())
    }
}
fn execute(core: &Core, request: Request) -> Result<ActionOutcome, String> {
    if let Request::Status(action) = request {
        return Ok(ActionOutcome::Status(Box::new(match *action {
            status_update::Action::Apply(submit) => {
                crate::membership::status_update::apply(core, &submit)
            }
            status_update::Action::Retry { preview, intent } => {
                crate::membership::status_update::retry(core, &preview, intent)
            }
        })));
    }
    let outcome = match request {
        Request::CronPreview(task) => {
            return Ok(ActionOutcome::CronPreview(Box::new(
                cronboard::preview::run(*task, jiff::Timestamp::now().as_millisecond()),
            )));
        }
        Request::WorkspaceOperation(change) => {
            return Ok(match change {
                workspace::manage::Change::Membership(operation) => {
                    ActionOutcome::Workspace(Box::new(Config::load(core).and_then(|mut config| {
                        crate::membership::workspace::apply(core, &mut config, operation)
                    })))
                }
                workspace::manage::Change::RenameMember(request) => {
                    let outcome = request.apply(core);
                    ActionOutcome::RenameMember(Box::new(workspace::rename_member::Completed {
                        request,
                        outcome,
                    }))
                }
            });
        }
        Request::Status(_) => unreachable!("handled above"),
        Request::DigestOpen(ask) => {
            return digest::grant(core, ask)
                .map(|granted| ActionOutcome::Digest(Box::new(granted)));
        }
        Request::Digest(set) => digest::apply(core, set),
        Request::DigestSwitch(on) => digest::switch(core, on),
        Request::Jump(member) => back::jump(core, &member)
            .map(|(focus, warning)| match warning {
                None => format!("Showing {member} ({}).", focus.pane),
                Some(warning) => format!(
                    "Showing {member} ({}); back will not return here: {warning}",
                    focus.pane
                ),
            })
            .map_err(|error| error.message),
        Request::Swap {
            target,
            name,
            direction,
        } => effects::swap(core, &target, &direction)
            .map(|swapped| {
                if swapped {
                    format!("Swapped {name} into the {direction} pane beside the board.")
                } else {
                    format!("{name} is already beside the board.")
                }
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
        Request::Copy {
            text,
            program,
            feedback,
        } => effects::copy(&text, program.as_deref(), effects::tmux_socket().as_deref())
            .map(|copied| match feedback {
                app::CopyFeedback::Destination => copied.describe().to_owned(),
                app::CopyFeedback::Selection => {
                    format!("⧉ Copied {} characters", text.chars().count())
                }
            })
            .map_err(|error| match feedback {
                app::CopyFeedback::Destination => error,
                app::CopyFeedback::Selection => format!("Could not copy: {error}"),
            }),
        Request::Run(argv) => effects::spawn(&argv).map(|()| format!("Started {}.", argv[0])),
        Request::PreparedRun { action, row, ops } => core
            .json(&["whoami"])
            .map_err(|error| error.message)
            .and_then(|caller| {
                action.argv_parameters(crate::template::Parameters {
                    row: &row,
                    ops: Some(&ops),
                    board_pane: caller["pane"].as_str(),
                })
            })
            .and_then(|argv| effects::spawn(&argv).map(|()| format!("Started {}.", argv[0]))),
        Request::Talk {
            me,
            squad,
            to,
            text,
        } => {
            return Ok(ActionOutcome::Send(Box::new(
                conversation::outgoing::Completed {
                    ticket: None,
                    outcome: send::talk(core, &squad, &me, &to, &text),
                    what: "Message".into(),
                    to,
                },
            )));
        }
        Request::Chat {
            sender,
            recipient,
            name,
            text,
            fallback,
        } => {
            return Ok(ActionOutcome::Send(Box::new(
                conversation::outgoing::Completed {
                    ticket: None,
                    outcome: Config::load(core).and_then(|config| {
                        send::chat(core, &config, &sender, &recipient, &text, &fallback)
                    }),
                    what: "Message".into(),
                    to: name,
                },
            )));
        }
        Request::Annotate {
            me,
            squad,
            to,
            row,
            text,
        } => {
            return Ok(ActionOutcome::Send(Box::new(
                conversation::outgoing::Completed {
                    ticket: None,
                    outcome: send::talk(core, &squad, &me, &to, &text),
                    what: format!("Note on {row}"),
                    to,
                },
            )));
        }
        Request::SendNow { me, request, to } => {
            // Core's answer, success or refusal, is the notice; the chip frees on either.
            let outcome = send::send_now(core, &me, &request, &to).map_err(|error| error.message);
            return Ok(ActionOutcome::SendNow(Box::new(
                conversation::SendNowDone { request, outcome },
            )));
        }
        Request::WorkspacePreferences {
            expected,
            next,
            saved,
        } => {
            Config::load(core)
                .and_then(|mut config| config.save_workspace_preferences(&expected, &next))
                .map_err(|error| error.message)?;
            return Ok(ActionOutcome::WorkspacePreferences(Box::new((
                next,
                saved.unwrap_or_else(|| "Workspace preferences saved.".to_owned()),
            ))));
        }
        Request::Cron(request) => cronboard::act(core, request),
    };
    outcome.map(ActionOutcome::Message)
}

/// At most this many queued wheel events join one frame; the rest follow next turn.
const WHEEL_BURST: usize = 64;

fn is_wheel(event: &ratatui::crossterm::event::MouseEvent) -> bool {
    matches!(
        event.kind,
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
    )
}

/// Applies a wheel event and the wheel and pointer-move events already queued
/// behind it, in order and without dropping any, so a burst paints one frame
/// instead of one per event. Pointer motion (any-motion reporting sends it between
/// wheel steps) moves the hover and changes nothing the frame must wait for. It stops
/// at the first other event (a click, a key, a resize, a focus change), which is
/// handed back to land in order on the next turn. A non-wheel `first` is applied alone.
fn coalesce_wheel(
    first: ratatui::crossterm::event::MouseEvent,
    input: &Receiver<BoardEvent>,
    mut apply: impl FnMut(ratatui::crossterm::event::MouseEvent) -> Effect,
) -> (Effect, Option<BoardEvent>) {
    let mut effect = apply(first);
    if !is_wheel(&first) {
        return (effect, None);
    }
    for _ in 0..WHEEL_BURST {
        // An effect to carry out ends the burst before another event can change it.
        if !matches!(effect, Effect::None) {
            return (effect, None);
        }
        match input.try_recv() {
            Ok(BoardEvent::Input(Event::Mouse(next)))
                if is_wheel(&next) || next.kind == MouseEventKind::Moved =>
            {
                effect = apply(next);
            }
            Ok(other) => return (effect, Some(other)),
            Err(_) => break,
        }
    }
    (effect, None)
}

fn reload_interval(app: &App) -> Option<Duration> {
    app.view
        .as_ref()
        .map_or(Some(crate::config::DEFAULT_REFRESH), |view| view.refresh)
}

/// The board loop, independent of the real terminal. It always returns within
/// one input wait of a stop signal or a closed input, whatever the reader does.
#[allow(clippy::too_many_arguments)] // The terminal-independent loop injects worker requests and effects.
fn session(
    app: &mut App,
    stop: &AtomicUsize,
    input: &Receiver<BoardEvent>,
    request: impl Fn(Option<String>, bool, bool, Option<rate::history::Demand>),
    selected_read: impl Fn(u64, Option<refresh::SelectedRead>),
    checklist: impl Fn(checklist::load::Task),
    io: impl Fn(lane::Job),
    labels_window: impl Fn(crate::config::DigestWindow),
    usage_wants: impl Fn(Option<usage_report::Request>),
    mut suspend: impl FnMut() -> io::Result<()>,
    mut reassert_mouse: impl FnMut() -> io::Result<()>,
    mut state: Option<&mut view_state::Session>,
    mut draw: impl FnMut(&mut App) -> io::Result<()>,
) -> io::Result<Option<i32>> {
    // Nothing below may start a command: jobs go to the lane, reads to the worker.
    let _no_commands = crate::runner::forbid_commands();
    let mut revision = 0;
    // A pending reload defers selection work until its snapshot publishes.
    app.selected_request = refresh::SelectedRequest::Ready(None);
    let mut refreshed = Instant::now();
    let mut dirty = true;
    let mut marks = view::time_marks(app, crate::status::now_ms());
    let mut spinner = view::spinner_frame(app, Instant::now());
    let mut last_input = Instant::now();
    let mut modal_was_open = false;
    // The terminal may have dropped the mouse request; it is asked for again
    // before the next frame, or at the end of this turn when none is due.
    let mut capture_lost = false;
    // The event that ended a wheel burst; it is the next turn's first event.
    let mut deferred: Option<BoardEvent> = None;
    let mut wanted: Option<usage_report::Request> = None;
    let mut history_demand = None;
    // Hands a job to the lane and says so at once; the event that comes back says how it ended.
    let submit = |app: &mut App, job: lane::Job| {
        let job = match job {
            lane::Job::Act(Request::Jump(member)) => {
                let seen = app.unread_jump(&member);
                lane::Job::Jump {
                    member,
                    seen,
                    publisher: app.unread.writer.clone(),
                }
            }
            other => other,
        };
        // An overlay opens once: a second request for one waits for the first.
        if !matches!(
            job,
            lane::Job::Act(_)
                | lane::Job::Jump { .. }
                | lane::Job::Send { .. }
                | lane::Job::Recovery(_)
        ) && app.in_flight > 0
        {
            app.set_notice(
                Some("Still working on the previous request.".to_owned()),
                false,
            );
            return;
        }
        // Recovery's pending task owns its progress and retained result inside
        // the modal. Do not leave a second stale progress notice on the board.
        if job.shows_progress() {
            app.set_progress(job.progress().to_owned());
        }
        app.in_flight += 1;
        io(job);
    };
    loop {
        if let Some(signal) = terminal::stop_signal(stop) {
            if signal != signal_hook::consts::SIGTSTP {
                return Ok(Some(signal));
            }
            // Keep the session's source revisions and pending work across a pause.
            // Never clear a TERM/HUP that superseded the suspension request.
            if let Err(signal) = stop.compare_exchange(
                signal_hook::consts::SIGTSTP as usize,
                0,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                return Ok(i32::try_from(signal).ok());
            }
            suspend()?;
            dirty = true;
            continue;
        }
        let now = Instant::now();
        if let Some(state) = &mut state {
            let (restored, publication) = state.tick(app, now);
            dirty |= restored;
            if let Some(writer) = publication {
                // Quiet, coalesced work goes straight to the existing lane.
                // It must not acquire the user-action notice/in-flight count.
                io(lane::Job::ViewState(writer));
            }
        }
        if let Some(writer) = app.unread.writer.offer() {
            io(lane::Job::ReadMarkers(writer));
        }
        let shown = app.conversation_notes_key();
        if let Some(task) = app.notes.offer(shown.as_ref(), now) {
            io(lane::Job::Notes(task));
        }
        // The footer names a waiting reload, so a new one repaints.
        dirty |= app.expire_notice(now);
        dirty |= app.reload.poll(now);
        if app.reload.pending()
            && !app.reload_blocked()
            && now.duration_since(last_input) >= app.reload.idle()
            && let Some(resume) = app.resume()
        {
            app.restart = Some(resume);
            return Ok(None);
        }
        dirty |= app.meter.as_mut().is_some_and(|meter| meter.tick(now));
        dirty |= app.home_counters.borrow_mut().tick(now);
        let next_spinner = view::spinner_frame(app, now);
        let next_marks = view::time_marks(app, crate::status::now_ms());
        dirty |= next_spinner != spinner || next_marks != marks;
        spinner = next_spinner;
        marks = next_marks;
        // The reader runs only while the panel is on screen, for its window.
        let want = app.usage_report_want();
        if want != wanted {
            usage_wants(want.clone());
            wanted = want;
        }
        let history = app.history_demand();
        let demand = history.map(|request| {
            (
                request,
                crate::status::now_ms() / rate::history::rollup::RESOLUTION_MS,
            )
        });
        if demand != history_demand {
            history_demand = demand;
            revision += 1;
            app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
            request(app.current.clone(), true, app.settings.is_some(), history);
            refreshed = Instant::now();
            dirty = true;
        }
        if dirty {
            draw(app)?;
            app.unread_drawn();
            // Painting may admit/drop clock surfaces. Set the comparison base
            // from that frame, so initial paint or clipping does not cause a
            // second unchanged frame on the next loop tick.
            marks = view::time_marks(app, crate::status::now_ms());
            if app.timing.is_some() {
                let usable = app.view.is_some() && !app.loading();
                if let Some(timing) = &mut app.timing {
                    timing.drawn(app.current.as_deref(), usable);
                }
            }
            dirty = false;
        }
        let identity = app.selected_read();
        if let refresh::SelectedRequest::Ready(requested) = &app.selected_request
            && requested != &identity
        {
            revision += 1;
            selected_read(revision, identity.clone());
            app.selected_request = refresh::SelectedRequest::Ready(identity);
        }
        // Timers still wake for stop signals and interval reloads, but a
        // silent wake never rebuilds an unchanged view.
        let interval = reload_interval(app);
        let mut wait =
            view::spinner_wait(app, Instant::now()).map_or(INPUT_WAIT, |wait| wait.min(INPUT_WAIT));
        if let Some(motion) = view::feedback_wait(app, crate::status::now_ms()) {
            wait = wait.min(motion);
        }
        if let Some(motion) = app.screen_wait() {
            wait = wait.min(motion);
        }
        if let Some(motion) = app
            .meter
            .as_ref()
            .and_then(|meter| meter.wait(Instant::now()))
        {
            wait = wait.min(motion);
        }
        if let Some(motion) = app.home_counters.borrow().wait(Instant::now()) {
            wait = wait.min(motion);
        }
        if let Some(interval) = interval {
            wait = wait.min(interval.saturating_sub(refreshed.elapsed()));
        }
        let received = match deferred.take() {
            Some(event) => Ok(event),
            None => input.recv_timeout(wait),
        };
        if matches!(received, Ok(BoardEvent::Input(_))) {
            last_input = Instant::now();
        }
        let effect = match received {
            Ok(BoardEvent::Migration(notice)) => {
                app.migration_notice = notice;
                dirty = true;
                Effect::None
            }
            Ok(BoardEvent::CallerFailed(why)) => {
                app.caller_failure = Some(why);
                Effect::None
            }
            Ok(BoardEvent::Checklist(completed)) => {
                app.finished_checklist(completed);
                dirty = true;
                Effect::None
            }
            Ok(BoardEvent::Recovery(completed)) => {
                app.in_flight = app.in_flight.saturating_sub(1);
                dirty = true;
                app.finished_recovery(completed)
            }
            Ok(BoardEvent::IdleAlert(completed)) => {
                app.in_flight = app.in_flight.saturating_sub(1);
                dirty = true;
                app.finished_idle_alert(completed)
            }
            Ok(BoardEvent::NewMember(completed)) => {
                app.in_flight = app.in_flight.saturating_sub(1);
                dirty = true;
                app.finished_new_member(completed)
            }
            Ok(BoardEvent::MemberSession(completed)) => {
                app.in_flight = app.in_flight.saturating_sub(1);
                dirty = true;
                app.finished_member_session(completed)
            }
            Ok(BoardEvent::Discovery(completed)) => {
                app.in_flight = app.in_flight.saturating_sub(1);
                dirty = true;
                app.finished_discovery(completed)
            }
            Ok(BoardEvent::Snapshot {
                cancellation,
                snapshot,
            }) => {
                if !cancellation.cancelled() {
                    // Keep the selected batch until the next paint restores
                    // viewport admission. The existing post-paint full-plan
                    // comparison preserves it only if the demand is unchanged;
                    // a source/selection/sender change starts a new revision.
                    // Explicit refresh already cleared Ready before requesting.
                    if app.selected_request.conversation().is_none() {
                        revision += 1;
                        app.selected_request = refresh::SelectedRequest::Ready(None);
                    }
                    let previous_window = app.digest_window();
                    app.apply(*snapshot);
                    if app.digest_window() != previous_window {
                        labels_window(app.digest_window());
                    }
                    dirty = true;
                    Effect::None
                } else {
                    Effect::None
                }
            }
            Ok(BoardEvent::Enriched {
                cancellation,
                squad,
                enrichment,
            }) => {
                if !cancellation.cancelled() && app.apply_enriched(&squad, enrichment) {
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::Status {
                cancellation,
                target,
                revision: read_revision,
                result,
            }) => {
                if !cancellation.cancelled()
                    && read_revision == revision
                    && app.selected_read() == Some(refresh::SelectedRead::Status(target.clone()))
                    && let Some(draft) = &mut app.status_draft
                {
                    draft.loaded(result);
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::Conversation {
                cancellation,
                plan,
                revision: read_revision,
                projection,
            }) => {
                if !cancellation.cancelled()
                    && read_revision == revision
                    && app.selected_read()
                        == Some(refresh::SelectedRead::Conversation(plan.clone()))
                {
                    app.apply_conversation(plan, projection);
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::Message {
                cancellation,
                key,
                revision: read_revision,
                body,
            }) => {
                if !cancellation.cancelled()
                    && read_revision == revision
                    && app.selected_read() == Some(refresh::SelectedRead::Message(key.clone()))
                {
                    app.apply_message(&key, body);
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::PaneOutput {
                cancellation,
                key,
                revision: read_revision,
                observation,
            }) => {
                if !cancellation.cancelled()
                    && read_revision == revision
                    && app.selected_read() == Some(refresh::SelectedRead::PaneOutput(key.clone()))
                {
                    app.apply_pane_output(key, observation);
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::PaneScreen {
                cancellation,
                key,
                round,
                revision: read_revision,
                observation,
            }) => {
                if !cancellation.cancelled()
                    && read_revision == revision
                    && app.selected_read()
                        == Some(refresh::SelectedRead::PaneScreen(key.clone(), round))
                {
                    app.apply_pane_screen(key, round, observation);
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::HomeLeads { cancellation, read }) => {
                if !cancellation.cancelled() && !app.loading() {
                    app.apply_home_leads(read);
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::History { cancellation, read }) => {
                if !cancellation.cancelled() {
                    dirty |= app.apply_history(read, Instant::now());
                }
                Effect::None
            }
            Ok(BoardEvent::HomeUsage {
                cancellation,
                input,
                limits,
            }) => {
                if !cancellation.cancelled() {
                    let now = Instant::now();
                    dirty |= app.sample_home(input.as_ref().map_err(|_| ()), now);
                    if let Some(snapshot) = limits {
                        dirty |= app.set_limits(snapshot);
                    }
                }
                Effect::None
            }
            Ok(BoardEvent::Usage {
                cancellation,
                room,
                input,
            }) => {
                if !cancellation.cancelled()
                    && !app.loading()
                    && let Some(meter) = app.meter.as_mut().filter(|meter| meter.room == room)
                {
                    let now = Instant::now();
                    meter.sample(input.as_ref().map_err(|_| ()), now);
                    app.project_usage(now);
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::Labels(supplied)) => {
                app.overview_rows.borrow_mut().clear();
                dirty |= app.labels != supplied;
                app.labels = supplied;
                Effect::None
            }
            Ok(BoardEvent::FileInfo(delivery)) => {
                dirty |= app.apply_file_info(delivery);
                Effect::None
            }
            Ok(BoardEvent::UsageReport(delivery)) => {
                app.usage_report.apply(delivery);
                app.workspace_reconcile();
                dirty = true;
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
            Ok(BoardEvent::Cron { cancellation, read }) => {
                if !cancellation.cancelled() {
                    app.overview_rows.borrow_mut().clear();
                    app.cron.replace(read);
                    app.reconcile_row_details();
                    dirty = true;
                }
                Effect::None
            }
            Ok(BoardEvent::Acted {
                jump,
                sends,
                outcome,
            }) => {
                app.in_flight = app.in_flight.saturating_sub(1);
                dirty = true;
                let jumped = jump && outcome.is_ok();
                if sends
                    && !matches!(
                        &outcome,
                        Ok(ActionOutcome::Send(_) | ActionOutcome::SendNow(_))
                    )
                {
                    app.finish_draft(outcome.is_ok());
                }
                match outcome {
                    Ok(ActionOutcome::CronPreview(outcome)) => {
                        let (task, result) = *outcome;
                        app.cron_review_ready(task, result);
                    }
                    Ok(ActionOutcome::Workspace(outcome)) => app.finished_workspace(*outcome),
                    Ok(ActionOutcome::RenameMember(outcome)) => {
                        app.finished_member_rename(*outcome)
                    }
                    Ok(ActionOutcome::Status(outcome)) => app.finished_status(*outcome),
                    Ok(ActionOutcome::Message(message)) => app.finished(Ok(message)),
                    Ok(ActionOutcome::JumpRead { message, markers }) => {
                        app.finished(Ok(message));
                        app.finished_unread(markers);
                    }
                    Ok(ActionOutcome::WorkspacePreferences(completed)) => {
                        let (preferences, message) = *completed;
                        app.finished_workspace_preferences(preferences, message);
                    }
                    Ok(ActionOutcome::Send(completed)) => app.finished_send(*completed),
                    Ok(ActionOutcome::SendNow(done)) => app.finished_send_now(*done),
                    Ok(ActionOutcome::Digest(granted)) => app.open_digest(*granted),
                    Err(error) => app.finished(Err(error)),
                }
                if jumped && app.popup && app.unread.writer.failure().is_none() {
                    return Ok(None);
                }
                if sends {
                    revision += 1;
                    app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                    request(
                        app.current.clone(),
                        false,
                        app.settings.is_some(),
                        app.history_demand(),
                    );
                    refreshed = Instant::now();
                }
                Effect::None
            }
            Ok(BoardEvent::Opened {
                opening,
                config,
                board_pane,
            }) => {
                app.in_flight = app.in_flight.saturating_sub(1);
                dirty = true;
                app.set_notice(None, false);
                let opened = match (opening, config) {
                    (_, Err(error)) => Err(error),
                    (lane::Opening::Settings, Ok(config)) => app
                        .open_settings(config)
                        .map_err(|error| error.message)
                        .map(|()| true),
                    (lane::Opening::SquadSettings(squad), Ok(config)) => app
                        .open_settings_in(config, Some(&squad))
                        .map_err(|error| error.message)
                        .map(|()| true),
                    (lane::Opening::HostAction(selection), Ok(config)) => app
                        .open_host_action(config, *selection, board_pane)
                        .map_err(|error| error.message)
                        .map(|()| true),
                    (lane::Opening::ThemePicker(squad), Ok(config)) => {
                        theme_picker::Picker::open(config, squad)
                            .map_err(|error| error.message)
                            .map(|picker| {
                                app.theme_picker = Some(picker);
                                app.help = false;
                                false
                            })
                    }
                };
                match opened {
                    // The picker or overlay opens over a preview of the live board.
                    Ok(reload) => {
                        if reload {
                            revision += 1;
                            app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                            request(
                                app.current.clone(),
                                true,
                                app.settings.is_some(),
                                app.history_demand(),
                            );
                            refreshed = Instant::now();
                        }
                    }
                    Err(error) => app.finished(Err(error)),
                }
                Effect::None
            }
            Ok(BoardEvent::TokenWindowSaved(result)) => {
                app.in_flight = app.in_flight.saturating_sub(1);
                dirty = true;
                match result {
                    Ok(config) => {
                        let notice = app.apply_token_window(&config);
                        app.set_notice(Some(notice), false);
                        revision += 1;
                        app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                        request(
                            app.current.clone(),
                            true,
                            app.settings.is_some(),
                            app.history_demand(),
                        );
                        refreshed = Instant::now();
                    }
                    Err(error) => app.finished(Err(error)),
                }
                Effect::None
            }
            Ok(BoardEvent::ViewStatePublished) => Effect::None,
            Ok(BoardEvent::NotesRead(completed)) => {
                app.finished_notes(completed, Instant::now());
                dirty = true;
                Effect::None
            }
            Ok(BoardEvent::VersionsRead(versions)) => {
                app.versions = versions;
                dirty = true;
                Effect::None
            }
            Ok(BoardEvent::ReadMarkersPublished(result)) => {
                app.finished_unread(result);
                dirty = true;
                Effect::None
            }
            Ok(BoardEvent::Input(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                dirty = true;
                app.key(key)
            }
            Ok(BoardEvent::Input(Event::Mouse(mouse))) => {
                if mouse.kind == MouseEventKind::Moved {
                    dirty |= app.move_pointer(mouse);
                    Effect::None
                } else {
                    dirty = true;
                    let (effect, next) =
                        coalesce_wheel(mouse, input, |event| app.mouse(event, Instant::now()));
                    deferred = next;
                    effect
                }
            }
            Ok(BoardEvent::Input(Event::Paste(text))) => {
                dirty = true;
                app.paste(&text)
            }
            Ok(BoardEvent::Input(Event::FocusGained)) => {
                capture_lost = true;
                dirty = true;
                Effect::None
            }
            Ok(BoardEvent::Input(Event::Resize(_, _))) => {
                capture_lost = true;
                app.composer.paste_preview.get_mut().take();
                if let Some(discovery) = &mut app.discovery {
                    discovery.changed();
                }
                app.meter_hover = None;
                app.header_hover = None;
                app.invalidate_frame_geometry();
                dirty = true;
                Effect::None
            }
            Ok(BoardEvent::InputClosed) | Err(RecvTimeoutError::Disconnected) => {
                return Ok(Some(terminal::stop_signal(stop).unwrap_or(HANGUP)));
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => Effect::None,
        };
        match effect {
            Effect::Foreground(command) => {
                app.foreground = Some(command);
                return Ok(None);
            }
            Effect::Quit => return Ok(None),
            Effect::Load(squad) => {
                revision += 1;
                app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                request(Some(squad), true, false, app.history_demand());
                refreshed = Instant::now();
            }
            Effect::Refresh => {
                revision += 1;
                app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                let history = app.history_demand();
                request(
                    app.current.clone(),
                    history.is_some(),
                    app.settings.is_some(),
                    history,
                );
                refreshed = Instant::now();
            }
            Effect::Checklist(task) => checklist(task),
            Effect::Recovery(task) => submit(app, lane::Job::Recovery(task)),
            Effect::IdleAlert(task) => {
                app.in_flight += 1;
                io(lane::Job::IdleAlert(task));
            }
            Effect::NewMember(task) => {
                app.in_flight += 1;
                io(lane::Job::NewMember(task));
            }
            Effect::MemberSession(task) => {
                app.in_flight += 1;
                io(lane::Job::MemberSession(task));
            }
            Effect::Notes(task) => io(lane::Job::Notes(task)),
            Effect::Discovery(task) => {
                app.in_flight += 1;
                io(lane::Job::Discovery(task));
            }
            Effect::Settings => submit(app, lane::Job::Open(lane::Opening::Settings)),
            Effect::SquadSettings(squad) => {
                submit(app, lane::Job::Open(lane::Opening::SquadSettings(squad)))
            }
            Effect::HostAction => submit(
                app,
                lane::Job::Open(lane::Opening::HostAction(Box::new(
                    app.host_action_selection(),
                ))),
            ),
            Effect::SaveSetting => {
                if app.settings.as_mut().is_some_and(|overlay| overlay.save()) {
                    app.settings_preview();
                    revision += 1;
                    app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                    request(app.current.clone(), true, true, app.history_demand());
                    refreshed = Instant::now();
                }
            }
            Effect::ResetSetting => {
                if app.settings.as_mut().is_some_and(|overlay| overlay.reset()) {
                    app.settings_preview();
                    if let Some(config) = app
                        .settings
                        .as_ref()
                        .and_then(|overlay| overlay.config())
                        .cloned()
                    {
                        app.apply_token_window(&config);
                    }
                    revision += 1;
                    app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                    request(app.current.clone(), true, true, app.history_demand());
                    refreshed = Instant::now();
                }
            }
            Effect::CycleTokenWindow => {
                if let Some(window) = app.next_token_window() {
                    if let Some(overlay) = &mut app.settings {
                        // The settings overlay already holds the config; its save is a bounded file write.
                        if overlay.save_window(window) {
                            let config = overlay.config().unwrap().clone();
                            let notice = app.apply_token_window(&config);
                            app.set_notice(Some(notice), false);
                            revision += 1;
                            app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                            request(app.current.clone(), true, true, app.history_demand());
                            refreshed = Instant::now();
                        }
                    } else {
                        submit(app, lane::Job::TokenWindow(window.label()));
                    }
                }
            }
            Effect::CancelSettings => {
                revision += 1;
                app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                request(app.current.clone(), true, false, app.history_demand());
                refreshed = Instant::now();
            }
            Effect::PickTheme => {
                let squad = app
                    .workspace_squad()
                    .map(|squad| squad.name.clone())
                    .or_else(|| app.current.clone().filter(|name| !tabs::aggregate(name)));
                submit(app, lane::Job::Open(lane::Opening::ThemePicker(squad)));
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
                            revision += 1;
                            app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
                            request(
                                app.current.clone(),
                                false,
                                app.settings.is_some(),
                                app.history_demand(),
                            );
                            refreshed = Instant::now();
                        }
                        Err(error) => picker.notice = Some(error.message),
                    }
                }
            }
            Effect::Act(action) => {
                let job = if let Some(ticket) = app.outgoing_ticket(&action) {
                    lane::Job::Send {
                        request: action,
                        ticket,
                    }
                } else {
                    lane::Job::Act(action)
                };
                submit(app, job);
            }
            Effect::None => {}
        }
        // A snapshot may change the interval, including turning reload off.
        if reload_interval(app).is_some_and(|interval| refreshed.elapsed() >= interval) {
            // Keep the in-flight batch admitted until a snapshot proves its
            // source plan changed. Base acquisition continues at its interval.
            if app.selected_request.conversation().is_none() {
                revision += 1;
                app.selected_request = refresh::SelectedRequest::AwaitingSnapshot;
            }
            request(
                app.current.clone(),
                false,
                app.settings.is_some(),
                app.history_demand(),
            );
            refreshed = Instant::now();
            // An idle board has no resize, focus or overlay to repair a dropped
            // request: the periodic refresh asks again (a few idempotent bytes, no frame).
            capture_lost = true;
        }
        // A closed overlay or menu is the last moment the board can be sure the
        // terminal still reports the mouse.
        let modal = app.modal_open();
        capture_lost |= modal_was_open && !modal;
        modal_was_open = modal;
        if std::mem::take(&mut capture_lost) {
            reassert_mouse()?;
        }
    }
}

/// Resolve every explicit launch name before the terminal is admitted. Order
/// retains the first requested squad; repeated aliases of one room collapse.
pub(super) fn launch_scope(
    core: &Core,
    names: &[String],
    resumed: bool,
) -> Result<Vec<crate::squad::Squad>, SquadError> {
    use crate::squad::Squad;
    // A carried UUID scope (including an explicit clear) supersedes argv.
    // A renamed/deleted launch name must not prevent the session from reloading.
    if resumed {
        return Ok(Vec::new());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut squads = Vec::new();
    for name in names {
        let squad = Squad::resolve_launch(core, name).map_err(|error| {
            if error.code != "SQUAD_NOT_FOUND" {
                return error;
            }
            let Ok(mut valid) = Squad::list(core) else {
                return error;
            };
            valid.sort_by(|a, b| a.name.cmp(&b.name));
            SquadError::new(
                &error.code,
                format!(
                    "{} Valid squads: {}.",
                    error.message,
                    if valid.is_empty() {
                        "(none)".into()
                    } else {
                        valid
                            .iter()
                            .map(|squad| squad.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                ),
            )
        })?;
        if seen.insert(squad.room_id.clone()) {
            squads.push(squad);
        }
    }
    Ok(squads)
}

/// Retired tab selection fails before terminal admission in every UI route.
pub(super) fn reject_retired_tabs(input: Option<&str>) -> Result<(), SquadError> {
    if input.is_some() {
        return Err(SquadError::hinted(
            "SQUAD_UI_TABS_RETIRED",
            "ui --tabs belongs to the retired squad-tab layout",
            "; use ",
            "Hide squad or ui --squad NAME (repeatable), with 1/2/3 for the shell tabs.",
        ));
    }
    Ok(())
}

/// `ui --reload-all`: every board that started before now reloads once it is idle.
pub fn request_reload_all(now_ms: u64) -> Result<(), SquadError> {
    reload::request_all(now_ms)
}

/// Returns the signal that ended the board; a popup closes after a successful jump.
pub fn run(
    core: Core,
    names: Vec<String>,
    popup: bool,
    layer: ViewLayer,
    reset_view: bool,
    startup_tab: Option<crate::config::StartupTab>,
    interaction: tmt_cli_style::Interaction,
) -> Result<Option<i32>, SquadError> {
    let trace = timing::Trace::from_env();
    let startup = trace.as_ref().map(timing::Trace::startup);
    terminal::restore_before_panic_reports();
    let stop = terminal::stop_requested().map_err(failed)?;
    core.paths.use_board_notices();
    // The state a reload left for this pid, consumed whether or not it is used.
    let now_ms = crate::status::now_ms();
    let resume = resume::directory()
        .and_then(|directory| resume::Resume::take(&directory, std::process::id(), now_ms));
    // An exec continues the admitted layer and never repeats the original reset.
    let layer = resume.as_ref().map_or(layer, |state| state.layer.clone());
    let scope = timing::measure(startup.as_ref(), "startup.launch_scope", || {
        launch_scope(&core, &names, resume.is_some())
    })?;
    // Cold launch names resolve before admission; a reload uses carried UUIDs.
    // Once admitted, paint the placeholder before configuration, worker setup
    // and the terminal background query. A later failure drops the guard and
    // restores the terminal before the error is reported.
    let mut guard = terminal::Guard::enter(terminal::Crossterm::default()).map_err(failed)?;
    let links = hyperlinks::Links::default();
    let mut screen = Terminal::new(terminal::Backend::with_links(io::stdout(), links.clone()))
        .map_err(failed)?;
    timing::measure(startup.as_ref(), "startup.placeholder", || {
        screen.draw(|frame| {
            let area = ratatui::layout::Rect {
                height: 1,
                ..frame.area()
            };
            tmt_tui::components::strip::paint_left(
                frame.buffer_mut(),
                area,
                ratatui::text::Line::from(" Opening…"),
            );
        })
    })
    .map_err(failed)?;
    // Resolve this disposable path only at the interactive entry, after the
    // placeholder. App/oracle constructors never resolve or read user state.
    let mut view_state = timing::measure(startup.as_ref(), "startup.view_state", || {
        crate::cache::directory("board")
            .map(|directory| {
                let path = layer.path(&directory);
                view_state::Session::open(path, resume.is_some(), !scope.is_empty(), reset_view)
                    .map_err(failed)
            })
            .transpose()
    })?;
    let (events, input) = mpsc::channel();
    // The worker's setup reads (caller identity, config location) overlap the config
    // read and the terminal's background query. Loads build looks from that
    // background, so the first request waits until `configure_background` has run.
    let invoker_tmux = effects::tmux_socket().is_some();
    let worker = refresh::Worker::spawn(core.clone(), invoker_tmux, events.clone(), trace.clone());
    let mut config = timing::measure(startup.as_ref(), "startup.Config::load", || {
        Config::load(&core)
    })?;
    crate::me::migrate(&core, &mut config)?;
    let (preference, startup_notice) = config.startup_tab();
    let cold_tab = view_state::startup_destination(startup_tab, preference, resume.is_some());
    if cold_tab.is_some()
        && let Some(state) = &mut view_state
    {
        state.explicit_startup();
    }
    let squad = Some(ALL.to_owned());
    // The labels read in the background; nothing waits for it, and a source that
    // does not answer simply never shows. Dropping it ends the thread.
    let labels = crate::labels::Reader::spawn(
        &core,
        config.label_sources()?,
        crate::labels::Timing::default(),
        config.workspace_preferences()?.digest_window,
        {
            let events = events.clone();
            move |supplied| events.send(BoardEvent::Labels(supplied)).is_ok()
        },
    );
    // The Usage report reads in the background and only while its panel shows.
    let usage_reader = usage_report::Reader::spawn(&core, usage_report::Timing::default(), {
        let events = events.clone();
        move |delivery| events.send(BoardEvent::UsageReport(delivery)).is_ok()
    });
    let usage_wants = usage_reader.wants();
    let initial_theme = config.theme(squad.as_deref().unwrap_or(""))?.0;
    let requested = initial_theme.base;
    let value = std::env::var("COLORFGBG").ok();
    let eligible = terminal::background::allowed(
        requested,
        interaction,
        tmt_cli_style::Terminal::stdout(false).color,
        std::env::var_os("NO_COLOR").is_some(),
    );
    let (signal, query) =
        terminal::background::observe(value.as_deref(), eligible, || guard.query_background());
    crate::look::configure_background(signal);
    worker.request(squad.clone(), false, false, None);
    let filter = query
        .as_ref()
        .map(|reply| terminal::background::ReplyFilter::seed(&reply.received, Instant::now()));
    let keyboard_enhanced = guard.enable_keyboard_enhancement().map_err(failed)?;
    links.enable(hyperlinks::detect());
    let mut app = App::new(squad);
    // Dropped files' first bytes are read away from the paste and draw paths.
    app.composer.probe = Some(composer::files::Probe::spawn({
        let events = events.clone();
        move |delivery| events.send(BoardEvent::FileInfo(delivery)).is_ok()
    }));
    app.keyboard_enhanced = keyboard_enhanced;
    app.invoker_tmux = invoker_tmux;
    if let Some(tab) = cold_tab {
        app.shell.select(tab, &mut app.workspace.selected);
    }
    let room_id = scope.first().map(|squad| squad.room_id.clone());
    app.workspace_startup = if room_id.is_some() || cold_tab.is_some() {
        Some(app::WorkspaceStartup {
            room_id,
            tab: cold_tab,
        })
    } else {
        None
    };
    app.workspace.visibility.scope = scope.into_iter().map(|squad| squad.room_id).collect();
    app.timing = trace.map(timing::Ui::new);
    app.migration_notice = core.paths.board_notice();
    app.set_notice(
        crate::migration::paths(&core, None)?
            .notice
            .filter(|_| app.migration_notice.is_none())
            .or_else(|| config.obsolete_board_notice().map(str::to_owned))
            .or(startup_notice),
        false,
    );
    app.initial_look = Some(crate::look::Look::new(initial_theme));
    app.popup = popup;
    app.view_layer = layer;
    app.reload = reload::Watch::start(now_ms);
    app.notes = conversation_notes::State::reading();
    if let Some(resume) = resume {
        app.carry(resume);
        if app.notice.is_none() {
            app.set_notice(Some(reload::reloaded()), false);
        }
    }
    let mut clock = crate::cron_clock::ClockWorker::spawn(core.clone(), config, false);
    let lane = lane::Lane::spawn(
        core.clone(),
        labels.as_ref().map(crate::labels::Reader::refresher),
        events.clone(),
    );
    spawn_input(events, filter);
    lane.submit(lane::Job::Versions);
    let resumed = std::cell::Cell::new(false);
    let result = session(
        &mut app,
        &stop,
        &input,
        |squad, preempt, preview, chart| worker.request(squad, preempt, preview, chart),
        |revision, identity| worker.selected(revision, identity),
        |task| worker.checklist(task),
        |job| lane.submit(job),
        |window| {
            if let Some(labels) = &labels {
                labels.refresher().window(window);
            }
        },
        |want| usage_wants.set(want),
        || {
            guard.suspend(|| signal_hook::low_level::raise(signal_hook::consts::SIGSTOP))?;
            resumed.set(true);
            Ok(())
        },
        || terminal::reassert_mouse(&mut io::stdout()),
        view_state.as_mut(),
        |app| {
            if resumed.replace(false) {
                // Fullscreen resize resets the diff buffer without clear()'s
                // cursor-position query competing with the active input reader.
                screen.resize(screen.size()?.into())?;
            }
            screen
                .draw(|frame| {
                    // The vertical board bands, including its body, occupy the full width.
                    view::prepare(app, frame.area());
                    view::render(frame, app);
                    if links.enabled() {
                        links.set(app.member_links.borrow().web_spans());
                    }
                })
                .map(|_| ())
        },
    );
    // Restore first, whatever happened; then report the session's outcome.
    let restored = guard.restore();
    lane.finish();
    // Also reports a failed queued save whose event the UI never consumed.
    // A last admitted draw drains once; failure alone never starts a retry.
    let read_markers = app.unread.writer.finish(&core);
    if restored.is_ok()
        && let Some(state) = &mut view_state
    {
        state.flush(&mut app, Instant::now());
    }
    drop(labels);
    drop(usage_reader);
    let stopped = clock.stop();
    let signal = result.map_err(failed)?;
    restored.map_err(failed)?;
    stopped?;
    read_markers?;
    if let Some(resume) = app.restart.take() {
        // The terminal is restored and the loop has ended. Stop the refresh worker,
        // then become the new board: this returns only when that failed.
        drop(worker);
        return Err(app.reload.restart(&resume, crate::status::now_ms()));
    }
    if signal.is_none()
        && let Some(command) = app.foreground.take()
    {
        // Same pane and inherited stdio: every board worker is stopped and
        // terminal modes are restored before Core takes the foreground.
        drop(worker);
        // A signal arriving during cleanup cancels the handoff too.
        if let Some(signal) = terminal::stop_signal(&stop) {
            return Ok(Some(signal));
        }
        return Err(failed(command.exec()));
    }
    Ok(signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reload_never_resolves_original_scope_names_again() {
        let core = Core::at(std::path::PathBuf::from("/dev/null/no-core-for-resume"));
        let original = vec!["renamed-or-deleted".to_owned()];
        assert!(launch_scope(&core, &original, true).unwrap().is_empty());
        assert!(launch_scope(&core, &original, false).is_err());
    }
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::{atomic::Ordering, mpsc::channel};

    /// A broken scripted input/expected-action chain must fail, not leave the
    /// test runner waiting forever on its own still-open event sender. The
    /// production stop fence ends the session; completion wakes the watchdog.
    #[allow(clippy::too_many_arguments)] // Match the existing injected session seam.
    pub(in crate::board) fn session(
        app: &mut App,
        stop: &AtomicUsize,
        input: &Receiver<BoardEvent>,
        request: impl Fn(Option<String>, bool, bool, Option<rate::history::Demand>),
        selected_read: impl Fn(u64, Option<refresh::SelectedRead>),
        checklist: impl Fn(checklist::load::Task),
        io: impl Fn(lane::Job),
        draw: impl FnMut(&mut App) -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let expired = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            let (complete, completion) = channel::<()>();
            let expired = &expired;
            scope.spawn(move || {
                if completion.recv_timeout(Duration::from_secs(10))
                    == Err(RecvTimeoutError::Timeout)
                {
                    expired.store(true, Ordering::SeqCst);
                    stop.store(signal_hook::consts::SIGTERM as usize, Ordering::SeqCst);
                }
            });
            let result = super::session(
                app,
                stop,
                input,
                request,
                selected_read,
                checklist,
                io,
                |_| {},
                |_| {},
                || Ok(()),
                || Ok(()),
                None,
                draw,
            );
            drop(complete);
            assert!(
                !expired.load(Ordering::SeqCst),
                "scripted session did not complete its input/action chain within 10 seconds"
            );
            result
        })
    }

    fn theme_board() -> App {
        let mut app = app::workspace::tests::board();
        app::workspace::tests::observe_waiting_member(&mut app);
        app.shell.tab = shell::OpsTab::Agents;
        app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
        app.workspace.filter = workspace::AgentsFilter::Agents;
        app.view.as_mut().unwrap().refresh = None;
        for bindings in app
            .view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .keys
            .values_mut()
        {
            bindings.insert("T".into(), crate::action::Action::parse("theme").unwrap());
        }
        app
    }

    #[test]
    fn selected_responses_are_lazy_and_late_selection_or_refresh_results_are_ignored() {
        use workspace::OpsTarget;
        const SECOND: &str = "77777777-7777-4777-8777-777777777777";
        let loaded = || {
            let mut app = app::workspace::tests::board();
            let view = app.view.as_mut().unwrap();
            view.refresh = None;
            view.replies = vec![
                serde_json::json!({"recipientId":app::workspace::tests::MEMBER,"requestId":"req-a","submittedAtMs":20,"response":null}),
                serde_json::json!({"recipientId":SECOND,"requestId":"req-b","submittedAtMs":30,"response":null}),
            ];
            let home = view.home.as_mut().unwrap();
            home.summary.members = 2;
            home.summary.working = 2;
            let snapshot = &mut home.workspace;
            let first = snapshot
                .roster
                .members
                .get_mut(app::workspace::tests::MEMBER)
                .unwrap();
            first.name = "alpha".into();
            let mut second = first.clone();
            second.id = SECOND.into();
            second.name = "beta".into();
            snapshot.roster.members.insert(SECOND.into(), second);
            snapshot.roster.squads[0].members.push(SECOND.into());
            let state =
                snapshot.roster.squads[0].member_states[app::workspace::tests::MEMBER].clone();
            snapshot.roster.squads[0]
                .member_states
                .insert(SECOND.into(), state);
            snapshot
                .rows
                .get_mut(&OpsTarget::Squad(app::workspace::tests::ROOM.into()))
                .unwrap()["memberIds"] = serde_json::json!([app::workspace::tests::MEMBER, SECOND]);
            for (identity, name) in [(app::workspace::tests::MEMBER, "alpha"), (SECOND, "beta")] {
                snapshot.rows.insert(OpsTarget::Member {room_id:Some(app::workspace::tests::ROOM.into()), identity_id:identity.into()},
                    serde_json::json!({"id":identity,"name":name,"squad":"product","state":"working","lifetime":"saved"}));
            }
            app.select(app.selected);
            app
        };
        let mut app = loaded();
        let (events, input) = channel();
        let reads = std::cell::RefCell::new(Vec::new());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 36)).unwrap();
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |revision, selected| {
                let Some(refresh::SelectedRead::Conversation(plan)) = selected else {
                    return;
                };
                let key = plan.test_message_key();
                reads.borrow_mut().push(plan.clone());
                let result = |plan: selected_conversation::Plan, revision, text: &str| {
                    BoardEvent::Conversation {
                        cancellation: Default::default(),
                        projection: plan.test_projection(text, None),
                        plan,
                        revision,
                    }
                };
                if key.lead == app::workspace::tests::MEMBER {
                    events.send(self::key(KeyCode::Down)).unwrap();
                    events.send(result(plan, revision, "OLD A")).unwrap();
                } else if reads.borrow().len() == 2 {
                    let mut fresh = loaded();
                    let mut snapshot = app::tests::snapshot(ALL, serde_json::json!([]));
                    snapshot.tabs.push(ALL.into());
                    snapshot.view = Ok(fresh.view.take().unwrap());
                    events
                        .send(BoardEvent::Input(Event::Key(KeyEvent::new(
                            KeyCode::Char('r'),
                            KeyModifiers::CONTROL,
                        ))))
                        .unwrap();
                    events.send(snapshot_event(snapshot)).unwrap();
                    events
                        .send(result(plan, revision, "BEFORE REFRESH"))
                        .unwrap();
                } else {
                    events
                        .send(result(reads.borrow()[0].clone(), revision, "WRONG ID"))
                        .unwrap();
                    events
                        .send(result(plan.clone(), revision - 1, "OLD REFRESH"))
                        .unwrap();
                    events.send(result(plan, revision, "CURRENT B")).unwrap();
                    events.send(self::key(KeyCode::Char('q'))).unwrap();
                }
            },
            |_| {},
            no_io,
            |app| {
                app.set_body_width(160);
                terminal.draw(|frame| view::render(frame, app)).unwrap();
                Ok(())
            },
        )
        .unwrap();
        let targets = reads
            .borrow()
            .iter()
            .map(|plan| plan.test_message_key().lead.clone())
            .collect::<Vec<_>>();
        assert_eq!(targets, [app::workspace::tests::MEMBER, SECOND, SECOND]);
        let target = |identity| {
            row_detail::Target::Row(app::RowTarget::Workspace(OpsTarget::Member {
                room_id: Some(app::workspace::tests::ROOM.into()),
                identity_id: identity,
            }))
        };
        let current = app.detail(&target(SECOND.into())).unwrap().reply.unwrap();
        assert_eq!(current.body, "CURRENT B");
        assert!(current.acquired && current.source);
        let old = app
            .detail(&target(app::workspace::tests::MEMBER.into()))
            .unwrap()
            .reply
            .unwrap();
        assert!(
            !old.acquired,
            "a result for the selection left behind is never admitted"
        );
        let tab = shell::OpsTab::Agents;
        app.shell.tab = tab;
        // Terminal::resize alone leaves TestBackend at160x36; draw would
        // auto-resize back and never exercise a hidden conversation.
        terminal.backend_mut().resize(60, 8);
        terminal
            .resize(ratatui::layout::Rect::new(0, 0, 60, 8))
            .unwrap();
        terminal
            .draw(|frame| {
                view::prepare(&mut app, frame.area());
                view::render(frame, &app);
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer().area,
            ratatui::layout::Rect::new(0, 0, 60, 8)
        );
        assert!(
            !app.scrolls.visible(crate::config::Pane::Replies),
            "{tab:?}"
        );
        assert_eq!(
            app.selected_read(),
            None,
            "hidden source surfaces start no background read: {tab:?}"
        );
    }

    #[test]
    fn selected_pane_output_rejects_late_selection_refresh_and_wrong_sender_results() {
        let loaded = || {
            let mut app = app::workspace::tests::board();
            app.view.as_mut().unwrap().refresh = None;
            app
        };
        let mut app = loaded();
        let (events, input) = channel();
        let reads = std::cell::Cell::new(0);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 36)).unwrap();
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |revision, selected| {
                let Some(refresh::SelectedRead::Conversation(plan)) = selected else {
                    return;
                };
                let count = reads.get() + 1;
                reads.set(count);
                let result = |plan: selected_conversation::Plan, revision, text: &str| {
                    BoardEvent::Conversation {
                        cancellation: Default::default(),
                        projection: plan.test_projection("", Some(text)),
                        plan,
                        revision,
                    }
                };
                if count == 1 {
                    events.send(self::key(KeyCode::Up)).unwrap();
                    events
                        .send(result(plan, revision, "OLD SELECTION"))
                        .unwrap();
                    events.send(self::key(KeyCode::Down)).unwrap();
                } else if count == 2 {
                    let mut fresh = loaded();
                    let mut snapshot = app::tests::snapshot(ALL, serde_json::json!([]));
                    snapshot.tabs.push(ALL.into());
                    snapshot.view = Ok(fresh.view.take().unwrap());
                    events
                        .send(BoardEvent::Input(Event::Key(KeyEvent::new(
                            KeyCode::Char('r'),
                            KeyModifiers::CONTROL,
                        ))))
                        .unwrap();
                    events.send(snapshot_event(snapshot)).unwrap();
                    events.send(result(plan, revision, "OLD REFRESH")).unwrap();
                } else {
                    let wrong = plan.clone().test_wrong_sender();
                    events
                        .send(result(wrong, revision, "WRONG SENDER"))
                        .unwrap();
                    events
                        .send(result(plan.clone(), revision - 1, "OLD REVISION"))
                        .unwrap();
                    events
                        .send(result(plan, revision, "CURRENT PANE OUTPUT"))
                        .unwrap();
                    events.send(self::key(KeyCode::Char('q'))).unwrap();
                }
            },
            |_| {},
            no_io,
            |app| {
                app.set_body_width(160);
                terminal.draw(|frame| view::render(frame, app)).unwrap();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(reads.get(), 3);
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("CURRENT PANE OUTPUT"), "{text}");
        for stale in [
            "OLD SELECTION",
            "OLD REFRESH",
            "WRONG SENDER",
            "OLD REVISION",
        ] {
            assert!(!text.contains(stale), "{stale}");
        }
        assert!(app.pane_output_read().is_none());
    }

    /// Runs the layout-save scenario with the board's periodic reload set to `refresh`, so no
    /// test depends on how long the scenario takes. Returns every reload request the session made.
    fn workspace_layout_session(
        refresh: Option<std::time::Duration>,
    ) -> Vec<(Option<String>, bool, bool)> {
        let directory = std::env::temp_dir().join(format!(
            "tmt-workspace-layout-session-{}-{}",
            std::process::id(),
            refresh.map_or("off".to_owned(), |interval| interval.as_nanos().to_string())
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ops.toml");
        std::fs::write(&path, "# intact\n").unwrap();
        let (events, input) = channel();
        let mut app = app::workspace::tests::board();
        app.view.as_mut().unwrap().refresh = refresh;
        for code in [KeyCode::Char(','), KeyCode::Down, KeyCode::Enter] {
            events.send(key(code)).unwrap();
        }
        let saves = std::cell::Cell::new(0);
        let jumps = std::cell::Cell::new(0);
        let reloads = std::cell::RefCell::new(Vec::new());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |owner, preempt, preview, _| {
                reloads.borrow_mut().push((owner.clone(), preempt, preview));
                let mut fresh = app::workspace::tests::board();
                fresh.view.as_mut().unwrap().refresh = refresh;
                fresh
                    .view
                    .as_mut()
                    .unwrap()
                    .home
                    .as_mut()
                    .unwrap()
                    .workspace
                    .configure(&Config::read(path.clone()).unwrap())
                    .unwrap();
                let mut snapshot = app::tests::snapshot(ALL, serde_json::json!([]));
                snapshot.tabs.push(ALL.into());
                snapshot.view = Ok(fresh.view.take().unwrap());
                events.send(snapshot_event(snapshot)).unwrap();
            },
            |_, _| {},
            |_| {},
            |job| match job {
                lane::Job::Act(Request::WorkspacePreferences {
                    expected,
                    next,
                    saved,
                }) => {
                    assert_eq!(saved, None);
                    assert_eq!(expected.layout, crate::config::OpsLayout::Auto);
                    assert_eq!(next.layout, crate::config::OpsLayout::Side);
                    saves.set(saves.get() + 1);
                    Config::read(path.clone())
                        .unwrap()
                        .save_workspace_preferences(&expected, &next)
                        .unwrap();
                    events
                        .send(acted(false, true, Ok("Workspace preferences saved.")))
                        .unwrap();
                    events.send(key(KeyCode::Char('j'))).unwrap();
                }
                lane::Job::Jump {
                    member: identity,
                    seen: None,
                    ..
                } => {
                    assert_eq!(identity, app::workspace::tests::MEMBER);
                    jumps.set(jumps.get() + 1);
                    events.send(acted(true, false, Ok("Jumped"))).unwrap();
                    events.send(key(KeyCode::Char('q'))).unwrap();
                }
                other => panic!("unexpected {other:?}"),
            },
            |app| {
                app.set_body_width(100);
                terminal.draw(|frame| view::render(frame, app)).unwrap();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!((saves.get(), jumps.get()), (1, 1));
        assert_eq!(
            app.workspace_snapshot().unwrap().preferences.layout,
            crate::config::OpsLayout::Side
        );
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("# intact"));
        assert_eq!(
            Config::read(path)
                .unwrap()
                .workspace_preferences()
                .unwrap()
                .layout,
            crate::config::OpsLayout::Side
        );
        assert!(app.menu.is_none());
        std::fs::remove_dir_all(directory).unwrap();
        reloads.into_inner()
    }

    #[test]
    fn workspace_layout_saves_once_on_the_lane_and_refreshes_before_host_navigation() {
        // Periodic reload off: the one reload is the save's own, however long the run takes.
        assert_eq!(
            workspace_layout_session(None),
            [(Some(ALL.into()), false, false)]
        );
    }

    #[test]
    fn a_due_periodic_reload_is_a_separate_request_from_the_save_refresh() {
        // #3884: on a loaded runner the default interval elapses mid-scenario and the board asks
        // again for the same tab. An interval that is always due shows that request is the timer's.
        let reloads = workspace_layout_session(Some(std::time::Duration::ZERO));
        assert!(reloads.len() > 1, "{reloads:?}");
        assert!(
            reloads
                .iter()
                .all(|reload| reload == &(Some(ALL.into()), false, false))
        );
    }

    #[test]
    fn theme_confirm_saves_once_then_restores_normal_row_actions() {
        let directory =
            std::env::temp_dir().join(format!("tmt-theme-session-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ops.toml");
        std::fs::write(&path, "# kept\n[board.theme]\nbase = \"tmt\"\n").unwrap();
        let (events, input) = channel();
        let mut app = theme_board();
        events.send(key(KeyCode::Char('T'))).unwrap();
        let reads = std::cell::Cell::new(0);
        let actions = std::cell::Cell::new(0);
        let reloads = std::cell::Cell::new(0);
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| reloads.set(reloads.get() + 1),
            |_, _| {},
            |_| {},
            |job| match job {
                lane::Job::Open(opening) => {
                    reads.set(reads.get() + 1);
                    let config = Config::read(path.clone()).map_err(|error| error.message);
                    events
                        .send(BoardEvent::Opened {
                            opening,
                            config,
                            board_pane: None,
                        })
                        .unwrap();
                    for code in [KeyCode::Down, KeyCode::Enter, KeyCode::Char('j')] {
                        events.send(key(code)).unwrap();
                    }
                }
                lane::Job::Jump {
                    member, seen: None, ..
                } => {
                    assert_eq!(member, app::workspace::tests::MEMBER);
                    actions.set(actions.get() + 1);
                    events.send(acted(true, false, Ok("Jumped"))).unwrap();
                    events.send(key(KeyCode::Char('q'))).unwrap();
                }
                other => panic!("unexpected {other:?}"),
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!((reads.get(), actions.get(), reloads.get()), (1, 1, 1));
        assert!(app.theme_picker.is_none());
        assert_eq!(app.look().theme.base, tmt_cli_style::Base::TmtLight);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# kept\n[board.theme]\nbase = \"tmt-light\"\n"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn overview_theme_picker_cannot_save_a_squad_scope() {
        let directory = std::env::temp_dir().join(format!("tmt-user-theme-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ops.toml");
        std::fs::write(&path, "# Overview has no squad scope\n").unwrap();
        let (events, input) = channel();
        let mut app = theme_board();
        events.send(key(KeyCode::Char('T'))).unwrap();
        let mut checked = false;
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            |job| {
                let lane::Job::Open(opening) = job else {
                    panic!("unexpected {job:?}")
                };
                let config = Config::read(path.clone()).map_err(|error| error.message);
                events
                    .send(BoardEvent::Opened {
                        opening,
                        config,
                        board_pane: None,
                    })
                    .unwrap();
                for code in [
                    KeyCode::Down,
                    KeyCode::Tab,
                    KeyCode::Enter,
                    KeyCode::Char('q'),
                ] {
                    events.send(key(code)).unwrap();
                }
            },
            |app| {
                if let Some(picker) = &app.theme_picker {
                    assert_eq!(picker.scope, crate::theme::ThemeScope::Board);
                    checked = true;
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(checked);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("[board.theme]"));
        assert!(!written.contains("[squad."));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn theme_session_reports_stale_save_without_retry_and_escape_never_writes() {
        let directory =
            std::env::temp_dir().join(format!("tmt-theme-session-stale-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ops.toml");
        std::fs::write(&path, "[board.theme]\nbase = \"tmt\"\n").unwrap();
        let (events, input) = channel();
        let mut app = theme_board();
        events.send(key(KeyCode::Char('T'))).unwrap();
        let mut changed = false;
        let mut failure_shown = false;
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| panic!("a failed save must not reload or retry"),
            |_, _| {},
            |_| {},
            |job| {
                let lane::Job::Open(opening) = job else {
                    panic!("unexpected {job:?}")
                };
                let config = Config::read(path.clone()).map_err(|error| error.message);
                events
                    .send(BoardEvent::Opened {
                        opening,
                        config,
                        board_pane: None,
                    })
                    .unwrap();
                for code in [
                    KeyCode::Down,
                    KeyCode::Enter,
                    KeyCode::Esc,
                    KeyCode::Char('q'),
                ] {
                    events.send(key(code)).unwrap();
                }
            },
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

    #[test]
    #[should_panic(expected = "input/render thread")]
    fn a_command_on_the_session_thread_is_a_test_failure() {
        let _ui = crate::runner::forbid_commands();
        let _ = crate::runner::run(
            std::path::Path::new("/usr/bin/true"),
            &[],
            b"",
            Duration::from_secs(1),
            1024,
        );
    }

    #[test]
    fn a_slow_send_runs_on_the_lane_while_the_board_keeps_painting() {
        let directory = std::env::temp_dir().join(format!("ops-lane-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let fake = directory.join("tmt");
        let driver = fake.with_extension("py");
        std::fs::write(&driver, r#"import json, sys, pathlib, time
root=pathlib.Path(__file__).parent
if sys.argv[1]=='config': out={'paths':{'global':str(root/'config.toml')}}
elif sys.argv[1]=='identity': out={'identityId':sys.argv[sys.argv.index('--identity')+1],'metadata':{'user':'true'}}
else:
 q=json.load(sys.stdin); op=q['operation']; i=q['input']
 if op=='storage.root': out={'dataRoot':str(root)}
 elif op=='identities.status': out={'identities':[{'id':id,'found':True,'canPassthrough':{'available':True}} for id in i['identityIds']]}
 elif op=='dispatch.create':
  time.sleep(0.5)
  out={'operationId':i['operationId'],'items':[{'recipientId':i['recipientIds'][0],'requestId':'req_1','acceptance':'queued'}],'wake':{'status':'sent'}}
 else: raise AssertionError(op)
print(json.dumps(out))
"#).unwrap();
        crate::test_support::write_ready_executable(
            &fake,
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 '{}' \"$@\"\n",
                driver.display()
            ),
        );
        let mut app = app::workspace::tests::board();
        let (events, input) = channel();
        events.send(key(KeyCode::Enter)).unwrap();
        events
            .send(BoardEvent::Input(Event::Paste("hello".into())))
            .unwrap();
        events.send(key(KeyCode::Enter)).unwrap();
        let lane = lane::Lane::spawn(Core::at(fake), None, events.clone());
        let started = Instant::now();
        let mut notices: Vec<(Duration, Option<String>)> = Vec::new();
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            |job| lane.submit(job),
            |app| {
                notices.push((started.elapsed(), app.notice.clone()));
                if app
                    .notice
                    .as_deref()
                    .is_some_and(|notice| notice == "Sent to worker")
                {
                    events.send(key(KeyCode::Esc)).unwrap();
                    events.send(key(KeyCode::Char('q'))).unwrap();
                }
                Ok(())
            },
        )
        .unwrap();
        lane.finish();
        let placeholder = notices
            .iter()
            .find(|(_, notice)| notice.as_deref() == Some("Sending…"))
            .expect("a placeholder frame");
        assert!(
            placeholder.0 < Duration::from_millis(300),
            "the placeholder paints long before the 500 ms command ends: {:?}",
            placeholder.0
        );
        let done = notices
            .iter()
            .find(|(_, notice)| notice.as_deref() == Some("Sent to worker"))
            .expect("typed completion frame");
        assert_eq!(done.1.as_deref(), Some("Sent to worker"));
        assert!(done.0 >= Duration::from_millis(450));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn an_overlay_is_requested_once_while_its_config_is_still_loading() {
        let mut app = theme_board();
        let (events, input) = channel();
        for code in [KeyCode::Char('T'), KeyCode::Char('T'), KeyCode::Char('q')] {
            events.send(key(code)).unwrap();
        }
        let jobs = std::cell::Cell::new(0);
        let mut notices = Vec::new();
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            |_| jobs.set(jobs.get() + 1),
            |app| {
                notices.push(app.notice.clone());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(jobs.get(), 1);
        assert!(
            notices.contains(&Some("Still working on the previous request.".to_owned())),
            "{notices:?}"
        );
    }

    fn acted(jump: bool, sends: bool, outcome: Result<&str, String>) -> BoardEvent {
        BoardEvent::Acted {
            jump,
            sends,
            outcome: outcome.map(|message| ActionOutcome::Message(message.to_owned())),
        }
    }

    fn no_io(job: lane::Job) {
        panic!("unexpected {job:?}")
    }

    /// A stand-in core that logs each call: `answer` succeeds, anything else
    /// fails, so the test proves the board needs no other core command.
    fn snapshot_event(snapshot: Snapshot) -> BoardEvent {
        BoardEvent::Snapshot {
            cancellation: crate::runner::Cancellation::default(),
            snapshot: Box::new(snapshot),
        }
    }

    #[test]
    fn a_published_digest_window_wakes_the_existing_reader_once_and_cancelled_views_do_not() {
        use crate::config::DigestWindow;
        let mut app = app::workspace::tests::board();
        let make = |window| {
            let mut snapshot = app::tests::snapshot(ALL, serde_json::json!([]));
            snapshot.view = Ok(app::workspace::tests::board().view.take().unwrap());
            snapshot
                .view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .preferences
                .digest_window = window;
            snapshot
        };
        let (events, input) = channel();
        events
            .send(snapshot_event(make(DigestWindow::Week)))
            .unwrap();
        events
            .send(snapshot_event(make(DigestWindow::Week)))
            .unwrap();
        let cancelled = crate::runner::Cancellation::default();
        cancelled.cancel();
        events
            .send(BoardEvent::Snapshot {
                cancellation: cancelled,
                snapshot: Box::new(make(DigestWindow::AllTime)),
            })
            .unwrap();
        events.send(BoardEvent::InputClosed).unwrap();
        let windows = std::cell::RefCell::new(Vec::new());
        super::session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            no_io,
            |window| windows.borrow_mut().push(window),
            |_| {},
            || Ok(()),
            || Ok(()),
            None,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(*windows.borrow(), [DigestWindow::Week]);
        assert_eq!(app.digest_window(), DigestWindow::Week);
    }

    fn key(code: KeyCode) -> BoardEvent {
        BoardEvent::Input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    #[test]
    fn long_chart_demand_uses_existing_refresh_and_does_not_reacquire_a_closed_range() {
        crate::status::with_now_ms(1_791_600_000_000, || {
            let mut app = theme_board();
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(103, 51)).unwrap();
            let paint =
                |app: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
                    terminal
                        .draw(|frame| {
                            view::prepare(app, frame.area());
                            view::render(frame, app);
                        })
                        .unwrap();
                };
            paint(&mut app, &mut terminal);
            for code in [
                KeyCode::Char('u'),
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Enter,
                KeyCode::Tab,
                KeyCode::Enter,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Enter,
                KeyCode::Esc,
            ] {
                assert_eq!(
                    app.key(KeyEvent::new(code, KeyModifiers::NONE)),
                    Effect::None
                );
                paint(&mut app, &mut terminal);
            }
            let expected = rate::history::rollup::Request::new(900_000, 21_600_000).unwrap();
            assert_eq!(app.retained_history_request(), Some(expected));
            let (events, input) = channel();
            let requests = std::cell::RefCell::new(Vec::new());
            let draws = std::cell::Cell::new(0);
            let ended = session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |owner, preempt, preview, chart| {
                    assert_eq!(owner.as_deref(), Some(ALL));
                    assert!(preempt && !preview);
                    requests.borrow_mut().push(chart);
                    match requests.borrow().len() {
                        // Moving within the closed range, then to Digest, which shows
                        // the same chart, changes nothing the refresh would reacquire.
                        1 => {
                            for code in [
                                KeyCode::Down,
                                KeyCode::Down,
                                KeyCode::Up,
                                KeyCode::Char('2'),
                                KeyCode::Char('q'),
                            ] {
                                events.send(key(code)).unwrap();
                            }
                        }
                        _ => panic!("unchanged closed range must not reacquire"),
                    }
                },
                |_, _| {},
                |_| {},
                |_| panic!("read-only controls must not act"),
                |app| {
                    draws.set(draws.get() + 1);
                    paint(app, &mut terminal);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(ended, None);
            assert_eq!(
                requests
                    .borrow()
                    .iter()
                    .map(|d| d.and_then(|d| d.chart))
                    .collect::<Vec<_>>(),
                [Some(expected)]
            );
            assert!(draws.get() >= 3);
        });
    }

    #[test]
    fn suspension_resumes_the_same_session_and_pending_selected_read() {
        let mut app = app::workspace::tests::board();
        app.view.as_mut().unwrap().replies = vec![serde_json::json!({
            "recipientId": app::workspace::tests::MEMBER,
            "requestId": "req-pending",
            "submittedAtMs": 20,
            "response": null
        })];
        app.workspace.selected = Some(workspace::OpsTarget::Member {
            room_id: Some(app::workspace::tests::ROOM.into()),
            identity_id: app::workspace::tests::MEMBER.into(),
        });
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 36)).unwrap();
        terminal
            .draw(|frame| {
                view::prepare(&mut app, frame.area());
                view::render(frame, &app);
            })
            .unwrap();
        assert!(
            app.selected_read().is_some(),
            "painted fixture has an unread source"
        );
        let stop = AtomicUsize::new(0);
        let (events, input) = channel();
        events.send(BoardEvent::Input(Event::FocusGained)).unwrap();
        events.send(BoardEvent::InputClosed).unwrap();
        let reads = std::cell::Cell::new(0);
        let pauses = std::cell::Cell::new(0);
        let paints = std::cell::Cell::new(0);
        super::session(
            &mut app,
            &stop,
            &input,
            |_, _, _, _| {},
            |_, _| {
                reads.set(reads.get() + 1);
                stop.store(signal_hook::consts::SIGTSTP as usize, Ordering::Relaxed);
            },
            |_| {},
            no_io,
            |_| {},
            |_| {},
            || {
                pauses.set(pauses.get() + 1);
                events.send(BoardEvent::InputClosed).unwrap();
                Ok(())
            },
            || Ok(()),
            None,
            |_| {
                paints.set(paints.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(pauses.get(), 1);
        assert_eq!(reads.get(), 1, "the pending selected read is not restarted");
        assert!(paints.get() >= 2, "resume repaints the retained state");
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
                |_, _, _, _| {},
                |_, _| {},
                |_| {},
                no_io,
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
            let mut app = app::workspace::tests::board();
            app.popup = popup;
            events.send(key(KeyCode::Char('j'))).unwrap();
            let jumps = std::cell::Cell::new(0);
            let ended = session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _, _, _| {},
                |_, _| {},
                |_| {},
                |job| {
                    let lane::Job::Jump {
                        member, seen: None, ..
                    } = job
                    else {
                        panic!("unexpected {job:?}")
                    };
                    assert_eq!(member, app::workspace::tests::MEMBER);
                    jumps.set(jumps.get() + 1);
                    events
                        .send(BoardEvent::Acted {
                            jump: true,
                            sends: false,
                            outcome: outcome.clone().map(ActionOutcome::Message),
                        })
                        .unwrap();
                    events.send(BoardEvent::InputClosed).unwrap();
                },
                |_| Ok(()),
            )
            .unwrap();
            (ended, jumps.get())
        };
        assert_eq!(run(true, Ok("Showing auth-fix.".into())), (None, 1));
        assert_eq!(run(true, Err("tmt focus failed".into())), (Some(HANGUP), 1));
        assert_eq!(
            run(false, Ok("Showing auth-fix.".into())),
            (Some(HANGUP), 1)
        );
    }

    #[test]
    fn timing_milestones_follow_successful_draw_and_snapshot_admission() {
        for case in ["fresh", "cancelled", "failed", "other-tab", "draw-error"] {
            let (trace, bytes) = timing::Trace::buffer();
            let mut app = App::new(Some("product".into()));
            app.timing = Some(timing::Ui::new(trace.clone()));
            let (events, input) = channel();
            let mut frames = 0;
            let result = session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _, _, _| {},
                |_, _| {},
                |_| {},
                no_io,
                |_| {
                    if case == "draw-error" {
                        return Err(io::Error::other("draw refused"));
                    }
                    frames += 1;
                    if frames == 1 {
                        let tab = if case == "other-tab" {
                            "infra"
                        } else {
                            "product"
                        };
                        let mut snapshot = app::tests::snapshot(tab, serde_json::json!([]));
                        snapshot.timing = Some(trace.load(Some(tab), 3));
                        if case == "failed" {
                            snapshot.view = Err("read failed".into());
                        }
                        let cancellation = crate::runner::Cancellation::default();
                        if case == "cancelled" {
                            cancellation.cancel();
                        }
                        events
                            .send(BoardEvent::Snapshot {
                                cancellation,
                                snapshot: Box::new(snapshot),
                            })
                            .unwrap();
                        events.send(key(KeyCode::Char('q'))).unwrap();
                    }
                    Ok(())
                },
            );
            assert_eq!(result.is_err(), case == "draw-error");
            let bytes = bytes.lock().unwrap();
            let records: Vec<serde_json::Value> = std::str::from_utf8(&bytes)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record["event"] == "first_frame")
                    .count(),
                usize::from(case != "draw-error"),
                "{case}"
            );
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record["event"] == "fresh_board")
                    .count(),
                usize::from(case == "fresh"),
                "{case}"
            );
            if case == "fresh" {
                assert_eq!(records[1]["generation"], 3);
            }
        }
    }

    #[test]
    fn a_failed_redraw_ends_the_session_with_its_error() {
        let (_events, input) = channel();
        let error = session(
            &mut App::new(None),
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            no_io,
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
            let mut app = app::workspace::tests::board();
            app.view.as_mut().unwrap().refresh = None;
            app.select(0);
            super::session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _, _, _| {},
                |_, _| {},
                |_| {},
                no_io,
                |_| {},
                |_| {},
                || Ok(()),
                || Ok(()),
                None,
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
        let mut replacement = app::workspace::tests::board();
        let mut snapshot = app::tests::snapshot(ALL, document.clone());
        replacement.view.as_mut().unwrap().refresh = None;
        replacement.view.as_mut().unwrap().document["sections"] = document.clone();
        snapshot.view = Ok(replacement.view.take().unwrap());
        snapshot.tabs.push(ALL.into());
        events.send(snapshot_event(snapshot)).unwrap();
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
    fn pointer_motion_repaints_only_when_meter_target_changes() {
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent};
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
                |_, _, _, _| {},
                |_, _| {},
                |_| {},
                no_io,
                |_| {},
                |_| {},
                || Ok(()),
                || Ok(()),
                None,
                |app| {
                    *app.hits.borrow_mut() = vec![app::Hit {
                        y: 1,
                        x: 40,
                        width: 10,
                        target: app::HitTarget::Meter(None),
                    }];
                    painted.send(app.meter_hover).unwrap();
                    Ok(())
                },
            )
            .unwrap()
        });
        assert_eq!(frames.recv_timeout(Duration::from_secs(2)).unwrap(), None);
        let moved = |column| {
            BoardEvent::Input(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }))
        };
        events.send(moved(40)).unwrap();
        assert_eq!(
            frames.recv_timeout(Duration::from_secs(2)).unwrap(),
            Some(None)
        );
        events.send(moved(45)).unwrap();
        assert!(
            frames.recv_timeout(INPUT_WAIT * 2).is_err(),
            "motion within the same group never repaints"
        );
        events.send(moved(39)).unwrap();
        assert_eq!(frames.recv_timeout(Duration::from_secs(2)).unwrap(), None);
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
                    |squad, _, _, _| {
                        requested.send(squad).unwrap();
                    },
                    |_, _| {},
                    |_| {},
                    no_io,
                    |_| {},
                    |_| {},
                    || Ok(()),
                    || Ok(()),
                    None,
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
                |_, _, _, _| {},
                |_, _| {},
                |_| {},
                no_io,
                |_| {},
                |_| {},
                || Ok(()),
                || Ok(()),
                None,
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
    fn supplied_labels_arrive_as_an_event_and_replace_what_was_shown() {
        use tmt_cli_style::Role;
        let labels = |text: &str| {
            crate::labels::Supplied::from_rows(vec![(
                "digest".into(),
                [(
                    "m".to_owned(),
                    vec![crate::labels::Label {
                        text: text.into(),
                        role: Role::Text,
                        action: None,
                    }],
                )]
                .into(),
            )])
        };
        let (events, input) = channel();
        let mut app = App::new(Some("product".into()));
        events.send(BoardEvent::Labels(labels("Auto"))).unwrap();
        events.send(BoardEvent::Labels(labels("Every 5m"))).unwrap();
        events.send(key(KeyCode::Char('q'))).unwrap();
        assert_eq!(
            session(
                &mut app,
                &AtomicUsize::new(0),
                &input,
                |_, _, _, _| {},
                |_, _| {},
                |_| {},
                no_io,
                |_| Ok(())
            )
            .unwrap(),
            None
        );
        assert_eq!(app.labels, labels("Every 5m"));
        // A source going away is an empty event: the policy chip shows again.
        let (events, input) = channel();
        events.send(BoardEvent::Labels(Default::default())).unwrap();
        events.send(key(KeyCode::Char('q'))).unwrap();
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            no_io,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(app.labels, Default::default());
    }

    #[test]
    fn a_failed_caller_lookup_arrives_as_an_event_and_is_kept_for_the_sender_prompt() {
        let (events, input) = channel();
        let mut app = App::new(Some("product".into()));
        assert!(app.caller_failure.is_none());
        events
            .send(BoardEvent::CallerFailed(
                "Herdr reported an invalid pane.".into(),
            ))
            .unwrap();
        events.send(key(KeyCode::Char('q'))).unwrap();
        session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            no_io,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            app.caller_failure.as_deref(),
            Some("Herdr reported an invalid pane.")
        );
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
                |_, _, _, _| {},
                |_, _| {},
                |_| {},
                no_io,
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
    /// A quiet product board whose installed binary was just replaced.
    fn replaced_board(name: &str) -> (App, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("ops-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("state")).unwrap();
        let program = root.join("tmt-ops");
        std::fs::write(&program, "one").unwrap();
        std::fs::set_permissions(
            &program,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let mut app = App::new(Some("product".into()));
        let mut loaded = app::tests::snapshot(
            "product",
            serde_json::json!([{"rows":[{"id":"A","name":"a"},{"id":"B","name":"b"}]}]),
        );
        loaded.view.as_mut().unwrap().refresh = None;
        app.apply(loaded);
        app.reload = reload::Watch::over(&program, &root.join("state"), 1_000);
        // The installed binary is replaced after the board started.
        std::fs::remove_file(&program).unwrap();
        std::fs::write(&program, "two").unwrap();
        std::fs::set_permissions(
            &program,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        (app, root)
    }

    fn run_session(app: &mut App, events: Vec<BoardEvent>) -> Option<i32> {
        let (sender, input) = channel();
        for event in events {
            sender.send(event).unwrap();
        }
        session(
            app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            no_io,
            |_| Ok(()),
        )
        .unwrap()
    }

    /// The terminal can drop the mouse request without telling the board (a
    /// nested client, a multiplexer or an emulator reset). Each of these moments
    /// asks for it again, so the wheel never falls to tmux's copy-mode.
    #[test]
    fn resize_focus_a_closed_overlay_and_the_refresh_tick_each_ask_for_the_mouse_again() {
        // Returns how often the mouse was asked for, how often a frame was drawn
        // and whether an overlay was ever open, for a session fed `events`.
        let run = |events: Vec<BoardEvent>, refresh: Option<Duration>| {
            let mut app = app::workspace::tests::board();
            app.view.as_mut().unwrap().refresh = refresh;
            let (sender, input) = channel();
            let asked = std::cell::Cell::new(0);
            let frames = std::cell::Cell::new(0);
            let overlay_seen = std::cell::Cell::new(false);
            let stop = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                // The ticking case needs wall time; the others end with their input.
                if refresh.is_some() {
                    let sender = sender.clone();
                    scope.spawn(move || {
                        std::thread::sleep(Duration::from_millis(200));
                        let _ = sender.send(BoardEvent::InputClosed);
                    });
                }
                for event in events {
                    sender.send(event).unwrap();
                }
                super::session(
                    &mut app,
                    &stop,
                    &input,
                    |_, _, _, _| {},
                    |_, _| {},
                    |_| {},
                    no_io,
                    |_| {},
                    |_| {},
                    || Ok(()),
                    || {
                        asked.set(asked.get() + 1);
                        Ok(())
                    },
                    None,
                    |app| {
                        frames.set(frames.get() + 1);
                        overlay_seen.set(overlay_seen.get() | app.modal_open());
                        Ok(())
                    },
                )
                .unwrap();
            });
            (asked.get(), frames.get(), overlay_seen.get())
        };
        let closed = || BoardEvent::InputClosed;
        assert_eq!(
            run(vec![closed()], None).0,
            0,
            "a quiet board asks for nothing"
        );
        assert_eq!(
            run(
                vec![BoardEvent::Input(Event::Resize(100, 30)), closed()],
                None
            )
            .0,
            1
        );
        assert_eq!(
            run(vec![BoardEvent::Input(Event::FocusGained), closed()], None).0,
            1
        );
        // An open overlay alone does not ask; closing it does.
        let opened = run(vec![key(KeyCode::Char('?')), closed()], None);
        assert_eq!(
            (opened.0, opened.2),
            (0, true),
            "help is open: nothing asked yet"
        );
        let shut = run(
            vec![key(KeyCode::Char('?')), key(KeyCode::Esc), closed()],
            None,
        );
        assert_eq!((shut.0, shut.2), (1, true), "closing help asks again");
        // An idle board repairs itself on the existing refresh tick, without a frame.
        let (asked, frames, _) = run(vec![], Some(Duration::from_millis(20)));
        assert!(asked >= 3, "each refresh tick asks again, asked {asked}");
        assert_eq!(frames, 1, "ticks are not paint changes");
    }

    fn wheel(kind: MouseEventKind, column: u16) -> ratatui::crossterm::event::MouseEvent {
        ratatui::crossterm::event::MouseEvent {
            kind,
            column,
            row: 3,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// A burst joins one frame without losing or reordering a step, and ends at
    /// the first event that is not a wheel step: that event is handed back to
    /// land before the wheel steps queued behind it.
    #[test]
    fn a_wheel_burst_stops_at_a_click_and_keeps_every_step_in_order() {
        let (sender, input) = channel();
        let down = |column| wheel(MouseEventKind::ScrollDown, column);
        let up = |column| wheel(MouseEventKind::ScrollUp, column);
        // Any-motion reporting sends pointer moves between the wheel steps.
        for event in [down(1), wheel(MouseEventKind::Moved, 7), up(2)] {
            sender.send(BoardEvent::Input(Event::Mouse(event))).unwrap();
        }
        sender
            .send(BoardEvent::Input(Event::Mouse(wheel(
                MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left),
                9,
            ))))
            .unwrap();
        for event in [down(3), down(4)] {
            sender.send(BoardEvent::Input(Event::Mouse(event))).unwrap();
        }
        let mut applied = Vec::new();
        let (effect, deferred) = coalesce_wheel(down(0), &input, |event| {
            applied.push((event.kind, event.column));
            Effect::None
        });
        assert_eq!(effect, Effect::None);
        assert_eq!(
            applied,
            [
                (MouseEventKind::ScrollDown, 0),
                (MouseEventKind::ScrollDown, 1),
                (MouseEventKind::Moved, 7),
                (MouseEventKind::ScrollUp, 2),
            ],
            "wheel, wheel, move, wheel in arrival order; none dropped"
        );
        assert!(matches!(
            deferred,
            Some(BoardEvent::Input(Event::Mouse(click))) if click.column == 9
        ));
        // The wheel steps behind the click are still queued, in order.
        let mut rest = Vec::new();
        while let Ok(BoardEvent::Input(Event::Mouse(event))) = input.try_recv() {
            rest.push(event.column);
        }
        assert_eq!(rest, [3, 4]);
        // A key, a resize and a focus change end a burst the same way.
        for other in [
            key(KeyCode::Char('x')),
            BoardEvent::Input(Event::Resize(80, 24)),
            BoardEvent::Input(Event::FocusGained),
        ] {
            sender.send(other).unwrap();
            sender
                .send(BoardEvent::Input(Event::Mouse(down(5))))
                .unwrap();
            let (_, deferred) = coalesce_wheel(down(0), &input, |_| Effect::None);
            assert!(
                deferred.is_some_and(|event| !matches!(event, BoardEvent::Input(Event::Mouse(_))))
            );
            assert!(matches!(
                input.try_recv(),
                Ok(BoardEvent::Input(Event::Mouse(_)))
            ));
        }
        // A burst is bounded; the remainder follows on the next turn.
        for column in 0..(WHEEL_BURST as u16 + 10) {
            sender
                .send(BoardEvent::Input(Event::Mouse(down(column))))
                .unwrap();
        }
        let mut count = 0;
        let _ = coalesce_wheel(down(0), &input, |_| {
            count += 1;
            Effect::None
        });
        assert_eq!(count, WHEEL_BURST + 1);
        assert_eq!(input.try_iter().count(), 10);
        // A non-wheel first event is applied alone.
        sender
            .send(BoardEvent::Input(Event::Mouse(down(1))))
            .unwrap();
        let mut count = 0;
        let click = wheel(MouseEventKind::Moved, 0);
        let _ = coalesce_wheel(click, &input, |_| {
            count += 1;
            Effect::None
        });
        assert_eq!(count, 1);
        assert_eq!(input.try_iter().count(), 1);
    }

    /// Ten queued wheel steps with a pointer move after each paint one frame (plus the first).
    #[test]
    fn a_wheel_burst_in_the_session_paints_one_frame() {
        let mut app = app::workspace::tests::board();
        let (sender, input) = channel();
        for column in 0..10 {
            for kind in [MouseEventKind::ScrollDown, MouseEventKind::Moved] {
                sender
                    .send(BoardEvent::Input(Event::Mouse(wheel(kind, column))))
                    .unwrap();
            }
        }
        sender.send(BoardEvent::InputClosed).unwrap();
        let frames = std::cell::Cell::new(0);
        super::session(
            &mut app,
            &AtomicUsize::new(0),
            &input,
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
            no_io,
            |_| {},
            |_| {},
            || Ok(()),
            || Ok(()),
            None,
            |_| {
                frames.set(frames.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            frames.get(),
            2,
            "the opening frame and one frame for the whole burst"
        );
    }

    #[test]
    fn an_idle_board_whose_binary_was_replaced_ends_its_session_to_reload_with_its_place() {
        let (mut app, root) = replaced_board("idle");
        app.selected = 1;
        assert_eq!(run_session(&mut app, vec![]), None);
        let resume = app.restart.take().expect("the session ended to reload");
        assert_eq!(resume.tab, "product");
        assert!(matches!(
            resume.selected,
            Some(app::RowTarget::Member { ref id, .. }) if id == "B"
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_board_with_a_draft_open_keeps_running_until_it_is_cancelled() {
        let draft = || app::Input {
            draft_key: None,

            inline_quotes: Default::default(),

            row_send: None,
            link: None,
            prompt: String::new(),
            text: "half a thought".into(),
            compose: app::Compose::AskLead {
                to: "lead".into(),
                sender: "ben".into(),
            },
            squad: "product".into(),
            hint: None,
        };
        // The draft is still there when input ends: the reload never ran.
        let (mut app, root) = replaced_board("draft");
        app.input = Some(draft());
        assert_eq!(
            run_session(&mut app, vec![BoardEvent::InputClosed]),
            Some(HANGUP)
        );
        assert!(app.restart.is_none() && app.input.is_some());
        assert!(
            app.reload_waiting(),
            "the footer says the reload is waiting"
        );

        // Cancelling the draft lets the pending reload through.
        let (mut app, _) = replaced_board("draft");
        app.input = Some(draft());
        assert_eq!(run_session(&mut app, vec![key(KeyCode::Esc)]), None);
        assert!(app.input.is_none() && app.restart.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_board_with_nothing_to_reload_ends_only_when_asked() {
        let mut quiet = App::new(Some("product".into()));
        quiet.apply(app::tests::snapshot("product", serde_json::json!([])));
        assert_eq!(run_session(&mut quiet, vec![key(KeyCode::Char('q'))]), None);
        assert!(quiet.restart.is_none(), "an inert watch never reloads");
        assert_eq!(
            run_session(&mut quiet, vec![BoardEvent::InputClosed]),
            Some(HANGUP)
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
                |_, _, _, _| panic!("automatic refresh was turned off by the snapshot"),
                |_, _| {},
                |_| {},
                no_io,
                |_| Ok(())
            )
            .unwrap(),
            None
        );
    }
}
