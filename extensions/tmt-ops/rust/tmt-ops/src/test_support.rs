//! Fixtures shared by Squad's unit tests.

use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_test_support::write_executable;

/// A directory private to one test under the temporary directory, removed with
/// everything in it when dropped, also when the test fails. It is the one owner of
/// test scratch space: a test never removes its own directory by hand.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    fn unique(label: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "tmt-ops-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// A new empty directory, spelled without symlinks (macOS keeps its temporary
    /// directory behind one).
    pub(crate) fn new(label: &str) -> Self {
        let path = Self::unique(label);
        std::fs::create_dir(&path).expect("create the scratch directory");
        Self(path.canonicalize().expect("resolve the scratch directory"))
    }

    /// A reserved path with no directory yet, for a test of code that creates its own.
    pub(crate) fn missing(label: &str) -> Self {
        Self(Self::unique(label))
    }
}

impl Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The first line that is a full-width rule directly followed by another: a
/// heading rule is mostly `─`, and neighbours share one. A pane's frame edge carries its corners and
/// is not a heading rule, so it may sit against one.
pub(crate) fn stacked_rules(lines: &[String]) -> Option<usize> {
    let rule = |line: &str| {
        let line = line.trim();
        !line.is_empty()
            && !line.contains(['┌', '┐', '└', '┘', '┏', '┓', '┗', '┛'])
            && line.chars().filter(|c| *c == '─').count() * 5 >= line.chars().count() * 2
    };
    lines
        .windows(2)
        .position(|pair| rule(&pair[0]) && rule(&pair[1]))
}

/// Prewarm a fixture without executing its payload. macOS first-exec assessment
/// queues fresh files under parallel load; keep it outside production deadlines.
/// Never rewrite the executable after this one bounded readiness probe.
pub(crate) fn write_ready_executable(path: &Path, script: &str) {
    let payload = script
        .strip_prefix("#!/bin/sh\n")
        .expect("shell fixture shebang");
    write_executable(
        path,
        format!("#!/bin/sh\nif [ \"$1\" = __tmt_fixture_ready ]; then exit 0; fi\n{payload}")
            .as_bytes(),
        0o755,
    )
    .expect("publish shell fixture");
    let ready = probe(path, std::time::Duration::from_secs(30))
        .expect("fixture readiness completes within 30 seconds");
    assert!(
        ready.success,
        "fixture readiness failed: {}",
        path.display()
    );
    assert!(
        ready.stdout.is_empty(),
        "fixture readiness has no payload output"
    );
}

fn probe(
    path: &Path,
    ceiling: std::time::Duration,
) -> Result<crate::runner::Finished, crate::runner::RunError> {
    crate::runner::run(path, &["__tmt_fixture_ready".into()], b"", ceiling, 1024)
}

/// The stored cron jobs document of an Ops tree laid out under `directory`, read behind
/// the store's back.
pub(crate) fn stored_jobs(directory: &Path) -> Vec<u8> {
    rusqlite::Connection::open(directory.join("ops/ops.db"))
        .unwrap()
        .query_row(
            "SELECT body FROM documents WHERE kind = 'cron' AND key = ''",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

/// Replace the stored cron jobs document with `bytes`, as a damaged store would hold.
pub(crate) fn replace_stored_jobs(directory: &Path, bytes: &[u8]) {
    let changed = rusqlite::Connection::open(directory.join("ops/ops.db"))
        .unwrap()
        .execute(
            "UPDATE documents SET body = ?1 WHERE kind = 'cron' AND key = ''",
            [bytes],
        )
        .unwrap();
    assert_eq!(changed, 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_scratch_directory_is_removed_when_dropped_and_when_the_test_fails() {
        let scratch = Scratch::new("guard");
        let path = scratch.to_path_buf();
        std::fs::write(path.join("kept"), b"x").unwrap();
        std::fs::create_dir_all(path.join("nested/deeper")).unwrap();
        drop(scratch);
        assert!(!path.exists());

        let escaped = std::cell::RefCell::new(PathBuf::new());
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let scratch = Scratch::new("guard");
            *escaped.borrow_mut() = scratch.to_path_buf();
            std::fs::write(scratch.join("kept"), b"x").unwrap();
            panic!("the test fails with its files in place");
        }));
        assert!(failed.is_err());
        assert!(!escaped.borrow().as_os_str().is_empty());
        assert!(!escaped.borrow().exists());
    }

    #[test]
    fn scratch_directories_are_distinct_and_a_missing_one_is_removed_after_the_code_creates_it() {
        let (first, second) = (Scratch::new("same"), Scratch::new("same"));
        assert_ne!(*first, *second);
        let absent = Scratch::missing("same");
        assert!(!absent.exists());
        std::fs::create_dir_all(absent.join("made/by/the/code")).unwrap();
        let path = absent.to_path_buf();
        drop(absent);
        assert!(!path.exists());
    }

    #[test]
    fn readiness_has_no_payload_effects_and_a_stalled_probe_is_bounded() {
        let dir = Scratch::new("ready");
        let file = dir.join("fixture");
        let marker = dir.join("called");
        write_ready_executable(
            &file,
            &format!(
                "#!/bin/sh\nprintf called > '{}'\nprintf payload\n",
                marker.display()
            ),
        );
        assert!(!marker.exists());
        let output = crate::runner::run(&file, &[], b"", Duration::from_secs(30), 1024).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, b"payload");
        assert_eq!(std::fs::read(&marker).unwrap(), b"called");
        let stalled = dir.join("stalled");
        write_executable(&stalled, b"#!/bin/sh\nexec /bin/sleep 30\n", 0o755).unwrap();
        assert!(matches!(
            probe(&stalled, Duration::from_millis(100)),
            Err(crate::runner::RunError::Timeout)
        ));
    }
}
