//! Invocation-owned coalescing and the recorded-socket public backstop.
use super::{CAPTURE_BUDGET, capture_snapshot, decode, generations, publish_gated};
use crate::{
    bounded_file,
    config::{ConfigFiles, ConfigPaths},
    host::Host,
    process::{CommandRunner, UnixCommandRunner, runtime::ProcessObservation},
};
use std::{fs, io, os::unix::fs::MetadataExt, path::Path, time::Instant};
use tmt_core::{
    endpoint::{ProcessIncarnation, ServerEvidence},
    host::HostKind,
    workspace::{MAX_BYTES, WorkspaceServer, WorkspaceSnapshot},
};
const CADENCE: &str = "capture-attempt.json";
const PERIOD_MS: u64 = 30_000;
const RECORD_LIMIT: usize = 64;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    version: u8,
    pid: u64,
    start: String,
    #[serde(rename = "attemptedAtMs")]
    attempted_at_ms: u64,
}
fn owned(path: &Path, directory: bool) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(io::Error::other(
            "Workspace capture coordination must be private and owned.",
        ));
    }
    Ok(())
}
fn enabled(paths: &ConfigPaths) -> io::Result<bool> {
    ConfigFiles {
        paths: paths.clone(),
    }
    .workspace_snapshot_enabled()
    .map_err(io::Error::other)
}

/// Caller owns publication.lock. This ephemeral coordination stamp is separate
/// from recovery documents. A failed attempt also consumes its interval.
fn admit(directory: &Path, process: &ProcessIncarnation, now_ms: u64) -> io::Result<bool> {
    let path = directory.join(CADENCE);
    match owned(&path, false) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        other => {
            other?;
            let bytes = bounded_file::read_no_follow(&path, 2048).map_err(io::Error::other)?;
            let value =
                crate::json_document::parse(std::str::from_utf8(&bytes).map_err(io::Error::other)?)
                    .map_err(io::Error::other)?;
            let previous: Attempt = serde_json::from_value(value).map_err(io::Error::other)?;
            if previous.version != 1
                || ProcessIncarnation::new(previous.pid, &previous.start).is_err()
                || previous.start.len() > 512
                || previous.attempted_at_ms > tmt_core::limits::MAX_JS_SAFE_INTEGER
            {
                return Err(io::Error::other(
                    "Malformed workspace capture coordination.",
                ));
            }
            if previous.pid == process.pid()
                && previous.start == process.start_identity()
                && now_ms >= previous.attempted_at_ms
                && now_ms - previous.attempted_at_ms < PERIOD_MS
            {
                return Ok(false);
            }
        }
    }
    let bytes = serde_json::to_vec(&Attempt {
        version: 1,
        pid: process.pid(),
        start: process.start_identity().to_owned(),
        attempted_at_ms: now_ms,
    })
    .map_err(io::Error::other)?;
    crate::private_file::replace(&path, &bytes)?;
    Ok(true)
}
fn native(process: u64, deadline: Instant) -> Option<ProcessIncarnation> {
    match UnixCommandRunner.process_observation(process, deadline)? {
        ProcessObservation::Live(process) | ProcessObservation::Stopped(process) => Some(process),
        _ => None,
    }
}

/// Already-admitted hooks only. Native acquisition has no process fallback;
/// optional failure cannot add per-invocation ps work or delay hook admission.
pub fn capture_hook(
    paths: &ConfigPaths,
    host: &Host,
    server: &ServerEvidence,
    deadline: Instant,
) -> io::Result<bool> {
    if server.host != HostKind::Tmux || Instant::now() >= deadline || !enabled(paths)? {
        return Ok(false);
    }
    let Some(process) = native(server.server_pid, deadline) else {
        return Ok(false);
    };
    let socket = &server.socket_path;
    publish_gated(
        paths,
        socket,
        |directory| admit(directory, &process, crate::request_runtime::wall_time_ms()),
        |_| true,
        || {
            let snapshot = capture_snapshot(paths, host, socket, Some(server), None, deadline)?;
            if snapshot.server.process != process {
                return Err(io::Error::other("Workspace server incarnation changed."));
            }
            Ok(snapshot)
        },
    )
}

#[derive(Default, Debug, serde::Serialize)]
pub struct WorkspaceCaptureReport {
    pub attempted: usize,
    pub published: usize,
    pub skipped: usize,
}

