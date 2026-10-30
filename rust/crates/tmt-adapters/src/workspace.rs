//! Private, advisory workspace publication. No scheduler or authority store.

mod capture;
pub use capture::{CaptureReason, WorkspaceCaptureReport, capture_hook, capture_recorded};
pub mod generations;
pub mod plan;
pub mod restore;
pub mod saved;
#[cfg(test)]
mod tests;
mod wire;

use crate::{
    config::{ConfigFiles, ConfigPaths},
    host::{CallerEnvironment, Host, OperationOptions},
    process::{CommandRunner, UnixCommandRunner, runtime::observe_starts},
    storage::Storage,
};
use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_core::{
    endpoint::ServerEvidence,
    host::HostKind,
    workspace::{
        ExternalCommand, MAX_BYTES, WorkspaceSnapshot,
        builtin::{BuiltinMember, BuiltinSnapshot},
        generations::WorkspaceGeneration,
        identity_for_pane,
    },
};

pub use wire::{decode, decode_builtin, decode_command, encode, encode_builtin, encode_command};

/// Which recovery store an operation addresses: one per tmux socket, or the built-in host's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store<'a> {
    Socket(&'a str),
    Builtin,
}

impl Store<'_> {
    pub fn directory(&self, paths: &ConfigPaths) -> PathBuf {
        match self {
            Self::Socket(socket) => paths.workspace_directory(socket),
            Self::Builtin => paths.builtin_workspace_directory(),
        }
    }
}

/// One kind of recovery document. The store's publication, retention and selection are
/// shared; only the codec, the counts and the shrink rule differ.
pub trait Recovery: Clone + PartialEq + Sized {
    /// Whether this capture (`entry`) is a shrink against the baseline generation and its
    /// document. A shrink never moves the protected baseline.
    fn shrinks(
        &self,
        entry: &WorkspaceGeneration,
        baseline: &Self,
        baseline_entry: &WorkspaceGeneration,
    ) -> bool;
    fn decode(bytes: &[u8]) -> io::Result<Self>;
    fn encode(&self) -> io::Result<Vec<u8>>;
    fn captured_at_ms(&self) -> u64;
    fn set_captured_at_ms(&mut self, captured_at_ms: u64);
    /// Panes (built-in members) and distinct annotated identities.
    fn counts(&self) -> (usize, usize);
    /// Whether these bytes belong to the store they were read from.
    fn belongs_to(&self, store: Store<'_>) -> bool;
    /// The document to publish for this capture against the baseline document.
    fn against(self, _baseline: &Self) -> Self {
        self
    }
}

impl Recovery for WorkspaceSnapshot {
    fn shrinks(
        &self,
        entry: &WorkspaceGeneration,
        _: &Self,
        baseline_entry: &WorkspaceGeneration,
    ) -> bool {
        entry.loses_half(baseline_entry)
    }
    fn decode(bytes: &[u8]) -> io::Result<Self> {
        decode(bytes)
    }
    fn encode(&self) -> io::Result<Vec<u8>> {
        encode(self)
    }
    fn captured_at_ms(&self) -> u64 {
        self.captured_at_ms
    }
    fn set_captured_at_ms(&mut self, captured_at_ms: u64) {
        self.captured_at_ms = captured_at_ms;
    }
    fn counts(&self) -> (usize, usize) {
        (self.panes.len(), self.agent_count())
    }
    fn belongs_to(&self, store: Store<'_>) -> bool {
        store == Store::Socket(&self.server.socket)
    }
}

impl Recovery for BuiltinSnapshot {
    fn shrinks(&self, _: &WorkspaceGeneration, baseline: &Self, _: &WorkspaceGeneration) -> bool {
        !self.covers(baseline)
    }
    fn decode(bytes: &[u8]) -> io::Result<Self> {
        decode_builtin(bytes)
    }
    fn encode(&self) -> io::Result<Vec<u8>> {
        encode_builtin(self)
    }
    fn captured_at_ms(&self) -> u64 {
        self.captured_at_ms
    }
    fn set_captured_at_ms(&mut self, captured_at_ms: u64) {
        self.captured_at_ms = captured_at_ms;
    }
    fn counts(&self) -> (usize, usize) {
        (self.members.len(), self.agent_count())
    }
    fn belongs_to(&self, store: Store<'_>) -> bool {
        store == Store::Builtin
    }
    fn against(self, baseline: &Self) -> Self {
        BuiltinSnapshot::against(self, baseline)
    }
}
pub const CAPTURE_BUDGET: Duration = Duration::from_millis(200);
/// Includes the subprocess owner's maximum cleanup time.
pub const HOOK_CAPTURE_BOUND: Duration =
    CAPTURE_BUDGET.saturating_add(crate::process::CLEANUP_TIMEOUT);
pub const HOOK_RESERVE: Duration = Duration::from_millis(200);

fn private_directory(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other(
            "Workspace directory must be private and owned.",
        ));
    }
    Ok(())
}

