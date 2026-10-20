//! Bounded display descriptions and demand-driven, serve-local composition. Grants nothing.
use crate::limits;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Instant,
};

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Rail {
    label: String,
    icon: String,
    path: String,
    order: u16,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Count {
    value: u16,
    kind: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Menu {
    id: String,
    label: String,
    path: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    group: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    count: Option<Count>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SettingsLink {
    id: String,
    label: String,
    path: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Home {
    id: String,
    title: String,
    path: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Description {
    #[serde(skip_serializing)]
    version: u8,
    rail: Rail,
    menu: Vec<Menu>,
    settings: Vec<SettingsLink>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    needs_you: Option<u16>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    home: Option<Vec<Home>>,
}
fn label(s: &str, cap: usize) -> bool {
    (1..=cap).contains(&s.chars().count())
        && !s.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
}
fn id(s: &str) -> bool {
    (1..=limits::DESCRIBE_ID).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn path(s: &str) -> bool {
    s.starts_with('/')
        && s.len() <= limits::DESCRIBE_PATH
        && !s.contains("//")
        && !s.split('/').any(|s| s == "..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'/' | b'-'))
}
fn unique<'a>(ids: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = HashSet::new();
    ids.into_iter().all(|s| id(s) && seen.insert(s))
}
impl Description {
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > limits::DESCRIBE_BYTES {
            return None;
        }
        let mut d: Self = serde_json::from_slice(bytes).ok()?;
        if d.version != 1
            || !label(&d.rail.label, limits::DESCRIBE_RAIL_LABEL)
            || !path(&d.rail.path)
            || d.rail.order > limits::DESCRIBE_ORDER
            || d.needs_you.is_some_and(|n| n > limits::DESCRIBE_COUNT)
            || d.menu.len() > limits::DESCRIBE_MENU
            || d.settings.len() > limits::DESCRIBE_SETTINGS
            || d.home
                .as_ref()
                .is_some_and(|h| h.len() > limits::DESCRIBE_HOME)
            || !unique(d.menu.iter().map(|m| m.id.as_str()))
            || !unique(d.settings.iter().map(|m| m.id.as_str()))
            || !unique(d.home.iter().flatten().map(|m| m.id.as_str()))
            || d.menu.iter().any(|m| {
                !label(&m.label, limits::DESCRIBE_LABEL)
                    || !path(&m.path)
                    || m.group
                        .as_ref()
                        .is_some_and(|g| !label(g, limits::DESCRIBE_GROUP))
                    || m.count.as_ref().is_some_and(|c| {
                        c.value > limits::DESCRIBE_COUNT
                            || !matches!(c.kind.as_str(), "needs-you" | "plain")
                    })
            })
            || d.settings
                .iter()
                .any(|s| !label(&s.label, limits::DESCRIBE_LABEL) || !path(&s.path))
            || d.home
                .iter()
                .flatten()
                .any(|h| !label(&h.title, limits::DESCRIBE_LABEL) || !path(&h.path))
            || d.rail.icon.is_empty()
            || !d
                .rail
                .icon
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return None;
        }
        if ![
            "house",
            "file-text",
            "monitor-smartphone",
            "users",
            "chart-column",
            "search",
            "settings",
            "box",
        ]
        .contains(&d.rail.icon.as_str())
        {
            d.rail.icon = "box".into();
        }
        Some(d)
    }
    fn module(mut self, extension: &str, base: &str) -> Value {
        let resolve = |p: &mut String| *p = format!("{}{p}", base.trim_end_matches('/'));
        resolve(&mut self.rail.path);
        for m in &mut self.menu {
            resolve(&mut m.path);
        }
        for s in &mut self.settings {
            resolve(&mut s.path);
        }
        for h in self.home.iter_mut().flatten() {
            resolve(&mut h.path);
        }
        let mut v = serde_json::to_value(self).expect("bounded display fields");
        v["extension"] = json!(extension);
        v
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Source {
    pub name: &'static str,
    pub socket: PathBuf,
    pub identity: Option<(u64, u64)>,
    pub host: String,
    pub mount: String,
}
struct Entry {
    source: Source,
    running: Option<Source>,
    good: Option<(Description, Instant, Source)>,
}
#[derive(Default)]
struct State {
    closed: bool,
    entries: BTreeMap<&'static str, Entry>,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
type Reader = dyn Fn(&Source) -> Option<Description> + Send + Sync;
/// Owns every finite describe worker. Only demand starts work; shutdown joins it.
pub(crate) struct Shell {
    shared: Arc<Shared>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    read: Arc<Reader>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}
impl Shell {
    pub(crate) fn new() -> Self {
        Self::with_reader(Arc::new(crate::mount::describe), Arc::new(Instant::now))
    }
    fn with_reader(read: Arc<Reader>, clock: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        Self {
            shared: Arc::default(),
            workers: Mutex::default(),
            read,
            clock,
        }
    }
    pub(crate) fn snapshot(&self, sources: Vec<Source>, origin: &str, deadline: Instant) -> Value {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return json!({"version":1,"modules":[]});
        }
        state
            .entries
            .retain(|name, _| sources.iter().any(|s| s.name == *name));
        let now = (self.clock)();
        for source in sources {
            let entry = state.entries.entry(source.name).or_insert_with(|| Entry {
                source: source.clone(),
                running: None,
                good: None,
            });
            if source.identity.is_some()
                && entry
                    .good
                    .as_ref()
                    .is_some_and(|(_, _, old)| old != &source)
            {
                entry.good = None;
            }
            entry.source = source.clone();
            if entry.running.is_some()
                || entry.good.as_ref().is_some_and(|(_, at, _)| {
                    now.saturating_duration_since(*at) < limits::DESCRIBE_FRESH
                })
            {
                continue;
            }
            entry.running = Some(source.clone());
            let shared = Arc::clone(&self.shared);
            let read = Arc::clone(&self.read);
            let clock = Arc::clone(&self.clock);
            let name = source.name;
            let spawned = thread::Builder::new()
                .name("remote-describe".into())
                .spawn(move || {
                    let answer = read(&source);
                    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                    if !state.closed
                        && let Some(entry) = state.entries.get_mut(name)
                        && entry.running.as_ref() == Some(&source)
                    {
                        entry.running = None;
                        if entry.source == source
                            && let Some(answer) = answer
                        {
                            entry.good = Some((answer, clock(), source));
                        }
                    }
                    shared.changed.notify_all();
                });
            if let Ok(worker) = spawned {
                let mut workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
                let mut i = 0;
                while i < workers.len() {
                    if workers[i].is_finished() {
                        let _ = workers.swap_remove(i).join();
                    } else {
                        i += 1;
                    }
                }
                workers.push(worker);
            } else {
                entry.running = None;
            }
        }
        while state.entries.values().any(|e| e.running.is_some()) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            let (next, timed) = self
                .shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner());
            state = next;
            if state.closed || timed.timed_out() {
                break;
            }
        }
        if state.closed {
            return json!({"version":1,"modules":[]});
        }
        let now = (self.clock)();
        let mut modules = vec![(
            200,
            "remote",
            json!({"extension":"remote","rail":{"label":"Remote","icon":"monitor-smartphone","path":format!("{origin}/remote/"),"order":200},"menu":[{"id":"remote","label":"Remote","path":format!("{origin}/remote/")}],"settings":[{"id":"settings","label":"Remote settings","path":format!("{origin}/remote/settings")}],"needsYou":0}),
        )];
        for (name, entry) in &state.entries {
            if let Some((answer, at, _)) = &entry.good
                && now.saturating_duration_since(*at) <= limits::DESCRIBE_STALE
            {
                modules.push((
                    answer.rail.order,
                    *name,
                    answer
                        .clone()
                        .module(name, &format!("{origin}{}", entry.source.mount)),
                ));
            }
        }
        modules.sort_by_key(|(order, name, _)| (*order, *name));
        json!({"version":1,"modules":modules.into_iter().map(|(_,_,v)|v).collect::<Vec<_>>()})
    }
    pub(crate) fn shutdown(&self) {
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            self.shared.changed.notify_all();
        }
        let workers = std::mem::take(&mut *self.workers.lock().unwrap_or_else(|e| e.into_inner()));
        for worker in workers {
            let _ = worker.join();
        }
    }
}
impl Drop for Shell {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
