//! Inspection and recovery of one Codex enrollment (`tmt channel`). The contract's
//! recovery rule: only the named generation, only when every recorded process is
//! conclusively gone, under the record lock and only while the record is the one
//! observed. The generation directory is cleaned only when its app-server was
//! recorded and is gone, file by known file and never recursively. Nothing here
//! signals, sends or pastes.

use super::{
    record::{Foreground, Process, Record, Removal, Store, shell_quote},
    server::{CAPABILITY_FILE, LOG_FILE},
};
use crate::{
    process::CommandRunner,
    runtime::channel::{
        ChannelFault, EnrollmentReport, EnrollmentState, EvidenceError, ProcessRole, ProcessState,
        RecordedPane, RecordedProcess, Recovery, RecoveryError, observe_recorded,
    },
};
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Instant,
};

pub fn inspect<R: CommandRunner>(
    runner: &R,
    directory: &Path,
    binding_id: &str,
    deadline: Instant,
) -> Result<Option<EnrollmentReport>, EvidenceError> {
    let store = Store::at(directory);
    Ok(read(&store, binding_id)?.map(|record| observe(runner, &store, &record, deadline).0))
}

pub fn recover<R: CommandRunner>(
    runner: &R,
    directory: &Path,
    binding_id: &str,
    generation: &str,
    deadline: Instant,
) -> Result<Recovery, RecoveryError> {
    let store = Store::at(directory);
    let Some(record) = read(&store, binding_id).map_err(RecoveryError::Invalid)? else {
        return Ok(Recovery::Absent);
    };
    let (report, generation_directory) = observe(runner, &store, &record, deadline);
    if record.generation != generation {
        return Ok(Recovery::OtherGeneration(report));
    }
    match report.state() {
        EnrollmentState::Running => return Err(RecoveryError::Running(Box::new(report))),
        EnrollmentState::Unverifiable => return Err(RecoveryError::Unverifiable(Box::new(report))),
        EnrollmentState::Unconfirmed | EnrollmentState::Ended => {}
    }
    let removal = store
        .recover(&record, || match &generation_directory {
            Some(GenerationDirectory::Proven(path)) => clean(path),
            Some(GenerationDirectory::Unproven(path)) => Ok((Vec::new(), vec![path.clone()])),
            None => Ok((Vec::new(), Vec::new())),
        })
        .map_err(|_| RecoveryError::Failed {
            path: report.record.clone(),
        })?;
    match removal {
        Removal::Absent => Ok(Recovery::Absent),
        Removal::Changed => Err(RecoveryError::Changed(Box::new(report))),
        Removal::Retained(path) => Err(RecoveryError::Failed { path }),
        Removal::Removed((cleaned, kept)) => Ok(Recovery::Recovered {
            removed: [vec![report.record.clone()], cleaned].concat(),
            report,
            kept,
        }),
    }
}

/// The record of the binding, or `None`. A binding ID that is not a UUID names no
/// record of this driver.
fn read(store: &Store, binding_id: &str) -> Result<Option<Record>, EvidenceError> {
    let Ok(path) = store.path(binding_id) else {
        return Ok(None);
    };
    store.read(binding_id).map_err(|_| {
        let action = match path.to_str() {
            Some(text) => format!("remove only this record with: rm -- {}", shell_quote(text)),
            None => "remove only this record with a tool that preserves its non-UTF-8 path".into(),
        };
        EvidenceError::at(ChannelFault::InvalidRecord, &path).with_detail(format!(
            "It names no process that could be verified, so it cannot be recovered with `tmt channel recover`. After confirming that the session it belonged to, its foreground and its app-server are gone, {action}."
        ))
    })
}

enum GenerationDirectory {
    /// Its app-server was recorded and every recorded process is gone.
    Proven(PathBuf),
    /// An app-server may run that the record never named (it was not ready yet).
    Unproven(PathBuf),
}

