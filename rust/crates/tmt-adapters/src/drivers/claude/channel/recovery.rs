//! Inspection and recovery of one Claude enrollment (`tmt channel`). The contract's
//! "Recovery" section owns the rule: only the named generation, only when every
//! recorded process is conclusively gone, under the directory lock and only while
//! the record is the one observed. Nothing here signals, sends or pastes.

use super::{
    LOCK_FILE, Process, RECORD_VERSION, Record, locked, read_record, record_path, socket_path,
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
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    time::Instant,
};

pub(super) fn inspect<R: CommandRunner>(
    runner: &R,
    directory: &Path,
    binding_id: &str,
    deadline: Instant,
) -> Result<Option<EnrollmentReport>, EvidenceError> {
    Ok(read(directory, binding_id)?.map(|record| report(runner, directory, &record, deadline)))
}

pub(super) fn recover<R: CommandRunner>(
    runner: &R,
    directory: &Path,
    binding_id: &str,
    generation: &str,
    deadline: Instant,
) -> Result<Recovery, RecoveryError> {
    let Some(record) = read(directory, binding_id).map_err(RecoveryError::Invalid)? else {
        return Ok(Recovery::Absent);
    };
    let report = report(runner, directory, &record, deadline);
    if record.generation != generation {
        return Ok(Recovery::OtherGeneration(report));
    }
    match report.state() {
        EnrollmentState::Running => {
            return Err(RecoveryError::Running(Box::new(report)));
        }
        EnrollmentState::Unverifiable => {
            return Err(RecoveryError::Unverifiable(Box::new(report)));
        }
        _ => {}
    }
    let lock = directory.join(LOCK_FILE);
    locked(directory, || {
        // An exact incarnation that is gone never returns, so the observation
        // above still holds as long as the record is the very one observed. Any
        // change (a published foreground, a replacement) refuses.
        match read_record(directory, binding_id) {
            Ok(Some(current)) if current == record => {}
            Ok(None) => return Ok(Recovery::Absent),
            _ => return Err(RecoveryError::Changed(Box::new(report.clone()))),
        }
        let mut removed = Vec::new();
        let mut kept = Vec::new();
        // The socket goes first: if anything fails, the record still names the
        // enrollment and the same recovery can run again.
        let socket = socket_path(directory, binding_id);
        match fs::symlink_metadata(&socket) {
            Ok(metadata) if metadata.file_type().is_socket() => {
                fs::remove_file(&socket).map_err(|_| RecoveryError::Failed {
                    path: socket.clone(),
                })?;
                removed.push(socket);
            }
            Ok(_) => kept.push(socket),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(RecoveryError::Failed { path: socket }),
        }
        let path = record_path(directory, binding_id);
        fs::remove_file(&path).map_err(|_| RecoveryError::Failed { path: path.clone() })?;
        removed.insert(0, path);
        Ok(Recovery::Recovered {
            report: report.clone(),
            removed,
            kept,
        })
    })
    .map_err(|_| RecoveryError::Failed { path: lock })?
}

/// This driver's record of the binding, or `None`. A binding ID that is not a
/// UUID names no record of this driver (and no path outside the directory).
fn read(directory: &Path, binding_id: &str) -> Result<Option<Record>, EvidenceError> {
    if !crate::runtime::channel::valid_enrollment_id(binding_id) {
        return Ok(None);
    }
    match read_record(directory, binding_id) {
        Ok(Some(record)) if record.version == RECORD_VERSION && record.binding_id == binding_id => {
            Ok(Some(record))
        }
        Ok(None) => Ok(None),
        Ok(Some(_)) | Err(_) => Err(EvidenceError::at(
            ChannelFault::InvalidRecord,
            &record_path(directory, binding_id),
        )
        .with_detail(format!(
            "It names no process that could be verified, so it cannot be recovered with `tmt channel recover`. After confirming that the session it belonged to is gone, remove it with: {}.",
            super::recovery(directory, binding_id)
        ))),
    }
}

fn report<R: CommandRunner>(
    runner: &R,
    directory: &Path,
    record: &Record,
    deadline: Instant,
) -> EnrollmentReport {
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
    processes.extend(
        record
            .foreground
            .iter()
            .map(|process| observe(ProcessRole::Foreground, process)),
    );
    processes.extend(
        record
            .claude
            .iter()
            .map(|process| observe(ProcessRole::Provider, process)),
    );
    // Only the Claude process stands in for an unpublished foreground: it is the
    // foreground itself (the channel server is its child).
    let foreground_recorded = record.foreground.is_some() || record.claude.is_some();
    let pane = record.pane.as_ref().map(|pane| RecordedPane {
        host: pane.host.clone(),
        server_id: pane.server_id.clone(),
        socket_path: pane.socket_path.clone(),
        pane_id: pane.pane_id.clone(),
        pane_pid: pane.pane_pid,
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
    let verification = if foreground_recorded {
        format!(
            "Every process this enrollment recorded must be gone; TMT checks them itself. Recovery leaves {where_} and every process alone."
        )
    } else {
        format!(
            "The launch ended before it recorded its Claude process, so TMT cannot check it. Before recovering, confirm that no Claude started by launch owner {} (started {}) still runs in {where_}.",
            record.launch_owner.pid, record.launch_owner.start
        )
    };
    let socket = socket_path(directory, &record.binding_id);
    let (removes, keeps): (Vec<PathBuf>, Vec<PathBuf>) = match fs::symlink_metadata(&socket) {
        Ok(metadata) if metadata.file_type().is_socket() => (vec![socket], Vec::new()),
        Ok(_) => (Vec::new(), vec![socket]),
        Err(_) => (Vec::new(), Vec::new()),
    };
    EnrollmentReport {
        record: record_path(directory, &record.binding_id),
        binding_id: record.binding_id.clone(),
        identity_id: record.identity_id.clone(),
        generation: record.generation.clone(),
        pane,
        processes,
        foreground_recorded,
        verification,
        removes: [vec![record_path(directory, &record.binding_id)], removes].concat(),
        keeps,
    }
}

#[cfg(test)]
mod tests;
