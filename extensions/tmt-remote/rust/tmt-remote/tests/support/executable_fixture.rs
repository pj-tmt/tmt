//! Test-only publication probe: never retry a CoreClient or remote operation.
use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const PROBE: &str = "__tmt_fixture_ready";
const WINDOW: Duration = Duration::from_millis(500);
/// Bound on the probe's completion, not an expected wait: the probe returns as
/// soon as the fixture runs. macOS assesses each newly written executable on
/// its first exec through one system-wide queue (about 0.1 s per file, 1.5 s
/// behind 16 concurrent first execs here), so parallel suites or other builds
/// on the machine can push one probe past seconds.
pub const COMPLETION_WINDOW: Duration = Duration::from_secs(30);
const INTERVAL: Duration = Duration::from_millis(10);

pub fn write_executable(path: &Path, payload: &str) -> io::Result<()> {
    // The guard precedes cd, logging, stdin reads and every payload effect.
    // Close the only writer before probing; never reopen this file for writing.
    fs::write(
        path,
        format!("#!/bin/sh\nif [ \"$1\" = {PROBE} ]; then exit 0; fi\n{payload}\n"),
    )?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    let deadline = Instant::now() + WINDOW;
    let child = spawn_probe(deadline, || {
        Command::new(path)
            .arg(PROBE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    })?;
    finish_probe(child, Instant::now() + COMPLETION_WINDOW)
}

fn spawn_probe(
    deadline: Instant,
    mut spawn: impl FnMut() -> io::Result<Child>,
) -> io::Result<Child> {
    for attempt in 1..=50 {
        match spawn() {
            Err(error)
                if error.kind() == io::ErrorKind::ExecutableFileBusy
                    && attempt < 50
                    && Instant::now() < deadline =>
            {
                thread::sleep(INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
                if Instant::now() >= deadline {
                    return Err(error);
                }
            }
            result => return result,
        }
    }
    unreachable!("the final spawn result is returned")
}

fn finish_probe(mut child: Child, deadline: Instant) -> io::Result<()> {
    let error = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!(
                        "fixture readiness probe exited {status}"
                    )))
                };
            }
            Err(error) => break error,
            Ok(None) if Instant::now() >= deadline => {
                break io::Error::new(io::ErrorKind::TimedOut, "fixture readiness probe timed out");
            }
            Ok(None) => {
                thread::sleep(INTERVAL.min(deadline.saturating_duration_since(Instant::now())))
            }
        }
    };
    // The reserved branch cannot create children. A stalled probe itself must
    // still be killed and waited; a failed wait never silently leaks its child.
    let kill = child.kill();
    let wait = child.wait();
    if wait.is_err() {
        return Err(io::Error::new(
            error.kind(),
            format!("{error}; probe cleanup: kill={kill:?}, wait={wait:?}"),
        ));
    }
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tmt-702-probe-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn readiness_has_no_payload_effects_and_product_runs_once() {
        let root = Directory::new();
        let path = root.0.join("core");
        write_executable(
            &path,
            &format!(
                "cd '{}'\nprintf '%s\\n' \"$*\" >> calls; cat > input",
                root.0.display()
            ),
        )
        .unwrap();
        assert!(!root.0.join("calls").exists());
        assert!(!root.0.join("input").exists());
        // Explicit cwd belongs to this test invocation, not to the readiness probe.
        let output = Command::new(&path)
            .arg("api")
            .current_dir(&root.0)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(fs::read_to_string(root.0.join("calls")).unwrap(), "api\n");
        assert!(fs::read(root.0.join("input")).unwrap().is_empty());
        let saved = root.0.clone();
        drop(root);
        assert!(!saved.exists());
    }

    #[test]
    fn only_raw_pre_exec_busy_is_retried_and_exhaustion_is_bounded() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::NotFound,
            io::ErrorKind::TimedOut,
            io::ErrorKind::Other,
        ] {
            let mut calls = 0;
            let error = spawn_probe(Instant::now() + WINDOW, || {
                calls += 1;
                Err(io::Error::new(kind, "original diagnostic"))
            })
            .unwrap_err();
            assert_eq!(calls, 1);
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), "original diagnostic");
        }
        let mut calls = 0;
        let wrapped = spawn_probe(Instant::now() + WINDOW, || {
            calls += 1;
            Err(io::Error::other(io::Error::from(
                io::ErrorKind::ExecutableFileBusy,
            )))
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(wrapped.kind(), io::ErrorKind::Other);
        let started = Instant::now();
        let mut calls = 0;
        let error = spawn_probe(started + Duration::from_millis(25), || {
            calls += 1;
            Err(io::Error::new(
                io::ErrorKind::ExecutableFileBusy,
                "busy original",
            ))
        })
        .unwrap_err();
        assert!((1..=50).contains(&calls));
        assert_eq!(error.kind(), io::ErrorKind::ExecutableFileBusy);
        assert_eq!(error.to_string(), "busy original");
        assert!(started.elapsed() < Duration::from_secs(1));
        let mut calls = 0;
        let child = spawn_probe(Instant::now() + WINDOW, || {
            calls += 1;
            if calls == 1 {
                Err(io::Error::from(io::ErrorKind::ExecutableFileBusy))
            } else {
                Command::new("/bin/sh").args(["-c", "exit 0"]).spawn()
            }
        })
        .unwrap();
        finish_probe(child, Instant::now() + WINDOW).unwrap();
        assert_eq!(calls, 2);
    }

    #[test]
    fn nonzero_probe_is_not_retried_and_stalled_probe_is_reaped() {
        let child = Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        let error = finish_probe(child, Instant::now() + WINDOW).unwrap_err();
        assert!(error.to_string().contains('7'), "{error}");
        let child = Command::new("/bin/sh")
            .args(["-c", "exec sleep 30"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let started = Instant::now();
        let error = finish_probe(child, started + Duration::from_millis(20)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            !Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success(),
            "probe process leaked"
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_open_writer_reports_raw_busy_then_closed_writer_is_ready() {
        let root = Directory::new();
        let path = root.0.join("core");
        let writer = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let error = write_executable(&path, "exit 9").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ExecutableFileBusy, "{error:?}");
        assert_eq!(error.raw_os_error(), Some(26));
        drop(writer);
        // Closing the held writer is the only change: never rewrite after publication.
        let child = spawn_probe(Instant::now() + WINDOW, || {
            Command::new(&path).arg(PROBE).spawn()
        })
        .unwrap();
        finish_probe(child, Instant::now() + COMPLETION_WINDOW).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_concurrent_publication_preserves_single_payload_execution() {
        thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..16 {
                        let root = Directory::new();
                        let path = root.0.join("core");
                        write_executable(
                            &path,
                            &format!("printf x >> '{}/calls'", root.0.display()),
                        )
                        .unwrap();
                        assert!(!root.0.join("calls").exists());
                        assert!(Command::new(&path).arg("api").status().unwrap().success());
                        assert_eq!(fs::read(root.0.join("calls")).unwrap(), b"x");
                        let saved = root.0.clone();
                        drop(root);
                        assert!(!saved.exists());
                    }
                });
            }
        });
    }
}
