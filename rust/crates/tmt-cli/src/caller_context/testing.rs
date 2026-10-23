//! A private composition for command tests: a caller selection that counts
//! its calls and always fails to clean up, and a private store directory.
//! No real selection read runs, and no host or user store is touched.

use super::Composition;
use crate::output::Failure;
use std::{
    cell::Cell,
    ffi::OsStr,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host, HostError},
    process::{CommandRequest, CommandRunner, UnixCommandRunner},
};

/// A private directory for one test's configuration and store, removed
/// when dropped.
pub struct Private(PathBuf);

impl Private {
    pub fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("tmt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("global")).unwrap();
        Self(root)
    }

    pub fn paths(&self) -> ConfigPaths {
        ConfigPaths {
            global_dir: self.0.join("global"),
            global_config: self.0.join("global/config.json"),
            local_config: self.0.join("local.json"),
            database: self.0.join("global/tmt.db"),
        }
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A selection read whose child could not be cleaned up. The command error
/// comes from a program that does not exist, so no process starts.
pub fn unclean() -> HostError {
    let mut error = UnixCommandRunner
        .execute(CommandRequest {
            program: OsStr::new("/nonexistent/tmt-selection-test"),
            args: &[],
            input: b"",
            deadline: Instant::now() + Duration::from_secs(1),
            max_output_bytes: 16,
        })
        .unwrap_err();
    error.cleanup_error = Some(std::io::Error::other("not reaped"));
    let error = HostError::Process(error);
    assert!(error.cleanup_failed());
    error
}

/// A caller selection that counts its calls and fails to clean up.
#[derive(Default)]
pub struct Selections(Cell<usize>);

impl Selections {
    pub fn select(&self, _environment: &CallerEnvironment) -> Result<Host, HostError> {
        self.0.set(self.0.get() + 1);
        Err(unclean())
    }

    pub fn calls(&self) -> usize {
        self.0.get()
    }
}

/// Runs `test` with a composition that selects through `selections` and
/// reads only `private`'s paths.
pub fn composed<T>(
    selections: &Selections,
    private: &Private,
    test: impl FnOnce(Composition<'_>) -> T,
) -> T {
    let select = |environment: &CallerEnvironment| selections.select(environment);
    let paths = private.paths();
    let config = || Ok::<_, Failure>(paths.clone());
    test(Composition {
        select: &select,
        config: &config,
    })
}
