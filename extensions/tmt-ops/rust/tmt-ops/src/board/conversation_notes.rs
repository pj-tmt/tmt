//! The Notes of one conversation (a member's list inside a room), read and written on the lane.
//! The board keeps only what it paints; Core's checklist stays the one owner of the items.
use crate::checklist::{
    Applied, Code, Error, ItemView, Preview,
    model::{Completion, Id},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

mod draft;
pub(in crate::board) mod overlay;
mod rows;
mod surface;
mod work;

pub(in crate::board) use draft::Draft;
pub(in crate::board) use work::{Write, complete};

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

/// What the lane does for one conversation: read its list, or apply one write to it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Work {
    Read,
    Write(Write),
}

/// One job for the lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) struct Task {
    key: Key,
    work: Work,
    /// The list generation a read began at; a write in between makes its answer stale.
    epoch: u64,
}

#[derive(Debug)]
enum Outcome {
    Read(Result<Preview, String>),
    Write(Result<Applied, Error>),
}

#[derive(Debug)]
pub(in crate::board) struct Completed {
    key: Key,
    epoch: u64,
    outcome: Outcome,
}

/// How a write ended, for the overlay that asked for it.
#[derive(Debug)]
pub(in crate::board) struct Written {
    pub key: Key,
    /// The item the write added or changed.
    pub item: Option<Id>,
    pub result: Result<(), Error>,
}

struct Entry {
    summary: Summary,
    /// The last list Core answered, for the overlay; the summary derives from it.
    preview: Option<Preview>,
    /// When the read in flight began, or the last one ended.
    at: Instant,
    in_flight: bool,
    /// Bumped by every write, so that a read begun before it cannot overwrite what it changed.
    epoch: u64,
    /// A write is on the lane; the next one waits for its answer.
    writing: bool,
}

/// A board built without a lane (a test, a preview) reads nothing; the running board is
/// started with [`State::reading`].
#[derive(Default)]
pub(in crate::board) struct State {
    reading: bool,
    lists: BTreeMap<Key, Entry>,
    /// An unsent form outlives the overlay that showed it.
    drafts: BTreeMap<Key, Draft>,
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
        let epoch = match self.lists.get_mut(key) {
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
                entry.epoch
            }
            None => {
                self.lists.insert(
                    key.clone(),
                    Entry {
                        summary: Summary::Pending,
                        preview: None,
                        at: now,
                        in_flight: true,
                        epoch: 0,
                        writing: false,
                    },
                );
                0
            }
        };
        Some(Task {
            key: key.clone(),
            work: Work::Read,
            epoch,
        })
    }

    /// The write for the lane, or None while the list is unread or another write is on it.
    pub fn write(&mut self, key: &Key, write: Write) -> Option<Task> {
        let entry = self.lists.get_mut(key).filter(|entry| !entry.writing)?;
        entry.writing = true;
        Some(Task {
            key: key.clone(),
            work: Work::Write(write),
            epoch: entry.epoch,
        })
    }

    pub fn finished(&mut self, completed: Completed, now: Instant) -> Option<Written> {
        let entry = self.lists.get_mut(&completed.key)?;
        match completed.outcome {
            Outcome::Read(result) => {
                entry.in_flight = false;
                entry.at = now;
                if completed.epoch != entry.epoch {
                    return None;
                }
                match result {
                    Ok(preview) => {
                        entry.summary = Summary::Counts(Counts::of(&preview.current.items));
                        entry.preview = Some(preview);
                    }
                    // A list already shown stays on a failed refresh; only a first read reports it.
                    Err(error) if matches!(entry.summary, Summary::Pending) => {
                        entry.summary = Summary::Unavailable(error)
                    }
                    Err(_) => {}
                }
                None
            }
            Outcome::Write(result) => {
                entry.writing = false;
                entry.epoch += 1;
                let draft = self.drafts.get_mut(&completed.key);
                match result {
                    Ok(applied) => {
                        entry.adopt(applied.current);
                        // A draft the write carried is sent.
                        if draft.is_some_and(|draft| draft.sent()) {
                            self.drafts.remove(&completed.key);
                        }
                        Some(Written {
                            key: completed.key,
                            item: applied.item_id,
                            result: Ok(()),
                        })
                    }
                    Err(error) => {
                        if let Some(current) = &error.current {
                            entry.adopt((**current).clone());
                        }
                        // The draft stays, aimed at the item as it is now.
                        if let Some(draft) = draft.filter(|draft| draft.sent()) {
                            draft.rejected(entry.preview.as_ref());
                        }
                        // After a conflict or an unknown outcome the list is read again now.
                        if matches!(error.code, Code::OutcomeUnknown | Code::Conflict) {
                            entry.at = now.checked_sub(FRESH).unwrap_or(now);
                        }
                        Some(Written {
                            key: completed.key,
                            item: None,
                            result: Err(error),
                        })
                    }
                }
            }
        }
    }

    pub fn summary(&self, key: &Key) -> Option<&Summary> {
        self.lists.get(key).map(|entry| &entry.summary)
    }

    /// The last list Core answered for this conversation.
    pub fn preview(&self, key: &Key) -> Option<&Preview> {
        self.lists.get(key)?.preview.as_ref()
    }

    pub fn writing(&self, key: &Key) -> bool {
        self.lists.get(key).is_some_and(|entry| entry.writing)
    }

    pub fn draft(&self, key: &Key) -> Option<&Draft> {
        self.drafts.get(key)
    }

    pub fn draft_mut(&mut self, key: &Key) -> Option<&mut Draft> {
        self.drafts.get_mut(key)
    }

    pub fn keep(&mut self, key: &Key, draft: Draft) {
        self.drafts.insert(key.clone(), draft);
    }

    pub fn drop_draft(&mut self, key: &Key) {
        self.drafts.remove(key);
    }
}

impl Entry {
    /// Takes the list a write answered with, and counts it.
    fn adopt(&mut self, current: crate::checklist::Current) {
        self.summary = Summary::Counts(Counts::of(&current.items));
        if let Some(preview) = &mut self.preview {
            preview.current = current;
        }
    }
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