/// Coalesce simultaneous requests without waiting. Keep the lock inode stable,
/// and publish complete bytes atomically only after all evidence is available.
pub fn publish_event(
    paths: &ConfigPaths,
    socket: &str,
    capture: impl FnOnce() -> io::Result<WorkspaceSnapshot>,
) -> io::Result<bool> {
    publish_store_event(paths, Store::Socket(socket), capture)
}

fn publish_store_event<D: Recovery>(
    paths: &ConfigPaths,
    store: Store<'_>,
    capture: impl FnOnce() -> io::Result<D>,
) -> io::Result<bool> {
    if !(ConfigFiles {
        paths: paths.clone(),
    })
    .workspace_snapshot_enabled()
    .map_err(io::Error::other)?
    {
        return Ok(false);
    }
    publish(paths, store, |_| true, capture)
}

/// A built-in launch, binding or lifecycle event. This is what creates the built-in
/// store: the launcher calls it once its member's marker is committed and ready.
pub fn capture_builtin_event(paths: &ConfigPaths, deadline: Instant) -> io::Result<bool> {
    publish_store_event(paths, Store::Builtin, || capture_builtin(paths, deadline))
}

/// A built-in member stopped or removed on purpose leaves the protected baseline, so the
/// default restore no longer brings it back; logout, reboot and crashes never call this.
/// Only an existing store changes. A busy publisher is waited for briefly, since dropping
/// an explicit stop would restore that member later.
pub fn forget_builtin_member(paths: &ConfigPaths, identity: &str) -> io::Result<bool> {
    let directory = paths.builtin_workspace_directory();
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        other => {
            other?;
        }
    }
    private_directory(&directory)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let _lock = loop {
        match crate::file_lock::exclusive(&directory.join("publication.lock")) {
            Ok(lock) => break lock,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    };
    let Some(catalogue) = generations::read(&directory)? else {
        return Ok(false);
    };
    let Some(protected) = catalogue.protected() else {
        return Ok(false);
    };
    let baseline = decode_builtin(&generations::acquire::<BuiltinSnapshot>(
        &directory, protected,
    )?)?;
    let Some(next) = baseline.without(identity, crate::request_runtime::wall_time_ms()) else {
        return Ok(false);
    };
    generations::publish_baseline(&directory, &next, catalogue)?;
    Ok(true)
}

/// The built-in members whose binding, supervisor and program incarnation agree, as the
/// tmux capture admits an annotation. Unverified members are left out; nothing is
/// started, stopped or repaired. A spent budget fails rather than publish a partial set.
fn capture_builtin(paths: &ConfigPaths, deadline: Instant) -> io::Result<BuiltinSnapshot> {
    Ok(BuiltinSnapshot {
        members: live_builtin_members(paths, deadline)?,
        captured_at_ms: crate::request_runtime::wall_time_ms(),
    })
}

