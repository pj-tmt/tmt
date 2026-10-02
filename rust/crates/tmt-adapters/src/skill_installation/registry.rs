//! Bounded installer intents for rediscovering custom managed targets.
//! A recorded path never authorizes overwriting it; links must prove ownership.

use super::files;
use crate::bounded_file;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
};

const MAXIMUM_BYTES: usize = 1_048_576;
const MAXIMUM_TARGETS: usize = 4096;

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid or oversized skill installation manifest; preserve it for inspection.",
    )
}

pub(super) fn path(global: &Path) -> PathBuf {
    global.join("skill-installations.json")
}

pub(super) fn read(global: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let file = path(global);
    if !files::exists(&file)? {
        return Ok(BTreeSet::new());
    }
    if !std::fs::symlink_metadata(&file)?.is_file() {
        return Err(invalid());
    }
    let bytes = bounded_file::read(&file, MAXIMUM_BYTES).map_err(|_| invalid())?;
    let document: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if document.as_object().is_none_or(|object| object.len() != 2) || document["version"] != 1 {
        return Err(invalid());
    }
    let entries = document["targets"].as_array().ok_or_else(invalid)?;
    if entries.len() > MAXIMUM_TARGETS {
        return Err(invalid());
    }
    entries
        .iter()
        .map(|value| {
            let entry = PathBuf::from(value.as_str().ok_or_else(invalid)?);
            if !entry.is_absolute() || entry.file_name().is_none() {
                return Err(invalid());
            }
            Ok(entry)
        })
        .collect()
}

/// Persist intent before links so a crash cannot lose a custom target. Later
/// refresh checks the actual link; a failed intent is not automatic authority.
pub(super) fn remember(
    global: &Path,
    targets: impl IntoIterator<Item = PathBuf>,
) -> io::Result<()> {
    let mut entries = read(global)?;
    let before = entries.clone();
    entries.extend(targets);
    if entries == before {
        return Ok(());
    }
    if entries.len() > MAXIMUM_TARGETS {
        return Err(invalid());
    }
    let strings = entries
        .iter()
        .map(|path| {
            if !path.is_absolute() || path.file_name().is_none() {
                return Err(invalid());
            }
            path.to_str().ok_or_else(invalid)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let bytes =
        serde_json::to_vec(&json!({"version": 1, "targets": strings})).map_err(|_| invalid())?;
    if bytes.len() > MAXIMUM_BYTES {
        return Err(invalid());
    }
    files::atomic_write(&path(global), &bytes)
}

/// Retire completed removals, including conflicts the user chose to keep.
/// Their old publication intent must not make future upgrades claim them.
/// Call while holding the skill installation lock.
pub(super) fn forget(global: &Path, targets: &[PathBuf]) -> io::Result<()> {
    let mut entries = read(global)?;
    if targets.iter().all(|target| !entries.contains(target)) {
        return Ok(());
    }
    entries.retain(|target| !targets.contains(target));
    let bytes =
        serde_json::to_vec(&json!({"version": 1, "targets": entries})).map_err(|_| invalid())?;
    files::atomic_write(&path(global), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::fs;

    #[test]
    fn invalid_documents_and_symlink_entries_are_preserved() {
        let root = TestDirectory::new();
        let file = path(&root.path);
        for document in [
            json!(null),
            json!({"version":1,"targets":["relative"]}),
            json!({"version":1,"targets":["/"]}),
            json!({"version":1,"targets":[3]}),
            json!({"version":1,"targets":[],"extra":true}),
        ] {
            let bytes = serde_json::to_vec(&document).unwrap();
            fs::write(&file, &bytes).unwrap();
            assert_eq!(
                read(&root.path).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            assert_eq!(fs::read(&file).unwrap(), bytes);
        }
        fs::remove_file(&file).unwrap();
        let external = root.path.join("external.json");
        fs::write(&external, br#"{"version":1,"targets":[]}"#).unwrap();
        std::os::unix::fs::symlink(&external, &file).unwrap();
        assert_eq!(
            read(&root.path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read_link(&file).unwrap(), external);
        assert_eq!(
            fs::read(external).unwrap(),
            br#"{"version":1,"targets":[]}"#
        );
    }

    #[test]
    fn capacity_and_invalid_new_paths_fail_without_changing_retained_intents() {
        let root = TestDirectory::new();
        remember(
            &root.path,
            (0..MAXIMUM_TARGETS).map(|index| root.path.join(format!("target-{index}"))),
        )
        .unwrap();
        let before = fs::read(path(&root.path)).unwrap();
        assert_eq!(read(&root.path).unwrap().len(), MAXIMUM_TARGETS);
        let error = remember(&root.path, [root.path.join("one-too-many")]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(path(&root.path)).unwrap(), before);
        let empty = TestDirectory::new();
        assert_eq!(
            remember(&empty.path, [PathBuf::from("relative")])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(!path(&empty.path).exists());
    }
}
