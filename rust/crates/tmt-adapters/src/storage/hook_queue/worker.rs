//! The queue's one event-driven startup owner; the CLI handles the finite drain.
use crate::process::detached;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::Instant,
};

pub(super) fn start(database: &Path, deadline: Instant) -> io::Result<()> {
    let Some(executable) = crate::core_executable::declared() else {
        return Ok(());
    };
    start_logged(directory(database)?, executable, deadline)
}

fn directory(database: &Path) -> io::Result<&Path> {
    let configured = crate::config::ConfigPaths::discover().map_err(io::Error::other)?;
    if configured.database != database {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The queue worker must use the declared Core data root",
        ));
    }
    database
        .parent()
        .ok_or_else(|| io::Error::other("No hook queue directory"))
}

pub(super) fn note_claim_failure(
    database: &Path,
    error: &io::Error,
    deadline: Instant,
) -> io::Result<()> {
    if crate::core_executable::declared().is_none() || Instant::now() >= deadline {
        return Ok(());
    }
    let (_, mut log) = open_log(directory(database)?)?;
    write_failure(&mut log, "claim", error)
}

fn open_log(directory: &Path) -> io::Result<(String, File)> {
    let _ = detached::prune_failed_logs(
        directory,
        |name| {
            name.strip_prefix("hook-flush-")
                .and_then(|name| name.strip_suffix(".log"))
                .is_some_and(|id| {
                    id.len() == 36
                        && id
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
                })
        },
        detached::FAILED_LOG_RETENTION,
    );
    let id = tmt_core::operation::new_operation_id();
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join(format!("hook-flush-{id}.log")))?;
    Ok((id, log))
}

fn write_failure(log: &mut File, stage: &str, error: &io::Error) -> io::Result<()> {
    // Fixed stage/error-kind tokens contain no final text, argv or private paths.
    writeln!(
        log,
        "hook_flush_start=unconfirmed stage={stage} queue_publication=accepted error_kind={:?}",
        error.kind()
    )
}

pub(super) fn start_logged(
    directory: &Path,
    executable: &Path,
    deadline: Instant,
) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Hook flusher startup budget expired",
        ));
    }
    let (id, log) = open_log(directory)?;
    let mut diagnostic = log.try_clone()?;
    let started = detached::start_until(
        executable.as_os_str(),
        &["__flush".into(), id.into()],
        log,
        deadline,
    );
    if let Err(error) = &started {
        // Preserve the original startup/cleanup error and its process owner.
        let _ = write_failure(&mut diagnostic, "spawn", error);
    }
    started
}