/// A built-in preview's input: current durable records for the recorded UUIDs, and the UUIDs
/// running on the built-in host now. Read only; it grants no restore authority.
pub fn builtin_plan_input(
    paths: &ConfigPaths,
    snapshot: &BuiltinSnapshot,
    deadline: Instant,
) -> io::Result<(
    Vec<tmt_core::workspace::StoredIdentity>,
    std::collections::HashSet<String>,
)> {
    if !paths.database.exists() {
        return Ok(Default::default());
    }
    let ids = snapshot
        .members
        .iter()
        .map(|member| member.identity.id.as_str())
        .collect::<Vec<_>>();
    let identities =
        Storage::workspace_plan_identities(&paths.database, &ids).map_err(io::Error::other)?;
    let live = live_builtin_members(paths, deadline)?
        .into_iter()
        .map(|member| member.identity.id)
        .collect();
    Ok((identities, live))
}

fn live_builtin_members(paths: &ConfigPaths, deadline: Instant) -> io::Result<Vec<BuiltinMember>> {
    let records = if paths.database.exists() {
        Storage::builtin_workspace_identities(&paths.database).map_err(io::Error::other)?
    } else {
        Vec::new()
    };
    let mut members = Vec::new();
    for binding in records
        .iter()
        .filter_map(|record| record.entry.binding.as_ref())
    {
        if Instant::now() >= deadline {
            break;
        }
        let Ok(snapshot) = Host::for_server(&binding.server).observe_snapshot(OperationOptions {
            deadline: Some(deadline),
            pane_ids: Some(std::slice::from_ref(&binding.pane_id)),
        }) else {
            continue;
        };
        members.extend(snapshot.panes.iter().filter_map(|pane| {
            Some(BuiltinMember {
                cwd: pane.cwd.clone(),
                identity: identity_for_pane(
                    &records,
                    &snapshot.server,
                    &pane.id,
                    pane.pane_pid,
                    pane.pane_incarnation.as_deref(),
                    pane.marker.as_ref(),
                )?,
            })
        }));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Workspace budget exhausted.",
        ));
    }
    // Storage order is not recovery content; an unchanged set must compare equal.
    members.sort_by(|left, right| {
        (&left.identity.id, &left.identity.binding)
            .cmp(&(&right.identity.id, &right.identity.binding))
    });
    Ok(members)
}

/// One previous-document read gates optional capture under the publication lock.
fn publish_refresh(
    paths: &ConfigPaths,
    socket: &str,
    now_ms: u64,
    capture: impl FnOnce() -> io::Result<WorkspaceSnapshot>,
) -> io::Result<bool> {
    let (enabled, interval_ms) = ConfigFiles {
        paths: paths.clone(),
    }
    .workspace_snapshot_policy()
    .map_err(io::Error::other)?;
    if !enabled || interval_ms == 0 {
        return Ok(false);
    }
    publish(
        paths,
        Store::Socket(socket),
        |previous| tmt_core::workspace::command_refresh_due(previous, now_ms, interval_ms),
        capture,
    )
}

fn publish<D: Recovery>(
    paths: &ConfigPaths,
    store: Store<'_>,
    due: impl FnOnce(Option<u64>) -> bool,
    capture: impl FnOnce() -> io::Result<D>,
) -> io::Result<bool> {
    publish_gated(paths, store, |_| Ok(true), due, capture)
}

