//! Opening a link in the person's browser, when that makes sense. Detection is a pure function of
//! the flags, the setting and the environment, so it is table-tested; only `launch` has effects.
use std::{
    ffi::OsStr,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// How long a failing opener may take to report before it is trusted to have worked.
const OPENER_WAIT: Duration = Duration::from_secs(3);

/// What the person asked for on this command line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flag {
    Open,
    NoOpen,
    Unset,
}
impl Flag {
    pub fn of(args: &clap::ArgMatches) -> Self {
        match (args.get_flag("open"), args.get_flag("no-open")) {
            (_, true) => Self::NoOpen,
            (true, false) => Self::Open,
            _ => Self::Unset,
        }
    }
}

/// The parts of the environment that decide whether a browser is reachable and wanted.
#[derive(Default, Debug)]
pub struct Env {
    pub terminal: bool,
    pub ci: bool,
    pub ssh: bool,
    pub linux: bool,
    pub wsl: bool,
    /// A local display server (`DISPLAY` or `WAYLAND_DISPLAY`).
    pub display: bool,
}
impl Env {
    pub fn current() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        Self {
            terminal: tmt_cli_style::Interaction::detect(false).stdout,
            ci: set("CI"),
            ssh: set("SSH_CONNECTION") || set("SSH_CLIENT") || set("SSH_TTY"),
            linux: cfg!(target_os = "linux"),
            wsl: set("WSL_DISTRO_NAME") || set("WSL_INTEROP"),
            display: set("DISPLAY") || set("WAYLAND_DISPLAY"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Open,
    /// Print the link only, for this reason.
    Skip(&'static str),
}

/// `--no-open` and `--json` always win. `--open` overrides the setting and the terminal, CI and
/// SSH/display checks; it never overrides a missing opener (checked at launch).
pub fn decide(flag: Flag, setting: bool, json: bool, env: &Env) -> Decision {
    if flag == Flag::NoOpen {
        return Decision::Skip("--no-open");
    }
    if json {
        return Decision::Skip("--json");
    }
    if flag == Flag::Open {
        return Decision::Open;
    }
    if !setting {
        return Decision::Skip("the open setting is off");
    }
    if !env.terminal {
        return Decision::Skip("no terminal");
    }
    if env.ci {
        return Decision::Skip("CI");
    }
    // WSL reaches the Windows browser without a Linux display server.
    if env.linux && !env.wsl && !env.display {
        return Decision::Skip("no display");
    }
    // A remote shell's browser, if any, is not the person's: only a forwarded display counts.
    if env.ssh && !env.display {
        return Decision::Skip("ssh without a display");
    }
    Decision::Open
}

/// The platform opener found on `path`: macOS `open`, Linux `xdg-open`, WSL `wslview` then
/// `explorer.exe`.
pub fn find_opener(env: &Env, path: &OsStr) -> Option<PathBuf> {
    let names: &[&str] = if env.wsl {
        &["wslview", "explorer.exe"]
    } else if env.linux {
        &["xdg-open"]
    } else if cfg!(target_os = "macos") {
        &["open"]
    } else {
        &[]
    };
    names
        .iter()
        .find_map(|name| tmt_invoke::find_executable(OsStr::new(name), path))
}

/// Why nothing was opened, or what the opener said.
#[derive(Debug)]
pub enum Outcome {
    Opened,
    Skipped,
    NoOpener,
    Failed(String),
}

/// Decide, find the opener and run it. A failing opener never fails the command.
pub fn open_link(link: &str, flag: Flag, setting: bool, json: bool) -> Outcome {
    let env = Env::current();
    match decide(flag, setting, json, &env) {
        Decision::Skip(_) => Outcome::Skipped,
        Decision::Open => {
            let path = std::env::var_os("PATH").unwrap_or_default();
            match find_opener(&env, &path) {
                None => Outcome::NoOpener,
                Some(opener) => launch(&opener, link),
            }
        }
    }
}

fn launch(opener: &PathBuf, link: &str) -> Outcome {
    let mut child = match Command::new(opener)
        .arg(link)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Outcome::Failed(error.to_string()),
    };
    let deadline = Instant::now() + OPENER_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Outcome::Opened,
            Ok(Some(status)) => return Outcome::Failed(format!("{opener:?} exited with {status}")),
            // Still running: a browser launcher that stays up has handed the link over. Reap it
            // in the background so it never lingers as a zombie.
            Ok(None) if Instant::now() >= deadline => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Outcome::Opened;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Outcome::Failed(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision::*, Env, Flag, decide};
    fn env(terminal: bool, ci: bool, ssh: bool, linux: bool, display: bool) -> Env {
        Env {
            terminal,
            ci,
            ssh,
            linux,
            wsl: false,
            display,
        }
    }
    #[test]
    fn json_and_no_open_always_skip_and_open_overrides_everything_else() {
        let bad = env(false, true, true, true, false);
        assert_eq!(decide(Flag::Open, true, false, &bad), Open);
        assert_eq!(decide(Flag::Open, false, false, &bad), Open);
        assert_eq!(decide(Flag::Open, true, true, &bad), Skip("--json"));
        let good = env(true, false, false, false, true);
        assert_eq!(decide(Flag::NoOpen, true, false, &good), Skip("--no-open"));
        assert_eq!(decide(Flag::Open, false, false, &good), Open);
        assert_eq!(decide(Flag::Open, true, true, &good), Skip("--json"));
        assert_eq!(decide(Flag::Unset, true, true, &good), Skip("--json"));
    }
    #[test]
    fn the_setting_and_each_environment_check_skips_on_its_own() {
        let good = env(true, false, false, false, true);
        assert_eq!(decide(Flag::Unset, true, false, &good), Open);
        assert_eq!(
            decide(Flag::Unset, false, false, &good),
            Skip("the open setting is off")
        );
        let no_terminal = Env {
            terminal: false,
            ..env(true, false, false, false, true)
        };
        assert_eq!(
            decide(Flag::Unset, true, false, &no_terminal),
            Skip("no terminal")
        );
        let ci = Env {
            ci: true,
            ..env(true, false, false, false, true)
        };
        assert_eq!(decide(Flag::Unset, true, false, &ci), Skip("CI"));
        assert_eq!(decide(Flag::Open, true, false, &ci), Open);
        let ssh = env(true, false, true, false, false);
        assert_eq!(
            decide(Flag::Unset, true, false, &ssh),
            Skip("ssh without a display")
        );
        assert_eq!(decide(Flag::Open, true, false, &ssh), Open);
        let forwarded = Env {
            display: true,
            ..ssh
        };
        assert_eq!(decide(Flag::Unset, true, false, &forwarded), Open);
        let no_display = env(true, false, false, true, false);
        assert_eq!(
            decide(Flag::Unset, true, false, &no_display),
            Skip("no display")
        );
        assert_eq!(decide(Flag::Open, true, false, &no_display), Open);
        let wsl = Env {
            wsl: true,
            ..no_display
        };
        assert_eq!(decide(Flag::Unset, true, false, &wsl), Open);
    }
    #[test]
    fn platform_opener_discovery_and_launch_use_only_the_supplied_path() {
        use super::{Outcome, find_opener, launch};
        use std::{ffi::OsStr, os::unix::fs::PermissionsExt};
        let root = std::path::PathBuf::from(format!(
            "/tmp/tmt-open-{}",
            crate::store::uuid_v4().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("xdg-open");
        std::fs::write(&target, "fixture").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        let linux = env(true, false, false, true, true);
        assert_eq!(find_opener(&linux, root.as_os_str()), Some(target));
        assert_eq!(find_opener(&linux, OsStr::new("")), None);
        let script = root.join("link.sh");
        // Passing the script as the opener's one argument verifies launch/reaping without
        // executing a freshly published stand-in or opening a real host browser.
        std::fs::write(&script, "exit 0\n").unwrap();
        assert!(matches!(
            launch(
                &std::path::PathBuf::from("/bin/sh"),
                script.to_str().unwrap()
            ),
            Outcome::Opened
        ));
        std::fs::write(&script, "exit 9\n").unwrap();
        assert!(matches!(
            launch(
                &std::path::PathBuf::from("/bin/sh"),
                script.to_str().unwrap()
            ),
            Outcome::Failed(_)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
