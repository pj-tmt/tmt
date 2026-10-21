//! Immutable recovery bytes precede the atomic catalogue commit. Pruning follows it.
use super::{decode, saved};
use crate::bounded_file;
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use tmt_core::{
    content_digest::sha256,
    workspace::{
        MAX_BYTES, WorkspaceSnapshot,
        generations::{Catalogue, GenerationId, WorkspaceGeneration},
    },
};

const INDEX: &str = "generations.json";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u64,
    protected: Option<String>,
    entries: Vec<Entry>,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    #[serde(rename = "capturedAtMs")]
    captured_at_ms: u64,
    panes: usize,
    agents: usize,
    shrink: bool,
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Malformed or unsupported workspace generations.",
    )
}
pub fn path(directory: &Path, id: &GenerationId) -> PathBuf {
    directory.join(format!("auto-{}.json", id.digest()))
}
fn owned_file(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(invalid());
    }
    Ok(())
}
pub fn read(directory: &Path) -> io::Result<Option<Catalogue>> {
    let filename = directory.join(INDEX);
    match owned_file(&filename) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        other => other?,
    }
    let bytes = bounded_file::read_no_follow(&filename, MAX_BYTES).map_err(io::Error::other)?;
    let value = crate::json_document::parse(std::str::from_utf8(&bytes).map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    let index: Index = serde_json::from_value(value).map_err(|_| invalid())?;
    if index.version != 1 {
        return Err(invalid());
    }
    let catalogue = Catalogue {
        protected: index
            .protected
            .map(|id| id.parse())
            .transpose()
            .map_err(|_| invalid())?,
        entries: index
            .entries
            .into_iter()
            .map(|entry| {
                Ok(WorkspaceGeneration {
                    id: entry.id.parse().map_err(|_| invalid())?,
                    captured_at_ms: entry.captured_at_ms,
                    panes: entry.panes,
                    agents: entry.agents,
                    shrink: entry.shrink,
                })
            })
            .collect::<io::Result<_>>()?,
    };
    if !catalogue.is_valid() || catalogue.entries.is_empty() {
        return Err(invalid());
    }
    Ok(Some(catalogue))
}
fn index_bytes(catalogue: &Catalogue) -> io::Result<Vec<u8>> {
    serde_json::to_vec(&Index {
        version: 1,
        protected: catalogue
            .protected
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        entries: catalogue
            .entries
            .iter()
            .map(|entry| Entry {
                id: entry.id.as_str().to_owned(),
                captured_at_ms: entry.captured_at_ms,
                panes: entry.panes,
                agents: entry.agents,
                shrink: entry.shrink,
            })
            .collect(),
    })
    .map_err(io::Error::other)
}

/// A completed explicit restore may lower its own observed baseline. A capture
/// that changed that baseline while restore ran wins instead.
pub(super) fn accept(
    directory: &Path,
    selected: &GenerationId,
    expected_protected: Option<&GenerationId>,
) -> io::Result<bool> {
    let Some(mut catalogue) = read(directory)? else {
        return Ok(false);
    };
    if catalogue.protected.as_ref() != expected_protected || catalogue.get(selected).is_none() {
        return Ok(false);
    }
    if catalogue.protected.as_ref() == Some(selected) {
        return Ok(true);
    }
    catalogue.protected = Some(selected.clone());
    crate::private_file::replace(&directory.join(INDEX), &index_bytes(&catalogue)?)?;
    Ok(true)
}
pub fn acquire(directory: &Path, entry: &WorkspaceGeneration) -> io::Result<Vec<u8>> {
    let filename = path(directory, &entry.id);
    owned_file(&filename)?;
    let bytes = bounded_file::read_no_follow(&filename, MAX_BYTES).map_err(io::Error::other)?;
    let snapshot = decode(&bytes)?;
    if sha256(&bytes) != entry.id.digest()
        || record(&snapshot, &bytes)
            != (WorkspaceGeneration {
                shrink: false,
                ..entry.clone()
            })
    {
        return Err(invalid());
    }
    Ok(bytes)
}
fn record(snapshot: &WorkspaceSnapshot, bytes: &[u8]) -> WorkspaceGeneration {
    WorkspaceGeneration {
        id: format!("g:{}", sha256(bytes))
            .parse()
            .expect("sha256 selector"),
        captured_at_ms: snapshot.captured_at_ms,
        panes: snapshot.panes.len(),
        agents: snapshot.agent_count(),
        shrink: false,
    }
}
fn retain_bytes(
    directory: &Path,
    snapshot: &WorkspaceSnapshot,
    bytes: &[u8],
) -> io::Result<WorkspaceGeneration> {
    let entry = record(snapshot, bytes);
    let filename = path(directory, &entry.id);
    match saved::publish_new(&filename, bytes) {
        Ok(()) => (),
        Err(saved::Error::Exists) if acquire(directory, &entry)? == bytes => (),
        Err(error) => return Err(io::Error::other(error)),
    }
    Ok(entry)
}

