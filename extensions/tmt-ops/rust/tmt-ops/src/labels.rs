//! Labels a tmt extension supplies for member rows. Ops asks each configured source
//! `tmt <source> status --json` off the paint path and shows what comes back: it
//! recalculates nothing a source decided. Typed projections share that same read. A source that
//! is absent, slow, failing or malformed leaves its labels unavailable.
use crate::{core::Core, runner::Cancellation};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, RecvTimeoutError},
    time::{Duration, Instant},
};
use tmt_cli_style::Role;

mod action;
pub mod digest;

pub use action::Choose;

const VERSION: u64 = 1;
/// A source supplies at most this many labels per member.
const MAX_LABELS: usize = 3;
const MAX_TEXT_CHARS: usize = 48;

/// One chip as its source supplied it.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: String,
    pub role: Role,
    /// What choosing on the label does, when its source lets the user choose.
    pub action: Option<Choose>,
}

/// One source's labels, by identity.
pub type Rows = BTreeMap<String, Vec<Label>>;

/// The labels of every source that is currently available, in configuration order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Supplied(Vec<(String, Snapshot)>);

/// Labels and optional typed evidence from one acquisition, with one lifetime.
#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    rows: Rows,
    digest: Option<digest::Status>,
    stale: bool,
}

impl Snapshot {
    fn parse(source: &str, document: &Value) -> Option<Self> {
        let rows = parse(source, document)?;
        let digest = digest::Status::from_source(source, document, &rows);
        Some(Self {
            rows,
            digest,
            stale: false,
        })
    }
}

impl Supplied {
    #[cfg(test)]
    pub fn from_document(source: &str, document: &Value, stale: bool) -> Self {
        let Some(mut snapshot) = Snapshot::parse(source, document) else {
            return Self::default();
        };
        snapshot.stale = stale;
        Self(vec![(source.into(), snapshot)])
    }

    #[cfg(test)]
    pub fn from_rows(sources: Vec<(String, Rows)>) -> Self {
        Self(
            sources
                .into_iter()
                .map(|(source, rows)| {
                    (
                        source,
                        Snapshot {
                            rows,
                            ..Snapshot::default()
                        },
                    )
                })
                .collect(),
        )
    }

    /// An identity's labels, each with the source that supplied it.
    pub fn of<'a>(&'a self, identity: &'a str) -> impl Iterator<Item = (&'a str, &'a Label)> {
        self.0.iter().flat_map(move |(source, snapshot)| {
            snapshot
                .rows
                .get(identity)
                .into_iter()
                .flatten()
                .map(move |label| (source.as_str(), label))
        })
    }

    /// The Digest projection and whether it is retained after a failed read.
    /// Its timestamps remain the source's observations, never the retry time.
    pub fn digest(&self) -> Option<(&digest::Status, bool)> {
        self.0.iter().find_map(|(_, snapshot)| {
            snapshot
                .digest
                .as_ref()
                .map(|status| (status, snapshot.stale))
        })
    }
}

/// Reads a `status` document: `None` unless it is version 1 with a member list.
/// A member whose labels are malformed is left out, never guessed; fields this
/// version does not know are ignored.
pub fn parse(source: &str, document: &Value) -> Option<Rows> {
    if document["version"].as_u64() != Some(VERSION) {
        return None;
    }
    let mut rows = Rows::new();
    for member in document["members"].as_array()? {
        if let (Some(id), Some(labels)) = (member["identityId"].as_str(), labels(source, member)) {
            rows.entry(id.to_owned()).or_insert(labels);
        }
    }
    Some(rows)
}

fn labels(source: &str, member: &Value) -> Option<Vec<Label>> {
    let labels = member["labels"]
        .as_array()
        .filter(|l| l.len() <= MAX_LABELS)?;
    labels.iter().map(|value| label(source, value)).collect()
}

fn label(source: &str, value: &Value) -> Option<Label> {
    let text = value["text"].as_str().filter(|text| {
        !text.is_empty()
            && text.chars().count() <= MAX_TEXT_CHARS
            && !text.chars().any(char::is_control)
    })?;
    let role = match value["colorClass"].as_str()? {
        "text" => Role::Text,
        "dim" => Role::Dim,
        "muted" => Role::Muted,
        "waiting" => Role::Waiting,
        _ => return None,
    };
    Some(Label {
        text: text.to_owned(),
        role,
        // An action Ops will not offer leaves the label display-only.
        action: Choose::parse(source, &value["action"]).ok(),
    })
}

