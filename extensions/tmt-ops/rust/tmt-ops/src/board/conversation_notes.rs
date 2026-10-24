//! The Notes of one conversation (a member's list inside a room), read and written on the lane.
//! The board keeps only what it paints; the admitted checklist service stays the one owner of the items.
use crate::checklist::{Applied, Code, Error, Preview, model::Id};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

mod draft;
mod summary;
pub(in crate::board) use summary::{Counts, Summary, Total};
pub(in crate::board) mod overlay;
mod rows;
mod surface;
mod work;

pub(in crate::board) use draft::{Draft, NoteKind, Seed};
pub(in crate::board) use work::{Write, complete};

/// How long a read stays current while its conversation is on screen: an agent can edit the
/// same list through the CLI, so the summary follows without a per-frame read.
const FRESH: Duration = Duration::from_secs(5);
/// A failed read is asked again later, not on every turn of the loop.
const RETRY: Duration = Duration::from_secs(30);
const BACKGROUND: Duration = Duration::from_secs(60);

/// A conversation: the room and the member, both canonical UUIDs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::board) struct Key {
    pub room: String,
    pub member: String,
}

impl Key {
    fn parts(target: &super::workspace::OpsTarget) -> Option<(&str, &str)> {
        match target {
            super::workspace::OpsTarget::Member {
                room_id: Some(room),
                identity_id,
            } => Some((room, identity_id)),
            _ => None,
        }
    }

    /// A member of a squad has a list in its room; a member of the Common pool has no room.
    pub fn of(target: &super::workspace::OpsTarget) -> Option<Self> {
        Self::parts(target).map(|(room, member)| Self {
            room: room.into(),
            member: member.into(),
        })
    }
}

/// What the lane does for one conversation: read its list, or apply one write to it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Work {
    Read,
    Counts,
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
    Counts(Result<Counts, String>),
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
    /// The last admitted list, for the overlay; the summary derives from it.
    preview: Option<Preview>,
    /// When the read in flight began, or the last one ended.
    at: Instant,
    preview_at: Option<Instant>,
    /// Bumped by every write, so that a read begun before it cannot overwrite what it changed.
    epoch: u64,
    /// A write is on the lane; the next one waits for its answer.
    writing: bool,
}

/// A board built without a lane (a test, a preview) reads nothing; the running board is
/// started with [`State::enable`].
#[derive(Default)]
pub(in crate::board) struct State {
    reading: bool,
    lists: BTreeMap<Key, Entry>,
    /// One quiet read owns the lane admission, including when selection changes.
    read: Option<Key>,
    /// The earliest background retry, so idle loop turns do not rescan the roster.
    background_at: Option<Instant>,
    /// Global admitted conversations, independent of search and the viewport.
    scope: Option<BTreeSet<Key>>,
    total: Option<Total>,
    /// An unsent form outlives the overlay that showed it.
    drafts: BTreeMap<Key, Draft>,
}

