//! Invocation-owned coalescing and the recorded-socket public backstop.
use super::{
    CAPTURE_BUDGET, Store, capture_builtin, capture_snapshot, decode, generations, publish_gated,
};
use crate::{
    bounded_file,
    config::{ConfigFiles, ConfigPaths},
    host::Host,
    process::{CommandRunner, UnixCommandRunner, runtime::ProcessObservation},
};
use std::{
    cell::Cell, collections::BTreeMap, fmt, fs, io, os::unix::fs::MetadataExt, path::Path,
    time::Instant,
};
use tmt_core::{
    endpoint::{ProcessIncarnation, ServerEvidence},
    host::HostKind,
    workspace::{MAX_BYTES, WorkspaceServer, WorkspaceSnapshot, builtin::BuiltinSnapshot},
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
/// The built-in store has no one server incarnation: its members' hooks coalesce on time
/// alone, one attempt per period. A backwards clock admits a fresh attempt.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PeriodAttempt {
    version: u8,
    #[serde(rename = "attemptedAtMs")]
    attempted_at_ms: u64,
}
fn admit_period(directory: &Path, now_ms: u64) -> io::Result<bool> {
    let path = directory.join(CADENCE);
    match owned(&path, false) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        other => {
            other?;
            let bytes = bounded_file::read_no_follow(&path, 2048).map_err(io::Error::other)?;
            let value =
                crate::json_document::parse(std::str::from_utf8(&bytes).map_err(io::Error::other)?)
                    .map_err(io::Error::other)?;
            let previous: PeriodAttempt =
                serde_json::from_value(value).map_err(io::Error::other)?;
            if previous.version != 1
                || previous.attempted_at_ms > tmt_core::limits::MAX_JS_SAFE_INTEGER
            {
                return Err(io::Error::other(
                    "Malformed workspace capture coordination.",
                ));
            }
            if now_ms >= previous.attempted_at_ms && now_ms - previous.attempted_at_ms < PERIOD_MS {
                return Ok(false);
            }
        }
    }
    let bytes = serde_json::to_vec(&PeriodAttempt {
        version: 1,
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
    let builtin = server.host == HostKind::pty();
    if !builtin && server.host != HostKind::Tmux || Instant::now() >= deadline || !enabled(paths)? {
        return Ok(false);
    }
    if builtin {
        return publish_gated(
            paths,
            Store::Builtin,
            |directory| admit_period(directory, crate::request_runtime::wall_time_ms()),
            |_| true,
            || capture_builtin(paths, deadline),
        );
    }
    let Some(process) = native(server.server_pid, deadline) else {
        return Ok(false);
    };
    let socket = &server.socket_path;
    publish_gated(
        paths,
        Store::Socket(socket),
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

/// Why an examined record ended without a published generation. A closed set: the report never
/// carries host text, paths or error strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureReason {
    /// The shared acquisition budget was spent, before the callback or inside it.
    Deadline,
    /// Another process holds the publication lock.
    Busy,
    /// The same server incarnation was attempted less than one period ago.
    NotDue,
    /// Coordination or the recorded record is malformed, symlinked or unreadable.
    Refused,
    /// The topology was read and nothing changed: the healthy outcome of an unchanged attempt.
    Unchanged,
    /// The native incarnation pre-check found the recorded server gone or replaced.
    ServerNotCurrent,
    /// The server, socket or incarnation differs after the read.
    ServerChanged,
    /// The host read, the parse or the write failed.
    Failed,
}

/// `published + sum(reasons)` is the number of examined recognized records. `skipped` keeps its
/// meaning, records refused before the callback; `reasons` is omitted when empty.
#[derive(Default, Debug, serde::Serialize)]
pub struct WorkspaceCaptureReport {
    pub attempted: usize,
    pub published: usize,
    pub skipped: usize,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub reasons: BTreeMap<CaptureReason, usize>,
}

impl WorkspaceCaptureReport {
    fn count(&mut self, reason: CaptureReason) {
        *self.reasons.entry(reason).or_default() += 1;
    }
    fn skip(&mut self, reason: CaptureReason) {
        self.skipped += 1;
        self.count(reason);
    }
}

/// Typed failures of the recorded-server callback, recovered from the `io::Error` that carries them.
#[derive(Debug)]
enum ServerFailure {
    NotCurrent,
    Changed,
}
impl fmt::Display for ServerFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotCurrent => "Recorded workspace server is not current.",
            Self::Changed => "Recorded workspace server changed.",
        })
    }
}
impl std::error::Error for ServerFailure {}
impl From<ServerFailure> for io::Error {
    fn from(failure: ServerFailure) -> Self {
        Self::other(failure)
    }
}