fn publish_gated<D: Recovery>(
    paths: &ConfigPaths,
    store: Store<'_>,
    admit: impl FnOnce(&Path) -> io::Result<bool>,
    due: impl FnOnce(Option<u64>) -> bool,
    capture: impl FnOnce() -> io::Result<D>,
) -> io::Result<bool> {
    fs::create_dir_all(&paths.global_dir)?;
    private_directory(&paths.global_dir.join("workspace"))?;
    let directory = store.directory(paths);
    private_directory(&directory)?;
    let _lock = match crate::file_lock::exclusive(&directory.join("publication.lock")) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
        Err(error) => return Err(error),
    };
    if !admit(&directory)? {
        return Ok(false);
    }
    let destination = directory.join("latest.json");
    // A malformed legacy projection is not permission to replace user bytes,
    // even when a separately committed catalogue still permits recovery.
    let legacy = match crate::bounded_file::read_no_follow(&destination, MAX_BYTES) {
        Ok(bytes) => {
            D::decode(&bytes)?;
            Some(bytes)
        }
        Err(crate::bounded_file::FileReadError::Io(error))
            if error.kind() == io::ErrorKind::NotFound =>
        {
            None
        }
        Err(error) => return Err(io::Error::other(error)),
    };
    let catalogue = generations::read(&directory)?;
    let previous = match &catalogue {
        Some(catalogue) => Some(generations::acquire::<D>(
            &directory,
            catalogue.latest().expect("nonempty catalogue"),
        )?),
        None => legacy,
    };
    let previous_snapshot = previous
        .as_ref()
        .map(|bytes| D::decode(bytes))
        .transpose()?;
    let previous_ms = previous_snapshot.as_ref().map(D::captured_at_ms);
    if !due(previous_ms) {
        return Ok(false);
    }
    let snapshot = capture()?;
    if !snapshot.belongs_to(store) {
        return Err(io::Error::other("Workspace socket changed."));
    }
    let bytes = snapshot.encode()?;
    if previous_snapshot.is_some_and(|mut previous| {
        previous.set_captured_at_ms(snapshot.captured_at_ms());
        previous == snapshot
    }) {
        return Ok(false);
    }
    generations::publish(
        &directory,
        previous.as_deref(),
        &snapshot,
        &bytes,
        catalogue,
    )?;
    Ok(true)
}

/// All optional work shares one finite deadline. Errors are advisory to callers.
pub fn capture_event<R: CommandRunner + Clone>(
    paths: &ConfigPaths,
    host: &Host<R>,
    server: Option<&ServerEvidence>,
    caller: Option<&CallerEnvironment>,
    deadline: Instant,
) -> io::Result<bool> {
    if server.is_some_and(|server| server.host == HostKind::pty()) {
        return capture_builtin_event(paths, deadline);
    }
    let socket = match server {
        Some(server) => server.socket_path.as_str(),
        None => match caller.and_then(|caller| caller.selected_server_socket().ok().flatten()) {
            Some(socket) => socket,
            None => return Ok(false),
        },
    };
    publish_event(paths, socket, || {
        capture_snapshot(paths, host, socket, server, caller, deadline)
    })
}

/// Candidate coordinates are cheap; a due capture still verifies native ancestry.
pub fn refresh_command<R: CommandRunner + Clone>(
    paths: &ConfigPaths,
    host: &Host<R>,
    caller: &CallerEnvironment,
    deadline: Instant,
) -> io::Result<bool> {
    let Some((_, _, Some(socket))) = caller
        .pane_locators()
        .into_iter()
        .find(|(kind, _, _)| *kind == tmt_core::host::HostKind::Tmux)
    else {
        return Ok(false);
    };
    publish_refresh(
        paths,
        socket,
        crate::request_runtime::wall_time_ms(),
        || capture_snapshot(paths, host, socket, None, Some(caller), deadline),
    )
}

fn capture_snapshot<R: CommandRunner + Clone>(
    paths: &ConfigPaths,
    host: &Host<R>,
    socket: &str,
    server: Option<&ServerEvidence>,
    caller: Option<&CallerEnvironment>,
    deadline: Instant,
) -> io::Result<WorkspaceSnapshot> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Workspace budget exhausted.",
        ));
    }
    let mut capture = host
        .workspace_capture(socket, server, caller, deadline)
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other("Workspace capture unsupported."))?;
    annotate(paths, &mut capture)?;
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Workspace budget exhausted.",
        ));
    }
    capture.snapshot.captured_at_ms = crate::request_runtime::wall_time_ms();
    Ok(capture.snapshot)
}

/// Event-only convenience. It never changes the trigger's output or result.
pub fn refresh_server(paths: &ConfigPaths, host: &Host, server: &ServerEvidence) {
    let _ = capture_event(
        paths,
        host,
        Some(server),
        None,
        Instant::now() + CAPTURE_BUDGET,
    );
}

/// An exec-surviving owned pane marker for any foreground external extension.
pub struct DispatchMarker<R = UnixCommandRunner> {
    paths: ConfigPaths,
    host: Host<R>,
    caller: CallerEnvironment,
    socket: String,
    pane: String,
    document: String,
}

