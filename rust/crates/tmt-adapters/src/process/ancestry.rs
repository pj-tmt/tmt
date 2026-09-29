//! A process's parent chain from bounded `ps` calls. A host proves that a
//! caller runs inside one of its panes by finding that pane's process in this
//! chain; the nearest such pane is the caller's own.

use super::{CommandError, CommandFailure, CommandRequest, CommandRunner};
use std::{ffi::OsString, time::Instant};
use tmt_core::limits::MAX_JS_SAFE_INTEGER;

const MAX_DEPTH: usize = 64;
const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug)]
pub enum AncestryError {
    Command(CommandError),
    /// The chain was malformed, cyclic, too deep or out of time.
    Unavailable,
}

/// `first` and its ancestors, nearest first, ending before PID 0.
pub fn chain<R: CommandRunner>(
    runner: &R,
    first: u64,
    deadline: Instant,
) -> Result<Vec<u64>, AncestryError> {
    let mut chain = Vec::new();
    let mut pid = first;
    for _ in 0..MAX_DEPTH {
        if pid == 0 || pid > MAX_JS_SAFE_INTEGER || chain.contains(&pid) {
            return Err(AncestryError::Unavailable);
        }
        chain.push(pid);
        if Instant::now() >= deadline {
            return Err(AncestryError::Command(CommandError::new(
                CommandFailure::Timeout,
            )));
        }
        let args: Vec<OsString> = ["-o", "pid=,ppid=", "-p", &pid.to_string()]
            .into_iter()
            .map(Into::into)
            .collect();
        let output = runner
            .execute(CommandRequest {
                program: "ps".as_ref(),
                args: &args,
                input: &[],
                deadline,
                max_output_bytes: MAX_OUTPUT,
            })
            .map_err(AncestryError::Command)?;
        let text = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<_> = text.trim().lines().collect();
        let [line] = lines.as_slice() else {
            return Err(AncestryError::Unavailable);
        };
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2 || decimal(fields[0]) != Some(pid) {
            return Err(AncestryError::Unavailable);
        }
        let parent = decimal(fields[1]).ok_or(AncestryError::Unavailable)?;
        if parent == 0 {
            return Ok(chain);
        }
        pid = parent;
    }
    Err(AncestryError::Unavailable)
}

/// `ps` prints decimal integers; signs, hex and exponents are not evidence.
fn decimal(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse()
        .ok()
        .filter(|value| tmt_core::limits::is_valid_js_safe_integer(*value))
}
