//! Process ownership for native Colab fixtures, including assertion unwinding.
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill, killpg},
    sys::wait::{WaitPidFlag, WaitStatus, waitpid},
    unistd::{Pid, getuid},
};
use rustix::process::{WaitId, WaitIdOptions, waitid};
use std::{
    fs,
    io::{self, Read},
    ops::{Deref, DerefMut},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output},
    thread,
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(5);

pub trait FixtureCommand {
    fn spawn_owned(&mut self) -> io::Result<OwnedChild>;
}

impl FixtureCommand for Command {
    fn spawn_owned(&mut self) -> io::Result<OwnedChild> {
        let child = self.process_group(0).spawn()?;
        let group = Some(Pid::from_raw(child.id() as i32));
        Ok(OwnedChild {
            child: Some(child),
            group,
        })
    }
}

/// The fresh group belongs to this fixture; dropping a std Child alone does not stop it.
pub struct OwnedChild {
    child: Option<Child>,
    group: Option<Pid>,
}

impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.child.as_ref().unwrap()
    }
}

impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.child.as_mut().unwrap()
    }
}

impl OwnedChild {
    /// The negative control removes group cleanup but keeps a handle for independent rescue.
    pub fn without_guard(mut self) -> Child {
        self.group = None;
        self.child.take().unwrap()
    }

    /// Observe exit without reaping while the group is owned: the unreaped leader
    /// reserves the numeric group, so cleanup can still signal it safely.
    fn observe(&mut self, options: WaitIdOptions) -> io::Result<Option<ExitStatus>> {
        let child = self.child.as_mut().unwrap();
        if self.group.is_none() {
            return if options.contains(WaitIdOptions::NOHANG) {
                child.try_wait()
            } else {
                child.wait().map(Some)
            };
        }
        let pid = rustix::process::Pid::from_raw(child.id() as i32)
            .ok_or_else(|| io::Error::other("invalid fixture PID"))?;
        waitid(
            WaitId::Pid(pid),
            options | WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
        )
        .map_err(io::Error::from)?
        .map(
            |status| match (status.exit_status(), status.terminating_signal()) {
                (Some(code), _) => Ok(ExitStatus::from_raw((code & 0xff) << 8)),
                (None, Some(signal)) => Ok(ExitStatus::from_raw(
                    signal | if status.dumped() { 0x80 } else { 0 },
                )),
                (None, None) => Err(io::Error::other("fixture exit status without a cause")),
            },
        )
        .transpose()
    }

