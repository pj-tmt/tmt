//! Codex invocation evidence. Shared app-server ancestry is not a client pane.

#[cfg(test)]
mod tests;

use crate::process::{CommandRunner, ps::query_ps};
use std::{
    collections::{HashMap, HashSet},
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

    /// Reused by hooks: the process is evidence of host mode, not identity.
    /// Callers still need a fresh incarnation and independent binding proof.
    pub fn observe_host(
        &self,
    ) -> Result<Option<(HostAttribution, u32)>, CallerObservationUnavailable> {
        self.observe_host_inner(Instant::now() + Duration::from_secs(3), false)
            .map_err(|_| CallerObservationUnavailable)
    }

    /// Optional environment discovery cannot resolve runtime config overrides.
    pub(super) fn observe_direct_host(
        &self,
        deadline: Instant,
    ) -> Result<Option<(HostAttribution, u32)>, CallerObservationUnavailable> {
        self.observe_host_inner(deadline, true)
            .map_err(|_| CallerObservationUnavailable)
    }

    fn observe_host_inner(
        &self,
        deadline: Instant,
        direct: bool,
    ) -> Result<Option<(HostAttribution, u32)>, ()> {
        let output = query_ps(
            self.runner,
            &["-A".into(), "-o".into(), "pid=,ppid=,comm=".into()],
            deadline,
            4 * 1024 * 1024,
        )
        .map_err(|_| ())?;
        let text = std::str::from_utf8(&output.stdout).map_err(|_| ())?;
        let mut processes = HashMap::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let (pid, rest) = line.trim().split_once(char::is_whitespace).ok_or(())?;
            let (parent, executable) = rest
                .trim_start()
                .split_once(char::is_whitespace)
                .ok_or(())?;
            let pid = pid.parse::<u32>().map_err(|_| ())?;
            let parent = parent.parse::<u32>().map_err(|_| ())?;
            if processes.insert(pid, (parent, executable.trim())).is_some() {
                return Err(());
            }
        }
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
            let &(parent, executable) = processes.get(&pid).ok_or(())?;
            if Path::new(executable)
                .file_name()
                .is_some_and(|name| name == super::NAME)
            {
                let args = query_ps(
                    self.runner,
                    &[
                        "-o".into(),
                        "args=".into(),
                        "-p".into(),
                        pid.to_string().into(),
                    ],
                    deadline,
                    16384,
                )
                .map_err(|_| ())?;
                let args = std::str::from_utf8(&args.stdout).map_err(|_| ())?.trim();
                let tail = args
                    .strip_prefix(executable)
                    .filter(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
                    .or_else(|| {
                        args.strip_prefix(super::NAME)
                            .filter(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
                    })
                    .or_else(|| {
                        // Linux comm is normally a basename while argv[0] may
                        // be absolute. Darwin commonly supplies the full path.
                        let split = args.find(char::is_whitespace).unwrap_or(args.len());
                        (Path::new(&args[..split]).file_name()? == super::NAME)
                            .then_some(&args[split..])
                    })
                    .ok_or(())?;
                // ps does not preserve argv boundaries. Conservatively fence a
                // possible app-server even when global options precede it. A literal
                // app-server word inside an exec prompt can conservatively match too.
                if tail.split_whitespace().any(|word| word == "app-server") {
                    return Ok(Some((HostAttribution::Ambiguous, pid)));
                }
                if direct
                    && tail.split_whitespace().any(|word| {
                        matches!(word, "--config" | "--profile")
                            || word.starts_with("--config=")
                            || word.starts_with("--profile=")
                            // Short options also accept attached values. ps
                            // cannot recover quoting, so refuse rather than
                            // infer which configuration the runtime selected.
                            || word.starts_with("-c")
                            || word.starts_with("-p")
                    })
                {
                    return Err(());
                }
                observed.get_or_insert((HostAttribution::Independent, pid));
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
            Err(_) if self.environment.thread_id.is_some() => {
                return ActionResult::Failed(CallerObservationUnavailable);
            }
            Err(_) => return ActionResult::Unsupported,
        };
        let Some((host, _)) = host else {
            return ActionResult::Unsupported;
        };
        let session = self
            .environment
            .thread_id
            .as_ref()
            .and_then(|value| value.to_str())
            .filter(|value| uuid::Uuid::parse_str(value).is_ok())
            .and_then(|value| ProviderSessionId::new(value).ok());
        ActionResult::Completed(RuntimeCaller {
            harness: HarnessId::new(super::NAME).expect("built-in driver ID"),
            session,
            host,
        })
    }
}
