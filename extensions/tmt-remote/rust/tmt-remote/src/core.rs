//! One-shot public subprocesses; no core dependencies, storage or PATH fallback.
use crate::error::RemoteError;
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use subprocess::{Exec, ExecExt, Job, JobExt, Redirection};

pub const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const PULSE: Duration = Duration::from_millis(50);

pub struct CoreClient {
    executable: PathBuf,
}
impl CoreClient {
    pub fn discover() -> Result<Self, RemoteError> {
        let path = std::env::var_os("TMT_EXECUTABLE")
            .map(PathBuf::from)
            .ok_or_else(|| failure("Run through tmt remote; TMT_EXECUTABLE is required."))?;
        Self::at(path)
    }
    pub fn at(executable: PathBuf) -> Result<Self, RemoteError> {
        if !executable.is_absolute() {
            return Err(failure("TMT_EXECUTABLE must be absolute."));
        }
        Ok(Self { executable })
    }
    pub fn capabilities(&self, stop: &AtomicBool) -> Result<Value, RemoteError> {
        self.call(
            &["api"],
            json!({"version":1,"operation":"capabilities","input":{}})
                .to_string()
                .as_bytes(),
            stop,
            Duration::from_secs(15),
            OUTPUT_LIMIT,
        )
    }
    pub fn agents(&self, stop: &AtomicBool) -> Result<Value, RemoteError> {
        self.call(
            &["list", "--json"],
            &[],
            stop,
            Duration::from_secs(15),
            OUTPUT_LIMIT,
        )
    }
    fn call(
        &self,
        argv: &[&str],
        input: &[u8],
        stop: &AtomicBool,
        timeout: Duration,
        limit: usize,
    ) -> Result<Value, RemoteError> {
        if stop.load(Ordering::Relaxed) {
            return Err(failure("Core observation interrupted."));
        }
        let deadline = Instant::now() + timeout;
        let mut job = Exec::cmd(&self.executable)
            .args(argv)
            .stdin(input.to_vec())
            .stdout(Redirection::Pipe)
            .stderr(Redirection::Pipe)
            .setpgid()
            .start()
            .map_err(|_| failure("Could not start the supplied tmt executable."))?;
        let result = observe(&mut job, deadline, limit, stop);
        if result.is_err() {
            let _ = job.send_signal_group(9);
            if !matches!(job.wait_timeout(Duration::from_secs(1)), Ok(Some(_))) {
                job.detach();
                return Err(failure(
                    "Core cleanup could not be confirmed; outcome is unknown.",
                ));
            }
        }
        let (success, bytes) = result?;
        let document: Value =
            serde_json::from_slice(&bytes).map_err(|_| failure("Core returned invalid JSON."))?;
        if success {
            return Ok(document);
        }
        match (
            document["error"]["code"].as_str(),
            document["error"]["message"].as_str(),
        ) {
            (Some(code), Some(message)) => Err(RemoteError::new(code, message)),
            _ => Err(failure("Core failed without a structured error.")),
        }
    }
}
fn failure(message: &str) -> RemoteError {
    RemoteError::new("REMOTE_CORE_UNAVAILABLE", message)
}
fn check(deadline: Instant, stop: &AtomicBool) -> Result<Duration, RemoteError> {
    if stop.load(Ordering::Relaxed) {
        return Err(failure("Core observation interrupted; outcome is unknown."));
    }
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| failure("Core timed out; outcome is unknown."))
}
fn observe(
    job: &mut Job,
    deadline: Instant,
    limit: usize,
    stop: &AtomicBool,
) -> Result<(bool, Vec<u8>), RemoteError> {
    let mut stdout = Capped {
        bytes: Vec::new(),
        limit,
    };
    let mut stderr = Capped {
        bytes: Vec::new(),
        limit,
    };
    {
        let mut communication = job.communicate()?.limit_time(PULSE);
        loop {
            communication = communication.limit_time(check(deadline, stop)?.min(PULSE));
            match communication.read_to(&mut stdout, &mut stderr) {
                Ok(()) => break,
                Err(e) if e.kind() == io::ErrorKind::TimedOut => continue,
                Err(_) => return Err(failure("Core output could not be read within its bound.")),
            }
        }
    }
    loop {
        let left = check(deadline, stop)?;
        if let Some(status) = job.wait_timeout(left.min(PULSE))? {
            return Ok((status.success(), stdout.bytes));
        }
    }
}
struct Capped {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Capped {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("output bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