/// Caller owns publication.lock. The catalogue is the one authoritative pointer;
/// latest.json remains a complete legacy projection, never the retention owner.
pub(super) fn publish(
    directory: &Path,
    previous: Option<&[u8]>,
    snapshot: &WorkspaceSnapshot,
    bytes: &[u8],
    catalogue: Option<Catalogue>,
) -> io::Result<()> {
    publish_with(directory, previous, snapshot, bytes, catalogue, |bytes| {
        crate::private_file::replace(&directory.join(INDEX), bytes)
    })
}
fn publish_with(
    directory: &Path,
    previous: Option<&[u8]>,
    snapshot: &WorkspaceSnapshot,
    bytes: &[u8],
    catalogue: Option<Catalogue>,
    commit: impl FnOnce(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut catalogue = match catalogue {
        Some(catalogue) => {
            // Never replace a corrupt/missing retained generation or forget its pin.
            for entry in &catalogue.entries {
                owned_file(&path(directory, &entry.id))?;
            }
            if let Some(protected) = catalogue.protected() {
                acquire(directory, protected)?;
            }
            catalogue
        }
        None => {
            let mut catalogue = Catalogue::default();
            if let Some(bytes) = previous {
                catalogue.append(retain_bytes(directory, &decode(bytes)?, bytes)?);
            }
            catalogue
        }
    };
    let entry = retain_bytes(directory, snapshot, bytes)?;
    catalogue.append(entry);
    commit(&index_bytes(&catalogue)?)?;
    crate::private_file::replace(&directory.join("latest.json"), bytes)?;
    prune(directory, &catalogue)
}

/// Only complete, owned files with an exact content-addressed automatic name.
/// A crash orphan is safe to remove only after a valid index is committed.
fn prune(directory: &Path, catalogue: &Catalogue) -> io::Result<()> {
    for (index, entry) in fs::read_dir(directory)?.enumerate() {
        if index >= 256 {
            return Err(invalid());
        }
        let entry = entry?;
        let name = entry.file_name();
        let Some(digest) = name
            .to_str()
            .and_then(|name| name.strip_prefix("auto-"))
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let Ok(id) = format!("g:{digest}").parse::<GenerationId>() else {
            continue;
        };
        if catalogue.get(&id).is_some() || owned_file(&entry.path()).is_err() {
            continue;
        }
        let Ok(bytes) = bounded_file::read_no_follow(&entry.path(), MAX_BYTES) else {
            continue;
        };
        if sha256(&bytes) == digest && decode(&bytes).is_ok() {
            fs::remove_file(entry.path())?;
        }
    }
    fs::File::open(directory)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::super::encode;
    use super::*;
    #[test]
    fn existing_v1_history_stays_readable_and_trims_only_on_changed_publication() {
        use std::os::unix::fs::MetadataExt;
        let root = crate::test_support::TestDirectory::new();
        let paths = super::super::tests::paths(&root.path);
        let mut protected = super::super::tests::sample();
        protected.panes[0].identity = Some(tmt_core::workspace::WorkspaceIdentity {
            id: "11111111-1111-4111-8111-111111111111".into(),
            name: "retained".into(),
            lifetime: "saved".into(),
            binding: "22222222-2222-4222-8222-222222222222".into(),
            harness: None,
            session: None,
            mode: None,
            channel: None,
        });
        let socket = protected.server.socket.clone();
        assert!(super::super::publish_event(&paths, &socket, || Ok(protected.clone())).unwrap());
        let directory = paths.workspace_directory(&socket);
        let protected_bytes = encode(&protected).unwrap();
        let protected_entry = record(&protected, &protected_bytes);
        let mut old = Catalogue {
            protected: Some(protected_entry.id.clone()),
            entries: vec![protected_entry.clone()],
        };
        let mut latest = protected.clone();
        latest.panes[0].identity = None;
        // Frozen v1 shape from the earlier 35-entry policy. Do not run today's
        // retention selector to construct the compatibility input.
        for index in 2..=35 {
            latest.captured_at_ms = index;
            latest.windows[0].name = format!("old-layout-{index}");
            let mut entry = retain_bytes(&directory, &latest, &encode(&latest).unwrap()).unwrap();
            entry.shrink = true;
            old.entries.push(entry);
        }
        crate::private_file::replace(&directory.join(INDEX), &index_bytes(&old).unwrap()).unwrap();
        crate::private_file::replace(&directory.join("latest.json"), &encode(&latest).unwrap())
            .unwrap();
        let inventory = || {
            let mut files = fs::read_dir(&directory)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    let metadata = entry.metadata().unwrap();
                    (
                        entry.file_name(),
                        fs::read(entry.path()).unwrap(),
                        metadata.ino(),
                        metadata.mtime(),
                        metadata.mtime_nsec(),
                    )
                })
                .collect::<Vec<_>>();
            files.sort_by(|left, right| left.0.cmp(&right.0));
            files
        };
        let before = inventory();
        let admitted = read(&directory).unwrap().unwrap();
        assert_eq!(admitted.entries.len(), 35);
        assert_eq!(admitted.preferred(), Some(&protected_entry));
        assert_eq!(
            acquire(&directory, admitted.protected().unwrap()).unwrap(),
            protected_bytes
        );
        assert_eq!(
            inventory(),
            before,
            "reading old history must not rewrite it"
        );
        latest.captured_at_ms = 36;
        assert!(!super::super::publish_event(&paths, &socket, || Ok(latest.clone())).unwrap());
        assert_eq!(
            inventory(),
            before,
            "unchanged capture must not migrate history"
        );
        latest.windows[0].name = "changed after upgrade".into();
        assert!(super::super::publish_event(&paths, &socket, || Ok(latest.clone())).unwrap());
        let trimmed = read(&directory).unwrap().unwrap();
        assert_eq!(trimmed.entries.len(), 6);
        assert_eq!(trimmed.preferred(), Some(&protected_entry));
        assert_eq!(
            acquire(&directory, trimmed.protected().unwrap()).unwrap(),
            protected_bytes
        );
        for entry in &trimmed.entries {
            acquire(&directory, entry).unwrap();
        }
        let automatic = fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("auto-"))
            .count();
        assert_eq!(automatic, 6);
        assert_eq!(
            acquire(&directory, trimmed.latest().unwrap()).unwrap(),
            encode(&latest).unwrap()
        );
    }
    #[test]
    fn failed_index_commit_keeps_old_index_and_complete_generation() {
        let root = crate::test_support::TestDirectory::new();
        let directory = &root.path;
        let old = super::super::tests::sample();
        let bytes = encode(&old).unwrap();
        publish(directory, None, &old, &bytes, None).unwrap();
        let index = fs::read(directory.join(INDEX)).unwrap();
        let mut next = old.clone();
        next.windows[0].name = "new layout".into();
        next.captured_at_ms = 2;
        assert!(
            publish_with(
                directory,
                Some(&bytes),
                &next,
                &encode(&next).unwrap(),
                read(directory).unwrap(),
                |_| Err(io::Error::other("crash before index rename"))
            )
            .is_err()
        );
        assert_eq!(fs::read(directory.join(INDEX)).unwrap(), index);
        let current = read(directory).unwrap().unwrap();
        assert_eq!(
            acquire(directory, current.latest().unwrap()).unwrap(),
            bytes
        );
        assert_eq!(fs::read(directory.join("latest.json")).unwrap(), bytes);
        // The complete unreferenced generation is recovered on the next update.
        next.windows[0].name = "later".into();
        next.captured_at_ms = 3;
        publish(
            directory,
            Some(&bytes),
            &next,
            &encode(&next).unwrap(),
            Some(current),
        )
        .unwrap();
        assert_eq!(
            fs::read_dir(directory)
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("auto-"))
                .count(),
            2
        );
    }
}

