//! Binary-private serve lifecycle composition: the foreground serve, the detached worker and the
//! launcher that starts it. The handoff, cleanup and failure-record rules live once in
//! `tmt-extension-serve`; this module only supplies Colab's readiness shape, wording and codes.
use crate::{open, status, supervisor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tmt_colab::{
    Result,
    assets::App,
    keyring::{Keyring, Layout},
    registration::Registration,
    socket::{MountSocket, Tunnels},
    store::Store,
};
use tmt_extension_serve::{
    ErrorRecord, Handoff, Handshake, Launch, StartupError, Timing, adopt, launch,
};

/// One absolute bound for everything before the handoff: lock, store, socket and door.
const STARTUP: Duration = Duration::from_secs(30);
/// How long a launcher waits for a cancelled worker to confirm its cleanup.
const CLEANUP_WAIT: Duration = Duration::from_secs(10);
/// The bound of the readiness frame and of the private failure record.
const RECORD_BYTES: usize = 16 * 1024;
const RUN_HINT: &str = "run tmt colab serve --foreground to see why";
const STOP_HINT: &str = "run tmt colab stop before starting again";

/// A startup outcome with a stable code and the next step, kept apart from the Colab fault types
/// because a launcher receives its code from the worker as text.
#[derive(Debug)]
pub(crate) struct StartFault {
    pub code: String,
    message: String,
    pub hint: Option<String>,
}
impl std::fmt::Display for StartFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for StartFault {}
impl StartFault {
    fn cancelled() -> Self {
        handshake().cancelled.into()
    }
}
impl From<StartupError> for StartFault {
    fn from(error: StartupError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            hint: error.hint,
        }
    }
}
impl From<StartFault> for StartupError {
    fn from(fault: StartFault) -> Self {
        let error = Self::new(fault.code, fault.message);
        match fault.hint {
            Some(hint) => error.with_hint(hint),
            None => error,
        }
    }
}

/// The two outcomes the launcher cannot settle, in Colab's words.
fn handshake() -> Handshake {
    Handshake {
        unconfirmed: StartupError::new(
            "COLAB_STARTUP_UNCONFIRMED",
            "Colab startup could not be confirmed.",
        )
        .with_hint(STOP_HINT),
        cancelled: StartupError::new(
            "COLAB_STARTUP_CANCELLED",
            "Colab startup was cancelled before handoff.",
        ),
        timing: Timing {
            startup: STARTUP,
            cleanup_wait: CLEANUP_WAIT,
            record_bytes: RECORD_BYTES,
        },
    }
}

/// A cancelled startup stops before the handoff. Only a worker can be cancelled that way: an
/// interrupted foreground serve keeps its ordinary clean stop.
fn fence(stop: &AtomicBool, detached: bool) -> Result<()> {
    if detached && stop.load(Ordering::SeqCst) {
        Err(StartFault::cancelled().into())
    } else {
        Ok(())
    }
}

/// What a person reads and an agent parses once the space is mounted. The worker builds it from
/// live state and sends it as the readiness record; the launcher presents it after it opened the
/// page, because only the launcher has the terminal.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Presentation {
    /// The `--json` line, except `opened` and `background`, which the presenter settles.
    status: Value,
    rows: Vec<(String, String)>,
    /// The link to open in a browser, only while a door runs.
    link: Option<String>,
    /// The line about missing browser access and its next step, if any.
    warning: Option<(String, Option<String>)>,
}
impl Presentation {
    fn from_live(status: &status::Status<'_>, access: &supervisor::Access) -> Self {
        Self {
            status: status.json(),
            rows: status
                .rows()
                .into_iter()
                .map(|(label, value)| (label.to_owned(), value))
                .collect(),
            link: status.open_link(),
            warning: access
                .warning()
                .map(|(what, hint)| (what.to_owned(), hint.map(str::to_owned))),
        }
    }
    /// Refuses anything a worker of this release would not send.
    fn validate(value: Value) -> std::result::Result<Value, StartupError> {
        let unconfirmed = || handshake().unconfirmed;
        let presentation: Self = serde_json::from_value(value).map_err(|_| unconfirmed())?;
        let plain = |text: &str, limit: usize| {
            text.len() <= limit && !text.chars().any(|c| c.is_control() && c != '\n')
        };
        let status = presentation.status.as_object().ok_or_else(unconfirmed)?;
        if status.get("profile").and_then(Value::as_str) != Some("colab-sync-v1")
            || status.get("state").and_then(Value::as_str) != Some("mounted")
            || presentation.rows.len() > 16
            || presentation
                .rows
                .iter()
                .any(|(label, value)| !plain(label, 32) || !plain(value, 2048))
            || presentation
                .link
                .as_deref()
                .is_some_and(|link| !plain(link, 1024) || !link.starts_with("http://127.0.0.1:"))
            || presentation.warning.as_ref().is_some_and(|(what, hint)| {
                !plain(what, 1024) || hint.as_deref().is_some_and(|hint| !plain(hint, 512))
            })
        {
            return Err(unconfirmed());
        }
        Ok(serde_json::to_value(presentation).expect("presentation serialization"))
    }
}

