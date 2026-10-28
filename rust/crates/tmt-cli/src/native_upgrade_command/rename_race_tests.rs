//! Real SQLite and private-tmux controls for the cutover's writer boundary.
use super::{migrate, tests::Directory};
use std::{
    cell::{Cell, RefCell},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_adapters::{
    host::{CallerEnvironment, Host},
    process::{
        CommandError, CommandFailure, CommandOutput, CommandRequest, CommandRunner,
        UnixCommandRunner,
    },
    storage::Storage,
};
use tmt_core::{
    binding::{BindingEntry, BindingRepository, bind_identity},
    identity::IdentityReader,
};

#[derive(Clone, Copy)]
struct PrivateRunner<'a> {
    root: &'a Path,
    socket: &'a Path,
}
impl CommandRunner for PrivateRunner<'_> {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        if request.program != OsStr::new("tmux") {
            return UnixCommandRunner.execute(request);
        }
        assert!(
            self.socket.starts_with(self.root),
            "private tmux socket must remain under its task root"
        );
        assert_ne!(
            self.socket.file_name(),
            Some(OsStr::new("default")),
            "never the host default socket"
        );
        assert_eq!(
            request.args.first().map(OsString::as_os_str),
            Some(OsStr::new("-u")),
            "private tmux command must start -u -S <socket>"
        );
        assert_eq!(
            request.args.get(1).map(OsString::as_os_str),
            Some(OsStr::new("-S"))
        );
        assert_eq!(
            request.args.get(2).map(OsString::as_os_str),
            Some(self.socket.as_os_str())
        );
        let mut args: Vec<OsString> = ["-u", "TMUX", "-u", "TMUX_PANE", "tmux"]
            .map(Into::into)
            .into();
        args.extend_from_slice(request.args);
        UnixCommandRunner.execute(CommandRequest {
            program: OsStr::new("/usr/bin/env"),
            args: &args,
            ..request
        })
    }
}
// Each rejected request names only an absent socket in a fresh owned directory.
// A missing assertion would make the should-panic control fail without reaching
// a host server or starting any tmux server.
fn probe_private_runner(root: &Path, socket: &Path, first: &str) {
    let args = [
        first.into(),
        "-S".into(),
        socket.as_os_str().to_owned(),
        "list-sessions".into(),
    ];
    let _ = PrivateRunner { root, socket }.execute(CommandRequest {
        program: OsStr::new("tmux"),
        args: &args,
        input: &[],
        deadline: Instant::now() + Duration::from_secs(1),
        max_output_bytes: 1024,
    });
}

#[test]
#[should_panic(expected = "private tmux socket must remain under its task root")]
fn a_private_runner_refuses_a_socket_outside_its_root_before_exec() {
    let root = Directory::new();
    let other = Directory::new();
    probe_private_runner(&root.0, &other.0.join("s"), "-u");
}

#[test]
#[should_panic(expected = "never the host default socket")]
fn a_private_runner_refuses_a_default_socket_name_before_exec() {
    let root = Directory::new();
    probe_private_runner(&root.0, &root.0.join("default"), "-u");
}

#[test]
#[should_panic(expected = "private tmux command must start -u -S <socket>")]
fn a_private_runner_refuses_a_changed_command_prefix_before_exec() {
    let root = Directory::new();
    probe_private_runner(&root.0, &root.0.join("s"), "-2");
}

struct PrivateServer {
    directory: Option<Directory>,
    socket: PathBuf,
}
impl PrivateServer {
    fn new() -> Self {
        let directory = Directory::new();
        let socket = directory.0.join("s");
        assert!(
            socket.as_os_str().len() < 104,
            "private socket fits macOS sun_path"
        );
        let server = Self {
            directory: Some(directory),
            socket,
        };
        server
            .command(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "cutover",
                "/bin/sh",
            ])
            .unwrap();
        server
    }
    fn root(&self) -> &Path {
        &self.directory.as_ref().unwrap().0
    }
    fn runner(&self) -> PrivateRunner<'_> {
        PrivateRunner {
            root: self.root(),
            socket: &self.socket,
        }
    }
    fn command(&self, words: &[&str]) -> Result<String, CommandError> {
        let mut args = vec!["-u".into(), "-S".into(), self.socket.as_os_str().to_owned()];
        args.extend(words.iter().map(OsString::from));
        self.runner()
            .execute(CommandRequest {
                program: OsStr::new("tmux"),
                args: &args,
                input: &[],
                deadline: Instant::now() + Duration::from_secs(10),
                max_output_bytes: 64 * 1024,
            })
            .map(|output| {
                String::from_utf8(output.stdout)
                    .unwrap()
                    .trim_end_matches('\n')
                    .to_owned()
            })
    }
}
/// How long a killed private server may take to disappear: the command returns first, the
/// server exits next and its socket file is removed last.
const ABSENCE_DEADLINE: Duration = Duration::from_secs(5);
const ABSENCE_POLL: Duration = Duration::from_millis(25);