#[cfg(test)]
mod crash_tests {
    use super::super::encode;
    use super::*;
    use crate::test_support::{TestChild, TestDirectory};
    use std::{
        io::{Read, Write},
        os::unix::fs::OpenOptionsExt,
        process::{Command, Stdio},
        time::Duration,
    };
    const ROOT: &str = "TMT_WORKSPACE_CRASH_ROOT";
    const MODE: &str = "TMT_WORKSPACE_CRASH_MODE";
    fn barrier(directory: &Path) {
        fs::write(directory.join("ready"), "ready").unwrap();
        // The parent SIGKILLs only this invocation-owned, unreaped child. No
        // timer or background publisher can complete work after that barrier.
        let mut input = [0];
        std::io::stdin().read_exact(&mut input).unwrap();
        panic!("crash fixture was released instead of killed");
    }
    #[test]
    fn publication_crash_child() {
        let Some(directory) = std::env::var_os(ROOT).map(PathBuf::from) else {
            return;
        };
        let old = super::super::tests::sample();
        let previous = encode(&old).unwrap();
        let mut next = old.clone();
        next.captured_at_ms = 2;
        next.windows[0].name = "after crash boundary".into();
        let bytes = encode(&next).unwrap();
        let mode = std::env::var(MODE).unwrap();
        if mode == "torn-generation" {
            let mut stage = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(directory.join(".auto-crash-stage.tmp"))
                .unwrap();
            stage.write_all(&bytes[..bytes.len() / 2]).unwrap();
            stage.sync_all().unwrap();
            barrier(&directory);
        }
        publish_with(
            &directory,
            Some(&previous),
            &next,
            &bytes,
            read(&directory).unwrap(),
            |index| {
                if mode == "torn-index" {
                    let mut stage = fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .mode(0o600)
                        .open(directory.join(".generations-crash-stage.tmp"))
                        .unwrap();
                    stage.write_all(&index[..index.len() / 2]).unwrap();
                    stage.sync_all().unwrap();
                } else if mode == "committed-index" {
                    crate::private_file::replace(&directory.join(INDEX), index).unwrap();
                } else {
                    assert_eq!(mode, "complete-generation");
                }
                barrier(&directory);
                unreachable!()
            },
        )
        .unwrap();
    }
    #[test]
    fn sigkill_at_publication_boundaries_never_replaces_good_bytes_with_a_torn_document() {
        for mode in [
            "torn-generation",
            "complete-generation",
            "torn-index",
            "committed-index",
        ] {
            let root = TestDirectory::new();
            let old = super::super::tests::sample();
            let bytes = encode(&old).unwrap();
            publish(&root.path, None, &old, &bytes, None).unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "workspace::generations::crash_tests::publication_crash_child",
                    "--nocapture",
                ])
                .env_remove("TMUX")
                .env_remove("TMUX_PANE")
                .env(ROOT, &root.path)
                .env(MODE, mode)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let mut child = TestChild::new(child);
            child.wait_for_content(&root.path.join("ready"), "ready", Duration::from_secs(5));
            child.child.kill().unwrap();
            let status = child.wait_for_exit(Duration::from_secs(5));
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(status.signal(), Some(9));
            let catalogue = read(&root.path).unwrap().unwrap();
            for entry in &catalogue.entries {
                acquire(&root.path, entry).unwrap();
            }
            let latest =
                decode(&acquire(&root.path, catalogue.latest().unwrap()).unwrap()).unwrap();
            assert_eq!(
                latest.windows[0].name,
                if mode == "committed-index" {
                    "after crash boundary"
                } else {
                    "shell"
                }
            );
            assert_eq!(fs::read(root.path.join("latest.json")).unwrap(), bytes);
            // A second update uses the committed index, not a torn stage or the
            // stale legacy projection, and leaves every retained document valid.
            let mut later = old.clone();
            later.captured_at_ms = 3;
            later.windows[0].name = "recovered".into();
            publish(
                &root.path,
                Some(&bytes),
                &later,
                &encode(&later).unwrap(),
                Some(catalogue),
            )
            .unwrap();
            for entry in &read(&root.path).unwrap().unwrap().entries {
                acquire(&root.path, entry).unwrap();
            }
        }
    }
    #[test]
    fn retention_prunes_only_verified_owned_automatic_documents() {
        use std::os::unix::fs::symlink;
        let root = TestDirectory::new();
        let foreign = root.path.join(format!("auto-{}.json", "a".repeat(64)));
        fs::write(&foreign, "foreign malformed content").unwrap();
        let target = root.path.join("foreign-target");
        fs::write(&target, "leave alone").unwrap();
        let link = root.path.join(format!("auto-{}.json", "b".repeat(64)));
        symlink(&target, &link).unwrap();
        let mut snapshot = super::super::tests::sample();
        for index in 1..=100 {
            snapshot.captured_at_ms = index * 300_000;
            snapshot.windows[0].name = format!("layout-{index}");
            publish(
                &root.path,
                None,
                &snapshot,
                &encode(&snapshot).unwrap(),
                read(&root.path).unwrap(),
            )
            .unwrap();
            let catalogue = read(&root.path).unwrap().unwrap();
            assert!(catalogue.entries.len() <= tmt_core::workspace::generations::MAX_GENERATIONS);
            let automatic = fs::read_dir(&root.path)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.file_name().to_string_lossy().starts_with("auto-")
                        && entry.path() != foreign
                        && entry.path() != link
                })
                .count();
            assert_eq!(automatic, catalogue.entries.len());
            assert!(automatic <= 6);
        }
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign malformed content");
        assert_eq!(fs::read_link(&link).unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), b"leave alone");
    }
}