/// An error from the callback, named. A budget that is spent by now explains the failure better
/// than a pre-check that ran out of it.
fn failure_reason(error: &io::Error, deadline: Instant) -> CaptureReason {
    let spent = error.kind() == io::ErrorKind::TimedOut || Instant::now() >= deadline;
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<ServerFailure>())
    {
        Some(ServerFailure::Changed) => CaptureReason::ServerChanged,
        Some(ServerFailure::NotCurrent) if !spent => CaptureReason::ServerNotCurrent,
        _ if spent => CaptureReason::Deadline,
        _ => CaptureReason::Failed,
    }
}

/// No socket discovery or server enrollment: only private, previously captured
/// records. All candidates share one acquisition deadline and cadence owner.
pub fn capture_recorded(paths: &ConfigPaths) -> io::Result<WorkspaceCaptureReport> {
    let host = Host::for_workspace_socket();
    let now_ms = crate::request_runtime::wall_time_ms();
    let deadline = Instant::now() + CAPTURE_BUDGET;
    let mut report = recorded_with(paths, now_ms, deadline, |previous, deadline| {
        if native(previous.process.pid(), deadline).as_ref() != Some(&previous.process) {
            return Err(ServerFailure::NotCurrent.into());
        }
        let snapshot = capture_snapshot(paths, &host, &previous.socket, None, None, deadline)?;
        if snapshot.server != *previous {
            return Err(ServerFailure::Changed.into());
        }
        Ok(snapshot)
    })?;
    recorded_builtin(paths, &mut report, now_ms, deadline, |deadline| {
        capture_builtin(paths, deadline)
    })?;
    Ok(report)
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
            report.skip(CaptureReason::Deadline);
            continue;
        }
        let load = (|| {
            owned(&entry.path(), true)?;
            let bytes = match generations::read(&entry.path())? {
                Some(catalogue) => generations::acquire::<WorkspaceSnapshot>(
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
            report.skip(CaptureReason::Refused);
            continue;
        };
        examine(
            paths,
            &mut report,
            deadline,
            Store::Socket(&previous.socket),
            |directory| admit(directory, &previous.process, now_ms),
            || capture(&previous, deadline),
        );
    }
    Ok(report)
}

/// The built-in store, only once an event or hook created it: the backstop never initializes it.
fn recorded_builtin(
    paths: &ConfigPaths,
    report: &mut WorkspaceCaptureReport,
    now_ms: u64,
    deadline: Instant,
    capture: impl FnOnce(Instant) -> io::Result<BuiltinSnapshot>,
) -> io::Result<()> {
    if !enabled(paths)? {
        return Ok(());
    }
    match owned(&paths.builtin_workspace_directory(), true) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            report.skip(CaptureReason::Refused);
            return Ok(());
        }
        Ok(()) => (),
    }
    if Instant::now() >= deadline {
        report.skip(CaptureReason::Deadline);
        return Ok(());
    }
    examine(
        paths,
        report,
        deadline,
        Store::Builtin,
        |directory| admit_period(directory, now_ms),
        || capture(deadline),
    );
    Ok(())
}

