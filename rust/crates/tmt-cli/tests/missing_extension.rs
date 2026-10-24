//! Missing official commands never acquire or install for automated callers.
#![cfg(unix)]

#[allow(dead_code)]
mod support;

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct World(PathBuf);
impl World {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-missing-extension-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn unchanged(&self) {
        assert!(!self.0.join("home/.local").exists());
        assert!(!support::state_dir(&self.0).exists());
        assert!(!self.0.join("home/.agents").exists());
    }
}
impl Drop for World {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn every_official_missing_command_returns_one_json_failure_and_an_exact_next_step() {
    let world = World::new();
    for name in ["ops", "remote", "colab", "digest"] {
        for args in [
            vec![name, "--json"],
            vec!["--json", name],
            vec![name, "sq", "ls", "--json"],
        ] {
            let output = support::command(&world.0, &args).output().unwrap();
            assert_eq!(output.status.code(), Some(3), "{args:?}: {output:?}");
            assert!(output.stderr.is_empty());
            let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(document["error"]["code"], "EXTENSION_NOT_INSTALLED");
            assert_eq!(
                document["error"]["suggestion"],
                format!("tmt extension install {name} --yes")
            );
            assert!(
                document["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("\"{name}\""))
            );
            world.unchanged();
        }
    }
}

#[test]
fn a_pipe_is_plain_even_without_an_agent_marker_and_unknown_names_stay_unknown() {
    let world = World::new();
    for args in [
        vec!["ops"],
        vec!["ops", "--", "--json"],
        vec!["--timeout=--json", "ops"],
    ] {
        let output = support::command(&world.0, &args)
            .env_remove("TMT_AGENT")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty());
        let text = String::from_utf8(output.stderr).unwrap();
        assert!(text.contains("tmt extension install ops --yes"));
        assert!(!text.contains("Install it now"));
        world.unchanged();
    }
    let output = support::command(&world.0, &["opss", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["error"]["code"], "USAGE_ERROR");
    world.unchanged();
}

#[test]
fn help_is_informational_and_completion_never_adds_absent_official_commands() {
    let world = World::new();
    let output = support::command(&world.0, &["help"]).output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Extensions:"));
    for name in ["ops", "remote", "colab", "digest"] {
        assert!(help.contains(&format!(
            "not installed; tmt extension install {name} --yes"
        )));
    }
    assert!(
        !help
            .lines()
            .any(|line| line.contains("not installed") && line.contains("tmt setup"))
    );
    for args in [
        vec!["help", "ops"],
        vec!["ops", "--help"],
        vec!["ops", "-h"],
    ] {
        let output = support::command(&world.0, &args).output().unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("Install it now")
        );
    }
    world.unchanged();
}

#[test]
fn an_agent_marker_suppresses_the_offer_even_with_terminal_streams() {
    use nix::{
        poll::{PollFd, PollFlags, poll},
        pty::openpty,
        unistd::read,
    };
    use std::{
        os::fd::AsFd,
        process::Stdio,
        time::{Duration, Instant},
    };
    let world = World::new();
    let pty = openpty(None, None).unwrap();
    let mut command = support::command(&world.0, &["ops"]);
    command
        .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
        .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
        .stderr(Stdio::from(pty.slave));
    let mut child = Some(command.spawn().unwrap());
    drop(command);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut screen = Vec::new();
    loop {
        if Instant::now() >= deadline {
            support::stop(&mut child);
            panic!("agent was asked for input");
        }
        let mut ready = [PollFd::new(pty.master.as_fd(), PollFlags::POLLIN)];
        poll(&mut ready, 200u16).unwrap();
        if ready[0]
            .revents()
            .unwrap()
            .intersects(PollFlags::POLLIN | PollFlags::POLLHUP)
        {
            let mut bytes = [0; 4096];
            match read(&pty.master, &mut bytes) {
                Ok(0) | Err(nix::errno::Errno::EIO) => break,
                Ok(count) => screen.extend_from_slice(&bytes[..count]),
                Err(error) => {
                    support::stop(&mut child);
                    panic!("{error}");
                }
            }
        }
    }
    assert_eq!(
        support::wait(&mut child, Duration::from_secs(2))
            .wait()
            .unwrap()
            .code(),
        Some(3)
    );
    let text = String::from_utf8(screen).unwrap();
    assert!(text.contains("tmt extension install ops --yes"));
    assert!(!text.contains("Install it now"));
    world.unchanged();
}
