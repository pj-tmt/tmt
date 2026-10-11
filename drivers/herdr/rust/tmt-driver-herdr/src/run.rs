//! The processes the driver runs, `herdr` and `ps`, each bounded by the
//! request's deadline through `tmt-invoke`.

use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};

/// What a finished process printed. A nonzero exit is data, not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub code: Option<u32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The process could not run, or did not finish in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunError {
    /// It never started (for example, `herdr` is not installed).
    NotStarted,
    /// It started and did not finish, or printed too much.
    Unfinished,
}

/// Runs one program; tests substitute their own.
pub trait Runner {
    /// `program` with `args`, and `env` set on top of the passed variables.
    fn run(
        &self,
        program: &str,
        env: &[(&str, &str)],
        args: &[String],
        deadline: Instant,
    ) -> Result<Output, RunError>;
}

/// The variables a child may see. `TMT_DRIVER_CALL` belongs to this call
/// alone (the contract): nothing the driver starts inherits it.
const PASSED_ENV: [&str; 15] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMPDIR",
    "TERM",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "HERDR_SOCKET_PATH",
];

const MAX_OUTPUT: usize = 1024 * 1024;

/// The real processes, through `/usr/bin/env` so `herdr` is found on `PATH`
/// and a socket can be named as the user's own `herdr` command would.
pub struct Processes;

impl Runner for Processes {
    fn run(
        &self,
        program: &str,
        env: &[(&str, &str)],
        args: &[String],
        deadline: Instant,
    ) -> Result<Output, RunError> {
        let allowed: Vec<OsString> = PASSED_ENV.iter().map(OsString::from).collect();
        let mut argv: Vec<OsString> = env
            .iter()
            .map(|(name, value)| format!("{name}={value}").into())
            .collect();
        argv.push(program.into());
        argv.extend(args.iter().map(OsString::from));
        let output = tmt_invoke::invoke(
            tmt_invoke::Request {
                program: Path::new("/usr/bin/env"),
                args: &argv,
                input: &[],
                deadline,
                max_stream_bytes: MAX_OUTPUT,
                launch: tmt_invoke::LaunchOptions {
                    environment: tmt_invoke::EnvironmentPolicy::ClearAllowlist(&allowed),
                    ..Default::default()
                },
            },
            None,
        )
        .map_err(|error| match error.kind {
            tmt_invoke::FailureKind::Spawn => RunError::NotStarted,
            _ => RunError::Unfinished,
        })?;
        Ok(Output {
            // `env` exits 127 when the program itself is missing.
            code: output
                .status
                .code
                .filter(|_| output.status.signal.is_none()),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// The deadline a request leaves the driver, kept a little short of core's
/// own so the answer arrives before core stops waiting.
pub fn deadline(deadline_ms: u64) -> Instant {
    let margin = Duration::from_millis(20);
    Instant::now() + Duration::from_millis(deadline_ms).saturating_sub(margin)
}
