//! Codex invocation evidence. Shared app-server ancestry is not a client pane.

#[cfg(test)]
mod tests;

use crate::process::{CommandRequest, CommandRunner};
use std::{
    collections::HashSet,
    ffi::OsString,
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::{
    binding::session::{HarnessId, ProviderSessionId},
    driver::{
        ActionResult, Driver,
        caller::{HostAttribution, RuntimeCaller},
    },
};

pub struct CallerEnvironment {
    pub thread_id: Option<OsString>,
    pub process_id: u32,
}

impl CallerEnvironment {
    pub fn current() -> Self {
        Self {
            thread_id: std::env::var_os("CODEX_THREAD_ID"),
            process_id: std::process::id(),
        }
    }
}

pub struct CodexCaller<'a, R> {
    runner: &'a R,
    environment: CallerEnvironment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallerObservationUnavailable;

impl std::fmt::Display for CallerObservationUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Could not establish runtime caller evidence.")
    }
}

impl std::error::Error for CallerObservationUnavailable {}

impl<'a, R: CommandRunner> CodexCaller<'a, R> {
    pub fn new(runner: &'a R, environment: CallerEnvironment) -> Self {
        Self {
            runner,
            environment,
        }
    }

    fn observe_host(&self) -> Result<Option<HostAttribution>, ()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut pid = self.environment.process_id;
        let mut seen = HashSet::new();
        let mut observed = None;
        for _ in 0..64 {
            if pid <= 1 {
                return Ok(observed);
            }
            if !seen.insert(pid) {
                return Err(());
            }
            let output = self
                .runner
                .execute(CommandRequest {
                    program: std::ffi::OsStr::new("/bin/ps"),
                    args: &[
                        "-o".into(),
                        "ppid=,comm=".into(),
                        "-p".into(),
                        pid.to_string().into(),
                    ],
                    input: &[],
                    deadline,
                    max_output_bytes: 4096,
                })
                .map_err(|_| ())?;
            let text = std::str::from_utf8(&output.stdout).map_err(|_| ())?.trim();
            if text.contains('\n') {
                return Err(());
            }
            let split = text.find(char::is_whitespace).ok_or(())?;
            let parent = text[..split].parse::<u32>().map_err(|_| ())?;
            let executable = text[split..].trim();
            if Path::new(executable)
                .file_name()
                .is_some_and(|name| name == "codex")
            {
                let args = self
                    .runner
                    .execute(CommandRequest {
                        program: std::ffi::OsStr::new("/bin/ps"),
                        args: &[
                            "-o".into(),
                            "args=".into(),
                            "-p".into(),
                            pid.to_string().into(),
                        ],
                        input: &[],
                        deadline,
                        max_output_bytes: 16384,
                    })
                    .map_err(|_| ())?;
                let args = std::str::from_utf8(&args.stdout).map_err(|_| ())?.trim();
                let tail = args
                    .strip_prefix(executable)
                    .filter(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
                    .or_else(|| {
                        args.strip_prefix("codex")
                            .filter(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
                    })
                    .or_else(|| {
                        // Linux comm is normally a basename while argv[0] may
                        // be absolute. Darwin commonly supplies the full path.
                        let split = args.find(char::is_whitespace).unwrap_or(args.len());
                        (Path::new(&args[..split]).file_name()? == "codex")
                            .then_some(&args[split..])
                    })
                    .ok_or(())?;
                // ps does not preserve argv boundaries. Conservatively fence a
                // possible app-server even when global options precede it.
                if tail.split_whitespace().any(|word| word == "app-server") {
                    return Ok(Some(HostAttribution::Ambiguous));
                }
                observed = Some(HostAttribution::Independent);
            }
            pid = parent;
        }
        Err(())
    }
}

impl<R: CommandRunner> Driver for CodexCaller<'_, R> {
    type Target = ();
    type Error = CallerObservationUnavailable;
    type Launch = ();

    fn identify_caller(&mut self) -> ActionResult<RuntimeCaller, Self::Error> {
        let host = match self.observe_host() {
            Ok(observation) => observation,
            Err(()) => return ActionResult::Failed(CallerObservationUnavailable),
        };
        if self.environment.thread_id.is_none() && host.is_none() {
            return ActionResult::Unsupported;
        }
        let session = self
            .environment
            .thread_id
            .as_ref()
            .and_then(|value| value.to_str())
            .filter(|value| uuid::Uuid::parse_str(value).is_ok())
            .and_then(|value| ProviderSessionId::new(value).ok());
        // Missing or unreadable process evidence is not permission to attribute
        // a command to the pane inherited by a shared provider host.
        let host = if self.environment.thread_id.is_some() && session.is_none() {
            HostAttribution::Ambiguous
        } else {
            // An inherited marker can outlive its runtime. Absence of a shared
            // ancestor is not positive evidence of an independent invocation.
            host.unwrap_or(HostAttribution::Ambiguous)
        };
        ActionResult::Completed(RuntimeCaller {
            harness: HarnessId::new("codex").expect("built-in driver ID"),
            session,
            host,
        })
    }
}
