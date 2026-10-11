//! Process ownership for native Colab fixtures, including assertion unwinding.
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill, killpg},
    sys::wait::{WaitPidFlag, WaitStatus, waitpid},
    unistd::{Pid, getuid},
};
use std::{
    fs, io,
    ops::{Deref, DerefMut},
    os::unix::process::CommandExt,
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

    fn forget_finished_group(&mut self) {
        if self
            .group
            .is_some_and(|group| killpg(group, None) == Err(Errno::ESRCH))
        {
            self.group = None;
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.as_mut().unwrap().try_wait()?;
        if status.is_some() {
            self.forget_finished_group();
        }
        Ok(status)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.as_mut().unwrap().wait()?;
        self.forget_finished_group();
        Ok(status)
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

    pub fn wait_with_output(mut self) -> io::Result<Output> {
        let output = self.child.take().unwrap().wait_with_output();
        self.forget_finished_group();
        output
    }

    /// Darwin may deny a zombie-only group. Like tmt-invoke::process::confirm,
    /// denial clears only after independently observing the recorded leader's absence.
    fn group_absent(
        &mut self,
        observed: Result<(), Errno>,
        denied: &mut Option<String>,
    ) -> Result<bool, String> {
        match observed {
            Ok(()) => Ok(false),
            Err(error @ (Errno::ESRCH | Errno::EPERM)) => {
                let reap = match &mut self.child {
                    Some(child) => child.try_wait().map_err(|error| error.to_string())?,
                    None => None,
                };
                if error == Errno::ESRCH || reap.is_some() {
                    Ok(true)
                } else {
                    let leader = self.group.unwrap();
                    let probe = kill(leader, None);
                    if probe == Err(Errno::ESRCH) {
                        Ok(true)
                    } else {
                        *denied = Some(format!(
                            "group {leader}, leader {leader}, child {:?}, reap {reap:?}, leader probe {probe:?}: {error}",
                            self.child.as_ref().map(Child::id),
                        ));
                        // Unconfirmed denial is pending within the caller's existing bound.
                        Ok(false)
                    }
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn finish_group(&mut self) -> Result<(), String> {
        self.finish_group_with(killpg)
    }

    fn finish_group_with(
        &mut self,
        mut signal_group: impl FnMut(Pid, Option<Signal>) -> Result<(), Errno>,
    ) -> Result<(), String> {
        let Some(group) = self.group else {
            return Ok(());
        };
        let mut denied = None;
        if self.group_absent(signal_group(group, Some(Signal::SIGTERM)), &mut denied)? {
            self.group = None;
            return Ok(());
        }
        for signal in [Signal::SIGTERM, Signal::SIGKILL] {
            if signal == Signal::SIGKILL
                && self.group_absent(signal_group(group, Some(signal)), &mut denied)?
            {
                self.group = None;
                return Ok(());
            }
            let deadline = Instant::now() + WAIT;
            loop {
                if let Some(child) = &mut self.child {
                    child.try_wait().map_err(|error| error.to_string())?;
                }
                if self.group_absent(signal_group(group, None), &mut denied)? {
                    self.group = None;
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        let context = denied.map_or(String::new(), |context| format!("; last denial: {context}"));
        Err(format!(
            "fixture process group {group} survived SIGKILL{context}"
        ))
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

    #[test]
    fn pending_denial_finishes_only_after_the_recorded_child_is_reaped() {
        let mut child = Command::new("/bin/sleep").arg("300").spawn_owned().unwrap();
        let mut probes = 0;
        child
            .finish_group_with(|group, signal| {
                assert_ne!(signal, Some(Signal::SIGKILL));
                if signal.is_none() {
                    probes += 1;
                    if probes == 2 {
                        // The first denied probe sees a genuinely live, unreaped child.
                        // Only the next probe releases it; immediate refusal cannot pass.
                        killpg(group, Signal::SIGTERM).unwrap();
                    }
                    Err(Errno::EPERM)
                } else {
                    Ok(())
                }
            })
            .unwrap();
        assert!(probes >= 2);
        assert!(child.child.as_mut().unwrap().try_wait().unwrap().is_some());
        assert!(child.group.is_none());
    }

    #[test]
    fn live_denied_group_fails_at_the_deadline_with_recorded_leader_context() {
        let mut live = Command::new("/bin/sleep").arg("300").spawn_owned().unwrap();
        let began = Instant::now();
        let mut signals = Vec::new();
        let denied = live
            .finish_group_with(|_, signal| {
                if let Some(signal) = signal {
                    signals.push(signal);
                }
                Err(Errno::EPERM)
            })
            .unwrap_err();
        assert!(began.elapsed() >= WAIT * 2);
        assert_eq!(signals, [Signal::SIGTERM, Signal::SIGKILL]);
        assert!(denied.contains(&format!("group {}, leader {}", live.id(), live.id())));
        assert!(denied.contains(&format!("child Some({}), reap None", live.id())));
        assert!(denied.contains("leader probe Ok(())"));
        assert_eq!(live.try_wait().unwrap(), None);
        // The real guard still owns and independently cleans the denied live child.
        drop(live);
    }

    #[test]
    fn denied_group_requires_a_reaped_or_absent_leader() {
        let mut live = Command::new("/bin/sleep").arg("300").spawn_owned().unwrap();
        let mut denied = None;
        assert!(!live.group_absent(Err(Errno::EPERM), &mut denied).unwrap());
        assert_eq!(live.try_wait().unwrap(), None);
        assert!(!live.group_absent(Ok(()), &mut denied).unwrap());
        assert!(live.group_absent(Err(Errno::EIO), &mut denied).is_err());
        let pid = Pid::from_raw(live.id() as i32);
        live.kill().unwrap();
        live.child.as_mut().unwrap().wait().unwrap();
        // Keep the group recorded to exercise a denied signal or inspection after reap.
        assert!(live.group_absent(Err(Errno::EPERM), &mut denied).unwrap());
        assert_eq!(kill(pid, None), Err(Errno::ESRCH));
        // wait_with_output takes the Child handle; absence must still be observed.
        live.child = None;
        assert!(live.group_absent(Err(Errno::EPERM), &mut denied).unwrap());
        assert!(live.group_absent(Err(Errno::ESRCH), &mut denied).unwrap());
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
