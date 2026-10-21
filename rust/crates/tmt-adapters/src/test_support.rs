use std::{
    fs,
    path::Path,
    path::PathBuf,
    process::{Child, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

/// One invocation-owned filesystem fixture shared by native adapter tests.
pub(crate) struct TestDirectory {
    pub path: PathBuf,
}

impl TestDirectory {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-native-adapter-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // Refuse to reuse an existing directory; cleanup owns only this creation.
        fs::create_dir(&path).expect("create unique adapter test directory");
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            if std::thread::panicking() {
                eprintln!(
                    "Could not remove adapter fixture {}: {error}",
                    self.path.display()
                );
            } else {
                panic!(
                    "Could not remove adapter fixture {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

/// Owns a task-local child process and guarantees bounded cleanup on every test path.
///
/// `try_wait` marks the child as reaped before returning its status. Drop therefore
/// never signals a child after observing that it has exited, avoiding a recycled-PID
/// signal while still killing and reaping a live child after a panic or timeout.
pub(crate) struct TestChild {
    pub child: Child,
}

impl TestChild {
    pub(crate) fn new(child: Child) -> Self {
        Self { child }
    }

    pub(crate) fn wait_for_content(&mut self, path: &Path, expected: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if fs::read_to_string(path).ok().as_deref() == Some(expected) {
                return;
            }
            if let Some(status) = self
                .child
                .try_wait()
                .expect("inspect task-owned test child status")
            {
                panic!("test child exited before {}: {status}", path.display());
            }
            assert!(
                Instant::now() < deadline,
                "test child did not publish {expected:?} to {}",
                path.display(),
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub(crate) fn wait_for_exit(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .expect("inspect task-owned test child status")
            {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let status = self.child.wait().expect("reap timed-out test child");
                panic!("test child did not exit before deadline: {status}");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) | Err(_) => {}
            Ok(None) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
}

/// Where the host's tmux server keeps its sockets, from the ambient environment.
///
/// A test process inherits `TMUX` and `TMUX_PANE` from the pane it runs in, and `$TMUX`
/// outranks `TMUX_TMPDIR`, so a bare tmux reaches the host's server. `PrivateServer` and
/// every other fixture that starts one therefore proves its socket private first; the
/// TypeScript helper and `scripts/private-tmux.sh` hold the same rule, which
/// the tmt-dev skill's focused-checks reference states once.
pub(crate) struct HostTmux {
    uid: u32,
    tmux_tmpdir: Option<PathBuf>,
    tmux: Option<String>,
}

impl HostTmux {
    pub(crate) fn from_process() -> Self {
        Self {
            uid: nix::unistd::getuid().as_raw(),
            tmux_tmpdir: std::env::var_os("TMUX_TMPDIR").map(PathBuf::from),
            tmux: std::env::var("TMUX").ok(),
        }
    }

    /// The server socket `$TMUX` names, if any.
    fn named_socket(&self) -> Option<PathBuf> {
        let named = self.tmux.as_deref()?.split(',').next()?;
        (!named.is_empty()).then(|| PathBuf::from(named))
    }

    fn socket_directories(&self) -> Vec<PathBuf> {
        let uid = self.uid;
        let mut directories = vec![
            PathBuf::from(format!("/tmp/tmux-{uid}")),
            PathBuf::from(format!("/private/tmp/tmux-{uid}")),
        ];
        if let Some(root) = &self.tmux_tmpdir {
            directories.push(root.join(format!("tmux-{uid}")));
        }
        if let Some(parent) = self.named_socket().as_deref().and_then(Path::parent) {
            directories.push(parent.to_path_buf());
        }
        directories
            .into_iter()
            .filter(|directory| directory.is_absolute())
            .map(|directory| resolve_loosely(&directory))
            .collect()
    }
}

/// The path tmux binds is limited to 104 bytes on macOS, NUL included.
const SOCKET_PATH_LIMIT: usize = 103;

/// The physical form of `path`, even when its tail does not exist yet.
fn resolve_loosely(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut tail = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        tail.push(name.to_owned());
        if !existing.pop() {
            break;
        }
    }
    let mut resolved = fs::canonicalize(&existing).unwrap_or(existing);
    resolved.extend(tail.iter().rev());
    resolved
}

/// Why `socket` is not a private socket inside `root`, or `None` when it is. `root` is the
/// directory the test owns; the socket must be a new or existing file strictly inside it.
pub(crate) fn private_socket_refusal(
    socket: &Path,
    root: &Path,
    host: &HostTmux,
) -> Option<String> {
    if !socket.is_absolute() {
        return Some(format!("{} is not absolute", socket.display()));
    }
    // A missing component followed by `..` would be created and then climbed out of.
    if socket
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Some(format!("{} has a .. component", socket.display()));
    }
    let Ok(root) = fs::canonicalize(root) else {
        return Some(format!("the test root {} does not exist", root.display()));
    };
    if fs::symlink_metadata(socket).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Some(format!("{} is a symlink", socket.display()));
    }
    let physical = resolve_loosely(socket);
    if !physical.starts_with(&root) || physical == root {
        return Some(format!(
            "{} is outside the test root {}",
            physical.display(),
            root.display()
        ));
    }
    if let Some(directory) = host
        .socket_directories()
        .into_iter()
        .find(|directory| physical.starts_with(directory))
    {
        return Some(format!(
            "{} lies inside the host tmux socket directory {}",
            physical.display(),
            directory.display()
        ));
    }
    // The kernel measures the string tmux binds, not its resolved form.
    (socket.as_os_str().len() > SOCKET_PATH_LIMIT)
        .then(|| format!("{} is too long for a Unix socket path", socket.display()))
}

/// Panics, before any tmux command runs, unless `socket` is private to `root`.
pub(crate) fn assert_private_socket(socket: &Path, root: &Path) {
    if let Some(reason) = private_socket_refusal(socket, root, &HostTmux::from_process()) {
        panic!("private tmux refused: {reason}");
    }
}

#[cfg(test)]
mod private_socket_tests;

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use nix::{
        errno::Errno,
        sys::wait::{WaitPidFlag, WaitStatus, waitpid},
        unistd::Pid,
    };
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        process::{Command, Stdio},
    };

    const READY_ENV: &str = "TMT_TEST_CHILD_READY";
    const MARKER_ENV: &str = "TMT_TEST_CHILD_MARKER";
    const RELEASE_ENV: &str = "TMT_TEST_CHILD_RELEASE";
    const CHILD_TEST: &str = "test_support::tests::cleanup_child_process";
    const CHILD_START_TIMEOUT: Duration = Duration::from_secs(2);

    #[test]
    fn cleanup_child_process() {
        let Some(ready) = std::env::var_os(READY_ENV) else {
            return;
        };
        let marker = std::env::var_os(MARKER_ENV).expect("cleanup marker environment");
        let release = PathBuf::from(std::env::var_os(RELEASE_ENV).expect("cleanup release path"));
        fs::write(ready, "ready").expect("publish cleanup child readiness");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !release.exists() {
            assert!(
                Instant::now() < deadline,
                "parent did not release or stop cleanup child"
            );
            thread::sleep(Duration::from_millis(10));
        }
        fs::write(marker, "survived").expect("publish cleanup child marker");
    }

    fn spawn_cleanup_child(directory: &TestDirectory) -> TestChild {
        let ready = directory.path.join("ready");
        let marker = directory.path.join("marker");
        let child = Command::new(std::env::current_exe().expect("locate test executable"))
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .env(READY_ENV, &ready)
            .env(MARKER_ENV, &marker)
            .env(RELEASE_ENV, directory.path.join("release"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cleanup child");
        let mut child = TestChild::new(child);
        child.wait_for_content(&ready, "ready", CHILD_START_TIMEOUT);
        child
    }

    fn assert_reaped(pid: u32) {
        match waitpid(
            Pid::from_raw(i32::try_from(pid).expect("child PID fits in pid_t")),
            Some(WaitPidFlag::WNOHANG),
        ) {
            Err(Errno::ECHILD) => {}
            Ok(WaitStatus::StillAlive) => panic!("test child {pid} was not reaped"),
            Ok(status) => panic!("unexpected status for test child {pid}: {status:?}"),
            Err(error) => panic!("inspect test child {pid}: {error}"),
        }
    }

    fn assert_marker_absent(directory: &TestDirectory) {
        assert!(!directory.path.join("marker").exists());
    }

    #[test]
    fn drop_reaps_a_live_child_during_panic_unwind() {
        let directory = TestDirectory::new();
        let mut child = Some(spawn_cleanup_child(&directory));
        let pid = child.as_ref().expect("cleanup child").child.id();
        let started = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _child = child.take().expect("take cleanup child");
            panic!("force cleanup during panic unwind");
        }));
        assert!(result.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cleanup waited for the child's fallback deadline"
        );
        assert_reaped(pid);
        assert_marker_absent(&directory);
    }

    #[test]
    fn timeout_reaps_a_live_child_before_panicking() {
        let directory = TestDirectory::new();
        let mut pid = None;
        let started = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut child = spawn_cleanup_child(&directory);
            pid = Some(child.child.id());
            let _ = child.wait_for_exit(Duration::from_millis(20));
        }));
        assert!(result.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "timeout did not stop its child promptly"
        );
        assert_reaped(pid.expect("cleanup child PID"));
        assert_marker_absent(&directory);
    }

    #[test]
    fn drop_does_not_signal_a_child_after_normal_exit() {
        let directory = TestDirectory::new();
        let mut child = spawn_cleanup_child(&directory);
        let pid = child.child.id();
        fs::write(directory.path.join("release"), "finish").unwrap();
        let status = child.wait_for_exit(Duration::from_secs(2));
        assert!(status.success());
        assert_eq!(
            fs::read_to_string(directory.path.join("marker")).expect("normal-exit marker"),
            "survived"
        );
        drop(child);
        assert_reaped(pid);
    }
}