fn observe<R: CommandRunner>(
    runner: &R,
    store: &Store,
    record: &Record,
    deadline: Instant,
) -> (EnrollmentReport, Option<GenerationDirectory>) {
    let observe = |role, process: &Process| RecordedProcess {
        role,
        pid: process.pid,
        start: process.start.clone(),
        state: process
            .incarnation()
            .map_or(ProcessState::Unobservable, |incarnation| {
                observe_recorded(runner, &incarnation, deadline)
            }),
    };
    let mut processes = vec![observe(ProcessRole::LaunchOwner, &record.launch_owner)];
    if let Foreground::Known(foreground) = &record.foreground {
        processes.push(observe(ProcessRole::Foreground, foreground));
    }
    if let Some(ready) = &record.ready {
        processes.push(observe(ProcessRole::Endpoint, &ready.server));
    }
    let all_gone = processes
        .iter()
        .all(|process| process.state == ProcessState::Gone);
    let generation_directory = store
        .generation_directory(record)
        .ok()
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .map(|path| {
            if record.ready.is_some() && all_gone {
                GenerationDirectory::Proven(path)
            } else {
                GenerationDirectory::Unproven(path)
            }
        });
    let pane = record.attribution.as_ref().map(|attribution| RecordedPane {
        host: attribution.host.clone(),
        server_id: attribution.server_id.clone(),
        socket_path: attribution.socket_path.clone(),
        pane_id: attribution.pane_id.clone(),
        pane_pid: attribution.pane_pid,
    });
    let where_ = pane.as_ref().map_or_else(
        || "its pane (the record names none)".to_owned(),
        |pane| {
            format!(
                "pane {} on the tmux server at {}",
                pane.pane_id, pane.socket_path
            )
        },
    );
    let mut verification = match &record.foreground {
        Foreground::Known(_) => format!(
            "Every process this enrollment recorded must be gone; TMT checks them itself. Recovery leaves {where_} and every process alone."
        ),
        Foreground::Unknown => format!(
            "The launch ended before it recorded its Codex foreground, so TMT cannot check it. Before recovering, confirm that no Codex started by launch owner {} (started {}) still runs in {where_}.",
            record.launch_owner.pid, record.launch_owner.start
        ),
    };
    if record.ready.is_none() {
        verification.push_str(
            " Its app-server was never recorded, so its generation directory is left in place.",
        );
    }
    let (removes, keeps) = match &generation_directory {
        Some(GenerationDirectory::Proven(path)) => (
            vec![
                path.join(CAPABILITY_FILE),
                path.join(LOG_FILE),
                path.clone(),
            ],
            Vec::new(),
        ),
        Some(GenerationDirectory::Unproven(path)) => (Vec::new(), vec![path.clone()]),
        None => (Vec::new(), Vec::new()),
    };
    let path = store
        .path(&record.binding_id)
        .expect("a valid record names a UUID binding");
    let report = EnrollmentReport {
        record: path.clone(),
        binding_id: record.binding_id.clone(),
        identity_id: record
            .attribution
            .as_ref()
            .map(|attribution| attribution.identity_id.clone()),
        generation: record.generation.clone(),
        pane,
        processes,
        foreground_recorded: matches!(record.foreground, Foreground::Known(_)),
        verification,
        removes: [vec![path], removes].concat(),
        keeps,
    };
    (report, generation_directory)
}

/// Removes the launch's known files and then the directory itself, never
/// recursively. A file that is not a regular file of this user, and any entry
/// the launch did not create, is left and reported; the directory then stays.
/// A known file of this user that cannot be removed (a leftover capability) is
/// an `Err`, so the record stays and the same recovery can finish it later.
fn clean(directory: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), PathBuf> {
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    let owner = nix::unistd::geteuid().as_raw();
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() && metadata.uid() == owner => {}
        Ok(_) => return Ok((removed, vec![directory.to_owned()])),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((removed, kept)),
        Err(_) => return Err(directory.to_owned()),
    }
    for name in [CAPABILITY_FILE, LOG_FILE] {
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && metadata.uid() == owner => {
                fs::remove_file(&path).map_err(|_| path.clone())?;
                removed.push(path);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => kept.push(path),
            Err(_) => return Err(path),
        }
    }
    match fs::read_dir(directory) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) if !kept.contains(&entry.path()) => kept.push(entry.path()),
                    Ok(_) => {}
                    Err(_) => kept.push(directory.to_owned()),
                }
            }
        }
        Err(_) => kept.push(directory.to_owned()),
    }
    if kept.is_empty() {
        match fs::remove_dir(directory) {
            Ok(()) => removed.push(directory.to_owned()),
            Err(_) => kept.push(directory.to_owned()),
        }
    } else if !kept.iter().any(|path| path == directory) {
        kept.push(directory.to_owned());
    }
    Ok((removed, kept))
}

#[cfg(test)]
mod tests;