    /// An observed exit ends the fixture group: what remains of it is stopped
    /// before the leader's only reap.
    fn finish_after(&mut self, status: Option<ExitStatus>) -> io::Result<Option<ExitStatus>> {
        if status.is_some() {
            self.finish_group().map_err(io::Error::other)?;
        }
        Ok(status)
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.observe(WaitIdOptions::NOHANG)?;
        self.finish_after(status)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.child.as_mut().unwrap().stdin.take());
        let status = self.observe(WaitIdOptions::empty())?;
        self.finish_after(status)?
            .ok_or_else(|| io::Error::other("fixture wait returned no status"))
    }

    pub fn wait_until(&mut self, deadline: Instant) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "fixture command did not exit",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Like `Child::wait_with_output`, but the group is stopped before the leader's reap.
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        let child = self.child.as_mut().unwrap();
        drop(child.stdin.take());
        let stdout = child.stdout.take();
        let stderr = child.stderr.take().map(|mut pipe| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                pipe.read_to_end(&mut bytes).map(|_| bytes)
            })
        });
        let mut out = Vec::new();
        if let Some(mut pipe) = stdout {
            pipe.read_to_end(&mut out)?;
        }
        let err = match stderr {
            Some(reader) => reader
                .join()
                .map_err(|_| io::Error::other("fixture stderr reader panicked"))??,
            None => Vec::new(),
        };
        let status = self.wait()?;
        Ok(Output {
            status,
            stdout: out,
            stderr: err,
        })
    }

    fn finish_group(&mut self) -> Result<(), String> {
        self.finish_group_with(killpg)
    }

    /// Destructive signals precede the only reap; afterwards the numeric group is only
    /// probed. A leader this owner no longer holds unreaped is never signalled through.
    fn finish_group_with(
        &mut self,
        mut signal_group: impl FnMut(Pid, Option<Signal>) -> Result<(), Errno>,
    ) -> Result<(), String> {
        let Some(group) = self.group else {
            return Ok(());
        };
        if let Err(error) = self.observe(WaitIdOptions::NOHANG) {
            return Err(format!(
                "fixture process group {group} not signalled: its leader is no longer \
                 this guard's unreaped child ({error}); cleanup uncertain"
            ));
        }
        let mut denied = Vec::new();
        let mut signal = |signal| match signal_group(group, Some(signal)) {
            Ok(()) | Err(Errno::ESRCH) => {}
            Err(error) => denied.push(format!("{signal:?}: {error}")),
        };
        signal(Signal::SIGTERM);
        let grace = Instant::now() + WAIT;
        while self
            .observe(WaitIdOptions::NOHANG)
            .map_err(|error| error.to_string())?
            .is_none()
            && Instant::now() < grace
        {
            thread::sleep(Duration::from_millis(20));
        }
        signal(Signal::SIGKILL);
        let child = self.child.as_mut().unwrap();
        let deadline = Instant::now() + WAIT;
        while child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            if Instant::now() >= deadline {
                return Err(format!(
                    "fixture process group {group} leader {} survived SIGKILL; denied {denied:?}",
                    child.id()
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
        // Reaped: the number may now be reused, so only group ESRCH confirms absence.
        let deadline = Instant::now() + WAIT;
        loop {
            let probe = signal_group(group, None);
            if probe == Err(Errno::ESRCH) {
                self.group = None;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "fixture process group {group} absence unconfirmed after its leader was \
                     reaped; last probe {probe:?}, denied {denied:?}"
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.finish_group().expect("fixture process cleanup failed");
    }
}

fn capture(program: &str, args: &[&str]) -> Result<tmt_invoke::Output, String> {
    let args = args
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    tmt_invoke::invoke(
        tmt_invoke::Request {
            program: Path::new(program),
            args: &args,
            input: &[],
            deadline: Instant::now() + WAIT,
            max_stream_bytes: 4 * 1024 * 1024,
            launch: tmt_invoke::LaunchOptions::default(),
        },
        None,
    )
    .map_err(|error| format!("fixture process inspection: {error}"))
}

fn below(path: &str, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| Path::new(path).starts_with(root))
}

fn in_arguments(arguments: &str, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| {
        let root = root.to_str().unwrap();
        arguments.match_indices(root).any(|(start, _)| {
            let before = arguments[..start].chars().next_back();
            let after = arguments[start + root.len()..].chars().next();
            before.is_none_or(|c| c.is_whitespace() || c == '\'' || c == '"' || c == '=')
                && after.is_none_or(|c| c.is_whitespace() || c == '/' || c == '\'' || c == '"')
        })
    })
}

/// Only the current test's root is admitted, never a name shared by parallel fixtures.
pub fn residents(root: &Path) -> Result<Vec<Pid>, String> {
    let mut roots = vec![root.to_path_buf()];
    if let Ok(canonical) = fs::canonicalize(root) {
        roots.push(canonical);
    }
    let output = capture("/bin/ps", &["-axo", "uid=,pid=,command="])?;
    if !output.status.success() {
        return Err("ps failed during fixture cleanup".into());
    }
    let uid = getuid().as_raw().to_string();
    let mut pids = Vec::new();
    #[cfg(target_os = "linux")]
    let mut own_pids = Vec::new();
    for line in String::from_utf8(output.stdout)
        .map_err(|e| e.to_string())?
        .lines()
    {
        let mut parts = line.split_whitespace();
        let Some(owner) = parts.next() else { continue };
        let Some(pid) = parts.next().and_then(|s| s.trim().parse::<i32>().ok()) else {
            continue;
        };
        let args = parts.collect::<Vec<_>>().join(" ");
        #[cfg(target_os = "linux")]
        if owner == uid {
            own_pids.push(pid);
        }
        if owner == uid && pid != std::process::id() as i32 && in_arguments(&args, &roots) {
            pids.push(Pid::from_raw(pid));
        }
    }
    #[cfg(target_os = "linux")]
    for entry in fs::read_dir("/proc").map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        if pid == std::process::id() as i32 || !own_pids.contains(&pid) {
            continue;
        }
        if let Ok(cwd) = fs::read_link(entry.path().join("cwd"))
            && below(cwd.to_str().unwrap(), &roots)
        {
            pids.push(Pid::from_raw(pid));
        }
        // A background worker owns this lock even if it no longer shares the launcher cwd.
        if let Ok(files) = fs::read_dir(entry.path().join("fd")) {
            for file in files.flatten() {
                if let Ok(path) = fs::read_link(file.path())
                    && path.ends_with("colab/serve.lock")
                    && below(path.to_str().unwrap(), &roots)
                {
                    pids.push(Pid::from_raw(pid));
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let output = capture("/usr/sbin/lsof", &["-a", "-u", &uid, "-d", "cwd", "-Fpn"])?;
        if !output.status.success() {
            return Err("lsof cwd inspection failed".into());
        }
        let mut pid = None;
        for line in String::from_utf8(output.stdout)
            .map_err(|e| e.to_string())?
            .lines()
        {
            if let Some(value) = line.strip_prefix('p') {
                pid = value.parse::<i32>().ok();
            }
            if let Some(path) = line.strip_prefix('n')
                && below(path, &roots)
                && let Some(pid) = pid.filter(|pid| *pid != std::process::id() as i32)
            {
                pids.push(Pid::from_raw(pid));
            }
        }
        for lock in [
            root.join("selected/colab/serve.lock"),
            root.join("colab/serve.lock"),
        ] {
            if !lock.exists() {
                continue;
            }
            let output = capture(
                "/usr/sbin/lsof",
                &["-a", "-u", &uid, "-Fpn", lock.to_str().unwrap()],
            )?;
            // lsof returns 1 when no process has the existing file open.
            if !output.status.success()
                && (output.status.code != Some(1) || !output.stderr.is_empty())
            {
                return Err("lsof serve-lock inspection failed".into());
            }
            for line in String::from_utf8(output.stdout)
                .map_err(|e| e.to_string())?
                .lines()
            {
                if let Some(pid) = line.strip_prefix('p').and_then(|s| s.parse::<i32>().ok())
                    && pid != std::process::id() as i32
                {
                    pids.push(Pid::from_raw(pid));
                }
            }
        }
    }
    pids.sort_by_key(|pid| pid.as_raw());
    pids.dedup();
    Ok(pids)
}

pub fn assert_empty(root: &Path) {
    let pids = residents(root).expect("fixture survivor inspection failed");
    assert!(
        pids.is_empty(),
        "fixture root {} has surviving processes: {pids:?}",
        root.display()
    );
}

/// A detached worker is no longer a child. Its owner lock, cwd or argv fences the recovered PID.
/// Never delete a root while it still identifies a running process.
pub fn clean_root(root: &Path) {
    let mut owned = residents(root).expect("fixture process inspection failed");
    if owned.is_empty() {
        return;
    }
    for signal in [Signal::SIGTERM, Signal::SIGKILL] {
        // Recheck membership immediately before signalling, including after PID reuse.
        let current = residents(root).expect("fixture process inspection failed");
        owned.retain(|pid| current.contains(pid));
        for pid in &owned {
            match kill(*pid, signal) {
                Ok(()) | Err(Errno::ESRCH) => {}
                Err(error) => panic!("cannot stop fixture process {pid}: {error}"),
            }
        }
        let deadline = Instant::now() + WAIT;
        loop {
            // Child guards have already dropped. Reap an abandoned direct child too;
            // detached workers return ECHILD and are waited by their exit/absence.
            owned.retain(|pid| match waitpid(*pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(..) | WaitStatus::Signaled(..)) => false,
                Ok(_) | Err(Errno::ECHILD) => kill(*pid, None) != Err(Errno::ESRCH),
                Err(error) => panic!("cannot wait for fixture process {pid}: {error}"),
            });
            if owned.is_empty() {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if owned.is_empty() {
            break;
        }
    }
    assert_empty(root);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The leader is still this process's unreaped child (live or zombie).
    fn unreaped(pid: Pid) -> bool {
        let pid = rustix::process::Pid::from_raw(pid.as_raw()).unwrap();
        waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )
        .is_ok()
    }

    /// Records each call and whether the leader was still unreaped at that moment.
    fn recording(
        leader: Pid,
        calls: &mut Vec<(Option<Signal>, bool)>,
    ) -> impl FnMut(Pid, Option<Signal>) -> Result<(), Errno> + '_ {
        move |group, signal| {
            calls.push((signal, unreaped(leader)));
            killpg(group, signal)
        }
    }

    fn assert_no_signal_after_reap(calls: &[(Option<Signal>, bool)]) {
        assert!(
            calls
                .iter()
                .all(|(signal, unreaped)| signal.is_none() || *unreaped),
            "{calls:?}"
        );
        assert!(
            calls
                .iter()
                .any(|(signal, unreaped)| signal.is_none() && !unreaped),
            "absence must be probed after the reap: {calls:?}"
        );
    }

    #[test]
    fn an_observed_exit_stops_the_rest_of_the_group_before_the_only_reap() {
        let root = std::env::temp_dir().join(format!("tmt-colab-group-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 300 & echo $! > descendant; exit 3"])
            .current_dir(&root)
            .spawn_owned()
            .unwrap();
        let leader = Pid::from_raw(child.id() as i32);
        let deadline = Instant::now() + WAIT;
        while child.observe(WaitIdOptions::NOHANG).unwrap().is_none() {
            assert!(Instant::now() < deadline, "leader did not exit");
            thread::sleep(Duration::from_millis(20));
        }
        // Observing is not reaping: the exited leader still reserves the group.
        assert!(unreaped(leader), "observation reaped the group leader");
        let descendant = Pid::from_raw(
            fs::read_to_string(root.join("descendant"))
                .unwrap()
                .trim()
                .parse()
                .unwrap(),
        );
        assert_eq!(kill(descendant, None), Ok(()));
        let mut calls = Vec::new();
        child
            .finish_group_with(recording(leader, &mut calls))
            .unwrap();
        assert_no_signal_after_reap(&calls);
        assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(3));
        assert!(!unreaped(leader));
        assert!(child.group.is_none());
        let deadline = Instant::now() + WAIT;
        while kill(descendant, None) != Err(Errno::ESRCH) {
            assert!(
                Instant::now() < deadline,
                "descendant {descendant} survived"
            );
            thread::sleep(Duration::from_millis(20));
        }
        drop(child);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn wrapper_waits_return_the_status_after_the_group_is_gone() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 300 & exit 5"])
            .spawn_owned()
            .unwrap();
        let leader = Pid::from_raw(child.id() as i32);
        assert_eq!(child.wait().unwrap().code(), Some(5));
        assert!(child.group.is_none());
        assert_eq!(killpg(leader, None), Err(Errno::ESRCH));
        assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(5));
    }

    #[test]
    fn a_live_leader_is_signalled_before_its_only_reap() {
        let mut live = Command::new("/bin/sleep").arg("300").spawn_owned().unwrap();
        let leader = Pid::from_raw(live.id() as i32);
        let mut calls = Vec::new();
        live.finish_group_with(recording(leader, &mut calls))
            .unwrap();
        assert_eq!(calls[0], (Some(Signal::SIGTERM), true));
        assert_no_signal_after_reap(&calls);
        assert_eq!(kill(leader, None), Err(Errno::ESRCH));
    }

    #[test]
    fn wait_with_output_collects_both_streams_and_stops_the_group() {
        let child = Command::new("/bin/sh")
            .args(["-c", "cat; echo err >&2; exit 4"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn_owned()
            .unwrap();
        let leader = Pid::from_raw(child.id() as i32);
        let mut child = child;
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), b"in").unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(4));
        assert_eq!(output.stdout, b"in");
        assert_eq!(output.stderr, b"err\n");
        // The group was stopped and the leader reaped once before the status returned.
        assert_eq!(kill(leader, None), Err(Errno::ESRCH));
    }

    #[test]
    fn a_leader_reaped_outside_the_guard_is_never_signalled_through() {
        let mut lost = Command::new("/bin/sleep").arg("300").spawn_owned().unwrap();
        // A bypass of the wrapper reaps the leader: its numeric group may be reused.
        let child = lost.child.as_mut().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        let mut calls = Vec::new();
        let error = lost
            .finish_group_with(|_, signal| {
                calls.push(signal);
                Ok(())
            })
            .unwrap_err();
        assert!(calls.is_empty(), "{calls:?}");
        assert!(error.contains("not signalled"), "{error}");
        // The guard reported uncertainty; nothing remains for its own Drop to signal.
        lost.group = None;
    }

    #[test]
    fn a_denied_probe_after_the_reap_stays_uncertain() {
        let mut child = Command::new("/bin/sleep").arg("300").spawn_owned().unwrap();
        let leader = Pid::from_raw(child.id() as i32);
        let mut calls = Vec::new();
        let error = child
            .finish_group_with(|group, signal| {
                calls.push((signal, unreaped(leader)));
                match signal {
                    Some(signal) => killpg(group, signal),
                    // EPERM with a reaped leader no longer claims the group is gone.
                    None => Err(Errno::EPERM),
                }
            })
            .unwrap_err();
        assert!(error.contains("absence unconfirmed"), "{error}");
        assert!(error.contains("EPERM"), "{error}");
        assert_no_signal_after_reap(&calls);
        assert_eq!(kill(leader, None), Err(Errno::ESRCH));
        // The real group is gone; release the guard without a second cleanup.
        child.group = None;
    }

    #[test]
    fn a_denied_live_group_fails_at_the_deadline_and_the_guard_still_rescues_it() {
        let mut live = Command::new("/bin/sleep").arg("300").spawn_owned().unwrap();
        let began = Instant::now();
        let mut signals = Vec::new();
        let denied = live
            .finish_group_with(|_, signal| {
                signals.push(signal);
                Err(Errno::EPERM)
            })
            .unwrap_err();
        assert!(began.elapsed() >= WAIT * 2);
        assert_eq!(signals, [Some(Signal::SIGTERM), Some(Signal::SIGKILL)]);
        assert!(
            denied.contains(&format!("leader {} survived SIGKILL", live.id())),
            "{denied}"
        );
        assert!(denied.contains("SIGTERM: EPERM"), "{denied}");
        assert_eq!(live.try_wait().unwrap(), None);
        // The real guard still owns and independently cleans the denied live child.
        drop(live);
    }

    #[test]
    fn root_matching_requires_a_complete_path_component() {
        let roots = [PathBuf::from("/tmp/fixture-1")];
        assert!(in_arguments("/tmp/fixture-1/bin/serve --worker", &roots));
        assert!(in_arguments("serve --root=/tmp/fixture-1", &roots));
        assert!(!in_arguments("/tmp/fixture-10/bin/serve", &roots));
        assert!(!in_arguments("/other/tmp/fixture-1/bin/serve", &roots));
        assert!(!below("/tmp/fixture-10", &roots));
    }
}
