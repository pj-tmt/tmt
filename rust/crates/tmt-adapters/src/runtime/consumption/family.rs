//! The Codex sessions a bound session started (subagents and auto-review) count
//! toward the seat that owns the root of their parent chain.
//!
//! A started session is its own rollout under the sessions tree. Its first line
//! names its parent, which is how a rollout is placed: only the header and the
//! token counters are read, never the conversation. Counting starts when the
//! family is first scanned (rollouts that already existed are baselined at
//! their current totals), and a rollout found later counts from zero. The
//! per-file cursors live in storage because the sampler is a fresh process each
//! tick; the 1 KiB driver state only carries the sum.
use super::{
    Consumption, Counts, MAX_MODELS, ModelUsage, SourceCursor, State, codex_with, transcript,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Instant, UNIX_EPOCH},
};

/// A followed rollout nobody wrote to for this long is folded into the closed
/// sum and forgotten; one that wakes up later is not counted again.
pub const QUIET_MS: u64 = 24 * 60 * 60 * 1000;
/// Followed rollouts per family. Past it the family reports itself incomplete.
pub const MAX_MEMBERS: usize = 1024;
/// Directory entries listed per tick: names only, so a busy day stays cheap.
pub const SCAN_ENTRIES: usize = 65_536;
/// Rollout headers read per tick.
const CLASSIFY_PER_TICK: usize = 64;
/// Followed rollouts read per tick (headers aside).
const READ_PER_TICK: usize = 64;
/// The longest header line, as the existing exact-path header read allows.
const HEADER_BYTES: usize = 64 * 1024;
/// Rollouts remembered as already placed at the scan position.
pub const MAX_EDGE: usize = 64;
/// A header not yet written is waited for this long, then skipped.
const PENDING_GRACE_MS: u64 = 60_000;

/// What the sessions a session started added since counting began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contribution {
    pub(super) counts: Counts,
    /// The models of this tick's increments, the same at-most split as the
    /// session's own: a rollout that could not be attributed contributes none.
    pub(super) rows: Option<Vec<ModelUsage>>,
    /// Every started session was found and followed up to now.
    pub(super) complete: bool,
}

impl Contribution {
    pub fn complete(&self) -> bool {
        self.complete
    }
}

/// The family of one session, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub session: String,
    /// The creation time in a rollout's name before which every rollout has
    /// been placed; names at or after it are still looked at.
    pub scan: Option<String>,
    /// The rollouts named at exactly `scan` that were already placed, so the
    /// second can be looked at again for late arrivals without counting twice.
    pub edge: Vec<String>,
    /// What followed rollouts contributed before they were folded away.
    pub closed: Counts,
    /// Rollouts found now existed before counting began: they are baselined.
    pub warm: bool,
}

impl Header {
    fn fresh(session: &str) -> Self {
        Self {
            session: session.to_owned(),
            scan: None,
            edge: Vec::new(),
            closed: Counts::default(),
            warm: true,
        }
    }
}

impl Header {
    /// A stored family names its scan position as a creation time, or none.
    pub fn valid(&self) -> bool {
        (1..=256).contains(&self.session.len())
            && self
                .scan
                .as_deref()
                .is_none_or(|time| time.len() == 19 && day_of(time).is_some())
            && self.edge.len() <= MAX_EDGE
            && self
                .edge
                .iter()
                .all(|file| relative_rollout(file).is_some())
            && self.closed.valid()
    }
}

/// One followed rollout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Relative to the sessions root.
    pub file: String,
    pub child: String,
    /// The session this one was started by: the bound session or another followed one.
    pub parent: String,
    /// The rollout's counters when counting began (zero for one found live).
    pub base: Counts,
    /// Its own counters and read cursor; only the sums and the cursor are kept.
    pub state: State,
    pub touched_ms: u64,
}

impl Member {
    /// A stored row names a rollout under the sessions tree by its own name.
    pub fn valid(&self) -> bool {
        relative_rollout(&self.file).is_some_and(|(_, id)| id == self.child)
            && uuid::Uuid::parse_str(&self.parent).is_ok()
            && self.base.valid()
            && self.state.value.counts().covers(self.base)
            && self.state.family.is_none()
    }

