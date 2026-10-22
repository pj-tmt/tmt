//! Owner browser reads of the Ops board and Digest usage. Each read runs one fixed public
//! command through the supplied `tmt` executable and answers `{version,state,...}`; the
//! contract is "Owner browser reads" in `remote-channel-v1.md`. Nothing here grants anything:
//! the route admission is the composed shell's, and no read is a generic command endpoint.
use crate::limits;
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, atomic::AtomicBool},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tmt_invoke::{Cleanup, FailureKind, InvokeError, Request};

mod ops;
#[cfg(test)]
mod tests;

/// The reports `tmt digest usage report` offers; the browser derives shorter windows itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Range {
    Day,
    Week,
    Month,
}
impl Range {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "24h" => Some(Self::Day),
            "7d" => Some(Self::Week),
            "30d" => Some(Self::Month),
            _ => None,
        }
    }
    fn text(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Target {
    Ops,
    Digest(Range),
}
impl Target {
    /// `rest` is the path after `<prefix>/x/-/`: `ops`, or `digest/24h`, `digest/7d`,
    /// `digest/30d`. The range is a path segment because the edge refuses every query string
    /// outside its two fixed forms; any other path is not a read route.
    pub(crate) fn route(rest: &str) -> Option<Self> {
        match rest {
            "ops" => Some(Self::Ops),
            _ => rest
                .strip_prefix("digest/")
                .and_then(Range::parse)
                .map(Self::Digest),
        }
    }
    fn cap(self) -> usize {
        match self {
            Self::Ops => limits::READ_OPS_BYTES,
            Self::Digest(_) => limits::READ_DIGEST_BYTES,
        }
    }
}

/// Why a read has nothing to show. The closed set `remote-channel-v1.md` names; each code is
/// one empty state in the web app, and an unavailable read is never an empty list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// The extension command is absent.
    NotInstalled,
    CoreUnavailable,
    Timeout,
    /// Non-zero exit, invalid JSON or an unexpected document version.
    ReadFailed,
    TooLarge,
}
impl Reason {
    fn code(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::CoreUnavailable => "core_unavailable",
            Self::Timeout => "timeout",
            Self::ReadFailed => "read_failed",
            Self::TooLarge => "too_large",
        }
    }
}
fn reason_of(error: InvokeError) -> Reason {
    if matches!(error.cleanup, Cleanup::Unconfirmed(_)) {
        return Reason::CoreUnavailable;
    }
    match error.kind {
        FailureKind::Deadline => Reason::Timeout,
        FailureKind::OutputLimit(_) => Reason::TooLarge,
        _ => Reason::CoreUnavailable,
    }
}

