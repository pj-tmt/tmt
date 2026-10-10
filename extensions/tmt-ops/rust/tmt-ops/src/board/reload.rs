//! In-place reload. A board notices that the installed `tmt-ops` was replaced,
//! or that `tmt ops ui --reload-all` asked, and re-executes the command that
//! started it in the same pane. Core is not involved: the signal is the
//! launcher's own file and a timestamp in the user's cache.

use super::resume::{self, Resume};
use crate::{cache, core::SquadError};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    os::unix::{ffi::OsStrExt, fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const REQUEST: &str = "request";
/// The file checks run on the board's own wakes, at most this often.
const CHECK_EVERY: Duration = Duration::from_secs(1);
/// A reload waits for this long without input, so no keystroke is lost to the exec.
const IDLE: Duration = Duration::from_secs(1);

/// Appended to the footer while a reload is due but something open must be finished first.
pub(super) const WAITING: &str = "  reload waiting";

/// Shown by the board a reload started.
pub(super) fn reloaded() -> String {
    format!("Board reloaded · tmt-ops {}", env!("CARGO_PKG_VERSION"))
}

/// Asks every board that started before now to reload. Boards older than this
/// request mechanism do not read it and keep running until restarted by hand.
pub(crate) fn request_all(now_ms: u64) -> Result<(), SquadError> {
    let directory = resume::directory().ok_or_else(|| {
        SquadError::new(
            "SQUAD_RELOAD_FAILED",
            "There is no cache directory to record the request in.",
        )
    })?;
    record(&directory, now_ms)
}

fn record(directory: &Path, now_ms: u64) -> Result<(), SquadError> {
    cache::replace(&directory.join(REQUEST), now_ms.to_string().as_bytes()).map_err(|error| {
        SquadError::new(
            "SQUAD_RELOAD_FAILED",
            format!("The reload request could not be recorded: {error}"),
        )
    })
}

/// The command that started this board, so a reload can start it again.
#[derive(Debug, Clone)]
pub(super) struct Launcher {
    program: PathBuf,
    args: Vec<OsString>,
}

/// What the launcher file was when the board started: the release it links to
/// and that file's identity, so both a flipped `current` link and a rewritten
/// binary count as a replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    target: PathBuf,
    device: u64,
    inode: u64,
    modified: (i64, i64),
}

impl Launcher {
    fn current() -> Option<Self> {
        Self::resolve(std::env::args_os(), std::env::var_os("PATH").as_deref())
    }

    /// `argv[0]` as a path: kept when it names a directory, else found on `path`
    /// like the shell did.
    fn resolve(argv: impl IntoIterator<Item = OsString>, path: Option<&OsStr>) -> Option<Self> {
        let mut argv = argv.into_iter();
        let name = PathBuf::from(argv.next()?);
        let program = if name.as_os_str().as_bytes().contains(&b'/') {
            name
        } else {
            std::env::split_paths(path?)
                .map(|directory| directory.join(&name))
                .find(|candidate| runnable(candidate))?
        };
        Some(Self {
            program,
            args: argv.collect(),
        })
    }

    fn fingerprint(&self) -> Option<Fingerprint> {
        let target = fs::canonicalize(&self.program).ok()?;
        let metadata = fs::metadata(&target).ok()?;
        Some(Fingerprint {
            target,
            device: metadata.dev(),
            inode: metadata.ino(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
        })
    }

    /// Replaces this process; returns only the reason it could not.
    fn exec(&self) -> io::Error {
        Command::new(&self.program).args(&self.args).exec()
    }
}

fn runnable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.mode() & 0o111 != 0)
}

/// Why a reload is pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reason {
    Replaced,
    Requested,
}

/// Notices a replaced binary or a reload request. The default is inert: boards
/// in tests, and processes whose launcher cannot be found, never reload.
#[derive(Default)]
pub(super) struct Watch {
    launcher: Option<Launcher>,
    baseline: Option<Fingerprint>,
    started_ms: u64,
    idle: Duration,
    /// Where snapshots and the reload request live.
    directory: Option<PathBuf>,
    checked: Option<Instant>,
    reason: Option<Reason>,
}

impl Watch {
    pub fn start(now_ms: u64) -> Self {
        let launcher = Launcher::current();
        let baseline = launcher.as_ref().and_then(Launcher::fingerprint);
        Self {
            launcher,
            baseline,
            started_ms: now_ms,
            idle: IDLE,
            directory: resume::directory(),
            ..Self::default()
        }
    }

    /// How long the board must have been without input before it may reload.
    pub fn idle(&self) -> Duration {
        self.idle
    }

