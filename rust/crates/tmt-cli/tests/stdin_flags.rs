//! Real CLI cancellation must not change its launcher's shared stdin description.
#![cfg(unix)]

use nix::{
    errno::Errno,
    fcntl::{FcntlArg, OFlag, fcntl},
    poll::{PollFd, PollFlags, poll},
    sys::signal::{Signal, kill},
    unistd::{Pid, pipe, write},
};
use std::{
    fs,
    os::{fd::AsFd, unix::process::ExitStatusExt},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    home: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new(signal: Signal) -> Self {
        let home =
            std::env::temp_dir().join(format!("tmt-stdin-flags-{}-{signal}", std::process::id()));
        fs::create_dir(&home).expect("create owned disposable home");
        Self { home, child: None }
    }

    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                self.child = None;
                return status;
            }
            assert!(Instant::now() < deadline, "signalled CLI did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child
            && !matches!(child.try_wait(), Ok(Some(_)))
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        fs::remove_dir_all(&self.home).expect("remove owned disposable home");
    }
}

fn shared_pipe_after_signal(signal: Signal) {
    let mut fixture = Fixture::new(signal);
    let (reader, writer) = pipe().unwrap();
    let original = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
    assert_eq!(
        OFlag::from_bits_retain(original) & OFlag::O_NONBLOCK,
        OFlag::empty()
    );

    // Fill only our writer nonblocking. Its becoming writable proves the CLI
    // consumed input; the held-open writer prevents EOF and any storage access.
    let writer_flags = OFlag::from_bits_retain(fcntl(&writer, FcntlArg::F_GETFL).unwrap());
    fcntl(&writer, FcntlArg::F_SETFL(writer_flags | OFlag::O_NONBLOCK)).unwrap();
    let mut filled = 0;
    loop {
        match write(&writer, &[b'x'; 8192]) {
            Ok(count) => filled += count,
            Err(Errno::EAGAIN) => break,
            other => panic!("fill owned pipe: {other:?}"),
        }
        assert!(
            filled < 1_048_576,
            "pipe capacity exceeds input fixture bound"
        );
    }
    assert!(filled > 0);
    fcntl(&writer, FcntlArg::F_SETFL(writer_flags)).unwrap();
    let mut readiness = [PollFd::new(writer.as_fd(), PollFlags::POLLOUT)];
    assert_eq!(poll(&mut readiness, 0u16).unwrap(), 0);

    let child_reader = reader.try_clone().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmt"));
    command
        .args(["api"])
        .stdin(Stdio::from(child_reader))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir(&fixture.home)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("TMT_DRIVER_CALL");
    for variable in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "TMT_GLOBAL_DIR",
    ] {
        command.env(variable, &fixture.home);
    }
    fixture.child = Some(command.spawn().unwrap());
    drop(command);
    assert_eq!(
        poll(&mut readiness, 3000u16).unwrap(),
        1,
        "CLI did not consume input"
    );
    assert!(readiness[0].revents().unwrap().contains(PollFlags::POLLOUT));
    assert!(
        fixture
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );
    let during = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
    let pid = Pid::from_raw(i32::try_from(fixture.child.as_ref().unwrap().id()).unwrap());
    kill(pid, signal).unwrap();
    let status = fixture.wait();
    let after = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
    // Restore a broken baseline as well, so even the red test has no fd side effect.
    fcntl(
        &reader,
        FcntlArg::F_SETFL(OFlag::from_bits_retain(original)),
    )
    .unwrap();
    assert_eq!(status.signal(), Some(signal as i32));
    assert_eq!(
        after, original,
        "signal termination changed parent-shared stdin"
    );
    assert_eq!(during, original, "acquisition changed parent-shared stdin");
    assert_eq!(
        fs::read_dir(&fixture.home).unwrap().count(),
        0,
        "input opened storage"
    );
}

#[test]
fn sigterm_preserves_parent_shared_stdin_flags() {
    shared_pipe_after_signal(Signal::SIGTERM);
}

#[test]
fn sigint_preserves_parent_shared_stdin_flags() {
    shared_pipe_after_signal(Signal::SIGINT);
}