/// How the reader paces itself.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Between reads while every source answers.
    pub every: Duration,
    /// One read's limit.
    pub deadline: Duration,
    /// How long the last good answer stands in for a source that stops answering.
    pub keep: Duration,
    /// Failing reads back off to this.
    pub slowest: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            every: Duration::from_secs(5),
            deadline: Duration::from_secs(5),
            keep: Duration::from_secs(30),
            slowest: Duration::from_secs(60),
        }
    }
}

enum Signal {
    Read,
    Window(crate::config::DigestWindow),
    Stop,
}

/// Coalesce read and selection requests in the one worker that owns acquisition.
/// A selection received during a child read fences its old-window publication.
fn pending(
    wake: &mpsc::Receiver<Signal>,
    first: Option<Signal>,
    window: &mut crate::config::DigestWindow,
) -> Option<bool> {
    let mut again = false;
    for signal in first.into_iter().chain(wake.try_iter()) {
        match signal {
            Signal::Stop => return None,
            Signal::Read => again = true,
            Signal::Window(value) if value != *window => {
                *window = value;
                again = true;
            }
            Signal::Window(_) => {}
        }
    }
    Some(again)
}

/// Asks the reader to read again at once, such as after the user changed what a
/// source supplies. Cheap to clone; ignored once the reader is gone.
#[derive(Clone)]
pub struct Refresh(mpsc::Sender<Signal>);

impl Refresh {
    /// Select a supplier window and wake this same reader.
    pub fn window(&self, window: crate::config::DigestWindow) {
        let _ = self.0.send(Signal::Window(window));
    }

    pub fn now(&self) {
        let _ = self.0.send(Signal::Read);
    }
}

/// The background reader: one thread, one read in flight, newest answer delivered.
/// Dropping it cancels the read in flight and waits for the thread.
pub struct Reader {
    cancellation: Cancellation,
    signals: mpsc::Sender<Signal>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Reader {
    /// Delivers a changed [`Supplied`] to `deliver`, which returns `false` once
    /// nothing is listening. `None` when no source is configured.
    pub fn spawn(
        core: &Core,
        sources: Vec<String>,
        timing: Timing,
        initial_window: crate::config::DigestWindow,
        mut deliver: impl FnMut(Supplied) -> bool + Send + 'static,
    ) -> Option<Self> {
        if sources.is_empty() {
            return None;
        }
        let cancellation = Cancellation::default();
        let core = core.cancellable(cancellation.clone());
        let (signals, wake) = mpsc::channel::<Signal>();
        let thread = std::thread::spawn(move || {
            let mut last: Vec<Option<(Snapshot, Instant)>> = vec![None; sources.len()];
            let mut sent = Supplied::default();
            let mut wait = timing.every;
            let mut window = initial_window;
            loop {
                let read_window = window;
                let mut failed = false;
                for (source, last) in sources.iter().zip(&mut last) {
                    let read = core
                        .json_within(
                            &digest::status_arguments(source, read_window),
                            timing.deadline,
                        )
                        .ok()
                        .and_then(|document| Snapshot::parse(source, &document));
                    match read {
                        Some(rows) => *last = Some((rows, Instant::now())),
                        None => {
                            failed = true;
                            if let Some((snapshot, _)) = last.as_mut() {
                                snapshot.stale = true;
                            }
                            if last
                                .as_ref()
                                .is_some_and(|(_, at)| at.elapsed() > timing.keep)
                            {
                                *last = None;
                            }
                        }
                    }
                }
                let Some(again) = pending(&wake, None, &mut window) else {
                    return;
                };
                if window != read_window {
                    wait = timing.every;
                    continue;
                }
                let supplied = Supplied(
                    sources
                        .iter()
                        .zip(&last)
                        .filter_map(|(source, last)| {
                            last.as_ref()
                                .map(|(rows, _)| (source.clone(), rows.clone()))
                        })
                        .collect(),
                );
                if supplied != sent {
                    sent = supplied.clone();
                    if !deliver(supplied) {
                        return;
                    }
                }
                wait = if failed {
                    (wait * 2).min(timing.slowest)
                } else {
                    timing.every
                };
                if again {
                    wait = timing.every;
                    continue;
                }
                let deadline = Instant::now() + wait;
                loop {
                    match wake.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Err(RecvTimeoutError::Timeout) => break,
                        Ok(signal) => match pending(&wake, Some(signal), &mut window) {
                            None => return,
                            Some(false) => continue,
                            Some(true) => {
                                wait = timing.every;
                                break;
                            }
                        },
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        });
        Some(Self {
            cancellation,
            signals,
            thread: Some(thread),
        })
    }

    pub fn refresher(&self) -> Refresh {
        Refresh(self.signals.clone())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let _ = self.signals.send(Signal::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