#[derive(Default)]
struct Slot {
    running: bool,
    fresh: Option<(Instant, Arc<Vec<u8>>)>,
}
/// Demand-driven and serve-local: no worker, no timer and no persisted cache. One request at
/// a time reads per key; the rest wait for its answer, and a finished answer (ok or not)
/// serves every request in the next `READ_FRESH`.
pub struct Reads {
    executable: PathBuf,
    stop: Arc<AtomicBool>,
    slots: Mutex<HashMap<Target, Slot>>,
    done: Condvar,
    wait: Duration,
}
impl Reads {
    /// `executable` is the absolute public `tmt` the door was started with.
    pub fn new(executable: PathBuf, stop: Arc<AtomicBool>) -> Self {
        Self {
            executable,
            stop,
            slots: Mutex::default(),
            done: Condvar::new(),
            wait: limits::READ_WAIT,
        }
    }
    #[cfg(test)]
    fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }
    fn lock(&self) -> MutexGuard<'_, HashMap<Target, Slot>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The JSON body to send for `read`, waiting at most `READ_WAIT` in total.
    pub(crate) fn body(&self, read: Target) -> Arc<Vec<u8>> {
        let deadline = Instant::now() + self.wait;
        let mut slots = self.lock();
        loop {
            let slot = slots.entry(read).or_default();
            if let Some((at, body)) = &slot.fresh
                && at.elapsed() < limits::READ_FRESH
            {
                return Arc::clone(body);
            }
            if !slot.running {
                slot.running = true;
                break;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Arc::new(unavailable(Reason::Timeout));
            }
            slots = self
                .done
                .wait_timeout(slots, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        drop(slots);
        let mut flight = Flight {
            reads: self,
            read,
            body: None,
        };
        let body = Arc::new(self.answer(read, deadline));
        flight.body = Some(Arc::clone(&body));
        body
    }
    fn answer(&self, read: Target, deadline: Instant) -> Vec<u8> {
        match self.fetch(read, deadline) {
            Ok(payload) => {
                let mut object = Map::new();
                object.insert("version".into(), json!(1));
                object.insert("state".into(), json!("ok"));
                object.insert("observedAtMs".into(), json!(now_ms()));
                object.extend(payload);
                let bytes = serde_json::to_vec(&Value::Object(object)).expect("JSON value");
                if bytes.len() > read.cap() {
                    return unavailable(Reason::TooLarge);
                }
                bytes
            }
            Err(reason) => unavailable(reason),
        }
    }
    fn fetch(&self, read: Target, deadline: Instant) -> Result<Map<String, Value>, Reason> {
        match read {
            Target::Ops => {
                let (board, timers) = thread::scope(|scope| {
                    let timers = scope
                        .spawn(|| self.json(&["ops", "squad", "cron", "ls", "--json"], deadline));
                    let board = self.json(&["ops", "squad", "ls", "--json"], deadline);
                    (board, timers.join())
                });
                // Timers are optional: a failed cron read leaves them null, never empty.
                let timers = timers.ok().and_then(Result::ok);
                ops::project(&board?, timers.as_ref()).ok_or(Reason::ReadFailed)
            }
            Target::Digest(range) => {
                let report = self.json(
                    &[
                        "digest",
                        "usage",
                        "report",
                        "--since",
                        range.text(),
                        "--json",
                    ],
                    deadline,
                )?;
                if report["version"] != 1 || report["since"] != range.text() {
                    return Err(Reason::ReadFailed);
                }
                let Value::Object(payload) = json!({"since": range.text(), "report": report})
                else {
                    unreachable!("object literal")
                };
                Ok(payload)
            }
        }
    }
    /// One fixed public command; exit 0 must print one JSON document.
    fn json(&self, argv: &[&str], deadline: Instant) -> Result<Value, Reason> {
        let args: Vec<OsString> = argv.iter().map(OsString::from).collect();
        let output = tmt_invoke::invoke(
            Request {
                program: &self.executable,
                args: &args,
                input: &[],
                deadline,
                max_stream_bytes: limits::READ_SUBPROCESS_BYTES,
                launch: Default::default(),
            },
            Some(&self.stop),
        )
        .map_err(reason_of)?;
        let document: Option<Value> = serde_json::from_slice(&output.stdout).ok();
        if output.status.success() {
            return document.ok_or(Reason::ReadFailed);
        }
        // A fixed argv can only be a usage error when its extension command is absent.
        Err(
            match document.as_ref().and_then(|d| d["error"]["code"].as_str()) {
                Some("USAGE_ERROR") => Reason::NotInstalled,
                _ => Reason::ReadFailed,
            },
        )
    }
}

/// Ends a read: frees the key (also when the read panics) and publishes its answer.
struct Flight<'a> {
    reads: &'a Reads,
    read: Target,
    body: Option<Arc<Vec<u8>>>,
}
impl Drop for Flight<'_> {
    fn drop(&mut self) {
        let mut slots = self.reads.lock();
        let slot = slots.entry(self.read).or_default();
        slot.running = false;
        if let Some(body) = self.body.take() {
            slot.fresh = Some((Instant::now(), body));
        }
        self.reads.done.notify_all();
    }
}

fn unavailable(reason: Reason) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "version": 1,
        "state": "unavailable",
        "observedAtMs": now_ms(),
        "reason": reason.code(),
    }))
    .expect("JSON value")
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}