    /// What this rollout has added to the seat.
    fn contribution(&self) -> Counts {
        self.state.value.counts().gained(self.base)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub header: Header,
    pub members: Vec<Member>,
}

/// What to write back after a tick that was accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    /// The stored family belonged to another session and is replaced.
    pub reset: bool,
    pub header: Option<Header>,
    pub upserts: Vec<Member>,
    pub deletes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub contribution: Contribution,
    pub write: Write,
}

/// What the sampler read from storage before the source work.
pub struct Input {
    pub stored: Option<Stored>,
    /// The stored family could not be read: it is replaced, not extended.
    pub corrupt: bool,
    /// Other seats' bound sessions: they count only toward their own seat.
    pub others: HashSet<String>,
}

pub struct Tick<'a> {
    /// The provider's sessions tree.
    pub root: &'a Path,
    /// The bound session's own rollout.
    pub main: &'a Path,
    pub session: &'a str,
    /// Other seats' bound sessions: they count only toward their own seat.
    pub others: &'a HashSet<String>,
    pub now: u64,
    pub deadline: Instant,
    /// The stored family could not be read: it is replaced, not extended.
    pub corrupt: bool,
    /// Directory names listed per tick; [`SCAN_ENTRIES`] in production.
    pub entries: usize,
}

/// The union of two at-most splits: a model appearing in both is summed, and a
/// fifth model drops the split built so far.
pub(super) fn merge_rows(
    own: Option<Vec<ModelUsage>>,
    more: Option<Vec<ModelUsage>>,
) -> Option<Vec<ModelUsage>> {
    match (own, more) {
        (None, None) => None,
        (Some(rows), None) | (None, Some(rows)) => Some(rows),
        (Some(mut rows), Some(more)) => {
            for row in more {
                if let Some(old) = rows.iter_mut().find(|old| old.model_id == row.model_id) {
                    old.merge(&row)?;
                } else if rows.len() < MAX_MODELS {
                    rows.push(row);
                } else {
                    return None;
                }
            }
            Some(rows)
        }
    }
}

/// `rollout-<YYYY-MM-DDThh-mm-ss>-<uuid>.jsonl` → (creation time, session id).
fn rollout_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let (time, id) = rest.split_at_checked(19)?;
    let id = id.strip_prefix('-')?;
    let numeric = |range: std::ops::Range<usize>| {
        time.get(range)
            .is_some_and(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
    };
    (numeric(0..4)
        && numeric(5..7)
        && numeric(8..10)
        && numeric(11..13)
        && numeric(14..16)
        && numeric(17..19)
        && time.as_bytes()[4] == b'-'
        && time.as_bytes()[7] == b'-'
        && time.as_bytes()[10] == b'T'
        && time.as_bytes()[13] == b'-'
        && time.as_bytes()[16] == b'-'
        && uuid::Uuid::parse_str(id).is_ok())
    .then_some((time, id))
}

/// The year, month and day in a creation time.
fn day_of(time: &str) -> Option<(u32, u32, u32)> {
    Some((
        time.get(0..4)?.parse().ok()?,
        time.get(5..7)?.parse().ok()?,
        time.get(8..10)?.parse().ok()?,
    ))
}

/// `YYYY/MM/DD/<name>` of a rollout: its creation time and session id.
fn relative_rollout(file: &str) -> Option<(&str, &str)> {
    let parts = file.split('/').collect::<Vec<_>>();
    let [year, month, day, name] = parts.as_slice() else {
        return None;
    };
    let (time, id) = rollout_name(name)?;
    (time.get(0..4) == Some(year) && time.get(5..7) == Some(month) && time.get(8..10) == Some(day))
        .then_some((time, id))
}