/// What one `list-sessions` request on the owned socket showed.
enum Listing {
    Sessions(String),
    Failed { exit: Option<u32>, stderr: String },
    Unobserved(String),
}
impl Listing {
    fn observe(result: Result<String, CommandError>) -> Self {
        match result {
            Ok(sessions) => Self::Sessions(sessions),
            Err(error) => match (error.kind, error.cleanup_error, error.output) {
                (CommandFailure::Exit { code, signal: None }, None, Some(output)) => Self::Failed {
                    exit: code,
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                },
                (kind, cleanup, _) => Self::Unobserved(format!("{kind:?}, cleanup {cleanup:?}")),
            },
        }
    }
    /// Only tmux saying that no server answers on the socket proves absence. A refused
    /// connection on a socket file that is not yet removed is a server still shutting down.
    fn proves_absence(&self) -> bool {
        matches!(self, Self::Failed { exit: Some(1), stderr }
            if stderr.starts_with("no server running on ")
                || (stderr.starts_with("error connecting to ")
                    && stderr.trim_end().ends_with("(No such file or directory)")))
    }
    fn describe(&self) -> String {
        match self {
            Self::Sessions(sessions) => format!("a server still lists sessions: {sessions}"),
            Self::Failed { exit, stderr } => format!("exit {exit:?}: {}", stderr.trim_end()),
            Self::Unobserved(why) => format!("not observed: {why}"),
        }
    }
}

/// Polls until a listing proves absence or `wait` has passed; the error is the last listing.
fn await_absence(
    mut list: impl FnMut() -> Listing,
    wait: Duration,
    poll: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + wait;
    loop {
        let listing = list();
        if listing.proves_absence() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(listing.describe());
        }
        std::thread::sleep(poll);
    }
}

impl Drop for PrivateServer {
    fn drop(&mut self) {
        let _ = self.command(&["kill-server"]);
        let absent = await_absence(
            || Listing::observe(self.command(&["list-sessions"])),
            ABSENCE_DEADLINE,
            ABSENCE_POLL,
        );
        if let Err(last) = absent {
            let directory = self.directory.take().unwrap();
            eprintln!(
                "Private cutover server cleanup unconfirmed: {} (last listing: {last})",
                directory.0.display()
            );
            std::mem::forget(directory);
            assert!(
                std::thread::panicking(),
                "private cutover server cleanup failed"
            );
        }
    }
}
fn gone(stderr: &str) -> Listing {
    Listing::Failed {
        exit: Some(1),
        stderr: stderr.into(),
    }
}
const NO_SERVER: &str = "no server running on /tmp/s";
const REFUSED: &str = "error connecting to /tmp/s (Connection refused)";
const NO_SOCKET: &str = "error connecting to /tmp/s (No such file or directory)\n";

#[test]
fn only_tmux_saying_no_server_answers_proves_absence() {
    assert!(gone(NO_SERVER).proves_absence());
    assert!(gone(NO_SOCKET).proves_absence());
    for not_proof in [
        gone(REFUSED),
        gone("error connecting to /tmp/s (Permission denied)"),
        gone("something else"),
        Listing::Failed {
            exit: Some(2),
            stderr: NO_SERVER.into(),
        },
        Listing::Sessions("cutover: 1 windows".into()),
        Listing::Unobserved("Timeout".into()),
    ] {
        assert!(!not_proof.proves_absence(), "{}", not_proof.describe());
    }
}

#[test]
fn a_server_still_shutting_down_is_waited_for_not_declared_unconfirmed() {
    let mut calls = 0;
    let result = await_absence(
        || {
            calls += 1;
            if calls < 4 {
                gone(REFUSED)
            } else {
                gone(NO_SERVER)
            }
        },
        Duration::from_secs(5),
        Duration::from_millis(1),
    );
    assert_eq!(result, Ok(()));
    assert_eq!(calls, 4);
}

#[test]
fn absence_seen_at_once_is_not_polled_again() {
    let mut calls = 0;
    let result = await_absence(
        || {
            calls += 1;
            gone(NO_SOCKET)
        },
        Duration::from_secs(5),
        Duration::from_millis(1),
    );
    assert_eq!(result, Ok(()));
    assert_eq!(calls, 1);
}

#[test]
fn a_server_that_never_goes_away_fails_after_the_deadline_with_its_last_listing() {
    let started = Instant::now();
    let mut calls = 0;
    let result = await_absence(
        || {
            calls += 1;
            gone(REFUSED)
        },
        Duration::from_millis(60),
        Duration::from_millis(5),
    );
    assert_eq!(
        result,
        Err(format!("exit Some(1): {REFUSED}")),
        "the evidence is the last listing"
    );
    assert!(calls > 1, "it polled before giving up");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the wait is bounded"
    );
}

