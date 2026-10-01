//! Observed content age, shared by status readers and future reminder context.
//! Core exposes current values, not modification times: first observation is
//! the earliest age we can prove. This cache never stores notebook contents.

use crate::{
    cache,
    config::Reminders,
    provider,
    requests::Window,
    squad::{Member, Squad},
};
use nix::fcntl::{Flock, FlockArg, OFlag};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

mod preflight;
pub use preflight::candidates;

const FILE_LIMIT: u64 = 512 * 1024;
const MEMBERS_LIMIT: usize = 128;
const VERSION: u64 = 1;
/// Unchanged observations advance the persisted rollback watermark at most
/// once per minute. Content/evidence changes always publish immediately.
const WATERMARK_INTERVAL_MS: u64 = 60_000;

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Stable shape even when no observation is possible. Unknown is never stale.
pub fn unavailable(state: &str) -> Value {
    json!({"state": state, "unchangedSinceMs": null, "ageMs": null,
        "activityAfterUpdate": false, "reasons": []})
}

fn age(since: u64, now: u64, settings: Reminders, reasons: &Value) -> Value {
    let Some(age) = now.checked_sub(since) else {
        return unavailable("unknown");
    };
    json!({"state": if age >= settings.stale_after.as_millis() as u64 { "stale" } else { "fresh" },
        "unchangedSinceMs": since, "ageMs": age,
        "activityAfterUpdate": reasons.as_array().is_some_and(|items| !items.is_empty()),
        "reasons": reasons})
}

/// Projection can be applied after providers, column sources and sections have
/// arranged rows. Every occurrence of the same UUID gets the same value.
pub struct Snapshot {
    members: BTreeMap<String, Value>,
    notes: Value,
    fallback: Value,
}

impl Snapshot {
    fn unavailable(state: &str) -> Self {
        Self {
            members: BTreeMap::new(),
            notes: unavailable(state),
            fallback: unavailable(state),
        }
    }

    pub fn apply(&self, document: &mut Value) {
        let apply = |row: &mut Value| {
            if row.is_object() {
                row["staleness"] = row["id"]
                    .as_str()
                    .and_then(|id| self.members.get(id))
                    .unwrap_or(&self.fallback)
                    .clone();
            }
        };
        apply(&mut document["squad"]["lead"]);
        for section in document["sections"].as_array_mut().into_iter().flatten() {
            for row in section["rows"].as_array_mut().into_iter().flatten() {
                apply(row);
            }
        }
        document["squad"]["notesStaleness"] = self.notes.clone();
    }
}

/// Inputs shared by ordinary observation and a reminder claim.
pub struct Input<'a> {
    pub members: &'a [Member],
    pub providers: &'a [provider::Provider],
    pub fields: &'a provider::Cache,
    pub notes: Option<&'a Value>,
    pub room: Option<&'a Window>,
    pub now: u64,
}

/// Exactly the generations committed as claimed, before context handoff.
#[derive(Debug, PartialEq, Eq)]
pub struct Reminder {
    pub members: Vec<String>,
    pub notes: bool,
}

/// Owns one nonblocking lock from before reading the roster through publication.
/// A competing invocation returns unknown, and does no optional core reads.
pub struct Observer {
    settings: Reminders,
    path: Option<PathBuf>,
    _lock: Option<Flock<File>>,
    document: Value,
}

impl Observer {
    pub fn begin(config: &Path, squad: &Squad, settings: Reminders) -> Self {
        Self::at(config, squad, settings, cache::directory("staleness"))
    }