impl DispatchMarker {
    pub fn prepare(name: &str, args: &[std::ffi::OsString]) -> Option<Self> {
        let pid = u64::from(std::process::id());
        prepare_dispatch(
            name,
            args,
            pid,
            crate::process::terminal::foreground,
            || {
                let paths = ConfigPaths::discover().ok()?;
                // Only tmux has a workspace marker port. Optional marker preparation
                // never discovers external drivers or probes their caller.
                let caller = CallerEnvironment {
                    tmux: std::env::var_os("TMUX"),
                    pane: std::env::var_os("TMUX_PANE"),
                    process_id: pid,
                    driver_env: Default::default(),
                };
                // No driver environment, so selection reads nothing that could fail to
                // clean up; optional marker preparation still skips on any error.
                let host = Host::for_caller_with(&caller, UnixCommandRunner).ok()?;
                Some((paths, caller, host, UnixCommandRunner))
            },
        )
    }
}

fn prepare_dispatch<R: CommandRunner + Clone>(
    name: &str,
    args: &[std::ffi::OsString],
    pid: u64,
    foreground: impl Fn(&str, u64) -> bool,
    context: impl FnOnce() -> Option<(ConfigPaths, CallerEnvironment, Host<R>, R)>,
) -> Option<DispatchMarker<R>> {
    // Native controlling-tty evidence precedes config and subprocess work.
    // Agent tool children are not the pane's foreground owner.
    if !foreground("/dev/tty", pid) {
        return None;
    }
    let deadline = Instant::now() + CAPTURE_BUDGET;
    let (paths, caller, host, runner) = context()?;
    if !(ConfigFiles {
        paths: paths.clone(),
    })
    .workspace_snapshot_enabled()
    .ok()?
    {
        return None;
    }
    let socket = caller.selected_server_socket().ok()??.to_owned();
    let pane = caller.pane.as_ref()?.to_str()?.to_owned();
    let tty = host.workspace_pane_tty(&socket, &pane, deadline).ok()??;
    if !foreground(&tty, pid) {
        return None;
    }
    let owner = observe_starts(&runner, &[pid], deadline)
        .ok()?
        .remove(&pid)?;
    let mut argv = vec!["tmt".to_owned(), name.to_owned()];
    argv.extend(
        args.iter()
            .map(|value| value.to_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?,
    );
    let document = encode_command(&ExternalCommand { argv, owner }).ok()?;
    host.workspace_command_marker(&socket, &pane, Some(&document), deadline)
        .ok()?;
    // Advisory data only: the next eligible capture admits the marker through
    // exact incarnation, ancestry and foreground evidence. No snapshot IO here.
    Some(DispatchMarker {
        paths,
        host,
        caller,
        socket,
        pane,
        document,
    })
}

impl<R: CommandRunner + Clone> DispatchMarker<R> {
    /// Called only after failed exec while this exact incarnation is still here.
    pub fn failed(self) {
        let _ = self.host.clear_workspace_command(
            &self.socket,
            &self.pane,
            &self.document,
            Instant::now() + CAPTURE_BUDGET,
        );
        let _ = capture_event(
            &self.paths,
            &self.host,
            None,
            Some(&self.caller),
            Instant::now() + CAPTURE_BUDGET,
        );
    }
}

fn annotate(paths: &ConfigPaths, capture: &mut crate::host::WorkspaceCapture) -> io::Result<()> {
    let records = if paths.database.exists() {
        Storage::workspace_identities(&paths.database, &capture.snapshot.server.socket)
            .map_err(io::Error::other)?
    } else {
        Vec::new()
    };
    if let Some(server) = &capture.binding_server {
        for pane in &mut capture.snapshot.panes {
            if let Some((_, pid, marker)) =
                capture.evidence.iter().find(|(id, _, _)| id == &pane.id)
            {
                pane.identity = identity_for_pane(
                    &records,
                    server,
                    &pane.id,
                    *pid,
                    capture.starts.get(pid).map(|value| value.start_identity()),
                    marker.as_ref(),
                );
            }
        }
    }
    Ok(())
}
