//! Startup ownership for one bounded request observer, not a resident service.

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};
use subprocess::{Exec, JobExt, Redirection};

/// Child entry before opening storage or starting probes. All terminal streams
/// were redirected by the parent; setsid also drops the controlling terminal.
pub fn enter() -> io::Result<()> {
    nix::unistd::setsid().map(|_| ()).map_err(Into::into)
}

pub fn ready() -> io::Result<()> {
    io::stdout().lock().write_all(b"R")?;
    io::stdout().lock().flush()
}

/// The child must call enter, initialize its bounded work, then ready. Until
/// that acknowledgment the parent retains kill/reap responsibility.
pub fn start(program: &OsStr, args: &[OsString], log: File) -> io::Result<()> {
    let mut job = Exec::cmd(program)
        .args(args)
        .stdin(Redirection::Null)
        .stdout(Redirection::Pipe)
        .stderr(log)
        .start()?;
    let started = (|| {
        let pipe = job
            .stdout
            .as_mut()
            .ok_or_else(|| io::Error::other("Missing startup pipe"))?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "Observer startup timed out")
                })?;
            let mut descriptors = [PollFd::new(pipe.as_fd(), PollFlags::POLLIN)];
            match poll(
                &mut descriptors,
                PollTimeout::try_from(remaining).unwrap_or(PollTimeout::MAX),
            ) {
                Err(nix::errno::Errno::EINTR) => continue,
                Err(error) => return Err(io::Error::from(error)),
                Ok(0) => continue,
                Ok(_) => {}
            }
            let mut byte = [0];
            pipe.read_exact(&mut byte)?;
            return if byte == *b"R" {
                Ok(())
            } else {
                Err(io::Error::other("Invalid observer acknowledgment"))
            };
        }
    })();
    if let Err(error) = started {
        // The unreaped child PID cannot be recycled. Before enter there is no
        // owned group; after enter both signals target only this worker.
        let _ = job.send_signal_group(nix::sys::signal::Signal::SIGKILL as i32);
        let _ = job.kill();
        if !matches!(job.wait_timeout(Duration::from_secs(1)), Ok(Some(_))) {
            job.detach();
            return Err(io::Error::other(format!(
                "{error}; observer cleanup unavailable"
            )));
        }
        return Err(error);
    }
    job.stdout.take();
    // Detaching never restarts the process. Its stored request deadline owns
    // its lifetime; the short-lived talk caller may now exit independently.
    job.detach();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    #[test]
    fn rejected_startup_is_reaped_before_returning_ownership() {
        let directory = TestDirectory::new();
        let pid_file = directory.path.join("worker.pid");
        let log = File::create(directory.path.join("worker.log")).unwrap();
        // The shell writes its own PID before the invalid acknowledgment.
        // exec keeps the same owned child; it does not create a loose descendant.
        let error = start(
            OsStr::new("/bin/sh"),
            &[
                "-c".into(),
                "printf '%s' \"$$\" > \"$1\"; printf X; exec sleep 30".into(),
                "observer-fixture".into(),
                pid_file.as_os_str().to_owned(),
            ],
            log,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid observer acknowledgment")
        );
        let pid = std::fs::read_to_string(pid_file)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        assert_eq!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
            Err(nix::errno::Errno::ESRCH),
            "failed startup must not return with a live or unreaped child"
        );
    }
}