    fn at(config: &Path, squad: &Squad, settings: Reminders, directory: Option<PathBuf>) -> Self {
        let mut observer = Self {
            settings,
            path: None,
            _lock: None,
            document: Value::Null,
        };
        if !settings.enabled {
            return observer;
        }
        let Some(directory) = directory else {
            return observer;
        };
        if !crate::config::uuid_like(&squad.room_id) {
            return observer;
        }
        let scope = digest(config.as_os_str().as_encoded_bytes());
        let path = directory.join(format!("{scope}-{}.json", squad.room_id));
        if fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .is_err()
        {
            return observer;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
            .open(path.with_extension("lock"))
            .ok()
            .filter(|file| file.metadata().is_ok_and(|metadata| metadata.is_file()))
            .and_then(|file| Flock::lock(file, FlockArg::LockExclusiveNonblock).ok());
        let Some(lock) = lock else {
            return observer;
        };
        observer.document =
            read_cache(&path).unwrap_or_else(|| json!({"version": VERSION, "members": {}}));
        observer.document["source"] = json!({
            "config": config.to_str(), "squad": squad.name, "roomId": squad.room_id,
        });
        observer.path = Some(path);
        observer._lock = Some(lock);
        observer
    }

    /// Optional notes/history reads only occur when the cache can be committed.
    pub fn active(&self) -> bool {
        self.path.is_some()
    }

    /// Call after reading raw members under this observer's lock. `notes` is
    /// the public notes.read result, and `room` the existing bounded history.
    /// Caller decides which effects to run; this owner computes and publishes.
    pub fn record(
        self,
        members: &[Member],
        providers: &[provider::Provider],
        fields: &provider::Cache,
        notes: Option<&Value>,
        room: Option<&Window>,
        now: u64,
    ) -> Snapshot {
        self.record_inner(
            Input {
                members,
                providers,
                fields,
                notes,
                room,
                now,
            },
            false,
        )
        .0
    }

    /// The caller has already verified the hook identity; the current roster
    /// must independently establish that it is this squad's only lead.
    pub fn record_for_reminder(self, input: Input<'_>, lead: &str) -> (Snapshot, Option<Reminder>) {
        let mut leaders = input.members.iter().filter(|member| member.is_lead());
        if !leaders.next().is_some_and(|member| member.id == lead) || leaders.next().is_some() {
            return (Snapshot::unavailable("unknown"), None);
        }
        self.record_inner(input, true)
    }

    fn record_inner(mut self, input: Input<'_>, claim: bool) -> (Snapshot, Option<Reminder>) {
        let Input {
            members,
            providers,
            fields,
            notes,
            room,
            now,
        } = input;
        if !self.settings.enabled {
            return (Snapshot::unavailable("disabled"), None);
        }
        let Some(path) = self.path.as_ref() else {
            return (Snapshot::unavailable("unknown"), None);
        };
        if members.len() > MEMBERS_LIMIT {
            return (Snapshot::unavailable("unknown"), None);
        }
        let previous = self.document.clone();
        let rollback = self.document["observedAtMs"]
            .as_u64()
            .is_some_and(|previous| previous > now);
        if rollback {
            self.document["members"] = json!({});
            self.document["notes"] = Value::Null;
        }
        let mut snapshot = Snapshot::unavailable("unknown");
        let mut retained = serde_json::Map::new();
        for member in members {
            let task = member.fields.get("task");
            let state = member.fields.get("state");
            if task.is_none() && state.is_none() {
                continue;
            }
            let fingerprint = digest(json!([task, state]).to_string().as_bytes());
            let old = &self.document["members"][&member.id];
            let same = old["fingerprint"] == fingerprint;
            let since = if same {
                old["sinceMs"]
                    .as_u64()
                    .filter(|since| *since <= now)
                    .unwrap_or(now)
            } else {
                now
            };
            let mut reasons = if same
                && old["reasons"].as_array().is_some_and(|items| {
                    items.len() <= 4
                        && items.iter().all(|reason| {
                            matches!(
                                reason.as_str(),
                                Some(
                                    "pr_link_changed" | "pr_opened" | "pr_merged" | "member_final"
                                )
                            )
                        })
                }) {
                old["reasons"].clone()
            } else {
                json!([])
            };
            let link = member
                .fields
                .get("pr_link")
                .map(|link| digest(link.as_bytes()));
            let previous_link = old["prLink"].as_str();
            if same
                && since < now
                && old.get("prLink").is_some()
                && previous_link != link.as_deref()
            {
                add_reason(&mut reasons, "pr_link_changed");
            }
            let current_states = fields.github_pr_states(providers, member, now);
            let mut states = if same && previous_link == link.as_deref() {
                old["prStates"].as_object().cloned().unwrap_or_default()
            } else {
                serde_json::Map::new()
            };
            for (field, state) in current_states {
                if same
                    && since < now
                    && states
                        .get(&field)
                        .and_then(Value::as_str)
                        .is_some_and(|previous| previous != state)
                {
                    match state.as_str() {
                        "open" => add_reason(&mut reasons, "pr_opened"),
                        "merged" => add_reason(&mut reasons, "pr_merged"),
                        _ => {}
                    }
                }
                states.insert(field, state.into());
            }
            if room.is_some_and(|room| {
                room.items.iter().any(|item| {
                    item["kind"] == "request"
                        && item["recipientId"] == member.id
                        && matches!(
                            item["final"]["status"].as_str(),
                            Some("retained" | "expired" | "unavailable")
                        )
                        && item["final"]["submittedAtMs"]
                            .as_u64()
                            .is_some_and(|at| at > since && at <= now)
                })
            }) {
                add_reason(&mut reasons, "member_final");
            }
            snapshot.members.insert(
                member.id.clone(),
                if rollback || same && old["sinceMs"].as_u64().is_some_and(|since| since > now) {
                    unavailable("unknown")
                } else {
                    age(since, now, self.settings, &reasons)
                },
            );
            retained.insert(
                member.id.clone(),
                json!({"fingerprint": fingerprint, "sinceMs": since,
                "prLink": link, "prStates": states, "reasons": reasons,
                "claimed": same && old["claimed"] == true}),
            );
        }
        self.document["members"] = Value::Object(retained);
        let lead = members.iter().find(|member| member.is_lead());
        self.document["leadId"] = lead.map(|member| member.id.clone()).into();
        if let Some(lead) = lead {
            if let Some(notes) = notes
                .filter(|notes| notes["identityId"] == lead.id)
                .and_then(|notes| notes["content"].as_str())
            {
                let fingerprint = digest(notes.as_bytes());
                let old = &self.document["notes"];
                let same = old["leadId"] == lead.id && old["fingerprint"] == fingerprint;
                let future = same && old["sinceMs"].as_u64().is_some_and(|since| since > now);
                let since = if same {
                    old["sinceMs"]
                        .as_u64()
                        .filter(|since| *since <= now)
                        .unwrap_or(now)
                } else {
                    now
                };
                self.document["notes"] = json!({"leadId": lead.id, "fingerprint": fingerprint, "sinceMs": since,
                        "claimed": same && old["claimed"] == true});
                snapshot.notes = if rollback || future {
                    unavailable("unknown")
                } else {
                    age(since, now, self.settings, &json!([]))
                };
            } else if self.document["notes"]["leadId"] != lead.id {
                self.document["notes"] = Value::Null;
            }
        } else {
            self.document["notes"] = Value::Null;
        }
        let reminder = claim
            .then(|| claim_generations(&mut self.document, &snapshot))
            .flatten();
        let watermark_due = previous["observedAtMs"]
            .as_u64()
            .is_none_or(|at| now.saturating_sub(at) >= WATERMARK_INTERVAL_MS);
        let changed = self.document != previous;
        if !changed && !watermark_due && !rollback {
            return (snapshot, None);
        }
        self.document["observedAtMs"] = now.into();
        let bytes = self.document.to_string();
        if bytes.len() as u64 > FILE_LIMIT {
            return (Snapshot::unavailable("unknown"), None);
        }
        if cache::replace(path, bytes.as_bytes()).is_err() {
            return (Snapshot::unavailable("unknown"), None);
        }
        (snapshot, reminder)
    }
}

fn claim_generations(document: &mut Value, snapshot: &Snapshot) -> Option<Reminder> {
    let mut reminder = Reminder {
        members: Vec::new(),
        notes: false,
    };
    for (id, age) in &snapshot.members {
        if age["state"] == "stale"
            && age["activityAfterUpdate"] == true
            && document["members"][id]["claimed"] != true
        {
            document["members"][id]["claimed"] = true.into();
            reminder.members.push(id.clone());
        }
    }
    // Notes need no activity evidence; stale member rows do.
    if snapshot.notes["state"] == "stale" && document["notes"]["claimed"] != true {
        document["notes"]["claimed"] = true.into();
        reminder.notes = true;
    }
    (!reminder.members.is_empty() || reminder.notes).then_some(reminder)
}

fn add_reason(reasons: &mut Value, reason: &str) {
    let items = reasons.as_array_mut().expect("reasons were validated");
    if !items.iter().any(|item| item == reason) {
        items.push(reason.into());
    }
}

fn read_cache(path: &Path) -> Option<Value> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > FILE_LIMIT {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > FILE_LIMIT {
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    (value["version"] == VERSION && value["members"].is_object()).then_some(value)
}

/// Compact age label used by the text list and the board's marks.
pub fn label(value: &Value) -> Option<String> {
    if value["state"] != "stale" {
        return None;
    }
    let seconds = value["ageMs"].as_u64()? / 1000;
    Some(format!(
        "stale {}",
        if seconds >= 3600 {
            format!("{}h", seconds / 3600)
        } else if seconds >= 60 {
            format!("{}m", seconds / 60)
        } else {
            format!("{seconds}s")
        }
    ))
}

#[cfg(test)]
mod tests;
