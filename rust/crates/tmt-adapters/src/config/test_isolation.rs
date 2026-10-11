//! Test builds admit one private invocation world; ordinary builds omit this module.
use super::ConfigError;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

const GUIDANCE: &str = "This executable was built with test isolation; supply the private test root through TMT_TEST_ROOT and isolated environment paths, or build without tests (cargo build --release -p tmt-cli).";

fn failure(detail: impl std::fmt::Display) -> ConfigError {
    ConfigError {
        code: "TEST_ISOLATION",
        message: format!("{GUIDANCE} {detail}"),
    }
}

pub(super) fn process_root() -> Result<PathBuf, ConfigError> {
    let result = admit();
    // An indirect ambient call must fail its unit test, even if its caller would
    // normally discard a discovery error. Library consumers use the feature.
    #[cfg(test)]
    if let Err(error) = &result {
        panic!("{error}");
    }
    result
}

fn admit() -> Result<PathBuf, ConfigError> {
    let root = env::var_os("TMT_TEST_ROOT")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| failure("No private root was admitted."))?;
    if !root.is_absolute() || root.parent().is_none() {
        return Err(failure(
            "The private root must be an absolute directory below the filesystem root.",
        ));
    }
    let metadata = fs::symlink_metadata(&root).map_err(failure)?;
    if !metadata.is_dir() {
        return Err(failure(
            "The private root must be a directory, not a symlink.",
        ));
    }
    let root = fs::canonicalize(root).map_err(failure)?;
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "CODEX_HOME",
        "TMPDIR",
        "TMUX_TMPDIR",
    ] {
        let path = env::var_os(key)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| failure(format!("Missing isolated {key}.")))?;
        check_path(&path, &root)?;
    }
    for key in [
        "TMT_HOME",
        "ZDOTDIR",
        "CLAUDE_CONFIG_DIR",
        "PI_CODING_AGENT_DIR",
        "OPENCODE_CONFIG_DIR",
    ] {
        if let Some(path) = env::var_os(key).filter(|s| !s.is_empty()) {
            check_path(Path::new(&path), &root)?;
        }
    }
    check_path(&env::current_dir().map_err(failure)?, &root)?;
    Ok(root)
}

pub(super) fn check_path(path: &Path, root: &Path) -> Result<(), ConfigError> {
    if !path.is_absolute() {
        return Err(failure(format!(
            "Expected an absolute private path: {}",
            path.display()
        )));
    }
    let canonical = existing_parent(path).map_err(failure)?;
    if !canonical.starts_with(root) {
        return Err(failure(format!(
            "Path escapes the private root: {}",
            path.display()
        )));
    }
    Ok(())
}

// Preserve filesystem traversal semantics (including symlink/.. pairs). Missing
// suffixes are permitted, but their nearest existing ancestor must be admitted.
fn existing_parent(path: &Path) -> io::Result<PathBuf> {
    let mut cursor = path;
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(cursor) {
            Ok(mut canonical) => {
                for component in suffix.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if fs::symlink_metadata(cursor).is_ok_and(|metadata| metadata.is_symlink()) {
                    return Err(io::Error::other(
                        "Private path contains an unresolved symlink",
                    ));
                }
                suffix.push(cursor.file_name().ok_or(error)?);
                cursor = cursor
                    .parent()
                    .ok_or_else(|| io::Error::other("Private path has no existing ancestor"))?;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestChild, TestDirectory};
    use std::{
        process::{Command, Stdio},
        time::Duration,
    };

    #[test]
    fn swallowed_ambient_discovery_still_fails_the_unit_test() {
        const PROBE: &str = "TMT_TEST_ISOLATION_PROBE";
        if env::var_os(PROBE).is_some() {
            // The caller would normally swallow this error. The unit-test guard
            // must fail before looking at the fake host's existing configuration.
            let _ = super::super::ConfigPaths::discover();
            return;
        }
        let directory = TestDirectory::new();
        let config = directory.path.join(".config/tmt/config.json");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "fake host sentinel").unwrap();
        let output = directory.path.join("probe-output");
        let file = fs::File::create(&output).unwrap();
        let mut child = TestChild::new(Command::new(env::current_exe().unwrap())
            .args(["--exact", "config::test_isolation::tests::swallowed_ambient_discovery_still_fails_the_unit_test", "--nocapture"])
            .env_clear()
            .env("HOME", &directory.path)
            .env(PROBE, "1")
            .env("TMT_AGENT", "1")
            .env("BROWSER", "/usr/bin/true")
            .current_dir(&directory.path)
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn().unwrap());
        let status = child.wait_for_exit(Duration::from_secs(15));
        assert!(!status.success());
        let output = fs::read_to_string(output).unwrap();
        assert!(output.contains("No private root was admitted."), "{output}");
        assert_eq!(fs::read_to_string(config).unwrap(), "fake host sentinel");
    }
}