#[test]
fn a_listing_that_cannot_be_read_never_counts_as_absence() {
    let listing = Listing::Unobserved("Timeout, cleanup None".into());
    assert!(
        await_absence(
            || Listing::Unobserved("Timeout".into()),
            Duration::ZERO,
            Duration::ZERO
        )
        .is_err()
    );
    assert_eq!(listing.describe(), "not observed: Timeout, cleanup None");
}

struct Fixture {
    server: PrivateServer,
    database: PathBuf,
    caller: CallerEnvironment,
    entry: BindingEntry,
    raw: String,
}
impl Fixture {
    fn new() -> Self {
        let server = PrivateServer::new();
        let database = server.root().join("state.db");
        let pane = server
            .command(&["display-message", "-p", "-t", "cutover:0.0", "#{pane_id}"])
            .unwrap();
        let pid = server
            .command(&["display-message", "-p", "-t", &pane, "#{pid}"])
            .unwrap();
        let caller = CallerEnvironment {
            tmux: Some(format!("{},{pid},0", server.socket.display()).into()),
            pane: Some(pane.clone().into()),
            process_id: u64::from(std::process::id()),
            driver_env: Default::default(),
        };
        let host = Host::for_caller_with(&caller, server.runner()).unwrap();
        let mut storage = Storage::open(&database).unwrap();
        let bound =
            bind_identity(&mut storage, &mut host.session(), &pane, "Original", true).unwrap();
        let entry = BindingEntry {
            identity: bound.identity,
            binding: bound.binding,
        };
        storage.close().unwrap();
        let raw = server
            .command(&["show-options", "-pqv", "-t", &pane, "@tmt.agent"])
            .unwrap();
        assert!(!raw.is_empty());
        server
            .command(&[
                "set-option",
                "-s",
                "@tmux-team.server-id",
                &entry.binding.as_ref().unwrap().server.server_id,
            ])
            .unwrap();
        server
            .command(&["set-option", "-p", "-t", &pane, "@tmux-team.agent", &raw])
            .unwrap();
        server
            .command(&["set-option", "-pu", "-t", &pane, "@tmt.agent"])
            .unwrap();
        Self {
            server,
            database,
            caller,
            entry,
            raw,
        }
    }
    fn marker(&self) -> String {
        self.server
            .command(&[
                "show-options",
                "-pqv",
                "-t",
                &self.entry.binding.as_ref().unwrap().pane_id,
                "@tmt.agent",
            ])
            .unwrap()
    }
    fn bind(&self, name: &str) -> BindingEntry {
        let host = Host::for_caller_with(&self.caller, self.server.runner()).unwrap();
        let mut storage = Storage::open(&self.database).unwrap();
        let bound = bind_identity(
            &mut storage,
            &mut host.session(),
            &self.entry.binding.as_ref().unwrap().pane_id,
            name,
            true,
        )
        .unwrap();
        storage.close().unwrap();
        BindingEntry {
            identity: bound.identity,
            binding: bound.binding,
        }
    }
    fn current(&self) -> BindingEntry {
        let mut storage = Storage::open(&self.database).unwrap();
        let entry = storage
            .with_binding_transaction(|records| records.entry_by_id(&self.entry.identity.id))
            .unwrap()
            .unwrap();
        storage.close().unwrap();
        entry
    }
}
#[derive(Clone, Copy)]
enum Boundary {
    Prepared,
    Cas,
}
struct AtBoundary<'a, F> {
    runner: PrivateRunner<'a>,
    boundary: Boundary,
    action: RefCell<Option<F>>,
    cas_calls: Cell<usize>,
}
impl<F: FnOnce()> CommandRunner for AtBoundary<'_, F> {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let cas = request.args.iter().any(|arg| arg == "if-shell");
        if cas {
            self.cas_calls.set(self.cas_calls.get() + 1);
        }
        let prepared = request
            .args
            .last()
            .is_some_and(|arg| arg == "@tmux-team.border");
        if cas
            && matches!(self.boundary, Boundary::Cas)
            && let Some(action) = self.action.borrow_mut().take()
        {
            action();
        }
        let result = self.runner.execute(request);
        if prepared
            && matches!(self.boundary, Boundary::Prepared)
            && let Some(action) = self.action.borrow_mut().take()
        {
            action();
        }
        result
    }
}
#[test]
fn an_independent_writer_commits_while_the_cutover_cas_is_held() {
    let fixture = Fixture::new();
    std::thread::scope(|scope| {
        let (start, wait) = std::sync::mpsc::channel();
        let (done, result) = std::sync::mpsc::channel();
        let database = &fixture.database;
        scope.spawn(move || {
            wait.recv().unwrap();
            let mut storage = Storage::open(database).unwrap();
            tmt_core::identity::create_or_resolve(
                &mut storage,
                "Concurrent",
                tmt_core::identity::Lifetime::Saved,
            )
            .unwrap();
            storage.close().unwrap();
            // The assertion receiver can disappear while its failure unwinds.
            let _ = done.send(());
        });
        let runner = AtBoundary {
            runner: fixture.server.runner(),
            boundary: Boundary::Cas,
            cas_calls: Cell::new(0),
            action: RefCell::new(Some(|| {
                start.send(()).unwrap();
                // The host CAS cannot finish until this receive returns. A writer
                // blocked by that CAS could never complete while this hold lasts.
                result
                    .recv_timeout(Duration::from_secs(2))
                    .expect("independent writer completes while host work remains held");
            })),
        };
        assert_eq!(
            migrate(&fixture.database, &fixture.caller, &runner),
            ["Pane option cutover: 1 converted, 0 left unchanged"]
        );
    });
    assert_eq!(fixture.marker(), fixture.raw);
    let mut storage = Storage::open(&fixture.database).unwrap();
    assert!(storage.find_identity("concurrent").unwrap().is_some());
    storage.close().unwrap();
}
#[test]
fn a_replacement_before_the_row_check_never_reaches_the_host_cas() {
    let fixture = Fixture::new();
    let replacement = RefCell::new(None);
    let runner = AtBoundary {
        runner: fixture.server.runner(),
        boundary: Boundary::Prepared,
        cas_calls: Cell::new(0),
        action: RefCell::new(Some(|| {
            replacement.replace(Some(fixture.bind("Replacement")));
        })),
    };
    let hints = migrate(&fixture.database, &fixture.caller, &runner);
    assert_eq!(
        runner.cas_calls.get(),
        0,
        "changed row refuses before any host CAS"
    );
    assert_eq!(hints.len(), 1);
    assert!(hints[0].contains("tmt this Original"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fixture.marker()).unwrap()["globalIdentity"]["bindingId"],
        replacement
            .borrow()
            .as_ref()
            .unwrap()
            .binding
            .as_ref()
            .unwrap()
            .id
    );
    assert_eq!(
        fixture
            .server
            .command(&[
                "show-options",
                "-pqv",
                "-t",
                &fixture.entry.binding.as_ref().unwrap().pane_id,
                "@tmux-team.agent"
            ])
            .unwrap(),
        fixture.raw
    );
}
#[test]
fn a_replacement_during_the_cas_keeps_its_marker_and_binding() {
    let fixture = Fixture::new();
    let replacement = RefCell::new(None);
    let runner = AtBoundary {
        runner: fixture.server.runner(),
        boundary: Boundary::Cas,
        cas_calls: Cell::new(0),
        action: RefCell::new(Some(|| {
            replacement.replace(Some(fixture.bind("Replacement")));
        })),
    };
    let hints = migrate(&fixture.database, &fixture.caller, &runner);
    assert_eq!(hints.len(), 1);
    assert!(hints[0].contains("tmt this Original"));
    let replacement = replacement.into_inner().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fixture.marker()).unwrap()["globalIdentity"]["bindingId"],
        replacement.binding.as_ref().unwrap().id
    );
    assert!(fixture.current().binding.is_none());
    let mut storage = Storage::open(&fixture.database).unwrap();
    assert_eq!(
        storage
            .with_binding_transaction(|records| records.entry_by_id(&replacement.identity.id))
            .unwrap(),
        Some(replacement)
    );
    storage.close().unwrap();
}
#[test]
fn a_detach_during_the_cas_leaves_only_a_marker_a_later_bind_reclaims() {
    let fixture = Fixture::new();
    let runner = AtBoundary {
        runner: fixture.server.runner(),
        boundary: Boundary::Cas,
        cas_calls: Cell::new(0),
        action: RefCell::new(Some(|| {
            let mut storage = Storage::open(&fixture.database).unwrap();
            storage
                .with_binding_transaction(|records| {
                    records.detach_binding(&fixture.entry.binding.as_ref().unwrap().id)
                })
                .unwrap();
            storage.close().unwrap();
        })),
    };
    let hints = migrate(&fixture.database, &fixture.caller, &runner);
    assert_eq!(hints.len(), 1);
    assert!(hints[0].contains("tmt this Original"));
    assert!(fixture.current().binding.is_none());
    assert_eq!(
        fixture.marker(),
        fixture.raw,
        "only the exact-old marker can survive the cutover"
    );
    let successor = fixture.bind("Successor");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fixture.marker()).unwrap()["globalIdentity"]["bindingId"],
        successor.binding.as_ref().unwrap().id
    );
    assert_ne!(
        successor.binding.as_ref().unwrap().id,
        fixture.entry.binding.as_ref().unwrap().id
    );
}