/// Opens the page when allowed, then prints the result: the state, the next step and how to use
/// the other mode.
fn present(
    mut presentation: Presentation,
    args: &clap::ArgMatches,
    root: &Path,
    json_output: bool,
    background: bool,
) -> Result<()> {
    let mut warnings = Vec::new();
    let mut opened = false;
    if let Some(link) = &presentation.link {
        // The door is ready: unreadable settings are the defaults, never a failed serve.
        let settings = tmt_colab::settings::read_or_default(root);
        let outcome = open::open_link(link, open::flag(args), settings.open(), json_output);
        opened = matches!(outcome, open::Outcome::Opened);
        warnings.extend(open::describe(&outcome, link).1);
        if settings.malformed {
            warnings.push(tmt_colab::settings::UNREADABLE.to_owned());
        }
    }
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        presentation.status["opened"] = json!(opened);
        if background {
            presentation.status["background"] = json!(true);
        }
        writeln!(output, "{}", presentation.status)?;
    } else {
        let terminal = output.terminal();
        if background {
            tmt_cli_style::message::success(
                &mut output,
                terminal,
                "Colab is running in the background",
            )?;
        }
        let rows: Vec<(&str, String)> = presentation
            .rows
            .iter()
            .map(|(label, value)| {
                let value = match (opened && label == "open", value) {
                    (true, value) => format!("opened in your browser: {value}"),
                    (false, value) => value.clone(),
                };
                (label.as_str(), value)
            })
            .collect();
        tmt_cli_style::detail::write(&mut output, terminal, "LOCAL SPACE", &rows)?;
        let note = if background {
            "Stop it with tmt colab stop. To keep it attached to this terminal: tmt colab serve --foreground"
        } else {
            "Stop it with Ctrl-C. To run it in the background next time: tmt colab serve"
        };
        writeln!(
            output,
            "{}",
            terminal.paint(
                tmt_cli_style::palette::Token::Dim,
                &tmt_cli_style::table::escape(note)
            )
        )?;
    }
    if let (Some((what, hint)), false) = (&presentation.warning, json_output) {
        let mut warning = tmt_cli_style::stream::stderr();
        let terminal = warning.terminal();
        tmt_cli_style::message::warning(&mut warning, terminal, what, hint.as_deref())?;
    }
    for what in warnings.iter().filter(|_| !json_output) {
        let mut warning = tmt_cli_style::stream::stderr();
        let terminal = warning.terminal();
        tmt_cli_style::message::warning(&mut warning, terminal, what, None)?;
    }
    output.flush()?;
    Ok(())
}

pub(crate) fn run(root: &Path, args: &clap::ArgMatches, stop: &Arc<AtomicBool>) -> Result<()> {
    let json_output = args.get_flag("json");
    if args.get_flag("worker") {
        let mut handoff = adopt(&handshake(), stop).map_err(StartFault::from)?;
        let result = serve(root, args, json_output, stop, Some(&mut handoff));
        match &result {
            Ok(()) => handoff.finish(),
            Err(error) => handoff.fail(&startup_failure(error.as_ref()), true),
        }
        return result;
    }
    if is_launcher(args) {
        background(root, args, json_output, stop)
    } else {
        serve(root, args, json_output, stop, None)
    }
}

/// Whether this invocation starts a detached serve instead of being one. Bare `--json` stays
/// foreground, so existing supervisors keep owning the serve they start.
pub(crate) fn is_launcher(args: &clap::ArgMatches) -> bool {
    !args.get_flag("worker")
        && !args.get_flag("foreground")
        && (!args.get_flag("json") || args.get_flag("background"))
}

