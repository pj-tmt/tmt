//! The one place a driver reads its own provider's transcript, and only for
//! usage numbers (#519), plus Codex's exact-ID metadata header admission. A path is trusted only
//! as a regular `.jsonl` file under the driver's own tree, opened without
//! following a final symlink or blocking on a FIFO. Tail and incremental reads
//! share this trust boundary. Tails have a byte bound; Claude incremental
//! scans have a deadline and a single-record byte bound. The formats
//! are unofficial: anything unexpected
//! yields nothing, never a guess.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
    path::{Component, Path},
};

/// The most bytes read from the end of a transcript.
pub const TAIL_LIMIT: u64 = 1024 * 1024;

/// The newest line of `path` that `parse` accepts, searching back through the
/// last [`TAIL_LIMIT`] bytes. A line cut by the window is skipped.
pub fn latest<T>(root: &Path, path: &Path, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    let mut file = open(root, path)?;
    let end = file.metadata().ok()?.len();
    latest_at(&mut file, end, parse)
}

/// Read the captured file/end together, so later appends or path replacement
/// cannot mismatch a consumption cursor with the evidence it describes.
pub(super) fn latest_at<T>(
    file: &mut File,
    end: u64,
    parse: impl Fn(&str) -> Option<T>,
) -> Option<T> {
    let tail = read_tail(file, end)?;
    parse_latest(&tail, parse)
}

/// Completed JSONL evidence only, retaining whether the captured tail ends
/// in a partial record. Consumers must not label that snapshot complete.
pub(super) fn latest_complete_at<T>(
    file: &mut File,
    end: u64,
    parse: impl Fn(&str) -> Option<T>,
) -> Option<(T, bool)> {
    let mut tail = read_tail(file, end)?;
    let complete = tail.last() == Some(&b'\n');
    if !complete {
        tail.truncate(tail.iter().rposition(|byte| *byte == b'\n')? + 1);
    }
    std::str::from_utf8(&tail).ok()?;
    Some((parse_latest(&tail, parse)?, complete))
}

fn parse_latest<T>(tail: &[u8], parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    tail.split(|byte| *byte == b'\n')
        .rev()
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter(|line| !line.trim().is_empty())
        .find_map(parse)
}

/// The shared provider-tree trust boundary for tail and appended-record reads.
pub(super) fn open(root: &Path, path: &Path) -> Option<File> {
    if !path.is_absolute()
        || path
            .extension()
            .is_none_or(|extension| extension != "jsonl")
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return None;
    }
    let root = root.canonicalize().ok()?;
    if !path.canonicalize().ok()?.starts_with(&root) {
        return None;
    }
    let flags = nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK;
    let file = File::options()
        .read(true)
        .custom_flags(flags.bits())
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    Some(file)
}

pub(super) fn read_tail(file: &mut File, end: u64) -> Option<Vec<u8>> {
    let start = end.saturating_sub(TAIL_LIMIT);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    file.take(end - start).read_to_end(&mut tail).ok()?;
    if tail.len() as u64 != end - start {
        return None;
    }
    if start > 0 {
        // The window began inside a line; that fragment is not a line.
        let first = tail.iter().position(|byte| *byte == b'\n')?;
        tail.drain(..=first);
    }
    Some(tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use std::{fs, io::Write};

    fn number(line: &str) -> Option<u64> {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()?
            .get("n")?
            .as_u64()
    }

    #[test]
    fn the_newest_accepted_line_wins_and_foreign_lines_are_skipped() {
        let root = TestDirectory::new();
        let path = root.path.as_path().join("session.jsonl");
        fs::write(&path, "{\"n\":1}\n{\"n\":2}\nnot json\n{\"other\":3}\n\n").unwrap();
        assert_eq!(latest(root.path.as_path(), &path, number), Some(2));
        fs::write(&path, "nothing\n").unwrap();
        assert_eq!(latest(root.path.as_path(), &path, number), None);
    }

    #[test]
    fn only_a_regular_jsonl_file_under_the_root_is_read() {
        let root = TestDirectory::new();
        let outside = TestDirectory::new();
        let inside = root.path.as_path().join("projects");
        fs::create_dir(&inside).unwrap();
        let good = inside.join("a.jsonl");
        fs::write(&good, "{\"n\":1}\n").unwrap();
        assert_eq!(latest(root.path.as_path(), &good, number), Some(1));

        let foreign = outside.path.as_path().join("b.jsonl");
        fs::write(&foreign, "{\"n\":1}\n").unwrap();
        let linked = inside.join("linked.jsonl");
        std::os::unix::fs::symlink(&foreign, &linked).unwrap();
        let inner_link = inside.join("inner.jsonl");
        std::os::unix::fs::symlink(&good, &inner_link).unwrap();
        let text = inside.join("a.txt");
        fs::write(&text, "{\"n\":1}\n").unwrap();
        let fifo = inside.join("fifo.jsonl");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).unwrap();
        let directory = inside.join("dir.jsonl");
        fs::create_dir(&directory).unwrap();
        for path in [
            foreign.clone(),
            linked,
            inner_link,
            text,
            fifo,
            directory,
            inside.join("missing.jsonl"),
            inside.join("../projects/a.jsonl"),
            std::path::PathBuf::from("projects/a.jsonl"),
        ] {
            assert_eq!(latest(root.path.as_path(), &path, number), None, "{path:?}");
        }
    }

    #[test]
    fn captured_descriptor_and_end_exclude_later_appends_and_replacement() {
        let root = TestDirectory::new();
        let path = root.path.join("session.jsonl");
        fs::write(&path, "{\"n\":1}\n").unwrap();
        let mut file = open(&root.path, &path).unwrap();
        let end = file.metadata().unwrap().len();
        let mut writer = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(writer, "{{\"n\":2}}").unwrap();
        let replacement = root.path.join("replacement.jsonl");
        fs::write(&replacement, "{\"n\":3}\n").unwrap();
        fs::rename(replacement, &path).unwrap();
        assert_eq!(latest_at(&mut file, end, number), Some(1));
        assert_eq!(latest(&root.path, &path, number), Some(3));
    }

    #[test]
    fn the_read_is_bounded_and_a_cut_line_is_skipped() {
        let root = TestDirectory::new();
        let path = root.path.as_path().join("large.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, "{{\"n\":1}}").unwrap();
        // A final line larger than the window can never be read.
        let huge = format!(
            "{{\"n\":2,\"pad\":\"{}\"}}",
            "x".repeat(TAIL_LIMIT as usize)
        );
        writeln!(file, "{huge}").unwrap();
        drop(file);
        assert_eq!(latest(root.path.as_path(), &path, number), None);

        // A line that fits after a cut one is found; the older line is not.
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{\"n\":3}}").unwrap();
        drop(file);
        assert_eq!(latest(root.path.as_path(), &path, number), Some(3));
    }
}