impl State {
    /// Unchanged rosters compare borrowed UUIDs without rebuilding the admission inventory.
    pub fn observe_roster<'a>(
        &mut self,
        targets: impl Iterator<Item = &'a super::workspace::OpsTarget> + Clone,
    ) {
        let admitted: BTreeSet<_> = targets.clone().filter_map(Key::parts).collect();
        if self.scope.as_ref().is_some_and(|scope| {
            scope.len() == admitted.len()
                && scope
                    .iter()
                    .zip(&admitted)
                    .all(|(key, (room, member))| key.room == *room && key.member == *member)
        }) {
            return;
        }
        self.reconcile(targets.filter_map(Key::of));
    }

    pub fn enable(&mut self) {
        self.reading = true;
    }

    pub fn reconcile(&mut self, keys: impl Iterator<Item = Key>) {
        let scope: BTreeSet<_> = keys.collect();
        if self.scope.as_ref() == Some(&scope) {
            return;
        }
        self.lists.retain(|key, entry| {
            scope.contains(key)
                || entry.writing
                || self.read.as_ref() == Some(key)
                || self.drafts.contains_key(key)
        });
        self.scope = Some(scope);
        self.background_at = None;
        self.recount();
    }

    fn recount(&mut self) {
        let mut total = Total::default();
        let keys: Box<dyn Iterator<Item = &Key> + '_> = match &self.scope {
            Some(keys) => Box::new(keys.iter()),
            None if self.lists.is_empty() => {
                self.total = None;
                return;
            }
            None => Box::new(self.lists.keys()),
        };
        for key in keys {
            total.observe(self.summary(key));
        }
        self.total = Some(total);
    }

    pub fn total(&self) -> Option<Total> {
        self.total
    }

    #[cfg(test)]
    pub fn total_label(&self) -> String {
        self.total.map_or_else(|| "… todo".into(), Total::label)
    }

    pub fn row_label(&self, target: &super::workspace::OpsTarget) -> Option<String> {
        let key = Key::of(target)?;
        match self.summary(&key) {
            Some(Summary::Counts(counts)) => {
                (counts.todo > 0).then(|| format!("{} todo", counts.todo))
            }
            None | Some(Summary::Pending | Summary::Unavailable(_)) => None,
        }
    }

    #[cfg(test)]
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
        if self.read.is_some() {
            return None;
        }
        let epoch = match self.lists.get_mut(key) {
            Some(entry) if entry.writing => return None,
            Some(entry) => {
                let wait = if matches!(entry.summary, Summary::Unavailable(_)) {
                    RETRY
                } else {
                    FRESH
                };
                let at = entry.preview_at.unwrap_or(entry.at);
                if (entry.preview_at.is_some() || matches!(entry.summary, Summary::Unavailable(_)))
                    && now.duration_since(at) < wait
                {
                    return None;
                }
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
                        preview_at: None,
                        epoch: 0,
                        writing: false,
                    },
                );
                0
            }
        };
        self.read = Some(key.clone());
        self.recount();
        Some(Task {
            key: key.clone(),
            work: Work::Read,
            epoch,
        })
    }

    /// Selected reads are offered first; a background turn admits just one due list.
    pub fn offer_background(&mut self, shown: Option<&Key>, now: Instant) -> Option<Task> {
        if !self.reading || self.read.is_some() || self.background_at.is_some_and(|at| now < at) {
            return None;
        }
        let candidate = self
            .scope
            .as_ref()?
            .iter()
            .filter(|key| {
                Some(*key) != shown && self.lists.get(*key).is_none_or(|entry| !entry.writing)
            })
            .map(|key| (self.lists.get(key).map(|entry| entry.at), key))
            .min_by_key(|(at, _)| *at);
        let Some((at, key)) = candidate else {
            self.background_at = Some(now + BACKGROUND);
            return None;
        };
        if let Some(at) = at.filter(|at| now.duration_since(*at) < BACKGROUND) {
            self.background_at = Some(at + BACKGROUND);
            return None;
        }
        let key = key.clone();
        let entry = self.lists.entry(key.clone()).or_insert_with(|| Entry {
            summary: Summary::Pending,
            preview: None,
            at: now,
            preview_at: None,
            epoch: 0,
            writing: false,
        });
        entry.at = now;
        let epoch = entry.epoch;
        self.read = Some(key.clone());
        Some(Task {
            key,
            work: Work::Counts,
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
        self.background_at = None;
        let written = self.adopt(completed, now);
        if let Some(scope) = &self.scope {
            self.lists.retain(|key, entry| {
                scope.contains(key)
                    || entry.writing
                    || self.read.as_ref() == Some(key)
                    || self.drafts.contains_key(key)
            });
        }
        self.recount();
        written
    }

    fn adopt(&mut self, completed: Completed, now: Instant) -> Option<Written> {
        if matches!(completed.outcome, Outcome::Read(_) | Outcome::Counts(_))
            && self.read.as_ref() == Some(&completed.key)
        {
            self.read = None;
        }
        let entry = self.lists.get_mut(&completed.key)?;
        match completed.outcome {
            Outcome::Read(result) => {
                entry.at = now;
                entry.preview_at = Some(now);
                if completed.epoch != entry.epoch {
                    return None;
                }
                match result {
                    Ok(preview) => {
                        entry.summary = Summary::Counts(Counts::of(&preview.current.items));
                        entry.preview = Some(preview);
                        entry.preview_at = Some(now);
                    }
                    // A list already shown stays on a failed refresh; only a first read reports it.
                    Err(error) if matches!(entry.summary, Summary::Pending) => {
                        entry.summary = Summary::Unavailable(error)
                    }
                    Err(_) => {}
                }
                None
            }
            Outcome::Counts(result) => {
                entry.at = now;
                if completed.epoch != entry.epoch {
                    return None;
                }
                match result {
                    Ok(counts) => entry.summary = Summary::Counts(counts),
                    Err(error) if matches!(entry.summary, Summary::Pending) => {
                        entry.summary = Summary::Unavailable(error);
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
                        entry.at = now;
                        entry.preview_at = Some(now);
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

    /// The last admitted list for this conversation.
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
