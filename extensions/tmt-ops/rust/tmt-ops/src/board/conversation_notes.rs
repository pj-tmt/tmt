//! The Notes of one conversation (a member's list inside a room), read on the lane. The board
//! keeps only what it paints; Core's checklist stays the one owner of the items.
use crate::{
    checklist::{
        ItemView, Service,
        model::{Completion, Id},
    },
    config::Config,
    core::Core,
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

/// How long a read stays current while its conversation is on screen: an agent can edit the
/// same list through the CLI, so the summary follows without a per-frame read.
const FRESH: Duration = Duration::from_secs(5);
/// A failed read is asked again later, not on every turn of the loop.
const RETRY: Duration = Duration::from_secs(30);

/// A conversation: the room and the member, both canonical UUIDs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::board) struct Key {
    pub room: String,
    pub member: String,
}

impl Key {
    /// A member of a squad has a list in its room; a member of the Common pool has no room.
    pub fn of(target: &super::workspace::OpsTarget) -> Option<Self> {
        match target {
            super::workspace::OpsTarget::Member {
                room_id: Some(room),
                identity_id,
            } => Some(Self {
                room: room.clone(),
                member: identity_id.clone(),
            }),
            _ => None,
        }
    }
}

/// What the summary counts; archived items belong to neither.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::board) struct Counts {
    pub links: usize,
    pub todo: usize,
    pub done: usize,
}

impl Counts {
    /// An item with a reference is a link; one without is a todo, open or done.
    pub fn of<'a>(items: impl IntoIterator<Item = &'a ItemView>) -> Self {
        let mut counts = Self::default();
        for view in items.into_iter().filter(|view| !view.item.archived) {
            match (&view.item.content.reference, view.item.completion) {
                (Some(_), _) => counts.links += 1,
                (None, Completion::Open) => counts.todo += 1,
                (None, Completion::Complete) => counts.done += 1,
            }
        }
        counts
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// What the board knows of one conversation's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) enum Summary {
    /// Asked, nothing back yet.
    Pending,
    Counts(Counts),
    /// Core's own words for why the list could not be read.
    Unavailable(String),
}

/// One read for the lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) struct Task {
    key: Key,
}

pub(in crate::board) struct Completed {
    key: Key,
    result: Result<Counts, String>,
}

struct Entry {
    summary: Summary,
    /// When the read in flight began, or the last one ended.
    at: Instant,
    in_flight: bool,
}

/// A board built without a lane (a test, a preview) reads nothing; the running board is
/// started with [`State::reading`].
#[derive(Default)]
pub(in crate::board) struct State {
    reading: bool,
    lists: BTreeMap<Key, Entry>,
}

impl State {
    pub fn reading() -> Self {
        Self {
            reading: true,
            ..Self::default()
        }
    }

    /// The read the conversation on screen needs now, if any: the first one, then one per
    /// [`FRESH`] while it stays on screen (a failure waits [`RETRY`]).
    pub fn offer(&mut self, shown: Option<&Key>, now: Instant) -> Option<Task> {
        let key = shown.filter(|_| self.reading)?;
        match self.lists.get_mut(key) {
            Some(entry) if entry.in_flight => return None,
            Some(entry) => {
                let wait = if matches!(entry.summary, Summary::Unavailable(_)) {
                    RETRY
                } else {
                    FRESH
                };
                if now.duration_since(entry.at) < wait {
                    return None;
                }
                entry.in_flight = true;
                entry.at = now;
            }
            None => {
                self.lists.insert(
                    key.clone(),
                    Entry {
                        summary: Summary::Pending,
                        at: now,
                        in_flight: true,
                    },
                );
            }
        }
        Some(Task { key: key.clone() })
    }

    pub fn finished(&mut self, completed: Completed, now: Instant) {
        let Some(entry) = self.lists.get_mut(&completed.key) else {
            return;
        };
        entry.in_flight = false;
        entry.at = now;
        match completed.result {
            Ok(counts) => entry.summary = Summary::Counts(counts),
            // A list already shown stays on a failed refresh; only a first read reports it.
            Err(error) if matches!(entry.summary, Summary::Pending) => {
                entry.summary = Summary::Unavailable(error)
            }
            Err(_) => {}
        }
    }

    pub fn summary(&self, key: &Key) -> Option<&Summary> {
        self.lists.get(key).map(|entry| &entry.summary)
    }
}

/// Reads the list of one conversation through Core's checklist service.
pub(in crate::board) fn complete(core: &Core, task: Task) -> Completed {
    let result = read(core, &task.key);
    Completed {
        key: task.key,
        result,
    }
}

fn read(core: &Core, key: &Key) -> Result<Counts, String> {
    let config = Config::load(core).map_err(|error| error.message)?;
    let (room, member) = (
        Id::parse(&key.room).map_err(|error| error.message)?,
        Id::parse(&key.member).map_err(|error| error.message)?,
    );
    let preview = Service::for_conversation(core, &config, room, member)
        .and_then(|service| service.preview())
        .map_err(|error| error.message)?;
    Ok(Counts::of(&preview.current.items))
}

/// The chip's words: `≡ Notes · 3 links · 2 todo · 1 done`, `≡ Notes · empty`, or the mark alone
/// until the first read answers.
pub(in crate::board) fn chip(summary: Option<&Summary>) -> String {
    match summary {
        None | Some(Summary::Pending) => "≡ Notes".into(),
        Some(Summary::Unavailable(_)) => "≡ Notes · unavailable".into(),
        Some(Summary::Counts(counts)) if counts.is_empty() => "≡ Notes · empty".into(),
        Some(Summary::Counts(counts)) => format!(
            "≡ Notes · {} {} · {} todo · {} done",
            counts.links,
            if counts.links == 1 { "link" } else { "links" },
            counts.todo,
            counts.done
        ),
    }
}

/// The chip fitted to `columns` so that it also fits where an East Asian locale draws the
/// ambiguous-width `≡` two cells wide.
pub(in crate::board) fn fitted(summary: Option<&Summary>, columns: usize) -> String {
    super::view::fit(&chip(summary), columns.saturating_sub(1))
        .trim_end()
        .to_owned()
}

#[cfg(test)]
pub(in crate::board) mod tests;
