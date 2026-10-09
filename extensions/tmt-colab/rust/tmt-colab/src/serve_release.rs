//! Whether the running serve is older than the Colab release installed beside it.
//!
//! `tmt upgrade` publishes a new release directory and flips `<lib>/tmt-colab/current`; a serve
//! already running keeps its old release directory and bundled app. This reads the install
//! layout only: two path resolutions and, when the active release changed, one bounded receipt
//! read. It never spawns a process, uses the network, signals the serve or changes state, and
//! any doubt means "not stale" so a warning is never a false alarm.
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

/// A receipt larger than this is not read (the CLI receipt bound is the largest in use).
const RECEIPT_BYTES: u64 = 64 * 1024;
const VERSION_BYTES: usize = 64;
/// The most often the layout is read, however many requests ask.
const CHECK_EVERY: Duration = Duration::from_secs(60);

/// The two versions a restart notice names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Stale {
    pub running: String,
    pub installed: String,
}

/// The serve's own install location: `<lib>/tmt-colab/releases/<id>/tmt-colab`.
pub struct Running {
    release: PathBuf,
    current: PathBuf,
    version: String,
}

impl Running {
    /// `None` for a development or relocated binary, which has no install layout to compare.
    pub fn detect() -> Option<Self> {
        Self::at(&std::env::current_exe().ok()?, env!("CARGO_PKG_VERSION"))
    }
    pub fn at(exe: &Path, version: &str) -> Option<Self> {
        let exe = exe.canonicalize().ok()?;
        let release = exe.parent()?;
        let releases = release.parent()?;
        let lib = releases.parent()?;
        if releases.file_name()? != "releases" || lib.file_name()? != "tmt-colab" {
            return None;
        }
        Some(Self {
            release: release.to_path_buf(),
            current: lib.join("current"),
            version: version.to_owned(),
        })
    }
    /// The installed version when it differs from this serve's, else `None`.
    pub fn stale(&self) -> Option<Stale> {
        let active = self.current.canonicalize().ok()?;
        if active == self.release {
            return None;
        }
        let installed = receipt_version(&active.join("receipt.json"))?;
        (installed != self.version).then(|| Stale {
            running: self.version.clone(),
            installed,
        })
    }
}

fn receipt_version(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > RECEIPT_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let version = value.get("version")?.as_str()?;
    let plain = !version.is_empty()
        && version.len() <= VERSION_BYTES
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'));
    plain.then(|| version.to_owned())
}

/// The serve's cached answer, shared by the terminal watcher and the browser route.
pub struct ServeRelease {
    running: Option<Running>,
    cache: Mutex<Option<(Instant, Option<Stale>)>>,
}

impl ServeRelease {
    pub fn new(running: Option<Running>) -> Self {
        Self {
            running,
            cache: Mutex::new(None),
        }
    }
    pub fn version(&self) -> &str {
        self.running
            .as_ref()
            .map_or(env!("CARGO_PKG_VERSION"), |r| &r.version)
    }
    /// An uncached read, for the one terminal watcher that already paces itself.
    pub fn fresh(&self) -> Option<Stale> {
        self.running.as_ref()?.stale()
    }
    /// At most one layout read per `CHECK_EVERY`; a poisoned cache is simply "not stale".
    pub fn stale(&self) -> Option<Stale> {
        let running = self.running.as_ref()?;
        let mut cache = self.cache.lock().ok()?;
        if let Some((at, answer)) = cache.as_ref()
            && at.elapsed() < CHECK_EVERY
        {
            return answer.clone();
        }
        let answer = running.stale();
        *cache = Some((Instant::now(), answer.clone()));
        answer
    }
}

