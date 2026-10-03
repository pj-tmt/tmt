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
fn only_hidden_route_generates_scripts_and_public_route_always_guides() {
    let mut f = Fixture::new();
    for shell in ["bash", "zsh", "fish"] {
        let guide = f.run(&["completion", shell], "/bin/unsupported");
        let hidden = f.run(&["__completion-script", shell], "/bin/unsupported");
        assert!(guide.status.success());
        assert!(hidden.status.success());
        assert!(guide.stderr.is_empty());
        assert!(hidden.stderr.is_empty());
        let guide = String::from_utf8(guide.stdout).unwrap();
        assert!(guide.contains("startup-file evidence"));
        assert!(!guide.contains("_tmt_static"));
        assert!(String::from_utf8_lossy(&hidden.stdout).contains("_tmt_static"));
        assert!(
            !String::from_utf8(hidden.stdout)
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
fn json_detection_and_override_check_files_without_changing_them() {
    let mut f = Fixture::new();
    for (shell, path, line) in [
        (
            "bash",
            "home/.bashrc",
            "source <(tmt __completion-script bash)",
        ),
        (
            "zsh",
            "home/.zshrc",
            "source <(tmt __completion-script zsh)",
        ),
        (
            "fish",
            "xdg-config/fish/config.fish",
            "tmt __completion-script fish | source",
        ),
    ] {
        let check = f.run(&["completion", "--json"], &format!("/bin/{shell}"));
        assert!(check.status.success());
        let check: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
        assert_eq!(check["shell"], shell);
        assert_eq!(check["installed"], false);
        assert_eq!(check["evidence"], "startup-file");
        let file = f.root.join(path);
        assert!(!file.exists(), "inspection must not create a startup file");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let original = format!("# custom\nsource \"$ZSH/oh-my-zsh.sh\"\n{line}\n");
        fs::write(&file, &original).unwrap();
        let check = f.run(&["completion", shell, "--json"], "/bin/unsupported");
        assert!(check.status.success());
        let check: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
        assert_eq!(check["installed"], true);
        assert_eq!(check["detectedShell"], serde_json::Value::Null);
        assert_eq!(check["line"], line);
        assert_eq!(check["file"], file.to_str().unwrap());
        assert_eq!(fs::read_to_string(file).unwrap(), original);
    }
    assert!(!support::state_dir(&f.root).exists());
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
        vec!["completion", "--install"],
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
impl Fixture {
    fn terminal_help(&mut self, args: &[&str], shell: &str) -> String {
        use nix::{
            poll::{PollFd, PollFlags, poll},
            pty::openpty,
            unistd::read,
        };
        use std::{os::fd::AsFd, time::Instant};
        let pty = openpty(None, None).unwrap();
        let mut command = support::command(&self.root, args);
        command
            .env("SHELL", shell)
            .env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::from(pty.slave))
            .stderr(Stdio::null());
        self.child = Some(command.spawn().unwrap());
        drop(command);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut screen = Vec::new();
        loop {
            assert!(Instant::now() < deadline, "help did not finish");
            let mut ready = [PollFd::new(pty.master.as_fd(), PollFlags::POLLIN)];
            poll(&mut ready, 100u16).unwrap();
            let events = ready[0].revents().unwrap();
            if events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP) {
                let mut bytes = [0u8; 4096];
                match read(&pty.master, &mut bytes) {
                    Ok(0) | Err(nix::errno::Errno::EIO) => break,
                    Ok(count) => screen.extend_from_slice(&bytes[..count]),
                    Err(error) => panic!("PTY read failed: {error}"),
                }
            }
        }
        assert!(
            support::wait(&mut self.child, Duration::from_secs(5))
                .wait()
                .unwrap()
                .success()
        );
        String::from_utf8(screen).unwrap().replace("\r\n", "\n")
    }
}

#[cfg(unix)]
#[test]
fn top_level_terminal_help_hints_only_when_not_configured() {
    const TIP: &str =
        "Tip: shell completion is not set up; run tmt completion to see the line to add.";
    let mut f = Fixture::new();
    for args in [vec![], vec!["help"], vec!["--help"]] {
        let text = f.terminal_help(&args, "/bin/zsh");
        assert!(text.trim_end().ends_with(TIP));
        assert_eq!(text.matches(TIP).count(), 1);
        assert!(!f.root.join("home/.zshrc").exists());
        let pipe = f.run(&args, "/bin/zsh");
        assert!(pipe.status.success());
        assert!(!String::from_utf8_lossy(&pipe.stdout).contains(TIP));
    }
    let file = f.root.join("home/.zshrc");
    let configured = "source \"$ZSH/oh-my-zsh.sh\"\nsource <(tmt __completion-script zsh)\n";
    fs::write(&file, configured).unwrap();
    for args in [vec![], vec!["help"], vec!["--help"]] {
        assert!(!f.terminal_help(&args, "/bin/zsh").contains(TIP));
    }
    assert_eq!(fs::read_to_string(&file).unwrap(), configured);
    fs::write(&file, [0xff]).unwrap();
    assert!(!f.terminal_help(&["--help"], "/bin/zsh").contains(TIP));
    assert_eq!(fs::read(&file).unwrap(), [0xff]);
    fs::remove_file(file).unwrap();
    assert!(
        !f.terminal_help(&["help", "completion"], "/bin/zsh")
            .contains(TIP)
    );
    assert!(
        !f.terminal_help(&["--help"], "/bin/unsupported")
            .contains(TIP)
    );
    let json = f.run(&["--help", "--json"], "/bin/zsh");
    assert!(!json.status.success());
    let _: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(!String::from_utf8_lossy(&json.stdout).contains(TIP));
    assert!(!String::from_utf8_lossy(&json.stderr).contains(TIP));
    assert!(!support::state_dir(&f.root).exists());
}