/// The bound session's rollout as `YYYY/MM/DD/<name>`, with its creation time.
fn locate(root: &Path, main: &Path, session: &str) -> Option<(String, String)> {
    let relative = main.strip_prefix(root).ok()?;
    let parts = relative
        .components()
        .map(|part| match part {
            std::path::Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let [year, month, day, name] = parts.as_slice() else {
        return None;
    };
    let (time, id) = rollout_name(name)?;
    (id == session
        && time.get(0..4) == Some(year)
        && time.get(5..7) == Some(month)
        && time.get(8..10) == Some(day))
    .then(|| Some((relative.to_str()?.to_owned(), time.to_owned())))
    .flatten()
}

#[derive(Deserialize)]
struct HeaderLine {
    #[serde(rename = "type")]
    kind: String,
    payload: HeaderPayload,
}

#[derive(Deserialize)]
struct HeaderPayload {
    id: String,
    parent_thread_id: Option<String>,
}

struct Meta {
    id: String,
    parent: Option<String>,
}

enum Line {
    Placed(Meta),
    /// Not written yet.
    Pending,
    /// Not a session header this reader understands.
    Foreign,
}

/// The first line only, within the header bound.
fn read_header(root: &Path, path: &Path) -> Line {
    let Some(mut file) = transcript::open(root, path) else {
        return Line::Foreign;
    };
    let mut line = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let Ok(read) = file.read(&mut chunk) else {
            return Line::Foreign;
        };
        if read == 0 {
            return Line::Pending;
        }
        if let Some(end) = chunk[..read].iter().position(|byte| *byte == b'\n') {
            line.extend_from_slice(&chunk[..end]);
            break;
        }
        line.extend_from_slice(&chunk[..read]);
        if line.len() > HEADER_BYTES {
            return Line::Foreign;
        }
    }
    if line.len() > HEADER_BYTES {
        return Line::Foreign;
    }
    match serde_json::from_slice::<HeaderLine>(&line) {
        Ok(header)
            if header.kind == "session_meta"
                && uuid::Uuid::parse_str(&header.payload.id).is_ok() =>
        {
            Line::Placed(Meta {
                id: header.payload.id,
                parent: header.payload.parent_thread_id,
            })
        }
        _ => Line::Foreign,
    }
}

fn mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

struct Candidate {
    time: String,
    id: String,
    relative: String,
    path: PathBuf,
    mtime_ms: u64,
}

/// A rollout's own counters at the start of following it from zero.
fn zero_state(dev: u64, ino: u64, now: u64) -> State {
    State {
        value: Consumption::baseline(now, false),
        cursor: Some(SourceCursor {
            dev,
            ino,
            offset: 0,
            last: None,
            request: None,
            counts: None,
            discard: false,
        }),
        family: None,
    }
}

/// Only what the next read of this rollout needs.
fn kept(mut state: State) -> State {
    state.value.rate_limits = None;
    state.value.credits = None;
    state.value.service_tier = None;
    state.value.model_id = None;
    state.value.delta_by_model = None;
    state.family = None;
    state
}

struct Run<'a> {
    tick: &'a Tick<'a>,
    header: Header,
    members: Vec<Member>,
    changed: BTreeMap<String, ()>,
    deletes: Vec<String>,
    rows: Option<Vec<ModelUsage>>,
    /// A fifth model was seen: the split built so far is gone for this tick.
    rows_dropped: bool,
    complete: bool,
    reads: usize,
}

