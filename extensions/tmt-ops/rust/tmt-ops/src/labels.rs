//! Labels a tmt extension supplies for member rows. Ops asks each configured source
//! `tmt <source> status --json` off the paint path and shows what comes back: it
//! recalculates nothing a source decided and names no source itself. A source that
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
pub struct Supplied(Vec<(String, Rows)>);

impl Supplied {
    #[cfg(test)]
    pub fn from_rows(sources: Vec<(String, Rows)>) -> Self {
        Self(sources)
    }

    /// An identity's labels, each with the source that supplied it.
    pub fn of<'a>(&'a self, identity: &'a str) -> impl Iterator<Item = (&'a str, &'a Label)> {
        self.0.iter().flat_map(move |(source, rows)| {
            rows.get(identity)
                .into_iter()
                .flatten()
                .map(move |label| (source.as_str(), label))
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
    Stop,
}

/// Asks the reader to read again at once, such as after the user changed what a
/// source supplies. Cheap to clone; ignored once the reader is gone.
#[derive(Clone)]
pub struct Refresh(mpsc::Sender<Signal>);

impl Refresh {
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
        mut deliver: impl FnMut(Supplied) -> bool + Send + 'static,
    ) -> Option<Self> {
        if sources.is_empty() {
            return None;
        }
        let cancellation = Cancellation::default();
        let core = core.cancellable(cancellation.clone());
        let (signals, wake) = mpsc::channel::<Signal>();
        let thread = std::thread::spawn(move || {
            let mut last: Vec<Option<(Rows, Instant)>> = vec![None; sources.len()];
            let mut sent = Supplied::default();
            let mut wait = timing.every;
            loop {
                let mut failed = false;
                for (source, last) in sources.iter().zip(&mut last) {
                    let read = core
                        .json_within(&[source, "status"], timing.deadline)
                        .ok()
                        .and_then(|document| parse(source, &document));
                    match read {
                        Some(rows) => *last = Some((rows, Instant::now())),
                        None => {
                            failed = true;
                            if last
                                .as_ref()
                                .is_some_and(|(_, at)| at.elapsed() > timing.keep)
                            {
                                *last = None;
                            }
                        }
                    }
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
                match wake.recv_timeout(wait) {
                    Err(RecvTimeoutError::Timeout) => {}
                    Ok(Signal::Read) => {
                        wait = timing.every;
                        // Requests that piled up behind one read are one read.
                        while let Ok(signal) = wake.try_recv() {
                            if matches!(signal, Signal::Stop) {
                                return;
                            }
                        }
                    }
                    Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => return,
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
mod tests;
