//! The selected npm launcher and its native foreground are different processes.
use crate::{
    bounded_file,
    process::{CommandRunner, ps::query_ps, runtime::ProcessObservation},
    runtime::{evidence, lifecycle::ForegroundLaunch},
    skill_installation::ProviderEnvironment,
};
use serde::Deserialize;
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PACKAGE_LIMIT: usize = 64 * 1024;
#[derive(Deserialize)]
struct Package {
    name: String,
    bin: Bins,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Bins {
    Named { codex: String },
    Single(String),
}
#[derive(PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: i64,
    nanos: i64,
}
fn stamp(path: &Path) -> Option<Stamp> {
    let m = fs::metadata(path).ok()?;
    m.is_file().then(|| Stamp {
        device: m.dev(),
        inode: m.ino(),
        size: m.len(),
        modified: m.mtime(),
        nanos: m.mtime_nsec(),
    })
}
struct NpmShim {
    selected: PathBuf,
    script: PathBuf,
    package: PathBuf,
    bytes: Vec<u8>,
    stamp: Stamp,
}
impl NpmShim {
    fn selected(
        command: &crate::runtime::RuntimeCommand,
        environment: &ProviderEnvironment,
    ) -> Result<Option<Self>, ()> {
        let Some(selected) = environment.find_executable(&command.executable) else {
            return Ok(None);
        };
        let script = fs::canonicalize(&selected).map_err(|_| ())?;
        // A directly selected native executable keeps its existing observation.
        if script.file_name().is_none_or(|n| n != "codex.js") {
            return Ok(None);
        }
        let root = script
            .parent()
            .filter(|p| p.file_name().is_some_and(|n| n == "bin"))
            .and_then(Path::parent)
            .ok_or(())?;
        let package = root.join("package.json");
        let bytes = bounded_file::read_no_follow(&package, PACKAGE_LIMIT).map_err(|_| ())?;
        let parsed: Package = serde_json::from_slice(&bytes).map_err(|_| ())?;
        if parsed.name != "@openai/codex" {
            return Err(());
        }
        let bin = match parsed.bin {
            Bins::Named { codex } => codex,
            Bins::Single(bin) => bin,
        };
        if fs::canonicalize(root.join(bin)).map_err(|_| ())? != script {
            return Err(());
        }
        let stamp = stamp(&script).ok_or(())?;
        Ok(Some(Self {
            selected,
            script,
            package,
            bytes,
            stamp,
        }))
    }
    fn unchanged(&self) -> bool {
        fs::canonicalize(&self.selected).ok().as_ref() == Some(&self.script)
            && stamp(&self.script).as_ref() == Some(&self.stamp)
            && bounded_file::read_no_follow(&self.package, PACKAGE_LIMIT)
                .ok()
                .as_ref()
                == Some(&self.bytes)
    }
    fn matches_arguments(&self, text: &str) -> bool {
        // ps cannot recover quoted argv. Ambiguity supplies no launcher proof.
        let mut words = text.split_whitespace();
        let Some(program) = words.next() else {
            return false;
        };
        if !Path::new(program)
            .file_name()
            .is_some_and(|name| name == "node" || name == "nodejs")
        {
            return false;
        }
        let Some(script) = words.next() else {
            return false;
        };
        fs::canonicalize(script).ok().as_ref() == Some(&self.script)
    }
}

pub(super) fn observe(
    runner: &dyn CommandRunner,
    launch: ForegroundLaunch<'_>,
    child: &ProcessObservation,
    deadline: Instant,
) -> ProcessObservation {
    if Instant::now() >= deadline {
        return ProcessObservation::Unknown;
    }
    let environment = match ProviderEnvironment::capture() {
        Ok(environment) => environment,
        Err(_) => return ProcessObservation::Unknown,
    };
    let shim = match NpmShim::selected(launch.command, &environment) {
        Ok(Some(shim)) => shim,
        Ok(None) => return child.clone(),
        Err(()) => return ProcessObservation::Unknown,
    };
    observe_selected(runner, launch, child, &shim, deadline)
}

