//! Binary-private serve lifecycle composition: the foreground serve, the detached worker and the
//! launcher and installed-release restart. Background handoff and failure-record encoding live
//! in `tmt-extension-serve`; Colab owns preflight, drain, normal stop and replacement startup.
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
const RECORD_BYTES: usize = tmt_colab::limits::REMOTE_FAILURE_BYTES;
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
        tmt_cli_style::message::success(
            &mut output,
            terminal,
            if background {
                "Colab is running in the background"
            } else {
                "Colab is running in this terminal"
            },
        )?;
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
    if args.get_flag("upgrade-check") {
        return upgrade_check(root, args);
    }
    if args.get_flag("worker") {
        let mut handoff = adopt(&handshake(), stop).map_err(StartFault::from)?;
        let result = serve(root, args, json_output, stop, Some(&mut handoff));
        match &result {
            Ok(()) => handoff.finish(),
            Err(error) => handoff.fail(
                &startup_failure(error.as_ref()),
                !error.is::<supervisor::CleanupUnconfirmed>(),
            ),
        }
        return result;
    }
    if is_launcher(args) {
        // A person who reruns `serve` wants the page: that is success, not an error. Scripts
        // keep the typed `COLAB_ALREADY_SERVING`.
        match background(root, args, json_output, stop) {
            Err(error) if !json_output && is_already_serving(error.as_ref()) => {
                already_running(root, args)
            }
            result => result,
        }
    } else {
        let result = serve(root, args, json_output, stop, None);
        if std::env::var_os("TMT_COLAB_REPLACEMENT").is_some_and(|value| value == "1") {
            result
                .map_err(|error| replacement_failure(root, env!("CARGO_PKG_VERSION"), Some(error)))
        } else {
            result
        }
    }
}

/// After the cutoff there is no rollback promise. This marker only selects diagnostics; it
/// grants no admission and cannot start, stop or open anything by itself.
fn replacement_failure(
    root: &Path,
    version: &str,
    cause: Option<Box<dyn std::error::Error + Send + Sync>>,
) -> Box<dyn std::error::Error + Send + Sync> {
    // Cleanup has its own truthful outcome and private record. Do not overwrite it with
    // the generic post-cutoff replacement marker; its original failure remains attached.
    if let Some(error) = cause
        && error.is::<supervisor::CleanupUnconfirmed>()
    {
        return error;
    }
    let message = format!("Colab {version} replacement failed after the previous service stopped.");
    let hint = "Run tmt colab stop, then tmt colab serve --background.";
    if let Ok(Some(layout)) = Layout::existing(root)
        && let Ok(file) = layout.file("serve-error.json")
        && let Ok(mut record) = ErrorRecord::clear(file, RECORD_BYTES)
    {
        record.failure(
            "replacement",
            "COLAB_RELEASE_REPLACEMENT",
            &format!("{message} {hint}"),
        );
    }
    StartFault {
        code: "COLAB_RELEASE_REPLACEMENT".into(),
        message,
        hint: Some(hint.into()),
    }
    .into()
}

fn is_already_serving(error: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    error
        .downcast_ref::<StartFault>()
        .is_some_and(|fault| fault.code == "COLAB_ALREADY_SERVING")
}

/// The serve is already running: show its page the way `tmt colab open` does, and open it under
/// serve's own setting and flags. Starts nothing.
fn already_running(root: &Path, args: &clap::ArgMatches) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(tmt_colab::keyring::StateFault::AlreadyServing)?;
    let keyring = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    let (pages, all_pages) = open_pages(&store, &keyring);
    let path = crate::reach::Reach::landing(&keyring.space_id, pages.as_deref());
    let reach = crate::reach::Reach::gather().with_pages(all_pages.as_deref().unwrap_or(&[]));
    let mut shown = reach.text(&path);
    let mut warnings = Vec::new();
    if let Some(link) = reach.short_link(&path).or_else(|| reach.link(&path)) {
        let settings = tmt_colab::settings::read_or_default(root);
        let outcome = open::open_link(&link, open::flag(args), settings.open(), false);
        let (text, failed) = open::describe(&outcome, &link);
        shown = text;
        warnings.extend(failed);
        if settings.malformed {
            warnings.push(tmt_colab::settings::UNREADABLE.to_owned());
        }
    }
    let mut rows = vec![("open", shown)];
    rows.extend(reach.step().map(|step| ("pair", step.to_owned())));
    let mut output = tmt_cli_style::stream::stdout(false);
    let terminal = output.terminal();
    tmt_cli_style::message::success(&mut output, terminal, "Colab is already running")?;
    tmt_cli_style::detail::write(&mut output, terminal, "OPEN COLAB", &rows)?;
    writeln!(
        output,
        "{}",
        terminal.paint(
            tmt_cli_style::palette::Token::Dim,
            &tmt_cli_style::table::escape("Stop it with tmt colab stop.")
        )
    )?;
    for what in &warnings {
        let mut warning = tmt_cli_style::stream::stderr();
        let terminal = warning.terminal();
        tmt_cli_style::message::warning(&mut warning, terminal, what, None)?;
    }
    output.flush()?;
    Ok(())
}