/// The typed failure a launcher receives: the stable code, the cause and the next step.
fn startup_failure(error: &(dyn std::error::Error + Send + Sync + 'static)) -> StartupError {
    if let Some(fault) = error.downcast_ref::<StartFault>() {
        let startup = StartupError::new(&fault.code, &fault.message);
        return match &fault.hint {
            Some(hint) => startup.with_hint(hint),
            None => startup,
        };
    }
    let code = crate::error_code(error);
    let startup = StartupError::new(code, error.to_string());
    match code {
        "COLAB_ALREADY_SERVING" => startup.with_hint(
            "Colab is already running: tmt colab open shows its page; tmt colab stop ends it",
        ),
        _ => startup.with_hint(RUN_HINT),
    }
}

fn background(
    root: &Path,
    args: &clap::ArgMatches,
    json_output: bool,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    let mut worker: Vec<std::ffi::OsString> = ["serve", "--foreground", "--worker"]
        .into_iter()
        .map(Into::into)
        .collect();
    if let Some(app) = args.get_one::<std::path::PathBuf>("app-dir") {
        worker.extend(["--app-dir".into(), app.into()]);
    }
    let program = std::env::current_exe()?;
    let ready = launch(
        &Launch {
            program: &program,
            args: &worker,
            handshake: &handshake(),
        },
        stop,
        Presentation::validate,
    )
    .map_err(StartFault::from)?;
    let presentation: Presentation = serde_json::from_value(ready).expect("validated presentation");
    present(presentation, args, root, json_output, true).map_err(|_| {
        Box::new(StartFault {
            code: "COLAB_READY_OUTPUT".into(),
            message: "Colab readiness output was not completed; it may be running.".into(),
            hint: Some("run tmt colab stop before starting again".into()),
        })
        .into()
    })
}

/// Bounded, sanitized words for the private failure record; the cause stays in the foreground.
fn record_failure(
    record: &mut ErrorRecord,
    error: &(dyn std::error::Error + Send + Sync + 'static),
) {
    let cancelled = error.downcast_ref::<StartFault>().is_some();
    record.failure(
        if cancelled { "handoff" } else { "serve" },
        crate::error_code(error),
        if cancelled {
            "Startup ended before handoff."
        } else {
            "Colab serving failed; run tmt colab serve --foreground to see why."
        },
    );
}

/// The serve itself. In a worker (`handoff` is `Some`) it sends the readiness record instead of
/// printing, never opens the browser, and records a failure it can no longer report.
fn serve(
    root: &Path,
    args: &clap::ArgMatches,
    json_output: bool,
    stop: &Arc<AtomicBool>,
    mut handoff: Option<&mut Handoff>,
) -> Result<()> {
    let app = App::selected(
        args.get_one::<std::path::PathBuf>("app-dir")
            .map(|path| path.as_path()),
    )?;
    let layout = Layout::open(root)?;
    let _lock = layout.serve_lock()?;
    let mut diagnostic = if handoff.is_some() {
        Some(ErrorRecord::clear(
            layout.file("serve-error.json")?,
            RECORD_BYTES,
        )?)
    } else {
        None
    };
    let detached = handoff.is_some();
    let result = (|| -> Result<()> {
        fence(stop, detached)?;
        let keyring = Keyring::open(&layout)?;
        let store = Store::open(&layout)?;
        let space_id = keyring.space_id.clone();
        let (pages, all_pages) = open_pages(&store, &keyring);
        let save_root = root.to_path_buf();
        let registration = Arc::new(Mutex::new(
            Registration::new(store, keyring, std::env::current_exe()?)?.with_save_source(
                Arc::new(move || {
                    tmt_colab::page::save::open_source(
                        &save_root,
                        tmt_colab::decoder::Config::new(std::env::current_exe()?),
                    )
                }),
            ),
        ));
        let release = Arc::new(tmt_colab::serve_release::ServeRelease::new(
            tmt_colab::serve_release::Running::detect(),
        ));
        fence(stop, detached)?;
        let socket = MountSocket::bind(&layout, &space_id, Tunnels::PRODUCT)?
            .with_registration(&layout, Arc::clone(&registration))?
            .with_release(Arc::clone(&release))
            .with_app(app);
        // The socket is bound first, so a door started now mounts it as soon as it is ready.
        let access = supervisor::Access::open(stop);
        fence(stop, detached)?;
        let socket = socket
            .with_door(match &access {
                supervisor::Access::Attached(_) => "attached",
                supervisor::Access::Started { .. } => "started",
                supervisor::Access::Unavailable { .. } => "unavailable",
            })
            .with_object_discovery(|| match crate::door::Door::lookup() {
                crate::door::Lookup::Running(door) => {
                    let host = door.origin().strip_prefix("http://")?.to_owned();
                    let mount = door.url("x/colab/").strip_prefix(door.origin())?.to_owned();
                    Some((host, mount))
                }
                _ => None,
            });
        let pairing = match &access {
            supervisor::Access::Unavailable { .. } => None,
            _ => Some(crate::door::Pairing::lookup()),
        };
        let status = status::Status {
            space: &space_id,
            socket: &socket.path,
            access: &access,
            pairing,
            pages,
            all_pages,
            opened: false,
        };
        let presentation = Presentation::from_live(&status, &access);
        match handoff.as_mut() {
            Some(handoff) => handoff
                .ready(&serde_json::to_value(&presentation).expect("presentation serialization"))
                .map_err(StartFault::from)?,
            None => present(presentation, args, root, json_output, false)?,
        }
        // The watcher ends with the serve, however the socket loop was stopped. A detached
        // worker has no terminal to tell.
        let serving = Arc::new(AtomicBool::new(true));
        let watcher =
            (!json_output && handoff.is_none()).then(|| watch_release(&release, &serving));
        let result = socket.run(stop);
        serving.store(false, Ordering::Relaxed);
        if let Some(watcher) = watcher {
            let _ = watcher.join();
        }
        let closed = Arc::try_unwrap(registration)
            .map_err(|_| "Registration worker retained.")?
            .into_inner()
            .map_err(|_| "Registration lock poisoned.")?
            .close();
        // The door Colab started stops after its own socket is closed.
        drop(access);
        result?;
        closed
    })();
    if let (Some(diagnostic), Err(error)) = (&mut diagnostic, &result) {
        record_failure(diagnostic, error.as_ref());
    }
    // The record is written and closed while the serve lock is still held.
    drop(diagnostic);
    result
}

/// Tells the foreground user, once, when the installed release is no longer the one serving.
/// The check reads only the install layout; it never restarts, signals or changes the serve.
/// It runs until `serving` clears and a stop within a tenth of a second.
fn watch_release(
    release: &Arc<tmt_colab::serve_release::ServeRelease>,
    serving: &Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let (release, serving) = (Arc::clone(release), Arc::clone(serving));
    std::thread::spawn(move || {
        // Check at most every `PERIOD`, but notice a stop within a tenth of a second.
        const PERIOD: u32 = 50;
        let mut ticks = PERIOD;
        while serving.load(Ordering::Relaxed) {
            if ticks >= PERIOD {
                ticks = 0;
                if let Some(stale) = release.fresh() {
                    let mut warning = tmt_cli_style::stream::stderr();
                    let terminal = warning.terminal();
                    let _ = tmt_cli_style::message::warning(
                        &mut warning,
                        terminal,
                        &tmt_colab::serve_release::restart_text(&stale),
                        Some(tmt_colab::serve_release::RESTART_HINT),
                    );
                    return;
                }
            }
            ticks += 1;
            std::thread::sleep(Duration::from_millis(100));
        }
    })
}

/// Ids of the pages that are not archived, for the start-up status. A catalog that cannot be
/// read is unknown, never a reason to refuse to serve.
fn open_pages(store: &Store, keyring: &Keyring) -> (Option<Vec<String>>, Option<Vec<String>>) {
    let Some(catalog) = tmt_colab::inspection::catalog(store, keyring).ok() else {
        return (None, None);
    };
    let ids = |rows: &serde_json::Value, active: bool| -> Option<Vec<String>> {
        rows.as_array()?
            .iter()
            .filter(|page| !active || page["archived"] != true)
            .map(|page| page["pageId"].as_str().map(str::to_owned))
            .collect()
    };
    (
        ids(&catalog["pages"], true),
        ids(&catalog["pageIds"], false),
    )
}