/// Exactly one outcome per examined record. A lock held by another process is the one
/// refusal that reaches neither the admission closure nor the callback.
fn examine<D: super::Recovery>(
    paths: &ConfigPaths,
    report: &mut WorkspaceCaptureReport,
    deadline: Instant,
    store: Store<'_>,
    admit: impl FnOnce(&Path) -> io::Result<bool>,
    capture: impl FnOnce() -> io::Result<D>,
) {
    let mut attempted = false;
    let gate = Cell::new(None);
    let failure = Cell::new(None);
    let published = publish_gated(
        paths,
        store,
        |directory| {
            if Instant::now() >= deadline {
                gate.set(Some(CaptureReason::Deadline));
                return Ok(false);
            }
            let admitted = admit(directory)?;
            if !admitted {
                gate.set(Some(CaptureReason::NotDue));
            }
            Ok(admitted)
        },
        |_| true,
        || {
            attempted = true;
            report.attempted += 1;
            capture().inspect_err(|error| failure.set(Some(failure_reason(error, deadline))))
        },
    );
    match (published, attempted) {
        (Ok(true), _) => report.published += 1,
        (Ok(false), true) => report.count(CaptureReason::Unchanged),
        (Err(_), true) => report.count(failure.get().unwrap_or(CaptureReason::Failed)),
        (Ok(false), false) => report.skip(gate.get().unwrap_or(CaptureReason::Busy)),
        (Err(_), false) => report.skip(CaptureReason::Refused),
    }
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
    fn reasons(report: &WorkspaceCaptureReport) -> Vec<(CaptureReason, usize)> {
        report
            .reasons
            .iter()
            .map(|(reason, count)| (*reason, *count))
            .collect()
    }
    fn failing(reason: CaptureReason, deadline: Instant) -> WorkspaceCaptureReport {
        let (_root, paths, _) = fixture();
        recorded_with(&paths, 1, deadline, |_, _| {
            Err(match reason {
                CaptureReason::ServerNotCurrent => ServerFailure::NotCurrent.into(),
                CaptureReason::ServerChanged => ServerFailure::Changed.into(),
                CaptureReason::Deadline => {
                    io::Error::new(io::ErrorKind::TimedOut, "Workspace budget exhausted.")
                }
                _ => io::Error::other("tmux read failed"),
            })
        })
        .unwrap()
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
    fn the_backstop_examines_the_built_in_store_only_once_it_exists_and_coalesces_it() {
        let root = TestDirectory::new();
        let paths = paths(&root.path);
        let mut report = WorkspaceCaptureReport::default();
        recorded_builtin(
            &paths,
            &mut report,
            1,
            Instant::now() + CAPTURE_BUDGET,
            |_| panic!("an absent store is never initialized"),
        )
        .unwrap();
        assert_eq!(
            (report.attempted, report.published, report.skipped),
            (0, 0, 0)
        );
        assert!(!paths.global_dir.join("workspace").exists());
        let snapshot = |at| BuiltinSnapshot {
            captured_at_ms: at,
            members: Vec::new(),
        };
        super::super::publish_store_event(&paths, Store::Builtin, || Ok(snapshot(1))).unwrap();
        let deadline = Instant::now() + CAPTURE_BUDGET;
        recorded_builtin(&paths, &mut report, 100_000, deadline, |_| Ok(snapshot(2))).unwrap();
        assert_eq!(reasons(&report), [(CaptureReason::Unchanged, 1)]);
        recorded_builtin(&paths, &mut report, 100_001, deadline, |_| {
            panic!("coalesced")
        })
        .unwrap();
        assert_eq!(report.skipped, 1);
        assert_eq!(report.reasons[&CaptureReason::NotDue], 1);
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
        assert_eq!(reasons(&first), [(CaptureReason::Unchanged, 1)]);
        assert_eq!(files(&directory), before);
        let stamp = fs::read(directory.join(CADENCE)).unwrap();
        let second = recorded_with(&paths, 39_999, deadline, |_, _| {
            panic!("unchanged attempt must coalesce")
        })
        .unwrap();
        assert_eq!((second.attempted, second.skipped), (0, 1));
        assert_eq!(reasons(&second), [(CaptureReason::NotDue, 1)]);
        assert_eq!(fs::read(directory.join(CADENCE)).unwrap(), stamp);
        assert_eq!(files(&directory), before);
        snapshot.windows[0].name = "changed".into();
        snapshot.captured_at_ms = 40_000;
        let changed = recorded_with(&paths, 40_000, deadline, |_, _| Ok(snapshot.clone())).unwrap();
        assert_eq!((changed.attempted, changed.published), (1, 1));
        assert!(changed.reasons.is_empty());
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
        assert_eq!(reasons(&failed), [(CaptureReason::Failed, 1)]);
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
        assert_eq!(reasons(&skipped), [(CaptureReason::NotDue, 1)]);
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
        assert_eq!(reasons(&busy), [(CaptureReason::Busy, 1)]);
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
        assert_eq!(reasons(&refused), [(CaptureReason::Refused, 1)]);
        assert_eq!(fs::read_to_string(target).unwrap(), "do not replace");
        fs::remove_file(directory.join(CADENCE)).unwrap();
        crate::private_file::replace(&directory.join(CADENCE), br#"{"version":99}"#).unwrap();
        let refused = recorded_with(&paths, 1, deadline, |_, _| {
            panic!("malformed coordination must skip")
        })
        .unwrap();
        assert_eq!(refused.skipped, 1);
        assert_eq!(reasons(&refused), [(CaptureReason::Refused, 1)]);
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
        assert_eq!(reasons(&expired), [(CaptureReason::Deadline, 1)]);
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
    #[test]
    fn a_failed_attempt_names_why_and_a_spent_budget_outranks_a_pre_check() {
        let live = Instant::now() + std::time::Duration::from_secs(5);
        for reason in [
            CaptureReason::ServerNotCurrent,
            CaptureReason::ServerChanged,
            CaptureReason::Deadline,
            CaptureReason::Failed,
        ] {
            let report = failing(reason, live);
            assert_eq!(
                (report.attempted, report.published, report.skipped),
                (1, 0, 0),
                "{reason:?}"
            );
            assert_eq!(reasons(&report), [(reason, 1)]);
        }
        let spent = Instant::now();
        assert_eq!(
            failure_reason(&ServerFailure::NotCurrent.into(), spent),
            CaptureReason::Deadline
        );
        assert_eq!(
            failure_reason(&ServerFailure::Changed.into(), spent),
            CaptureReason::ServerChanged
        );
    }
    #[test]
    fn every_examined_record_is_published_or_has_exactly_one_reason() {
        let (root, paths, first) = fixture();
        let mut second = sample();
        second.server.socket = root.path.join("second.sock").display().to_string();
        publish_event(&paths, &second.server.socket, || Ok(second.clone())).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let report = recorded_with(&paths, 1, deadline, |previous, _| {
            if previous.socket == first.server.socket {
                Ok(first.clone())
            } else {
                Err(io::Error::other("tmux read failed"))
            }
        })
        .unwrap();
        assert_eq!(report.attempted, 2);
        assert_eq!(report.published + report.reasons.values().sum::<usize>(), 2);
        assert_eq!(
            reasons(&report),
            [(CaptureReason::Unchanged, 1), (CaptureReason::Failed, 1)]
        );
    }
    #[test]
    fn reasons_are_closed_snake_case_tokens_and_omitted_when_empty() {
        assert_eq!(
            serde_json::to_value(WorkspaceCaptureReport::default()).unwrap(),
            serde_json::json!({"attempted": 0, "published": 0, "skipped": 0})
        );
        let mut report = WorkspaceCaptureReport::default();
        for reason in [
            CaptureReason::Deadline,
            CaptureReason::Busy,
            CaptureReason::NotDue,
            CaptureReason::Refused,
            CaptureReason::Unchanged,
            CaptureReason::ServerNotCurrent,
            CaptureReason::ServerChanged,
            CaptureReason::Failed,
        ] {
            report.count(reason);
        }
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({"attempted": 0, "published": 0, "skipped": 0, "reasons": {
                "deadline": 1, "busy": 1, "not_due": 1, "refused": 1, "unchanged": 1,
                "server_not_current": 1, "server_changed": 1, "failed": 1,
            }})
        );
    }
}