/// Read-only candidate prerequisites while the previous service owns the root.
/// The store's future-schema check runs without migration; only the later serve initializes it.
fn upgrade_check(root: &Path, args: &clap::ArgMatches) -> Result<()> {
    let app = App::selected(
        args.get_one::<std::path::PathBuf>("app-dir")
            .map(|path| path.as_path()),
    )?;
    if app.is_none() {
        return Err("The installed release has no usable browser app.".into());
    }
    let layout = Layout::existing(root)?.ok_or("The serving Colab root is unavailable.")?;
    let key = Keyring::read(&layout)?;
    let _store = Store::read(&layout)?;
    let mut output = tmt_cli_style::stream::stdout(true);
    writeln!(
        output,
        "{}",
        json!({
            "state": "prepared", "version": env!("CARGO_PKG_VERSION"),
            "spaceId": key.space_id, "schema": tmt_colab::store::SCHEMA_VERSION
        })
    )?;
    output.flush()?;
    Ok(())
}

/// Whether this invocation starts a detached serve instead of being one. Bare `--json` stays
/// foreground, so existing supervisors keep owning the serve they start.
pub(crate) fn is_launcher(args: &clap::ArgMatches) -> bool {
    !args.get_flag("worker")
        && !args.get_flag("upgrade-check")
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
    if let Some(cleanup) = error.downcast_ref::<supervisor::CleanupUnconfirmed>() {
        record.failure("cleanup", "COLAB_OUTCOME_UNKNOWN", &cleanup.to_string());
        return;
    }
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

/// The one line a person is shown when cloud sync cannot run.
fn warn(line: &str) {
    let mut warning = tmt_cli_style::stream::stderr();
    let terminal = warning.terminal();
    let _ = tmt_cli_style::message::warning(&mut warning, terminal, line, None);
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
    let lock = layout.serve_lock()?;
    let mut diagnostic = Some(ErrorRecord::clear(
        layout.file("serve-error.json")?,
        RECORD_BYTES,
    )?);
    let detached = handoff.is_some();
    let result = (|| -> Result<Option<tmt_colab::serve_release::Candidate>> {
        fence(stop, detached)?;
        let keyring = Keyring::open(&layout)?;
        let store = Store::open(&layout)?;
        let space_id = keyring.space_id.clone();
        let owner = keyring.owner_public();
        let (pages, all_pages) = open_pages(&store, &keyring);
        let save_root = root.to_path_buf();
        let summary_root = root.to_path_buf();
        let registration = Arc::new(Mutex::new(
            Registration::new(store, keyring, std::env::current_exe()?)?
                .with_save_source(Arc::new(move || {
                    tmt_colab::page::save::open_source(
                        &save_root,
                        tmt_colab::decoder::Config::new(std::env::current_exe()?),
                    )
                }))
                .with_summary_source(Arc::new(move || {
                    tmt_colab::page::save::open_source(
                        &summary_root,
                        tmt_colab::decoder::Config::new(std::env::current_exe()?),
                    )
                })),
        ));
        let release = Arc::new(tmt_colab::serve_release::ServeRelease::new(
            tmt_colab::serve_release::Running::detect(),
        ));
        fence(stop, detached)?;
        let mut socket = MountSocket::bind(&layout, &space_id, Tunnels::PRODUCT)?
            .with_registration(&layout, Arc::clone(&registration))?
            .with_release(Arc::clone(&release))
            .with_app(app);
        // An executable whose declaration cannot be produced refuses itself everywhere else;
        // here it simply never syncs.
        if let Ok(digest) = tmt_colab::deploy_declaration::declaration_digest() {
            socket = socket.with_cloud_sync(
                &layout,
                owner,
                tmt_colab::cloud_sync::Config {
                    activate: Arc::new(supervisor::activate_sync),
                    report: Arc::new(warn),
                    declaration_digest: digest,
                },
            )?;
        }
        // The socket is bound first, so a door started now mounts it as soon as it is ready.
        let access = supervisor::Access::open(stop);
        if let supervisor::Access::Unavailable {
            reason: supervisor::Reason::CleanupUnconfirmed(message),
        } = &access
            && let Some(mut record) = diagnostic.take()
        {
            // Preserve the first cleanup failure: ErrorRecord is a write-once owner.
            // Access::close carries this uncertainty alongside any later serve error.
            record.failure("cleanup", "COLAB_OUTCOME_UNKNOWN", message);
        }
        let candidate = Arc::new(Mutex::new(None));
        let outcome = (|| -> Result<bool> {
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
                    .ready(
                        &serde_json::to_value(&presentation).expect("presentation serialization"),
                    )
                    .map_err(StartFault::from)?,
                None => present(presentation, args, root, json_output, false)?,
            }
            let serving = Arc::new(AtomicBool::new(true));
            let requested = Arc::new(AtomicBool::new(false));
            let watcher =
                watch_release(&release, &serving, stop, &requested, &candidate, &space_id);
            let result = socket.run_until_replacement(stop, &requested);
            serving.store(false, Ordering::Release);
            let _ = watcher.join();
            let closed = Arc::try_unwrap(registration)
                .map_err(|_| "Registration worker retained.")?
                .into_inner()
                .map_err(|_| "Registration lock poisoned.")?
                .close();
            result.and_then(|replacing| closed.map(|()| replacing))
        })();
        // Close only the owned door, and preserve both a serve failure and cleanup uncertainty.
        if let Err(mut cleanup) = access.close() {
            cleanup.original = outcome.err();
            return Err(cleanup.into());
        }
        let replacing = outcome?;
        if replacing && !stop.load(Ordering::Acquire) {
            let candidate = candidate
                .lock()
                .map_err(|_| "Release lock poisoned.")?
                .take()
                .ok_or("Verified release missing.")?;
            Ok(Some(candidate))
        } else {
            Ok(None)
        }
    })();
    if let (Some(diagnostic), Err(error)) = (&mut diagnostic, &result) {
        record_failure(diagnostic, error.as_ref());
    }
    // The record is written and closed while the serve lock is still held.
    drop(diagnostic);
    drop(lock);
    match result? {
        Some(candidate) if !stop.load(Ordering::Acquire) => {
            use std::os::unix::process::CommandExt;
            let mut command = std::process::Command::new(candidate.program());
            command.args(["serve", "--foreground", "--no-open"]);
            if json_output || detached {
                command.arg("--json");
            }
            command.env("TMT_COLAB_REPLACEMENT", "1");
            // Exec retains ordinary foreground/background supervision and its owned PID.
            // Normal startup attaches to an untouched door or restarts the one just stopped.
            let _error = command.exec();
            Err(replacement_failure(root, candidate.version(), None))
        }
        _ => Ok(()),
    }
}