fn observe_selected(
    runner: &dyn CommandRunner,
    launch: ForegroundLaunch<'_>,
    child: &ProcessObservation,
    shim: &NpmShim,
    deadline: Instant,
) -> ProcessObservation {
    let ProcessObservation::Live(launcher) = child else {
        return ProcessObservation::Unknown;
    };
    loop {
        if Instant::now() >= deadline || !shim.unchanged() {
            return ProcessObservation::Unknown;
        }
        let command = match query_ps(
            &runner,
            &[
                "-o".into(),
                "comm=".into(),
                "-p".into(),
                launcher.pid().to_string().into(),
            ],
            deadline,
            16 * 1024,
        ) {
            Ok(out) => match String::from_utf8(out.stdout) {
                Ok(text) => text,
                Err(_) => return ProcessObservation::Unknown,
            },
            Err(_) => return ProcessObservation::Unknown,
        };
        let name = Path::new(command.trim()).file_name();
        if name.is_some_and(|n| n == "node" || n == "nodejs" || n == "node-MainThread") {
            let args = match query_ps(
                &runner,
                &[
                    "-o".into(),
                    "args=".into(),
                    "-p".into(),
                    launcher.pid().to_string().into(),
                ],
                deadline,
                32 * 1024,
            ) {
                Ok(out) => match String::from_utf8(out.stdout) {
                    Ok(text) => text,
                    Err(_) => return ProcessObservation::Unknown,
                },
                Err(_) => return ProcessObservation::Unknown,
            };
            if !shim.matches_arguments(&args) {
                return ProcessObservation::Unknown;
            }
            match evidence::observe_direct_main_child(
                &runner,
                launcher,
                launch.owner,
                launch.pane_pid,
                deadline,
                super::NAME,
            ) {
                Ok(Some(native)) => {
                    let unchanged_args = query_ps(
                        &runner,
                        &[
                            "-o".into(),
                            "args=".into(),
                            "-p".into(),
                            launcher.pid().to_string().into(),
                        ],
                        deadline,
                        32 * 1024,
                    )
                    .ok()
                    .and_then(|out| String::from_utf8(out.stdout).ok())
                    .is_some_and(|args| shim.matches_arguments(&args));
                    if unchanged_args && shim.unchanged() && Instant::now() < deadline {
                        return ProcessObservation::Live(native);
                    }
                    return ProcessObservation::Unknown;
                }
                Ok(None) => (),
                _ => return ProcessObservation::Unknown,
            }
        } else if name.is_none_or(|n| n != "env") {
            return ProcessObservation::Unknown;
        }
        // Observation readiness, never a launch or delivery retry. Share the
        // caller's existing monotonic deadline, without opening a writer.
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return ProcessObservation::Unknown;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{runtime::RuntimeCommand, test_support::TestDirectory};
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn shim(root: &Path) -> (RuntimeCommand, ProviderEnvironment) {
        let package = root.join("node_modules/@openai/codex");
        fs::create_dir_all(package.join("bin")).unwrap();
        let script = package.join("bin/codex.js");
        fs::write(&script, "#!/usr/bin/env node\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            package.join("package.json"),
            br#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
        )
        .unwrap();
        fs::create_dir(root.join("bin")).unwrap();
        symlink(&script, root.join("bin/codex")).unwrap();
        (
            RuntimeCommand {
                executable: "codex".into(),
                args: vec![],
            },
            ProviderEnvironment::from_parts(root, root, vec![root.join("bin")], []),
        )
    }

    #[test]
    fn selected_package_and_running_script_must_agree() {
        let directory = TestDirectory::new();
        let (command, environment) = shim(&directory.path);
        let selected = NpmShim::selected(&command, &environment).unwrap().unwrap();
        let args = format!("/usr/bin/node {} --no-daemon", selected.script.display());
        assert!(selected.matches_arguments(&args));
        assert!(!selected.matches_arguments("node /other/codex.js"));
        assert!(!selected.matches_arguments(&format!("arbitrary {}", selected.script.display())));
        assert!(selected.unchanged());
        fs::write(
            &selected.package,
            br#"{"name":"other","bin":{"codex":"bin/codex.js"}}"#,
        )
        .unwrap();
        assert!(!selected.unchanged());
        assert!(NpmShim::selected(&command, &environment).is_err());
    }

    #[test]
    fn direct_native_is_unchanged_but_an_unproved_npm_shape_is_not_native() {
        let directory = TestDirectory::new();
        let (command, environment) = shim(&directory.path);
        let selected = environment.find_executable(&command.executable).unwrap();
        fs::remove_file(&selected).unwrap();
        fs::write(&selected, "native fixture").unwrap();
        fs::set_permissions(&selected, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(NpmShim::selected(&command, &environment).unwrap().is_none());
        let script = directory
            .path
            .join("node_modules/@openai/codex/bin/codex.js");
        fs::remove_file(&selected).unwrap();
        symlink(script, &selected).unwrap();
        fs::write(
            directory
                .path
                .join("node_modules/@openai/codex/package.json"),
            br#"{"name":"@openai/codex","bin":{"codex":"wrong.js"}}"#,
        )
        .unwrap();
        assert!(NpmShim::selected(&command, &environment).is_err());
    }
}
