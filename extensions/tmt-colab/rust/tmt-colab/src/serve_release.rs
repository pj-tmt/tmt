//! The running Colab release, the installed selection and bounded restart verification.
//!
//! Installation moves `<lib>/tmt-colab/current`; the running service retains its release.
//! Notice reads stay bounded and read-only. Restart selection additionally checks the owned
//! executable against that release's receipt before the executable owner runs its preflight.
//! This module never spawns a process, uses the network, signals the serve or changes stored state.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

/// A receipt larger than this is not read (the CLI receipt bound is the largest in use).
const RECEIPT_BYTES: u64 = 64 * 1024;
const VERSION_BYTES: usize = 64;
/// Bound a restart verification read independently of the installed artifact size.
const EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
/// The most often the layout is read, however many requests ask.
const CHECK_EVERY: Duration = Duration::from_secs(60);

/// The two versions a restart notice names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Stale {
    pub running: String,
    pub installed: String,
}

/// A captured installed executable whose receipt still binds its bytes.
/// The executable owner additionally checks its app and store before the restart cutoff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    program: PathBuf,
    version: String,
    digest: [u8; 32],
}

impl Candidate {
    pub fn program(&self) -> &Path {
        &self.program
    }
    pub fn version(&self) -> &str {
        &self.version
    }
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
    /// Follow the installed current release; the installer owns version direction.
    /// No executable runs until its owned regular file matches the retained receipt.
    pub fn verified_current(&self) -> Result<Option<Candidate>, &'static str> {
        let active = self
            .current
            .canonicalize()
            .map_err(|_| "Installed current release is unavailable.")?;
        if active == self.release {
            return Ok(None);
        }
        let releases = self
            .release
            .parent()
            .ok_or("Installed release layout is invalid.")?;
        if active.parent() != Some(releases) {
            return Err("Installed current is outside the release directory.");
        }
        let receipt = owned_file(&active.join("receipt.json"))?;
        let mut bytes = Vec::new();
        receipt
            .take(RECEIPT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Installed receipt could not be read.")?;
        if bytes.len() as u64 > RECEIPT_BYTES {
            return Err("Installed receipt is too large.");
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "Installed receipt is invalid.")?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_str)
            .filter(|version| plain_version(version))
            .ok_or("Installed receipt version is invalid.")?;
        if version == self.version {
            return Ok(None);
        }
        let expected = value
            .get("file_sha256")
            .and_then(|files| files.get("tmt-colab"))
            .and_then(serde_json::Value::as_str)
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or("Installed receipt has no executable digest.")?;
        let program = active.join("tmt-colab");
        let mut file = owned_file(&program)?;
        let metadata = file
            .metadata()
            .map_err(|_| "Installed executable could not be inspected.")?;
        if metadata.mode() & 0o111 == 0 {
            return Err("Installed executable is not executable.");
        }
        if metadata.len() == 0 || metadata.len() > EXECUTABLE_BYTES {
            return Err("Installed executable exceeds restart verification bounds.");
        }
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut read_bytes = 0u64;
        let started = Instant::now();
        loop {
            let size = file
                .read(&mut buffer)
                .map_err(|_| "Installed executable could not be read.")?;
            if size == 0 {
                break;
            }
            read_bytes += size as u64;
            if read_bytes > EXECUTABLE_BYTES || started.elapsed() > Duration::from_secs(10) {
                return Err("Installed executable exceeds restart verification bounds.");
            }
            digest.update(&buffer[..size]);
        }
        let digest: [u8; 32] = digest.finalize().into();
        if read_bytes != metadata.len() || digest_text(&digest) != expected {
            return Err("Installed executable differs from its receipt.");
        }
        if self.current.canonicalize().ok().as_ref() != Some(&active) {
            return Err("Installed current changed during verification.");
        }
        Ok(Some(Candidate {
            program,
            version: version.to_owned(),
            digest,
        }))
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

fn digest_text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn owned_file(path: &Path) -> Result<std::fs::File, &'static str> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK).bits())
        .open(path)
        .map_err(|_| "Installed release file could not be opened.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "Installed release file could not be inspected.")?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.mode() & 0o7022 != 0
    {
        return Err("Installed release file is not an owned regular file.");
    }
    Ok(file)
}

fn plain_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= VERSION_BYTES
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
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
    plain_version(version).then(|| version.to_owned())
}