    pub fn pending(&self) -> bool {
        self.reason.is_some()
    }

    /// Looks again at most once a second. True when a reload just became pending.
    pub fn poll(&mut self, now: Instant) -> bool {
        let Some(launcher) = &self.launcher else {
            return false;
        };
        if self.reason.is_some()
            || self
                .checked
                .is_some_and(|last| now.duration_since(last) < CHECK_EVERY)
        {
            return false;
        }
        self.checked = Some(now);
        // A launcher that cannot be started right now (mid-upgrade) is not a reload yet.
        if !runnable(&launcher.program) {
            return false;
        }
        let replaced = self
            .baseline
            .as_ref()
            .zip(launcher.fingerprint())
            .is_some_and(|(started, now)| *started != now);
        self.reason = if replaced {
            Some(Reason::Replaced)
        } else if self.requested() {
            Some(Reason::Requested)
        } else {
            None
        };
        self.reason.is_some()
    }

    fn requested(&self) -> bool {
        let Some(path) = self
            .directory
            .as_ref()
            .map(|directory| directory.join(REQUEST))
        else {
            return false;
        };
        let mut text = String::new();
        fs::File::open(path)
            .and_then(|file| file.take(32).read_to_string(&mut text))
            .is_ok()
            && text
                .trim()
                .parse::<u64>()
                .is_ok_and(|asked| asked > self.started_ms)
    }

    /// Writes the snapshot and replaces this process with the launcher. The caller
    /// has restored the terminal; this returns only when the restart failed.
    pub fn restart(&self, resume: &Resume, now_ms: u64) -> SquadError {
        self.restart_with(resume, now_ms, Launcher::exec)
    }

    /// `restart` with the process replacement supplied, so a test can fail it without
    /// a real `exec`: a failed `exec` leaves SIGPIPE at its default for the whole process.
    fn restart_with(
        &self,
        resume: &Resume,
        now_ms: u64,
        exec: impl FnOnce(&Launcher) -> io::Error,
    ) -> SquadError {
        let Some(launcher) = &self.launcher else {
            return SquadError::new(
                "SQUAD_RELOAD_FAILED",
                "The board has no command to restart.",
            );
        };
        let directory = &self.directory;
        let pid = std::process::id();
        // A snapshot that cannot be written costs the state, not the restart.
        if let Some(directory) = directory {
            let _ = resume.write(directory, pid, now_ms);
        }
        let error = exec(launcher);
        if let Some(directory) = directory {
            Resume::discard(directory, pid);
        }
        SquadError::new(
            "SQUAD_RELOAD_FAILED",
            format!(
                "The board could not restart ({}): {error}",
                launcher.program.display()
            ),
        )
    }
}

#[cfg(test)]
impl Watch {
    /// A watch over `program`, as `start` builds it, with an injected state directory.
    pub(super) fn due() -> Self {
        Self {
            reason: Some(Reason::Requested),
            ..Self::default()
        }
    }

