//! Private, advisory workspace publication. No scheduler or authority store.

#[cfg(test)]
mod tests;
mod wire;

use crate::{
    config::{ConfigFiles, ConfigPaths},
    host::{CallerEnvironment, Host},
    process::{CommandRunner, UnixCommandRunner, runtime::observe_starts},
    storage::Storage,
};
use std::{
    fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::Path,
    time::{Duration, Instant},
};
use tmt_core::{
    endpoint::ServerEvidence,
    workspace::{ExternalCommand, MAX_BYTES, WorkspaceSnapshot, identity_for_pane},
};

pub use wire::{decode, decode_command, encode, encode_command};
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
    if !(ConfigFiles {
        paths: paths.clone(),
    })
    .workspace_snapshot_enabled()
    .map_err(io::Error::other)?
    {
        return Ok(false);
    }
    publish(paths, socket, |_| true, capture)
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
        socket,
        |previous| tmt_core::workspace::command_refresh_due(previous, now_ms, interval_ms),
        capture,
    )
}

fn publish(
    paths: &ConfigPaths,
    socket: &str,
    due: impl FnOnce(Option<u64>) -> bool,
    capture: impl FnOnce() -> io::Result<WorkspaceSnapshot>,
) -> io::Result<bool> {
    fs::create_dir_all(&paths.global_dir)?;
    private_directory(&paths.global_dir.join("workspace"))?;
    let directory = paths.workspace_directory(socket);
    private_directory(&directory)?;
    let _lock = match crate::file_lock::exclusive(&directory.join("publication.lock")) {
        Ok(lock) => lock,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
        Err(error) => return Err(error),
    };
    let destination = directory.join("latest.json");
    let previous_ms = match crate::bounded_file::read_no_follow(&destination, MAX_BYTES) {
        Ok(bytes) => Some(decode(&bytes)?.captured_at_ms),
        Err(crate::bounded_file::FileReadError::Io(error))
            if error.kind() == io::ErrorKind::NotFound =>
        {
            None
        }
        Err(error) => return Err(io::Error::other(error)),
    };
    if !due(previous_ms) {
        return Ok(false);
    }
    let snapshot = capture()?;
    if snapshot.server.socket != socket {
        return Err(io::Error::other("Workspace socket changed."));
    }
    let bytes = encode(&snapshot)?;
    crate::private_file::replace(&destination, &bytes)?;
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
                let host = Host::for_caller_with(&caller, UnixCommandRunner);
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