/// The serve's cached answer, shared by the terminal watcher and the browser route.
pub struct ServeRelease {
    running: Option<Running>,
    cache: Mutex<Option<(Instant, Option<Stale>)>>,
    failed: Mutex<Option<String>>,
}

impl ServeRelease {
    pub fn new(running: Option<Running>) -> Self {
        Self {
            running,
            cache: Mutex::new(None),
            failed: Mutex::new(None),
        }
    }
    pub fn version(&self) -> &str {
        self.running
            .as_ref()
            .map_or(env!("CARGO_PKG_VERSION"), |r| &r.version)
    }
    /// A failed pre-cutoff check is diagnostic only; the old service stays live.
    pub fn verification_failed(&self, version: String) {
        if let Ok(mut failed) = self.failed.lock() {
            *failed = Some(version);
        }
    }
    pub fn failed_version(&self) -> Option<String> {
        self.failed.lock().ok()?.clone()
    }
    /// Verify the selected release before the serving process begins its drain.
    pub fn verified_current(&self) -> Result<Option<Candidate>, &'static str> {
        self.running
            .as_ref()
            .map_or(Ok(None), Running::verified_current)
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
        "Colab {} is installed, but {} is still running.",
        stale.installed, stale.running
    )
}
pub const RESTART_HINT: &str = "Restart `tmt colab serve` to update.";

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

    fn restart_release(layout: &Layout, id: &str, version: &str) -> PathBuf {
        let bytes = b"verified executable fixture";
        let receipt = serde_json::json!({
            "version": version,
            "file_sha256": {"tmt-colab": digest_text(&Sha256::digest(bytes))}
        });
        let dir = layout.release(id, Some(&receipt.to_string()));
        fs::write(dir.join("tmt-colab"), bytes).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.join("tmt-colab"), fs::Permissions::from_mode(0o755)).unwrap();
        dir
    }

    #[test]
    fn restart_follows_current_without_second_guessing_the_version_direction() {
        let layout = Layout::new();
        restart_release(&layout, "old", "0.1.0-alpha.57");
        let active = restart_release(&layout, "selected", "0.1.0-alpha.46");
        layout.activate("old");
        let running = layout.running("old", "0.1.0-alpha.57");
        assert_eq!(running.verified_current(), Ok(None));
        layout.activate("selected");
        let candidate = running.verified_current().unwrap().unwrap();
        assert_eq!(candidate.program(), active.join("tmt-colab"));
        assert_eq!(candidate.version(), "0.1.0-alpha.46");
        assert!(layout.lib.join("releases/old/tmt-colab").exists());
    }

    #[test]
    fn a_changed_executable_or_unbound_receipt_never_becomes_a_restart_candidate() {
        let layout = Layout::new();
        restart_release(&layout, "old", "1");
        let active = restart_release(&layout, "selected", "2");
        let running = layout.running("old", "1");
        layout.activate("selected");
        assert!(running.verified_current().unwrap().is_some());
        fs::write(active.join("tmt-colab"), b"changed bytes").unwrap();
        assert_eq!(
            running.verified_current(),
            Err("Installed executable differs from its receipt.")
        );
        restart_release(&layout, "selected", "2");
        fs::write(active.join("receipt.json"), receipt("2")).unwrap();
        assert_eq!(
            running.verified_current(),
            Err("Installed receipt has no executable digest.")
        );
        restart_release(&layout, "selected", "2");
        fs::remove_file(active.join("tmt-colab")).unwrap();
        std::os::unix::fs::symlink(
            layout.lib.join("releases/old/tmt-colab"),
            active.join("tmt-colab"),
        )
        .unwrap();
        assert_eq!(
            running.verified_current(),
            Err("Installed release file could not be opened.")
        );
    }

    #[test]
    fn current_outside_the_release_directory_is_never_a_restart_candidate() {
        let layout = Layout::new();
        restart_release(&layout, "old", "1");
        let running = layout.running("old", "1");
        let foreign = Temp::new();
        fs::create_dir(foreign.0.join("foreign")).unwrap();
        std::os::unix::fs::symlink(foreign.0.join("foreign"), layout.lib.join("current")).unwrap();
        assert_eq!(
            running.verified_current(),
            Err("Installed current is outside the release directory.")
        );
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
