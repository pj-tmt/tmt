//! One detached timeout observer per accepted request. Replies notify through
//! the reply command; this worker never delivers a reply or resends a request.

use std::{
    fs::{self, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths, process::detached, request_runtime::wall_time_ms, storage::Storage,
};
use tmt_core::request::{RequestService, ResponseLookup};

/// Per-request diagnostics beside the database; one owner for both processes.
fn log_path(database: &Path, request_id: &str) -> io::Result<PathBuf> {
    Ok(database
        .parent()
        .ok_or_else(|| io::Error::other("No state directory"))?
        .join("request-observers")
        .join(format!("{request_id}.log")))
}

pub fn start(database: &Path, request_id: &str) -> io::Result<()> {
    let path = log_path(database, request_id)?;
    fs::create_dir_all(path.parent().expect("log directory"))?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    detached::start(
        std::env::current_exe()?.as_os_str(),
        &["__request-observer".into(), request_id.into()],
        log,
    )
}

pub fn execute(request_id: &str) -> io::Result<u8> {
    let result = (|| -> io::Result<PathBuf> {
        detached::enter()?;
        let paths = ConfigPaths::discover().map_err(io::Error::other)?;
        let mut storage = Storage::open(&paths.database).map_err(io::Error::other)?;
        let policy = RequestService::new(&mut storage, wall_time_ms)
            .notification(request_id)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other("Missing request observer policy"))?
            .policy;
        // Monotonic cap bounds lifetime even if the wall clock moves backward.
        let remaining = policy
            .deadline_ms
            .saturating_sub(wall_time_ms())
            .min(policy.timeout_ms);
        let deadline = Instant::now() + Duration::from_millis(remaining);
        eprintln!("observer_pid={}", std::process::id());
        detached::ready()?;
        loop {
            if RequestService::new(&mut storage, wall_time_ms)
                .notification(request_id)
                .map_err(io::Error::other)?
                .is_none()
            {
                break;
            }
            let response = RequestService::new(&mut storage, wall_time_ms).get_response(request_id);
            if !matches!(response, Ok(ResponseLookup::Unavailable)) {
                break;
            }
            if Instant::now() >= deadline {
                if let Some(hint) = RequestService::new(&mut storage, wall_time_ms)
                    .claim_timeout_hint(request_id)
                    .map_err(io::Error::other)?
                {
                    crate::delivery::notify(&mut storage, &hint);
                }
                break;
            }
            // Storage is not locked while waiting. No daemon, filesystem watch,
            // re-wake, process restart or message/receipt command-line argument.
            std::thread::sleep(
                Duration::from_millis(250).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        storage.close().map_err(io::Error::other)?;
        Ok(paths.database)
    })();
    match result {
        // A clean exit leaves nothing behind; a failure keeps its log as the
        // bounded diagnostic (PID line plus this message), as does a crash.
        Ok(database) => {
            if let Err(error) =
                log_path(&database, request_id).and_then(|path| detached::discard_own_log(&path))
            {
                eprintln!("tmt: could not remove the observer log: {error}");
            }
            Ok(0)
        }
        Err(_) => {
            eprintln!("tmt: request timeout observer unavailable; inspect the retained request.");
            Ok(1)
        }
    }
}