/// The candidate confirms assets and read-only store prerequisites while the old serve still
/// owns its lock. No migration or door operation occurs before the cutoff.
fn verify_replacement(
    candidate: &tmt_colab::serve_release::Candidate,
    space: &str,
    stop: &AtomicBool,
) -> Result<()> {
    let args = [
        "serve".into(),
        "--upgrade-check".into(),
        "--json".into(),
        "--no-open".into(),
    ];
    let output = tmt_invoke::invoke(
        tmt_invoke::Request {
            program: candidate.program(),
            args: &args,
            input: &[],
            deadline: std::time::Instant::now() + STARTUP,
            max_stream_bytes: RECORD_BYTES,
            launch: Default::default(),
        },
        Some(stop),
    )?;
    if !output.status.success() {
        return Err("Selected Colab release failed its readiness check.".into());
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    if value["state"] != "prepared"
        || value["version"] != candidate.version()
        || value["spaceId"] != space
        || value["schema"].as_u64().is_none()
        || value.as_object().is_none_or(|object| object.len() != 4)
    {
        return Err("Selected Colab release returned an invalid readiness check.".into());
    }
    Ok(())
}

/// Verify a selected release while serving, then hand the candidate to the socket's drain.
/// A failed check keeps the old service live and reports the manual recovery path once.
fn watch_release(
    release: &Arc<tmt_colab::serve_release::ServeRelease>,
    serving: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    requested: &Arc<AtomicBool>,
    candidate: &Arc<Mutex<Option<tmt_colab::serve_release::Candidate>>>,
    space: &str,
) -> std::thread::JoinHandle<()> {
    let (release, serving, stop, requested, candidate, space) = (
        Arc::clone(release),
        Arc::clone(serving),
        Arc::clone(stop),
        Arc::clone(requested),
        Arc::clone(candidate),
        space.to_owned(),
    );
    std::thread::spawn(move || {
        const PERIOD: u32 = 50;
        let mut ticks = PERIOD;
        let mut failed = None;
        while serving.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
            if ticks >= PERIOD {
                ticks = 0;
                if let Some(stale) = release.fresh() {
                    if failed.as_ref() == Some(&stale.installed) {
                        continue;
                    }
                    let checked = release
                        .verified_current()
                        .map_err(|reason| -> Box<dyn std::error::Error + Send + Sync> {
                            reason.into()
                        })
                        .and_then(|candidate| {
                            let Some(candidate) = candidate else {
                                return Ok(None);
                            };
                            verify_replacement(&candidate, &space, &stop)?;
                            // Selection can change while the candidate checks its prerequisites.
                            if release.verified_current().ok().flatten().as_ref()
                                != Some(&candidate)
                            {
                                return Err(
                                    "Selected Colab release changed during verification.".into()
                                );
                            }
                            Ok(Some(candidate))
                        });
                    match checked {
                        Ok(Some(verified)) => {
                            if let Ok(mut candidate) = candidate.lock() {
                                *candidate = Some(verified);
                                requested.store(true, Ordering::Release);
                            }
                            return;
                        }
                        Ok(None) => {}
                        Err(_) => {
                            failed = Some(stale.installed.clone());
                            release.verification_failed(stale.installed.clone());
                            let mut warning = tmt_cli_style::stream::stderr();
                            let terminal = warning.terminal();
                            let _ = tmt_cli_style::message::warning(
                                &mut warning,
                                terminal,
                                &format!(
                                    "Colab {} could not be verified; {} keeps running.",
                                    stale.installed, stale.running
                                ),
                                Some("Run tmt colab stop, then tmt colab serve --background."),
                            );
                        }
                    }
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

#[cfg(test)]
mod cleanup_record_tests {
    use super::*;
    #[test]
    fn replacement_error_keeps_cleanup_phase_record_and_original_failure() {
        use std::os::unix::fs::DirBuilderExt;
        let root =
            std::env::temp_dir().join(format!("tmt-2899-replacement-{}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let layout = Layout::open(&root).unwrap();
        let mut record =
            ErrorRecord::clear(layout.file("serve-error.json").unwrap(), RECORD_BYTES).unwrap();
        record.failure("cleanup", "COLAB_OUTCOME_UNKNOWN", "worker join timed out");
        let before = layout.serve_failure().unwrap().unwrap();
        let cause = supervisor::CleanupUnconfirmed {
            detail: "worker join timed out".into(),
            original: Some(std::io::Error::other("original startup failure").into()),
        };
        let error = replacement_failure(&root, "selected-release", Some(cause.into()));
        assert_eq!(crate::error_code(error.as_ref()), "COLAB_OUTCOME_UNKNOWN");
        assert!(error.to_string().contains("original startup failure"));
        assert!(error.to_string().contains("cleanup unconfirmed"));
        assert_eq!(layout.serve_failure().unwrap().unwrap(), before);
        drop(record);
        std::fs::remove_dir_all(root).unwrap();
    }
}
