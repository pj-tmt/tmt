//! The queue's one event-driven startup owner; the CLI handles the finite drain.
use crate::process::detached;
use std::{fs::OpenOptions, io, os::unix::fs::OpenOptionsExt, path::Path, time::Instant};
pub(super) fn start(database: &Path, deadline: Instant) -> io::Result<()> {
    let Some(executable) = crate::core_executable::declared() else {
        return Ok(());
    };
    let configured = crate::config::ConfigPaths::discover().map_err(io::Error::other)?;
    if configured.database != database {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The queue worker must use the declared Core data root",
        ));
    }
    let directory = database
        .parent()
        .ok_or_else(|| io::Error::other("No hook queue directory"))?;
    let log_id = tmt_core::operation::new_operation_id();
    let _ = detached::prune_failed_logs(
        directory,
        |name| {
            name.strip_prefix("hook-flush-")
                .and_then(|s| s.strip_suffix(".log"))
                .is_some_and(|id| {
                    id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
                })
        },
        detached::FAILED_LOG_RETENTION,
    );
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join(format!("hook-flush-{log_id}.log")))?;
    detached::start_until(
        executable.as_os_str(),
        &["__flush".into(), log_id.into()],
        log,
        deadline,
    )
}
