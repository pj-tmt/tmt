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
    workspace::{ExternalCommand, MAX_BYTES, Snapshot, identity_for_pane},
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
    capture: impl FnOnce() -> io::Result<Snapshot>,
) -> io::Result<bool> {
    if !(ConfigFiles {
        paths: paths.clone(),
    })
    .workspace_snapshot_enabled()
    .map_err(io::Error::other)?
    {
        return Ok(false);
    }
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
    match crate::bounded_file::read_no_follow(&destination, MAX_BYTES) {
        Ok(bytes) => {
            decode(&bytes)?;
        }
        Err(crate::bounded_file::FileReadError::Io(error))
            if error.kind() == io::ErrorKind::NotFound =>
        {
            ()
        }
        Err(error) => return Err(io::Error::other(error)),
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
    })
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
pub struct DispatchMarker {
    paths: ConfigPaths,
    host: Host,
    caller: CallerEnvironment,
    socket: String,
    pane: String,
    document: String,
}

impl DispatchMarker {
    pub fn prepare(name: &str, args: &[std::ffi::OsString]) -> Option<Self> {
        let paths = ConfigPaths::discover().ok()?;
        if !(ConfigFiles {
            paths: paths.clone(),
        })
        .workspace_snapshot_enabled()
        .ok()?
        {
            return None;
        }
        let caller = CallerEnvironment::current();
        let socket = caller.selected_server_socket().ok()??.to_owned();
        let pane = caller.pane.as_ref()?.to_str()?.to_owned();
        let host = Host::for_caller(&caller);
        let deadline = Instant::now() + CAPTURE_BUDGET;
        // The same batch proves native caller ancestry before publishing a marker.
        let mut capture = host
            .workspace_capture(&socket, None, Some(&caller), deadline)
            .ok()??;
        let pane_tty = capture.terminals.get(&pane)?.to_owned();
        if !crate::process::terminal::foreground(&pane_tty, u64::from(caller.process_id)) {
            return None;
        }
        let owner = observe_starts(
            &UnixCommandRunner,
            &[u64::from(caller.process_id)],
            deadline,
        )
        .ok()?
        .remove(&u64::from(caller.process_id))?;
        let mut argv = vec!["tmt".to_owned(), name.to_owned()];
        argv.extend(
            args.iter()
                .map(|value| value.to_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()?,
        );
        annotate(&paths, &mut capture).ok()?;
        let document = encode_command(&ExternalCommand { argv, owner }).ok()?;
        host.workspace_command_marker(&socket, &pane, Some(&document), deadline)
            .ok()?;
        // Capture again is unnecessary: amend the already verified topology.
        let mut snapshot = capture.snapshot;
        let command = decode_command(&document)?;
        snapshot
            .panes
            .iter_mut()
            .find(|value| value.id == pane)?
            .command = Some(command);
        let _ = publish_event(&paths, &socket, || {
            snapshot.captured_at_ms = crate::request_runtime::wall_time_ms();
            Ok(snapshot)
        });
        Some(Self {
            paths,
            host,
            caller,
            socket,
            pane,
            document,
        })
    }

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

fn annotate(paths: &ConfigPaths, capture: &mut crate::tmux::WorkspaceCapture) -> io::Result<()> {
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
