//! Isolation and direct-child lifetime shared by real CLI integration fixtures.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

pub fn state_dir(root: &Path) -> PathBuf {
    root.join("xdg-config/tmt")
}

pub fn command(root: &Path, args: &[&str]) -> Command {
    for directory in [
        "home",
        "xdg-config",
        "xdg-data",
        "xdg-state",
        "xdg-cache",
        "tmp",
        "tmux",
    ] {
        fs::create_dir_all(root.join(directory)).expect("create owned CLI environment directory");
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmt"));
    command
        .args(args)
        .env_clear()
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("CODEX_HOME", root.join("home/.codex"))
        .env("TMPDIR", root.join("tmp"))
        .env("TMUX_TMPDIR", root.join("tmux"))
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .current_dir(root);
    command
}

pub fn wait(child: &mut Option<Child>, timeout: Duration) -> Child {
    let deadline = Instant::now() + timeout;
    loop {
        if child.as_mut().unwrap().try_wait().unwrap().is_some() {
            return child.take().unwrap();
        }
        assert!(Instant::now() < deadline, "owned CLI did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn stop(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
        }
        child
            .wait()
            .expect("reap owned CLI before removing fixture");
    }
}