/// No socket discovery or server enrollment: only private, previously captured
/// records. All candidates share one acquisition deadline and cadence owner.
pub fn capture_recorded(paths: &ConfigPaths) -> io::Result<WorkspaceCaptureReport> {
    let host = Host::for_workspace_socket();
    recorded_with(
        paths,
        crate::request_runtime::wall_time_ms(),
        Instant::now() + CAPTURE_BUDGET,
        |previous, deadline| {
            if native(previous.process.pid(), deadline).as_ref() != Some(&previous.process) {
                return Err(io::Error::other(
                    "Recorded workspace server is not current.",
                ));
            }
            let snapshot = capture_snapshot(paths, &host, &previous.socket, None, None, deadline)?;
            if snapshot.server != *previous {
                return Err(io::Error::other("Recorded workspace server changed."));
            }
            Ok(snapshot)
        },
    )
}
fn recorded_with(
    paths: &ConfigPaths,
    now_ms: u64,
    deadline: Instant,
    mut capture: impl FnMut(&WorkspaceServer, Instant) -> io::Result<WorkspaceSnapshot>,
) -> io::Result<WorkspaceCaptureReport> {
    let mut report = WorkspaceCaptureReport::default();
    if !enabled(paths)? {
        return Ok(report);
    }
    let root = paths.global_dir.join("workspace");
    match owned(&root, true) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(report),
        other => other?,
    }
    for entry in fs::read_dir(&root)?.take(RECORD_LIMIT) {
        let entry = entry?;
        let name = entry.file_name();
        if !name
            .to_str()
            .is_some_and(tmt_core::content_digest::is_sha256)
        {
            continue;
        }
        if Instant::now() >= deadline {
            report.skipped += 1;
            continue;
        }
        let load = (|| {
            owned(&entry.path(), true)?;
            let bytes = match generations::read(&entry.path())? {
                Some(catalogue) => generations::acquire(
                    &entry.path(),
                    catalogue.latest().expect("nonempty catalogue"),
                )?,
                None => bounded_file::read_no_follow(&entry.path().join("latest.json"), MAX_BYTES)
                    .map_err(io::Error::other)?,
            };
            let snapshot = decode(&bytes)?;
            if paths.workspace_directory(&snapshot.server.socket) != entry.path() {
                return Err(io::Error::other("Recorded workspace socket changed."));
            }
            Ok::<_, io::Error>(snapshot.server)
        })();
        let Ok(previous) = load else {
            report.skipped += 1;
            continue;
        };
        let mut attempted = false;
        let published = publish_gated(
            paths,
            &previous.socket,
            |directory| {
                if Instant::now() >= deadline {
                    return Ok(false);
                }
                admit(directory, &previous.process, now_ms)
            },
            |_| true,
            || {
                attempted = true;
                report.attempted += 1;
                capture(&previous, deadline)
            },
        );
        if published.unwrap_or(false) {
            report.published += 1;
        }
        if !attempted {
            report.skipped += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        test_support::TestDirectory,
        workspace::{
            publish_event,
            tests::{paths, sample},
        },
    };
    fn fixture() -> (TestDirectory, ConfigPaths, WorkspaceSnapshot) {
        let root = TestDirectory::new();
        let paths = paths(&root.path);
        let snapshot = sample();
        publish_event(&paths, &snapshot.server.socket, || Ok(snapshot.clone())).unwrap();
        (root, paths, snapshot)
    }
    fn files(directory: &Path) -> Vec<(std::path::PathBuf, Vec<u8>, u64, i64, i64)> {
        let mut entries = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap() != CADENCE)
            .map(|path| {
                let metadata = fs::symlink_metadata(&path).unwrap();
                let bytes = fs::read(&path).unwrap();
                (
                    path,
                    bytes,
                    metadata.ino(),
                    metadata.mtime(),
                    metadata.mtime_nsec(),
                )
            })
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }
    #[test]
    fn unchanged_backstop_coalesces_without_replacing_recovery_bytes_and_new_incarnation_is_due() {
        let (_root, paths, mut snapshot) = fixture();
        let directory = paths.workspace_directory(&snapshot.server.socket);
        let before = files(&directory);
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let first = recorded_with(&paths, 10_000, deadline, |previous, _| {
            assert_eq!(previous, &snapshot.server);
            Ok(snapshot.clone())
        })
        .unwrap();
        assert_eq!((first.attempted, first.published, first.skipped), (1, 0, 0));
        assert_eq!(files(&directory), before);
        let stamp = fs::read(directory.join(CADENCE)).unwrap();
        let second = recorded_with(&paths, 39_999, deadline, |_, _| {
            panic!("unchanged attempt must coalesce")
        })
        .unwrap();
        assert_eq!((second.attempted, second.skipped), (0, 1));
        assert_eq!(fs::read(directory.join(CADENCE)).unwrap(), stamp);
        assert_eq!(files(&directory), before);
        snapshot.windows[0].name = "changed".into();
        snapshot.captured_at_ms = 40_000;
        let changed = recorded_with(&paths, 40_000, deadline, |_, _| Ok(snapshot.clone())).unwrap();
        assert_eq!((changed.attempted, changed.published), (1, 1));
        snapshot.server.process = ProcessIncarnation::new(11, "new-native-start").unwrap();
        snapshot.captured_at_ms += 1;
        publish_event(&paths, &snapshot.server.socket, || Ok(snapshot.clone())).unwrap();
        let restarted = recorded_with(&paths, 40_001, deadline, |previous, _| {
            assert_eq!(previous, &snapshot.server);
            Ok(snapshot.clone())
        })
        .unwrap();
        assert_eq!(restarted.attempted, 1);
    }
    #[test]
    fn failure_consumes_one_interval_and_clock_rollback_does_not_create_a_hot_loop() {
        let (_root, paths, snapshot) = fixture();
        let directory = paths.workspace_directory(&snapshot.server.socket);
        let before = files(&directory);
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let failed = recorded_with(&paths, 100_000, deadline, |_, _| {
            Err(io::Error::other("capture refused"))
        })
        .unwrap();
        assert_eq!(failed.attempted, 1);
        assert_eq!(failed.published, 0);
        recorded_with(&paths, 100_001, deadline, |_, _| {
            panic!("failed capture cannot retry immediately")
        })
        .unwrap();
        let rollback = recorded_with(&paths, 10, deadline, |_, _| {
            Err(io::Error::other("clock rolled back"))
        })
        .unwrap();
        assert_eq!(rollback.attempted, 1);
        let skipped = recorded_with(&paths, 11, deadline, |_, _| {
            panic!("rollback should reset cadence once")
        })
        .unwrap();
        assert_eq!(skipped.skipped, 1);
        assert_eq!(files(&directory), before);
    }
    #[test]
    fn busy_or_malformed_coordination_skips_without_host_work_and_foreign_content_survives() {
        use std::os::unix::fs::symlink;
        let (root, paths, snapshot) = fixture();
        let directory = paths.workspace_directory(&snapshot.server.socket);
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let before = files(&directory);
        let lock = crate::file_lock::exclusive(&directory.join("publication.lock")).unwrap();
        let busy =
            recorded_with(&paths, 1, deadline, |_, _| panic!("busy capture must skip")).unwrap();
        assert_eq!(busy.skipped, 1);
        assert_eq!(files(&directory), before);
        drop(lock);
        let target = root.path.join("foreign");
        fs::write(&target, "do not replace").unwrap();
        symlink(&target, directory.join(CADENCE)).unwrap();
        let refused = recorded_with(&paths, 1, deadline, |_, _| {
            panic!("symlink must refuse before capture")
        })
        .unwrap();
        assert_eq!(refused.skipped, 1);
        assert_eq!(fs::read_to_string(target).unwrap(), "do not replace");
        fs::remove_file(directory.join(CADENCE)).unwrap();
        crate::private_file::replace(&directory.join(CADENCE), br#"{"version":99}"#).unwrap();
        let refused = recorded_with(&paths, 1, deadline, |_, _| {
            panic!("malformed coordination must skip")
        })
        .unwrap();
        assert_eq!(refused.skipped, 1);
    }
    #[test]
    fn absent_disabled_and_expired_backstops_never_discover_or_start_servers() {
        let root = TestDirectory::new();
        let paths = paths(&root.path);
        let deadline = Instant::now();
        assert_eq!(
            recorded_with(&paths, 1, deadline, |_, _| panic!("no known sockets"))
                .unwrap()
                .attempted,
            0
        );
        assert!(!paths.global_dir.join("workspace").exists());
        let mut snapshot = sample();
        snapshot.server.socket = root.path.join("known.sock").display().to_string();
        publish_event(&paths, &snapshot.server.socket, || Ok(snapshot.clone())).unwrap();
        let expired =
            recorded_with(&paths, 1, deadline, |_, _| panic!("one aggregate deadline")).unwrap();
        assert_eq!(expired.skipped, 1);
        fs::write(
            paths.global_dir.join("config.json"),
            r#"{"workspace":{"snapshotEnabled":false},"opaque":{"keep":1}}"#,
        )
        .unwrap();
        let disabled = recorded_with(&paths, 1, Instant::now() + CAPTURE_BUDGET, |_, _| {
            panic!("disabled capture")
        })
        .unwrap();
        assert_eq!(disabled.attempted, 0);
        assert!(
            !paths
                .workspace_directory(&snapshot.server.socket)
                .join(CADENCE)
                .exists()
        );
    }
}
