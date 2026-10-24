//! PATH-based extension execution. No registration, storage or shell expansion.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use crate::process::{CommandRequest, CommandRunner, UnixCommandRunner};

use tmt_core::extension_command::valid_extension_name;

fn executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn absolute(path: PathBuf) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Exact lookup does not enumerate PATH directories. The CLI owns reserved names.
pub fn resolve(name: &str, search_path: &OsStr) -> io::Result<Option<PathBuf>> {
    if !valid_extension_name(name) {
        return Ok(None);
    }
    for directory in std::env::split_paths(search_path) {
        let candidate = directory.join(format!("tmt-{name}"));
        if executable(&candidate) {
            return absolute(candidate).map(Some);
        }
    }
    Ok(None)
}

/// Used only for help/completion/suggestions. Keep reserved spellings too: the
/// grammar owner reports collisions, including hidden commands such as __complete.
pub fn discover(search_path: &OsStr) -> io::Result<BTreeMap<String, PathBuf>> {
    let mut found = BTreeMap::new();
    for directory in std::env::split_paths(search_path) {
        let directory = absolute(directory)?;
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(name) = file_name
                .to_str()
                .and_then(|name| name.strip_prefix("tmt-"))
            else {
                continue;
            };
            if !name.is_empty() && executable(&entry.path()) {
                found.entry(name.to_owned()).or_insert_with(|| entry.path());
            }
        }
    }
    Ok(found)
}

/// The selected Core callback is child-local, including for bounded extension calls.
pub(crate) fn environment(tmt: &Path) -> [(OsString, OsString); 1] {
    [("TMT_EXECUTABLE".into(), tmt.as_os_str().to_owned())]
}

/// Success replaces this process. No wrapper owns signals, TTY or exit status.
pub fn execute(executable: &Path, args: &[OsString], tmt: &Path) -> io::Error {
    Command::new(executable)
        .args(args)
        .envs(environment(tmt))
        .exec()
}

/// Optional completion v1: literal UTF-8 lines only; failure means file fallback.
pub fn complete(executable: &Path, words: &[OsString], tmt: &Path) -> Option<Vec<String>> {
    let mut args = vec!["__complete".into(), "--".into()];
    args.extend_from_slice(words);
    let output = UnixCommandRunner
        .execute_with_environment(
            CommandRequest {
                program: executable.as_os_str(),
                args: &args,
                input: b"",
                deadline: Instant::now() + Duration::from_secs(1),
                max_output_bytes: 65_536,
            },
            &environment(tmt),
        )
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let candidates: Vec<_> = text
        .split_terminator('\n')
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if candidates.is_empty() || candidates.iter().any(|value| value.contains('\0')) {
        None
    } else {
        Some(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    #[test]
    fn completion_receives_the_literal_core_callback_without_mutating_the_parent() {
        let directory = TestDirectory::new();
        let program = directory.path.join("tmt-example");
        tmt_test_support::write_executable(
            &program,
            b"#!/bin/sh\nprintf '%s\\n' \"$TMT_EXECUTABLE\"\n",
            0o700,
        )
        .unwrap();
        let inherited = std::env::var_os("TMT_EXECUTABLE");
        let tmt = directory.path.join("Core path with spaces");
        assert!(tmt.is_absolute());
        assert_eq!(
            complete(&program, &[], &tmt),
            Some(vec![tmt.to_str().unwrap().to_owned()])
        );
        assert_eq!(std::env::var_os("TMT_EXECUTABLE"), inherited);
    }

    #[test]
    fn names_are_lowercase_ascii_commands_not_paths_or_options() {
        for name in ["office", "1", "headless-tmux", "a--b"] {
            assert!(valid_extension_name(name), "{name}");
        }
        for name in [
            "",
            "-x",
            ".",
            "../x",
            "a/b",
            "Office",
            "__complete",
            "a.b",
            "a b",
        ] {
            assert!(!valid_extension_name(name), "{name}");
        }
    }

    #[test]
    fn lookup_and_discovery_keep_path_precedence_and_ignore_non_executables() {
        let root = TestDirectory::new();
        let first = root.path.join("first");
        let second = root.path.join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        for directory in [&first, &second] {
            let executable = directory.join("tmt-example");
            fs::write(&executable, "fixture").unwrap();
            fs::set_permissions(executable, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(first.join("tmt-hidden"), "not executable").unwrap();
        fs::set_permissions(first.join("tmt-hidden"), fs::Permissions::from_mode(0o600)).unwrap();
        let search = std::env::join_paths([first.clone(), second]).unwrap();
        assert_eq!(
            resolve("example", &search).unwrap(),
            Some(first.join("tmt-example"))
        );
        assert!(resolve("hidden", &search).unwrap().is_none());
        assert_eq!(
            discover(&search).unwrap(),
            BTreeMap::from([("example".into(), first.join("tmt-example"))])
        );
    }
}