impl Run<'_> {
    fn expired(&self) -> bool {
        Instant::now() >= self.tick.deadline
    }

    fn add_rows(&mut self, incoming: Option<Vec<ModelUsage>>) {
        // The split is only as good as its parts: an unattributed rollout adds
        // nothing, and a fifth model drops what was built.
        if self.rows_dropped || incoming.is_none() {
            return;
        }
        self.rows = merge_rows(Some(self.rows.take().unwrap_or_default()), incoming);
        self.rows_dropped = self.rows.is_none();
    }

    fn fold(&mut self, member: &Member) -> Option<()> {
        self.header.closed = self.header.closed.add(member.contribution())?;
        self.deletes.push(member.file.clone());
        Some(())
    }

    /// Follow every rollout that grew; fold the ones nobody writes to.
    fn follow(&mut self) -> Option<()> {
        let now = self.tick.now;
        // A followed rollout may since have become another seat's bound session
        // (a seat resumed onto it): from now on it counts only there, with
        // everything it started. What it added here stays.
        let mut departed: HashSet<String> = self
            .members
            .iter()
            .filter(|member| self.tick.others.contains(&member.child))
            .map(|member| member.child.clone())
            .collect();
        loop {
            let before = departed.len();
            for member in &self.members {
                if departed.contains(&member.parent) {
                    departed.insert(member.child.clone());
                }
            }
            if departed.len() == before {
                break;
            }
        }
        let mut kept_members = Vec::with_capacity(self.members.len());
        for mut member in std::mem::take(&mut self.members) {
            if departed.contains(&member.child) {
                self.fold(&member)?;
                continue;
            }
            let path = self.tick.root.join(&member.file);
            let quiet = now.saturating_sub(member.touched_ms) > QUIET_MS;
            // One stat, not a trusted open: the read below opens it that way.
            let grown = fs::symlink_metadata(&path)
                .ok()
                .filter(fs::Metadata::is_file)
                .zip(member.state.cursor.as_ref())
                .is_some_and(|(metadata, cursor)| {
                    metadata.len() != cursor.offset
                        || metadata.ino() != cursor.ino
                        || metadata.dev() != cursor.dev
                });
            if !grown {
                if quiet {
                    self.fold(&member)?;
                } else {
                    kept_members.push(member);
                }
                continue;
            }
            if self.expired() || self.reads >= READ_PER_TICK {
                self.complete = false;
                kept_members.push(member);
                continue;
            }
            self.reads += 1;
            match codex_with(self.tick.root, &path, Some(&member.state), None, now) {
                // The rollout was replaced or its counters went back: what it
                // had added stays, and it is no longer followed.
                Some(next) if next.value.gap => self.fold(&member)?,
                Some(next) => {
                    self.add_rows(next.value.delta_by_model.clone());
                    member.state = kept(next);
                    member.touched_ms = now;
                    self.changed.insert(member.file.clone(), ());
                    kept_members.push(member);
                }
                None => {
                    self.complete = false;
                    kept_members.push(member);
                }
            }
        }
        self.members = kept_members;
        Some(())
    }

    /// The day directories from `from` on, in order, and the earliest creation
    /// time that could not be listed (a directory that cannot be read): the
    /// scan position never passes it.
    fn days(&mut self, from: (u32, u32, u32)) -> (Vec<(String, PathBuf)>, Option<String>) {
        let list = |directory: &Path| -> Option<Vec<(u32, String)>> {
            let mut names = Vec::new();
            for entry in fs::read_dir(directory).ok()? {
                let entry = entry.ok()?;
                if !entry.file_type().ok()?.is_dir() {
                    continue;
                }
                let name = entry.file_name().into_string().ok()?;
                if let Ok(number) = name.parse::<u32>() {
                    names.push((number, name));
                }
            }
            names.sort();
            Some(names)
        };
        let mut days = Vec::new();
        let mut ceiling: Option<String> = None;
        let mut unreadable = |time: String, run: &mut Self| {
            run.complete = false;
            if ceiling.as_ref().is_none_or(|old| time < *old) {
                ceiling = Some(time);
            }
        };
        let root = self.tick.root;
        let Some(years) = list(root) else {
            unreadable(format!("{:04}-01-01T00-00-00", from.0), self);
            return (days, ceiling);
        };
        for (year, year_name) in years.into_iter().filter(|(year, _)| *year >= from.0) {
            let Some(months) = list(&root.join(&year_name)) else {
                unreadable(format!("{year:04}-01-01T00-00-00"), self);
                continue;
            };
            for (month, month_name) in months
                .into_iter()
                .filter(|(month, _)| year > from.0 || *month >= from.1)
            {
                let directory = root.join(&year_name).join(&month_name);
                let Some(found) = list(&directory) else {
                    unreadable(format!("{year:04}-{month:02}-01T00-00-00"), self);
                    continue;
                };
                for (_, day_name) in found
                    .into_iter()
                    .filter(|(day, _)| year > from.0 || month > from.1 || *day >= from.2)
                {
                    days.push((
                        format!("{year_name}/{month_name}/{day_name}"),
                        directory.join(&day_name),
                    ));
                }
            }
        }
        (days, ceiling)
    }

    /// Place the rollouts created since the last tick: a rollout whose parent
    /// is the session or a followed rollout, and which is no other seat's own
    /// session, is followed.
    fn discover(&mut self, main_relative: &str, main_time: &str) {
        let now = self.tick.now;
        let floor = self
            .header
            .scan
            .clone()
            .filter(|time| day_of(time).is_some())
            .unwrap_or_else(|| main_time.to_owned());
        let from = day_of(&floor).unwrap_or_default();
        let known: HashSet<String> = self
            .members
            .iter()
            .map(|m| m.file.clone())
            .chain(self.header.edge.iter().cloned())
            .collect();
        let mut found = Vec::new();
        let mut visited = 0usize;
        let (days, mut ceiling) = self.days(from);
        // A day listed only in part, or not at all, caps the scan position at
        // its start: every earlier day was listed whole.
        let cap = |ceiling: &mut Option<String>, prefix: &str| {
            let start = format!("{}T00-00-00", prefix.replace('/', "-"));
            if ceiling.as_ref().is_none_or(|old| start < *old) {
                *ceiling = Some(start);
            }
        };
        'days: for (prefix, directory) in days {
            let Ok(entries) = fs::read_dir(&directory) else {
                cap(&mut ceiling, &prefix);
                continue;
            };
            for entry in entries {
                visited += 1;
                if visited > self.tick.entries || self.expired() {
                    cap(&mut ceiling, &prefix);
                    break 'days;
                }
                let Ok(entry) = entry else {
                    cap(&mut ceiling, &prefix);
                    continue;
                };
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                let Some((time, id)) = rollout_name(&name) else {
                    continue;
                };
                if time < floor.as_str() {
                    continue;
                }
                let relative = format!("{prefix}/{name}");
                if relative == main_relative || known.contains(&relative) {
                    continue;
                }
                found.push(Candidate {
                    time: time.to_owned(),
                    id: id.to_owned(),
                    mtime_ms: entry.metadata().map_or(0, |m| mtime_ms(&m)),
                    path: directory.join(&name),
                    relative,
                });
            }
        }
        found.sort_by(|a, b| (&a.time, &a.id).cmp(&(&b.time, &b.id)));
        let warm = self.header.warm;
        // Handled: placed for good, or not a family member for good. Anything
        // else is looked at again, and the scan position stops before it.
        let mut handled: Vec<(String, String)> = Vec::new();
        let mut unhandled: Option<String> = None;
        let mut placed: Vec<(Candidate, Meta)> = Vec::new();
        for candidate in found {
            if self.expired() || placed.len() >= CLASSIFY_PER_TICK {
                unhandled = Some(candidate.time);
                break;
            }
            // Before counting began, a rollout nobody wrote to lately has
            // nothing to baseline.
            if warm && now.saturating_sub(candidate.mtime_ms) > QUIET_MS {
                handled.push((candidate.time, candidate.relative));
                continue;
            }
            match read_header(self.tick.root, &candidate.path) {
                Line::Placed(meta) => placed.push((candidate, meta)),
                Line::Pending if now.saturating_sub(candidate.mtime_ms) < PENDING_GRACE_MS => {
                    unhandled = Some(candidate.time);
                    break;
                }
                Line::Pending | Line::Foreign => {
                    handled.push((candidate.time, candidate.relative));
                }
            }
        }
        // A child may sort before its parent within one second.
        let mut parents: HashSet<String> = self.members.iter().map(|m| m.child.clone()).collect();
        parents.insert(self.tick.session.to_owned());
        let mut accepted = vec![false; placed.len()];
        loop {
            let mut grew = false;
            for (index, (_, meta)) in placed.iter().enumerate() {
                if accepted[index]
                    || self.tick.others.contains(&meta.id)
                    || !meta
                        .parent
                        .as_ref()
                        .is_some_and(|parent| parents.contains(parent))
                {
                    continue;
                }
                if self.members.len() + accepted.iter().filter(|a| **a).count() >= MAX_MEMBERS {
                    self.complete = false;
                    continue;
                }
                accepted[index] = true;
                parents.insert(meta.id.clone());
                grew = true;
            }
            if !grew {
                break;
            }
        }
        let mut deferring = false;
        for (index, (candidate, meta)) in placed.into_iter().enumerate() {
            if accepted[index] && (deferring || self.expired() || self.reads >= READ_PER_TICK) {
                // Not read yet: the scan must not pass it.
                deferring = true;
                unhandled = Some(match unhandled.take() {
                    Some(time) if time < candidate.time => time,
                    _ => candidate.time,
                });
                continue;
            }
            handled.push((candidate.time.clone(), candidate.relative.clone()));
            if accepted[index] {
                self.create(candidate, meta, warm);
            }
        }
        let stopped = unhandled.is_some();
        let cut = unhandled
            .clone()
            .or_else(|| handled.iter().map(|(time, _)| time.clone()).max());
        let listing_complete = ceiling.is_none();
        let old = self.header.scan.clone();
        let cut = cut.map(|cut| match &ceiling {
            Some(ceiling) if *ceiling < cut => ceiling.clone(),
            _ => cut,
        });
        if let Some(cut) = cut.filter(|cut| old.as_deref().is_none_or(|old| cut.as_str() >= old)) {
            let mut edge = if old.as_deref() == Some(cut.as_str()) {
                std::mem::take(&mut self.header.edge)
            } else {
                Vec::new()
            };
            edge.extend(
                handled
                    .iter()
                    .filter(|(time, _)| *time == cut)
                    .map(|(_, relative)| relative.clone()),
            );
            edge.sort_unstable();
            edge.dedup();
            edge.truncate(MAX_EDGE);
            self.header.scan = Some(cut);
            self.header.edge = edge;
        }
        if !stopped && listing_complete {
            self.header.warm = false;
        }
        if stopped || !listing_complete || self.header.warm {
            self.complete = false;
        }
    }

    fn create(&mut self, candidate: Candidate, meta: Meta, warm: bool) {
        self.reads += 1;
        let now = self.tick.now;
        let Some(file) = transcript::open(self.tick.root, &candidate.path) else {
            return;
        };
        let Some(metadata) = file.metadata().ok() else {
            return;
        };
        drop(file);
        let zero = zero_state(metadata.dev(), metadata.ino(), now);
        let (state, base) = if warm {
            match codex_with(self.tick.root, &candidate.path, None, None, now) {
                Some(state) => {
                    let base = state.value.counts();
                    (kept(state), base)
                }
                None => (zero, Counts::default()),
            }
        } else {
            match codex_with(self.tick.root, &candidate.path, Some(&zero), None, now) {
                Some(state) if !state.value.gap => {
                    self.add_rows(state.value.delta_by_model.clone());
                    (kept(state), Counts::default())
                }
                _ => (zero, Counts::default()),
            }
        };
        let Some(parent) = meta.parent else { return };
        let member = Member {
            file: candidate.relative.clone(),
            child: meta.id,
            parent,
            base,
            state,
            touched_ms: now,
        };
        self.changed.insert(member.file.clone(), ());
        self.members.push(member);
    }
}