/// The restart instruction, shared by the serve terminal and tests.
pub fn restart_text(stale: &Stale) -> String {
    format!(
        "Colab {} is running but {} is installed.",
        stale.running, stale.installed
    )
}
pub const RESTART_HINT: &str = "Restart `tmt colab serve` to use the installed release.";

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    /// A unique temp directory removed on drop.
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "tmt-2218-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// `<lib>/tmt-colab/releases/<id>/tmt-colab` with a receipt per release and `current`.
    struct Layout {
        _root: Temp,
        lib: PathBuf,
    }
    impl Layout {
        fn new() -> Self {
            let root = Temp::new();
            let lib = root.0.join("lib/tmt-colab");
            fs::create_dir_all(lib.join("releases")).unwrap();
            Self { _root: root, lib }
        }
        fn release(&self, id: &str, receipt: Option<&str>) -> PathBuf {
            let dir = self.lib.join("releases").join(id);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("tmt-colab"), b"binary").unwrap();
            if let Some(receipt) = receipt {
                fs::write(dir.join("receipt.json"), receipt).unwrap();
            }
            dir
        }
        fn activate(&self, id: &str) {
            let current = self.lib.join("current");
            let _ = fs::remove_file(&current);
            std::os::unix::fs::symlink(format!("releases/{id}"), current).unwrap();
        }
        fn running(&self, id: &str, version: &str) -> Running {
            Running::at(
                &self.lib.join("releases").join(id).join("tmt-colab"),
                version,
            )
            .unwrap()
        }
    }
    fn receipt(version: &str) -> String {
        format!(r#"{{"schema_version":1,"version":"{version}"}}"#)
    }

    #[test]
    fn names_both_versions_only_after_current_moves_to_another_version() {
        let layout = Layout::new();
        layout.release("old", Some(&receipt("0.1.0-alpha.46")));
        layout.release("new", Some(&receipt("0.1.0-alpha.57")));
        layout.activate("old");
        let running = layout.running("old", "0.1.0-alpha.46");
        assert_eq!(
            running.stale(),
            None,
            "the running release is the active one"
        );
        layout.activate("new");
        assert_eq!(
            running.stale(),
            Some(Stale {
                running: "0.1.0-alpha.46".into(),
                installed: "0.1.0-alpha.57".into()
            })
        );
        // The serve keeps its own release; nothing was moved or removed.
        assert!(layout.lib.join("releases/old/tmt-colab").is_file());
    }

    #[test]
    fn a_reinstall_of_the_same_version_is_not_stale() {
        let layout = Layout::new();
        layout.release("a", Some(&receipt("0.1.0-alpha.57")));
        layout.release("b", Some(&receipt("0.1.0-alpha.57")));
        layout.activate("b");
        assert_eq!(layout.running("a", "0.1.0-alpha.57").stale(), None);
    }

    #[test]
    fn doubt_is_never_stale() {
        let layout = Layout::new();
        layout.release("old", Some(&receipt("0.1.0-alpha.46")));
        let running = layout.running("old", "0.1.0-alpha.46");
        // No `current` link at all.
        assert_eq!(running.stale(), None);
        for (id, body) in [
            ("missing", None),
            ("malformed", Some("{not json".to_owned())),
            ("no-version", Some(r#"{"schema_version":1}"#.to_owned())),
            ("number", Some(r#"{"version":57}"#.to_owned())),
            ("empty", Some(receipt(""))),
            ("control", Some(receipt("0.1.0\\nalpha"))),
            ("long", Some(receipt(&"9".repeat(VERSION_BYTES + 1)))),
            (
                "huge",
                Some(format!(
                    "{}{}",
                    " ".repeat(RECEIPT_BYTES as usize),
                    receipt("0.1.0-alpha.57")
                )),
            ),
        ] {
            layout.release(id, body.as_deref());
            layout.activate(id);
            assert_eq!(running.stale(), None, "{id}");
        }
        // A positive control: the same layout with a good receipt is stale.
        layout.release("good", Some(&receipt("0.1.0-alpha.57")));
        layout.activate("good");
        assert!(running.stale().is_some());
        // A dangling link is doubt too.
        let current = layout.lib.join("current");
        fs::remove_file(&current).unwrap();
        std::os::unix::fs::symlink("releases/gone", &current).unwrap();
        assert_eq!(running.stale(), None);
    }

    #[test]
    fn only_an_installed_layout_is_compared() {
        let layout = Layout::new();
        let release = layout.release("old", None);
        assert!(Running::at(&release.join("tmt-colab"), "1").is_some());
        // A cargo build or any relocated binary has no `releases/<id>` parent.
        let dev = Temp::new();
        fs::write(dev.0.join("tmt-colab"), b"b").unwrap();
        assert!(Running::at(&dev.0.join("tmt-colab"), "1").is_none());
        assert!(Running::at(&dev.0.join("absent"), "1").is_none());
        let wrong = layout.lib.parent().unwrap().join("other/releases/x");
        fs::create_dir_all(&wrong).unwrap();
        fs::write(wrong.join("tmt-colab"), b"b").unwrap();
        assert!(Running::at(&wrong.join("tmt-colab"), "1").is_none());
    }

    #[test]
    fn the_cache_reads_the_layout_at_most_once_per_interval() {
        let layout = Layout::new();
        layout.release("old", Some(&receipt("0.1.0-alpha.46")));
        layout.release("new", Some(&receipt("0.1.0-alpha.57")));
        layout.activate("old");
        let release = ServeRelease::new(Some(layout.running("old", "0.1.0-alpha.46")));
        assert_eq!(release.stale(), None);
        layout.activate("new");
        assert_eq!(release.stale(), None, "served from the cache");
        *release.cache.lock().unwrap() = Some((Instant::now() - CHECK_EVERY, None));
        assert!(release.stale().is_some());
        assert_eq!(ServeRelease::new(None).stale(), None);
    }
}
