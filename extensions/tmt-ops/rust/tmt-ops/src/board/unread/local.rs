//! Machine-local manual unread marks. Core acknowledgment never moves backwards.
//! The quiet lane applies deltas to current bytes under the shared Document lock.
use super::{Answer, BodyKind, Page};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    os::unix::fs::DirBuilderExt,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tmt_extension_state::Document;

pub(in crate::board) const MAX_MARKS: usize = 500;
const MAX_BYTES: u64 = 1024 * 1024;
const RETRY: &str = "Press r on the answer again, or repeat Mark all read.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Marks(BTreeMap<(String, String), Answer>);
impl Marks {
    fn key(answer: &Answer) -> (String, String) {
        (answer.member.clone(), answer.request.clone())
    }
    fn fresher(next: &Answer, held: &Answer) -> bool {
        next.revision > held.revision
            || next.revision == held.revision && held.kind.is_none() && next.kind.is_some()
    }
    fn contains(&self, answer: &Answer) -> bool {
        self.0.contains_key(&Self::key(answer))
    }
    fn decode(actor: &str, bytes: Option<&[u8]>) -> Result<Self, String> {
        let Some(bytes) = bytes else {
            return Ok(Self::default());
        };
        let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if value["version"] != 1 || value["actor"].as_str() != Some(actor) {
            return Err("Local unread cache has a different viewer or version".into());
        }
        let items = value["marks"]
            .as_array()
            .filter(|items| items.len() <= MAX_MARKS)
            .ok_or("Local unread cache has invalid marks")?;
        let mut marks = Self::default();
        let mut requests = std::collections::BTreeSet::new();
        for item in items {
            let request = item["request"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or("Invalid local unread request")?;
            let member = item["member"]
                .as_str()
                .filter(|s| crate::config::uuid_like(s))
                .ok_or("Invalid local unread member")?;
            let answer = Answer {
                request: request.into(),
                member: member.into(),
                revision: super::timestamp(&item["revision"])
                    .ok_or("Invalid local unread revision")?,
                answered_at_ms: super::timestamp(&item["answeredAtMs"])
                    .ok_or("Invalid local unread time")?,
                kind: match item["kind"].as_str() {
                    Some("final") => Some(BodyKind::Final),
                    Some("turn") => Some(BodyKind::Turn),
                    Some("unknown") => None,
                    _ => return Err("Invalid local unread kind".into()),
                },
                // The cache carries no Core acknowledgment evidence. A successful
                // body draw may offer the exact revision idempotently; the manual
                // mark still requires an explicit read to clear it.
                acknowledged: false,
            };
            if !requests.insert(answer.request.clone()) {
                return Err("Duplicate local unread request".into());
            }
            marks.0.insert(Self::key(&answer), answer);
        }
        Ok(marks)
    }
    fn bytes(&self, actor: &str) -> Vec<u8> {
        serde_json::to_vec(
            &json!({"version":1,"actor":actor,"marks":self.0.values().map(|a| {
            json!({"member":a.member,"request":a.request,"revision":a.revision,
                "answeredAtMs":a.answered_at_ms,"kind":match a.kind {
                    Some(BodyKind::Final)=>"final",Some(BodyKind::Turn)=>"turn",None=>"unknown"
                }})
        }).collect::<Vec<_>>()}),
        )
        .expect("local unread metadata is JSON")
    }
    fn apply(&mut self, change: &Change) -> Result<(), String> {
        match change {
            Change::Put(answer) => {
                let key = Self::key(answer);
                if !self.0.contains_key(&key) && self.0.len() >= MAX_MARKS {
                    return Err("500 marks full; read one.".into());
                }
                if self
                    .0
                    .get(&key)
                    .is_none_or(|held| Self::fresher(answer, held))
                {
                    self.0.insert(key, answer.clone());
                }
            }
            Change::Remove(answers) => {
                for answer in answers {
                    let key = Self::key(answer);
                    if self
                        .0
                        .get(&key)
                        .is_some_and(|held| held.revision <= answer.revision)
                    {
                        self.0.remove(&key);
                    }
                }
            }
            Change::Refresh(answers) => {
                for answer in answers {
                    if let Some(held) = self.0.get_mut(&Self::key(answer))
                        && Self::fresher(answer, held)
                    {
                        *held = answer.clone();
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum Change {
    Put(Answer),
    Remove(Vec<Answer>),
    Refresh(Vec<Answer>),
}

#[derive(Debug, Default)]
struct Viewer {
    saved: Marks,
    shown: Marks,
    pending: VecDeque<Change>,
    loaded: bool,
    queued: bool,
    failed: Option<String>,
    /// Only explicit read intent may clear a mark on a later Core completion.
    reading: BTreeMap<String, u64>,
}
impl Viewer {
    fn project(&mut self) {
        self.shown = self.saved.clone();
        for change in &self.pending {
            // Capacity is checked at admission and again against locked disk bytes.
            let _ = self.shown.apply(change);
        }
    }
}

#[derive(Debug, Default)]
struct Shared {
    viewers: Mutex<BTreeMap<String, Viewer>>,
    any: AtomicBool,
    started: AtomicBool,
}

#[derive(Clone, Debug, Default)]
pub(in crate::board) struct State(Arc<Shared>);
impl State {
    /// Only the interactive entry admits cold-cache I/O; pure App constructors do not.
    pub fn start(&self) {
        self.0.started.store(true, Ordering::Relaxed);
    }
    pub fn has_marks(&self, actor: &str) -> bool {
        self.0.any.load(Ordering::Relaxed)
            && self
                .0
                .viewers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(actor)
                .is_some_and(|viewer| !viewer.shown.0.is_empty())
    }
    pub fn contains(&self, actor: &str, answer: &Answer) -> bool {
        if !self.0.any.load(Ordering::Relaxed) {
            return false;
        }
        self.0
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(actor)
            .is_some_and(|viewer| viewer.shown.contains(answer))
    }
    pub fn answers(&self, actor: &str, member: &str) -> Vec<Answer> {
        if !self.0.any.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let viewers = self.0.viewers.lock().unwrap_or_else(|e| e.into_inner());
        viewers.get(actor).map_or_else(Vec::new, |viewer| {
            viewer
                .shown
                .0
                .range((member.to_owned(), String::new())..)
                .take_while(|((held, _), _)| held == member)
                .map(|(_, answer)| answer.clone())
                .collect()
        })
    }
    pub fn members(&self, actor: &str) -> Vec<String> {
        if !self.0.any.load(Ordering::Relaxed) {
            return Vec::new();
        }
        self.0
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(actor)
            .map_or_else(Vec::new, |viewer| {
                viewer
                    .shown
                    .0
                    .keys()
                    .map(|(member, _)| member.clone())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
    }
    fn change(&self, actor: &str, change: Change) -> Result<(), String> {
        let mut viewers = self.0.viewers.lock().unwrap_or_else(|e| e.into_inner());
        let viewer = viewers.entry(actor.into()).or_default();
        if viewer.pending.len() >= MAX_MARKS * 2 {
            return Err("Wait for the pending local unread changes, then retry.".into());
        }
        viewer.shown.apply(&change)?;
        viewer.pending.push_back(change);
        viewer.failed = None;
        self.0.any.store(
            viewers.values().any(|v| !v.shown.0.is_empty()),
            Ordering::Relaxed,
        );
        Ok(())
    }
    pub fn mark(&self, actor: &str, answer: &Answer) -> Result<(), String> {
        self.change(actor, Change::Put(answer.clone()))
    }
    pub fn remove(&self, actor: &str, answers: Vec<Answer>) -> Result<(), String> {
        let answers: Vec<_> = answers
            .into_iter()
            .filter(|answer| self.contains(actor, answer))
            .collect();
        if answers.is_empty() {
            return Ok(());
        }
        self.change(actor, Change::Remove(answers))
    }
    pub fn read_after_ack(&self, actor: &str, answer: &Answer) {
        self.0
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(actor.into())
            .or_default()
            .reading
            .insert(answer.request.clone(), answer.revision);
    }
    pub fn completed(&self, completed: &super::publish::Completed) -> Result<(), String> {
        let mut removed = Vec::new();
        {
            let mut viewers = self.0.viewers.lock().unwrap_or_else(|e| e.into_inner());
            let Some(viewer) = viewers.get_mut(&completed.actor) else {
                return Ok(());
            };
            for (answer, outcome) in &completed.items {
                let explicit = viewer.reading.get(&answer.request) == Some(&answer.revision);
                match outcome {
                    super::publish::Outcome::NotFound => removed.push(answer.clone()),
                    super::publish::Outcome::Acknowledged if explicit => {
                        removed.push(answer.clone())
                    }
                    _ => {}
                }
                if explicit {
                    viewer.reading.remove(&answer.request);
                }
            }
        }
        self.remove(&completed.actor, removed)
    }
    pub fn observe(&self, page: &Page) {
        if !self.0.any.load(Ordering::Relaxed) {
            return;
        }
        let changed = {
            let viewers = self.0.viewers.lock().unwrap_or_else(|e| e.into_inner());
            viewers.get(&page.actor).map_or_else(Vec::new, |viewer| {
                page.replies
                    .values()
                    .flatten()
                    .filter(|answer| {
                        viewer
                            .shown
                            .0
                            .get(&Marks::key(answer))
                            .is_some_and(|old| Marks::fresher(answer, old))
                    })
                    .cloned()
                    .collect()
            })
        };
        if !changed.is_empty() {
            // Metadata is advisory; an error does not remove the manual mark.
            let _ = self.change(&page.actor, Change::Refresh(changed));
        }
    }
    /// Load the active viewer once; all subsequent work is user-admitted deltas.
    pub fn offer(&self, actor: Option<&str>) -> Option<Task> {
        let mut viewers = self.0.viewers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(actor) = actor {
            viewers.entry(actor.into()).or_default();
        }
        let (actor, viewer) = viewers.iter_mut().find(|(id, viewer)| {
            !viewer.queued
                && viewer.failed.is_none()
                && (!viewer.pending.is_empty()
                    || !viewer.loaded
                        && self.0.started.load(Ordering::Relaxed)
                        && Some(id.as_str()) == actor)
        })?;
        viewer.queued = true;
        Some(Task {
            state: self.clone(),
            actor: actor.clone(),
            changes: viewer.pending.iter().cloned().collect(),
        })
    }
    pub fn pending(&self) -> bool {
        self.0
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|viewer| viewer.queued || !viewer.pending.is_empty())
    }
    pub fn finish(&self) -> Result<(), String> {
        while let Some(task) = self.offer(None) {
            task.publish()?;
        }
        if let Some(cause) = self
            .0
            .viewers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .find_map(|viewer| viewer.failed.clone())
        {
            return Err(format!("Local unread update failed: {cause} {RETRY}"));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(in crate::board) struct Task {
    state: State,
    actor: String,
    changes: Vec<Change>,
}
impl Task {
    pub fn actor(&self) -> &str {
        &self.actor
    }
    pub fn publish(self) -> Result<(), String> {
        let result = crate::cache::directory("unread")
            .ok_or("No Ops cache directory".to_owned())
            .and_then(|root| self.read_update(&root));
        self.settle(result)
    }
    fn settle(self, result: Result<Marks, String>) -> Result<(), String> {
        {
            let mut viewers = self
                .state
                .0
                .viewers
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let viewer = viewers.get_mut(&self.actor).expect("task viewer retained");
            viewer.pending.drain(..self.changes.len());
            match &result {
                Ok(marks) => {
                    viewer.saved = marks.clone();
                    viewer.loaded = true;
                }
                Err(cause) => viewer.failed = Some(cause.clone()),
            }
            viewer.queued = false;
            viewer.project();
            self.state.0.any.store(
                viewers.values().any(|v| !v.shown.0.is_empty()),
                Ordering::Relaxed,
            );
        }
        result
            .map(|_| ())
            .map_err(|cause| format!("Local unread update failed: {cause} {RETRY}"))
    }
    fn read_update(&self, root: &Path) -> Result<Marks, String> {
        if !crate::config::uuid_like(&self.actor) {
            return Err("Invalid local unread viewer".into());
        }
        let path = root.join(format!("{}.json", self.actor));
        let document = Document::new(root, &path, MAX_BYTES, RETRY).map_err(|e| e.to_string())?;
        if self.changes.is_empty() {
            return Marks::decode(
                &self.actor,
                document.read().map_err(|e| e.to_string())?.as_deref(),
            );
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)
            .map_err(|e| e.to_string())?;
        let mut published = None;
        document
            .update(|bytes| {
                let mut marks = Marks::decode(&self.actor, bytes)?;
                for change in &self.changes {
                    marks.apply(change)?;
                }
                let bytes = marks.bytes(&self.actor);
                published = Some(marks);
                Ok::<_, String>(bytes)
            })
            .map_err(|e| match e {
                tmt_extension_state::UpdateError::Change(e) => e,
                tmt_extension_state::UpdateError::State(e) => e.to_string(),
            })?;
        Ok(published.expect("successful publication ran the change once"))
    }
}

#[cfg(test)]
mod tests;
