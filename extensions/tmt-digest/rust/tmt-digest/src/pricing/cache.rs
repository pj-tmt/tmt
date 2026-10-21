//! The last valid fetched price file and the time of the last attempt, private
//! under Digest's state directory. Untrusted on read: any doubt means no cache.

use super::{ATTEMPT_EVERY_MS, Table};
use crate::{history::MAX, private};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

const FILE: &str = "fetched-v1.json";
const MAX_BYTES: u64 = 64 * 1024;
/// A temporary file this old belongs to no live writer.
const STALE_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub attempted_at_ms: u64,
    pub fetched_at_ms: Option<u64>,
    pub file: Option<Table>,
}

/// The file's shape; `version` is the cache's own, not the price file's.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Stored {
    version: u8,
    attempted_at_ms: u64,
    fetched_at_ms: Option<u64>,
    file: Option<Table>,
}

impl State {
    /// Whether the 30-second spacing between attempts has passed. A clock that
    /// moved back allows an attempt.
    pub fn due(&self, now: u64) -> bool {
        self.attempted_at_ms > now || now - self.attempted_at_ms >= ATTEMPT_EVERY_MS
    }

    fn valid(&self) -> bool {
        self.attempted_at_ms <= MAX
            && self.fetched_at_ms.is_none_or(|time| time <= MAX)
            && self.fetched_at_ms.is_some() == self.file.is_some()
            && self.file.as_ref().is_none_or(Table::valid)
    }
}

fn directory(root: &Path) -> PathBuf {
    root.join("digest").join("pricing")
}

pub fn read(root: &Path) -> Option<State> {
    let directory = directory(root);
    let path = directory.join(FILE);
    for parent in [root.join("digest"), directory] {
        private::is_private(&parent, true).ok().filter(|ok| *ok)?;
    }
    private::is_private(&path, false).ok().filter(|ok| *ok)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&path)
        .ok()?;
    let mut bytes = Vec::new();
    Read::take(&mut file, MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let stored: Stored = serde_json::from_slice(&bytes).ok()?;
    let state = State {
        attempted_at_ms: stored.attempted_at_ms,
        fetched_at_ms: stored.fetched_at_ms,
        file: stored.file,
    };
    (stored.version == 1 && state.valid()).then_some(state)
}

/// Removes the temporary files a writer left behind when the process ended before
/// its fetch did (`status` does not wait for a slow one). Only plain files named
/// exactly like a writer's (`.fetched-v1.json.<pid>`) and older than a minute are
/// taken, so a concurrent writer's file never is. Best effort.
fn remove_stale(directory: &Path) {
    let prefix = format!(".{FILE}.");
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_temporary = name
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()));
        let is_stale = entry.metadata().is_ok_and(|metadata| {
            metadata.is_file()
                && metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age >= STALE_AFTER)
        });
        if is_temporary && is_stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Replaces the cache atomically: a private temporary file, then a rename.
pub fn write(root: &Path, state: &State) -> io::Result<()> {
    let directory = directory(root);
    for parent in [root.join("digest"), directory.clone()] {
        if !private::ensure_dir(&parent)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "The price cache directory is not private",
            ));
        }
    }
    remove_stale(&directory);
    let bytes = serde_json::to_vec(&Stored {
        version: 1,
        attempted_at_ms: state.attempted_at_ms,
        fetched_at_ms: state.fetched_at_ms,
        file: state.file.clone(),
    })
    .map_err(io::Error::other)?;
    let temporary = directory.join(format!(".{FILE}.{}", std::process::id()));
    let _ = fs::remove_file(&temporary);
    let staged = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, directory.join(FILE)));
    if staged.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    staged?;
    File::open(&directory)?.sync_all()
}