/// One tick of a session's family: refresh what is followed, place what is new.
/// `None` leaves the family as it was (the session's totals then carry the
/// last contribution unchanged).
pub fn advance(tick: &Tick<'_>, stored: Option<Stored>) -> Option<Outcome> {
    let (main_relative, main_time) = locate(tick.root, tick.main, tick.session)?;
    let (header, members, existing, reset) = match stored {
        Some(stored) if stored.header.session == tick.session && !tick.corrupt => (
            stored.header.clone(),
            stored.members,
            Some(stored.header),
            false,
        ),
        Some(_) => (Header::fresh(tick.session), Vec::new(), None, true),
        None => (Header::fresh(tick.session), Vec::new(), None, tick.corrupt),
    };
    let mut run = Run {
        tick,
        header,
        members,
        changed: BTreeMap::new(),
        deletes: Vec::new(),
        rows: None,
        rows_dropped: false,
        complete: true,
        reads: 0,
    };
    run.follow()?;
    run.discover(&main_relative, &main_time);
    let mut counts = run.header.closed;
    for member in &run.members {
        counts = counts.add(member.contribution())?;
    }
    let upserts = run
        .members
        .iter()
        .filter(|member| run.changed.contains_key(&member.file))
        .cloned()
        .collect();
    let header_changed = existing.as_ref() != Some(&run.header);
    Some(Outcome {
        contribution: Contribution {
            counts,
            rows: run.rows.filter(|rows| !rows.is_empty()),
            complete: run.complete,
        },
        write: Write {
            reset,
            header: header_changed.then_some(run.header),
            upserts,
            deletes: run.deletes,
        },
    })
}

#[cfg(test)]
impl Stored {
    /// What a sampler's commit leaves in storage, for tests that have none.
    pub(crate) fn after(stored: Option<Stored>, write: &Write) -> Stored {
        let mut next = match (stored, write.reset) {
            (Some(stored), false) => stored,
            _ => Stored {
                header: write.header.clone().unwrap(),
                members: Vec::new(),
            },
        };
        if let Some(header) = &write.header {
            next.header = header.clone();
        }
        next.members.retain(|m| !write.deletes.contains(&m.file));
        for upsert in &write.upserts {
            match next.members.iter_mut().find(|m| m.file == upsert.file) {
                Some(old) => *old = upsert.clone(),
                None => next.members.push(upsert.clone()),
            }
        }
        for member in &next.members {
            assert!(member.valid(), "{member:?}");
        }
        assert!(next.header.valid());
        next
    }
}

#[cfg(test)]
mod tests;
