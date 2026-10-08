//! Opt-in diagnostics for acquisition and successfully painted board milestones.
//! Stable record names and milestone semantics live in the Ops refresh guide.

use serde_json::json;
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const ENV: &str = "TMT_OPS_TIMING_TRACE";

enum Destination {
    Off,
    Stderr,
    File(std::path::PathBuf),
}

fn destination(value: Option<&OsStr>) -> Destination {
    match value {
        None => Destination::Off,
        Some(value) if value.is_empty() || value == "0" => Destination::Off,
        Some(value) if value == "1" => Destination::Stderr,
        Some(value) => Destination::File(value.into()),
    }
}

enum Sink {
    Stderr,
    File(File),
    Disabled,
    #[cfg(test)]
    Buffer(Arc<Mutex<Vec<u8>>>),
}

impl Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::Stderr => {
                let mut stream = tmt_cli_style::stream::stderr();
                stream.write_all(bytes)?;
                stream.flush()
            }
            Self::File(file) => {
                file.write_all(bytes)?;
                file.flush()
            }
            Self::Disabled => Ok(()),
            #[cfg(test)]
            Self::Buffer(buffer) => {
                buffer.lock().unwrap().extend_from_slice(bytes);
                Ok(())
            }
        }
    }
}

struct State {
    started: Instant,
    next: AtomicU64,
    sink: Mutex<Sink>,
}

#[derive(Clone)]
pub(super) struct Trace(Arc<State>);

fn report(error: &io::Error) {
    let _ = writeln!(
        tmt_cli_style::stream::stderr(),
        "Ops timing trace disabled: {error}"
    );
}

impl Trace {
    pub fn from_env() -> Option<Self> {
        Self::open(destination(std::env::var_os(ENV).as_deref()))
    }

    fn open(destination: Destination) -> Option<Self> {
        let sink = match destination {
            Destination::Off => return None,
            Destination::Stderr => Sink::Stderr,
            Destination::File(path) => match OpenOptions::new()
                .append(true)
                .create(true)
                .mode(0o600)
                .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
                .open(path)
                .and_then(|file| {
                    if file.metadata()?.is_file() {
                        Ok(file)
                    } else {
                        Err(io::Error::other("trace destination must be a regular file"))
                    }
                }) {
                Ok(file) => Sink::File(file),
                Err(error) => {
                    report(&error);
                    return None;
                }
            },
        };
        Some(Self::new(sink))
    }

    fn new(sink: Sink) -> Self {
        Self(Arc::new(State {
            started: Instant::now(),
            next: AtomicU64::new(1),
            sink: Mutex::new(sink),
        }))
    }

    #[cfg(test)]
    pub fn buffer() -> (Self, Arc<Mutex<Vec<u8>>>) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        (Self::new(Sink::Buffer(buffer.clone())), buffer)
    }

    pub fn startup(&self) -> Load {
        Load {
            trace: self.clone(),
            id: 0,
            generation: None,
            tab: None,
            partial: false,
            pending: false,
        }
    }

    pub fn load(&self, tab: Option<&str>, generation: u64) -> Load {
        Load {
            trace: self.clone(),
            id: self.0.next.fetch_add(1, Ordering::Relaxed),
            generation: Some(generation),
            tab: tab.map(str::to_owned),
            partial: false,
            pending: false,
        }
    }
}

#[derive(Clone)]
pub(super) struct Load {
    trace: Trace,
    id: u64,
    generation: Option<u64>,
    tab: Option<String>,
    partial: bool,
    pending: bool,
}

impl Load {
    pub fn completed(&mut self, snapshot: &super::app::Snapshot, pending: bool) {
        self.tab.clone_from(&snapshot.squad);
        self.partial = snapshot
            .view
            .as_ref()
            .is_ok_and(|view| view.document["partial"] == true);
        self.pending = pending;
    }

    fn emit(&self, event: &str, stage: &str, duration: Duration, ok: bool) {
        let mut sink = self.trace.0.sink.lock().unwrap();
        if matches!(*sink, Sink::Disabled) {
            return;
        }
        let record = json!({
            "version": 1, "pid": std::process::id(), "load_id": self.id,
            "generation": self.generation, "tab": self.tab, "event": event, "stage": stage,
            "duration_us": duration.as_micros(), "elapsed_us": self.trace.0.started.elapsed().as_micros(),
            "status": if !ok { "error" } else if self.partial { "partial" } else { "ok" },
            "deferred_pending": self.pending,
        });
        let mut bytes = serde_json::to_vec(&record).expect("timing record is JSON");
        bytes.push(b'\n');
        if let Err(error) = sink.write(&bytes) {
            *sink = Sink::Disabled;
            report(&error);
        }
    }

    pub fn finish(&self, started: Instant, ok: bool) {
        self.emit("stage", "load", started.elapsed(), ok);
    }
}

pub(super) fn measure<T, E>(
    trace: Option<&Load>,
    stage: &str,
    work: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    measured(trace, stage, work, Instant::now)
}

fn measured<T, E>(
    trace: Option<&Load>,
    stage: &str,
    work: impl FnOnce() -> Result<T, E>,
    now: impl FnOnce() -> Instant,
) -> Result<T, E> {
    let Some(trace) = trace else {
        return work();
    };
    let started = now();
    let result = work();
    trace.emit("stage", stage, started.elapsed(), result.is_ok());
    result
}

pub(super) struct Ui {
    trace: Trace,
    first: bool,
    fresh: bool,
    pending: Option<Load>,
}

impl Ui {
    pub fn new(trace: Trace) -> Self {
        Self {
            trace,
            first: false,
            fresh: false,
            pending: None,
        }
    }

    pub fn snapshot(&mut self, load: Option<Load>, accepted: bool) {
        if !self.fresh {
            self.pending = if accepted { load } else { None };
        }
    }

    /// Called only after a successful draw, never on acquisition or cache adoption.
    pub fn drawn(&mut self, tab: Option<&str>, usable: bool) {
        if !self.first {
            let mut first = self.trace.startup();
            first.tab = tab.map(str::to_owned);
            first.emit("first_frame", "draw", self.trace.0.started.elapsed(), true);
            self.first = true;
        }
        if !self.fresh
            && let Some(load) = self.pending.take()
            && usable
            && load.tab.as_deref() == tab
        {
            load.emit("fresh_board", "draw", self.trace.0.started.elapsed(), true);
            self.fresh = true;
        }
    }
}

#[cfg(test)]
mod tests;
