//! The time of the last admitted tick: liveness evidence for status, nothing more.
//! It lives in Digest's own directory, is written by the tick that holds the tick
//! lock, and never gates, schedules or orders delivery.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

const FILE: &str = "tick.json";
const LIMIT: u64 = 256;

/// Best effort: a failed write leaves the previous record, and never fails a tick.
pub fn record(directory: &Path, at_ms: u64) {
    let temporary = directory.join("tick.json.tmp");
    let _ = fs::remove_file(&temporary);
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&temporary)?;
        file.write_all(format!("{{\"version\":1,\"lastAtMs\":{at_ms}}}").as_bytes())?;
        fs::rename(&temporary, directory.join(FILE))
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

/// The recorded time, or `None` when no tick has been recorded or the record is not
/// a small private regular file holding a safe integer. Reading changes nothing.
pub fn read(directory: &Path) -> Option<u64> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(directory.join(FILE))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.mode() & 0o077 != 0 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LIMIT {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if value["version"] != 1 {
        return None;
    }
    value["lastAtMs"]
        .as_u64()
        .filter(|at| *at <= crate::history::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn directory(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("digest-tick-last-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn nothing_is_recorded_until_a_tick_writes_and_the_newest_write_wins() {
        let path = directory("roundtrip");
        assert_eq!(read(&path), None);
        record(&path, 1_792_000_000_000);
        assert_eq!(read(&path), Some(1_792_000_000_000));
        record(&path, 1_792_000_060_000);
        assert_eq!(read(&path), Some(1_792_000_060_000));
        assert_eq!(
            fs::metadata(path.join(FILE)).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!path.join("tick.json.tmp").exists());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn status_reads_the_record_from_the_digest_directory_of_the_data_root() {
        let root = directory("root");
        assert_eq!(crate::tick::last_tick_ms(&root), None);
        fs::create_dir(root.join("digest")).unwrap();
        record(&root.join("digest"), 42);
        assert_eq!(crate::tick::last_tick_ms(&root), Some(42));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unsafe_or_malformed_record_reads_as_none_and_a_failed_write_keeps_the_old_one() {
        let path = directory("unsafe");
        for body in [
            "not json",
            r#"{"version":2,"lastAtMs":5}"#,
            r#"{"version":1,"lastAtMs":-5}"#,
            r#"{"version":1,"lastAtMs":9007199254740992}"#,
            r#"{"version":1,"lastAtMs":"5"}"#,
            &format!(
                "{{\"version\":1,\"lastAtMs\":5,\"pad\":\"{}\"}}",
                "x".repeat(300)
            ),
        ] {
            fs::write(path.join(FILE), body).unwrap();
            fs::set_permissions(path.join(FILE), fs::Permissions::from_mode(0o600)).unwrap();
            assert_eq!(read(&path), None, "{body}");
        }
        fs::write(path.join(FILE), r#"{"version":1,"lastAtMs":5}"#).unwrap();
        fs::set_permissions(path.join(FILE), fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            read(&path),
            None,
            "a record others can write is not trusted"
        );
        fs::remove_file(path.join(FILE)).unwrap();
        fs::write(path.join("real"), r#"{"version":1,"lastAtMs":5}"#).unwrap();
        fs::set_permissions(path.join("real"), fs::Permissions::from_mode(0o600)).unwrap();
        symlink(path.join("real"), path.join(FILE)).unwrap();
        assert_eq!(read(&path), None, "a symlink is not followed");
        fs::remove_file(path.join(FILE)).unwrap();
        record(&path, 7);
        assert_eq!(read(&path), Some(7));
        // A write that cannot happen (no directory) is silent.
        record(&path.join("missing"), 9);
        assert_eq!(read(&path), Some(7));
        fs::remove_dir_all(path).unwrap();
    }
}
