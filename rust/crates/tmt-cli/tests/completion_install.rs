//! Public completion setup never reaches the developer's shell files.
mod support;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-completion-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root, child: None }
    }
    fn run(&mut self, args: &[&str], shell: &str) -> Output {
        // Scripts exceed a pipe buffer; capture to owned files while the
        // bounded child waiter runs, rather than waiting before draining pipes.
        let stdout = self.root.join("stdout");
        let stderr = self.root.join("stderr");
        self.child = Some(
            support::command(&self.root, args)
                .env("SHELL", shell)
                .stdin(Stdio::null())
                .stdout(fs::File::create(&stdout).unwrap())
                .stderr(fs::File::create(&stderr).unwrap())
                .spawn()
                .unwrap(),
        );
        let status = support::wait(&mut self.child, Duration::from_secs(15))
            .wait()
            .unwrap();
        Output {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn scripts_are_byte_identical_and_hidden_from_generated_help() {
    let mut f = Fixture::new();
    for shell in ["bash", "zsh", "fish"] {
        let legacy = f.run(&["completion", shell], "/bin/unsupported");
        let hidden = f.run(&["__completion-script", shell], "/bin/unsupported");
        assert!(legacy.status.success());
        assert!(hidden.status.success());
        assert!(legacy.stderr.is_empty());
        assert!(hidden.stderr.is_empty());
        assert_eq!(legacy.stdout, hidden.stdout);
        assert!(
            !String::from_utf8(legacy.stdout)
                .unwrap()
                .contains("__completion-script")
        );
    }
    let help = f.run(&["--help"], "/bin/bash");
    assert!(
        !String::from_utf8(help.stdout)
            .unwrap()
            .contains("__completion-script")
    );
    assert!(!support::state_dir(&f.root).exists());
}

#[test]
fn json_detection_override_install_and_idempotence_preserve_startup_files() {
    let mut f = Fixture::new();
    for (shell, path) in [
        ("bash", "home/.bashrc"),
        ("zsh", "home/.zshrc"),
        ("fish", "xdg-config/fish/config.fish"),
    ] {
        let check = f.run(&["completion", "--json"], &format!("/bin/{shell}"));
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        let check: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
        assert_eq!(check["shell"], shell);
        assert_eq!(check["installed"], false);
        assert_eq!(check["evidence"], "startup-file");
        let file = f.root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let original = if shell == "zsh" {
            "# custom\ncompinit\n"
        } else {
            "# custom\n"
        };
        fs::write(&file, original).unwrap();
        let refused = f.run(&["completion", shell, "--install"], "/bin/unsupported");
        assert!(!refused.status.success());
        assert_eq!(fs::read_to_string(&file).unwrap(), original);
        assert!(String::from_utf8_lossy(&refused.stderr).contains("consent"));
        let install = f.run(
            &["completion", shell, "--install", "--yes", "--json"],
            "/bin/unsupported",
        );
        assert!(
            install.status.success(),
            "{}",
            String::from_utf8_lossy(&install.stderr)
        );
        let installed: serde_json::Value = serde_json::from_slice(&install.stdout).unwrap();
        assert_eq!(installed["installed"], true);
        assert_eq!(installed["changed"], true);
        assert_eq!(installed["detectedShell"], serde_json::Value::Null);
        let bytes = fs::read(&file).unwrap();
        assert_eq!(
            bytes,
            format!("{original}{}\n", installed["line"].as_str().unwrap()).as_bytes()
        );
        let repeat = f.run(
            &["completion", shell, "--install", "--yes", "--json"],
            "/bin/bash",
        );
        assert!(repeat.status.success());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&repeat.stdout).unwrap()["changed"],
            false
        );
        assert_eq!(fs::read(file).unwrap(), bytes);
    }
}

#[test]
fn no_argument_is_a_guide_even_when_piped_and_bad_shells_refuse() {
    let mut f = Fixture::new();
    let guide = f.run(&["completion"], "/bin/bash");
    assert!(guide.status.success());
    let text = String::from_utf8(guide.stdout).unwrap();
    assert!(text.contains("startup-file evidence"));
    assert!(text.contains("source <("));
    assert!(!text.contains("_tmt_static"));
    for args in [
        vec!["completion"],
        vec!["completion", "bad"],
        vec!["completion", "--yes"],
        vec!["completion", "--script"],
        vec!["__completion-script", "bash", "--json"],
    ] {
        assert!(
            !f.run(&args, "/bin/unsupported").status.success(),
            "{args:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn real_terminal_shows_guide_and_consent_decline_keeps_file_unchanged() {
    use nix::{
        poll::{PollFd, PollFlags, poll},
        pty::openpty,
        unistd::{read, write},
    };
    use std::{os::fd::AsFd, time::Instant};
    let mut f = Fixture::new();
    // Initialize only this fixture's environment, then preserve a real rc file.
    let mut command = support::command(&f.root, &["completion", "bash", "--install"]);
    let rc = f.root.join("home/.bashrc");
    fs::write(&rc, "# user's existing content\n").unwrap();
    let pty = openpty(None, None).unwrap();
    command
        .env("SHELL", "/bin/zsh")
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
        .stdout(Stdio::from(pty.slave.try_clone().unwrap()))
        .stderr(Stdio::from(pty.slave));
    f.child = Some(command.spawn().unwrap());
    drop(command);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut screen = Vec::new();
    while !String::from_utf8_lossy(&screen).contains("[y/N]") {
        assert!(
            Instant::now() < deadline,
            "missing consent prompt: {}",
            String::from_utf8_lossy(&screen)
        );
        let mut ready = [PollFd::new(pty.master.as_fd(), PollFlags::POLLIN)];
        assert!(poll(&mut ready, 1000u16).unwrap() >= 0);
        if ready[0].revents().unwrap().contains(PollFlags::POLLIN) {
            let mut bytes = [0u8; 4096];
            let count = read(&pty.master, &mut bytes).unwrap();
            assert!(count > 0);
            screen.extend_from_slice(&bytes[..count]);
        }
    }
    let screen = String::from_utf8(screen).unwrap();
    assert!(screen.contains("startup-file evidence"));
    assert!(screen.contains(rc.to_str().unwrap()));
    assert!(screen.contains("source <(tmt __completion-script bash)"));
    assert!(!screen.contains("_tmt_static"));
    write(&pty.master, b"n\n").unwrap();
    assert!(
        support::wait(&mut f.child, Duration::from_secs(10))
            .wait()
            .unwrap()
            .success()
    );
    assert_eq!(
        fs::read_to_string(rc).unwrap(),
        "# user's existing content\n"
    );
}