    pub(super) fn over(program: &Path, directory: &Path, started_ms: u64) -> Self {
        let launcher = Launcher {
            program: program.to_owned(),
            args: vec!["ui".into()],
        };
        Self {
            baseline: launcher.fingerprint(),
            launcher: Some(launcher),
            started_ms,
            directory: Some(directory.to_owned()),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ops-reload-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn install(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Two releases and a `tmt-ops` -> `current` -> release chain, like the managed install.
    fn installation(root: &Path) -> PathBuf {
        for name in ["one", "two"] {
            fs::create_dir_all(root.join(name)).unwrap();
            install(&root.join(name).join("tmt-ops"), name);
        }
        flip(root, "one");
        std::os::unix::fs::symlink("current/tmt-ops", root.join("tmt-ops")).unwrap();
        root.join("tmt-ops")
    }

    fn flip(root: &Path, name: &str) {
        let staged = root.join("current.new");
        let _ = fs::remove_file(&staged);
        std::os::unix::fs::symlink(name, &staged).unwrap();
        fs::rename(staged, root.join("current")).unwrap();
    }

    fn after(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    #[test]
    fn a_flipped_current_link_or_a_rewritten_binary_is_a_replacement() {
        let root = scratch("replaced");
        let program = installation(&root);
        let state = root.join("state");
        let start = Instant::now();

        let mut watch = Watch::over(&program, &state, 1_000);
        assert!(!watch.poll(start), "an unchanged install is not a reload");
        flip(&root, "two");
        assert!(!watch.poll(start), "a check runs at most once a second");
        assert!(watch.poll(after(start, 1)), "the flipped link is noticed");
        assert_eq!(watch.reason, Some(Reason::Replaced));
        assert!(
            !watch.poll(after(start, 2)),
            "a pending reload is reported once"
        );
        assert!(watch.pending());

        // Rewriting the file in place counts too, though no link moved.
        let mut watch = Watch::over(&program, &state, 1_000);
        fs::remove_file(root.join("two").join("tmt-ops")).unwrap();
        install(&root.join("two").join("tmt-ops"), "two, rebuilt");
        assert!(watch.poll(start));
        assert_eq!(watch.reason, Some(Reason::Replaced));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_launcher_that_cannot_start_yet_waits_for_the_next_check() {
        let root = scratch("transient");
        let program = installation(&root);
        let state = root.join("state");
        let start = Instant::now();
        let mut watch = Watch::over(&program, &state, 1_000);
        fs::create_dir_all(&state).unwrap();
        fs::write(state.join(REQUEST), "2000").unwrap();

        fs::remove_file(root.join("one").join("tmt-ops")).unwrap();
        assert!(!watch.poll(start), "a missing launcher never reloads");
        assert!(!watch.pending());
        install(&root.join("one").join("tmt-ops"), "one");
        assert!(
            watch.poll(after(start, 1)),
            "the next check sees the request"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_request_reloads_only_boards_that_started_before_it() {
        let root = scratch("request");
        let program = installation(&root);
        let state = root.join("state");
        fs::create_dir_all(&state).unwrap();
        let start = Instant::now();

        let mut old = Watch::over(&program, &state, 1_000);
        let mut new = Watch::over(&program, &state, 3_000);
        assert!(!old.poll(start), "no request yet");
        fs::write(state.join(REQUEST), "2000").unwrap();
        assert!(old.poll(after(start, 1)));
        assert_eq!(old.reason, Some(Reason::Requested));
        assert!(
            !new.poll(after(start, 1)),
            "a board started after the request ignores it"
        );
        for text in ["", "soon", "-5", "99999999999999999999999999"] {
            fs::write(state.join(REQUEST), text).unwrap();
            assert!(
                !Watch::over(&program, &state, 1_000).poll(start),
                "{text:?} is not a request"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn request_all_records_a_time_that_only_older_boards_obey() {
        let root = scratch("record");
        let program = installation(&root);
        let state = root.join("cache");
        record(&state, 7_000).unwrap();
        let start = Instant::now();
        assert!(Watch::over(&program, &state, 6_999).poll(start));
        assert!(!Watch::over(&program, &state, 7_000).poll(start));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_inert_watch_never_reloads() {
        let mut watch = Watch::default();
        assert!(!watch.poll(Instant::now()) && !watch.pending());
    }

    #[test]
    fn the_launcher_is_argv0_found_on_path_and_keeps_its_arguments() {
        let root = scratch("launcher");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        install(&bin.join("tmt-ops"), "board");
        fs::write(bin.join("not-executable"), "").unwrap();
        let path = std::env::join_paths([root.join("missing"), bin.clone()]).unwrap();
        let argv = |program: &str| [program, "ui", "--tabs=infra"].map(OsString::from);
        let by_name = Launcher::resolve(argv("tmt-ops"), Some(&path)).unwrap();
        assert_eq!(by_name.program, bin.join("tmt-ops"));
        assert_eq!(by_name.args, ["ui", "--tabs=infra"]);
        let by_path = Launcher::resolve(argv("/opt/tmt-ops"), None).unwrap();
        assert_eq!(by_path.program, Path::new("/opt/tmt-ops"));
        assert!(Launcher::resolve(argv("not-executable"), Some(&path)).is_none());
        assert!(Launcher::resolve(argv("tmt-ops"), None).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_restart_that_cannot_exec_reports_it_and_keeps_no_snapshot() {
        let root = scratch("restart");
        let state = root.join("state");
        let resume = Resume {
            tab: "product".into(),
            selected: None,
            focus: 0,
            follow: true,
            scrolls: Vec::new(),
            expanded: Vec::new(),
            folds: Vec::new(),
            picks: None,
            search: String::new(),
        };
        let error =
            Watch::over(&root.join("gone"), &state, 1_000)
                .restart_with(&resume, 5_000, |_| io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(error.code, "SQUAD_RELOAD_FAILED");
        assert!(
            error.message.contains("could not restart"),
            "{}",
            error.message
        );
        let leftovers = fs::read_dir(&state).map_or(0, Iterator::count);
        assert_eq!(leftovers, 0, "a failed exec discards its snapshot");
        let _ = fs::remove_dir_all(root);
    }
}
