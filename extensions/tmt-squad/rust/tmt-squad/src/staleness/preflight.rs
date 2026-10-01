//! An advisory cache-only gate. A candidate is never a claim: the warm path
//! must validate the current data root and roster, then observe under the lock.

use super::{age, cache, digest, read_cache};
use crate::{config::Config, squad::Squad};
use serde_json::Value;
use std::{fs, path::PathBuf, time::Instant};

pub struct Candidate {
    pub config: PathBuf,
    pub squad: Squad,
}

/// Cold, disabled, fresh and already-claimed observations call no core and
/// acquire no room lock. Ordinary ls/board observations prime this cache.
pub fn candidates(lead: &str, now: u64, deadline: Instant) -> Vec<Candidate> {
    candidates_at(cache::directory("staleness"), lead, now, deadline)
}

pub(super) fn candidates_at(
    directory: Option<PathBuf>,
    lead: &str,
    now: u64,
    deadline: Instant,
) -> Vec<Candidate> {
    let mut result = Vec::new();
    let Some(entries) = directory.and_then(|path| fs::read_dir(path).ok()) else {
        return result;
    };
    for entry in entries.take(128) {
        if Instant::now() >= deadline {
            break;
        }
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Some(document) = read_cache(&path) else {
            continue;
        };
        if document["leadId"] != lead || document["observedAtMs"].as_u64().is_none_or(|at| at > now)
        {
            continue;
        }
        let (Some(config), Some(name), Some(room_id)) = (
            document["source"]["config"].as_str(),
            document["source"]["squad"].as_str(),
            document["source"]["roomId"].as_str(),
        ) else {
            continue;
        };
        let config = PathBuf::from(config);
        if !config.is_absolute() || !crate::config::uuid_like(room_id) {
            continue;
        }
        let expected = format!(
            "{}-{room_id}.json",
            digest(config.as_os_str().as_encoded_bytes())
        );
        if path
            .file_name()
            .is_none_or(|name| name != expected.as_str())
        {
            continue;
        }
        if Instant::now() >= deadline {
            break;
        }
        let Ok(settings) = Config::read(config.clone()).and_then(|config| config.reminders(name))
        else {
            continue;
        };
        if !settings.enabled {
            continue;
        }
        let pending = |record: &Value, needs_activity: bool| {
            if record["claimed"] == true {
                return false;
            }
            let Some(since) = record["sinceMs"].as_u64() else {
                return false;
            };
            let observed = age(since, now, settings, &record["reasons"]);
            observed["state"] == "stale"
                && (!needs_activity || observed["activityAfterUpdate"] == true)
        };
        if (document["notes"]["leadId"] == lead && pending(&document["notes"], false))
            || document["members"]
                .as_object()
                .is_some_and(|members| members.values().any(|row| pending(row, true)))
        {
            result.push(Candidate {
                config,
                squad: Squad {
                    name: name.into(),
                    room_id: room_id.into(),
                },
            });
        }
    }
    result.sort_by(|a, b| (&a.config, &a.squad.name).cmp(&(&b.config, &b.squad.name)));
    result
}
