//! Guided setup's selection and consent use only an owned terminal and private files.
#![cfg(unix)]
#[allow(dead_code)]
mod support;

use nix::{
    poll::{PollFd, PollFlags, poll},
    pty::{Winsize, openpty},
    sys::termios::{LocalFlags, tcgetattr},
    unistd::{read, write},
};
use std::{
    fs,
    os::fd::AsFd,
    path::PathBuf,
    process::{Child, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct World {
    root: PathBuf,
    child: Option<Child>,
}
impl World {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-setup-selection-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root, child: None }
    }
    fn unchanged(&self) {
        for path in ["home/.local", "home/.agents", "home/.claude", "home/.codex"] {
            assert!(!self.root.join(path).exists(), "setup wrote {path}");
        }
        assert!(!support::state_dir(&self.root).exists());
    }
}
impl Drop for World {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn selection_toggles_dependency_and_declining_restores_terminal_without_writes() {
    interactive(false);
}
#[test]
fn cancelling_selection_restores_terminal_and_never_asks_for_approval() {
    interactive(true);
}

fn interactive(cancel: bool) {
    let mut world = World::new();
    let pty = openpty(
        Some(&Winsize {
            ws_row: 30,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }),
        None,
    )
    .unwrap();
    let initial = tcgetattr(&pty.slave).unwrap();
    let bin = world.root.join("bin");
    fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tmt"), bin.join("tmt")).unwrap();
    let mut command = support::command(&world.root, &["setup"]);
    command
        .env("TERM", "xterm-256color")
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
        .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
        .stderr(Stdio::from(pty.slave.try_clone().unwrap()));
    world.child = Some(command.spawn().unwrap());
    drop(command);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut screen = Vec::new();
    let mut selected = false;
    let mut declined = false;
    let status = loop {
        assert!(
            Instant::now() < deadline,
            "setup did not finish: {}",
            String::from_utf8_lossy(&screen)
        );
        let mut ready = [PollFd::new(pty.master.as_fd(), PollFlags::POLLIN)];
        poll(&mut ready, 100u16).unwrap();
        if ready[0].revents().unwrap().contains(PollFlags::POLLIN) {
            let mut bytes = [0; 8192];
            let count = read(&pty.master, &mut bytes).unwrap();
            screen.extend_from_slice(&bytes[..count]);
        }
        let text = String::from_utf8_lossy(&screen);
        if !selected && text.contains("Ctrl-C cancel") {
            selected = true;
            // Space on Remote unchecks Colab. The next Down/space checks Colab and Remote.
            write(
                &pty.master,
                if cancel {
                    b"\x03"
                } else {
                    b"\x1b[B\x1b[B \x1b[B \r"
                },
            )
            .unwrap();
        }
        if !declined && text.contains("Apply all of this?") {
            assert!(!cancel);
            declined = true;
            write(&pty.master, b"n\n").unwrap();
        }
        if let Some(status) = world.child.as_mut().unwrap().try_wait().unwrap() {
            break status;
        }
    };
    assert_eq!(
        status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&screen)
    );
    world.child.take().unwrap().wait().unwrap();
    let restored = tcgetattr(&pty.slave).unwrap();
    // PENDIN is kernel input bookkeeping on Darwin, not a mode changed by setup.
    let modes = LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ISIG | LocalFlags::IEXTEN;
    assert_eq!(restored.local_flags & modes, initial.local_flags & modes);
    assert!(restored.local_flags.contains(LocalFlags::ICANON));
    assert_eq!(restored.input_flags, initial.input_flags);
    assert_eq!(restored.output_flags, initial.output_flags);
    assert_eq!(restored.control_flags, initial.control_flags);
    assert_eq!(restored.control_chars, initial.control_chars);
    let text = String::from_utf8(screen).unwrap();
    assert_eq!(
        text.matches("Apply all of this?").count(),
        usize::from(!cancel)
    );
    if !cancel {
        assert!(text.contains("Colab was unchecked because it needs Remote."));
        assert!(text.contains("Remote was checked because Colab needs it."));
    }
    world.unchanged();
}
