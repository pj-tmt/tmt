//! Event-started, finite hook queue flusher. No timer, watch or idle daemon.
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    process::detached,
    storage::{
        Storage,
        hook_queue::{CaptureEvent, HookQueue},
    },
};

fn is_log_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub fn enqueue(
    database: &Path,
    event: &CaptureEvent,
    deadline: Instant,
) -> Result<(), tmt_adapters::storage::StorageError> {
    let wait = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(500));
    let queue = HookQueue::open(database, wait)?;
    queue.enqueue(event)?;
    // The durable append already succeeded. Startup diagnostics must never turn
    // an accepted final into a capture failure; a later writer can ingest it.
    let _ = queue.start_flusher(deadline);
    Ok(())
}
pub fn execute(log_id: &str) -> io::Result<u8> {
    if !is_log_id(log_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid hook flusher log id",
        ));
    }
    detached::enter()?;
    {
        use std::io::Write;
        writeln!(
            tmt_cli_style::stream::stderr(),
            "flusher_pid={}",
            std::process::id()
        )?;
    }
    // Acknowledge terminal/process-group detachment before any SQLite wait.
    detached::ready()?;
    let paths = ConfigPaths::discover().map_err(io::Error::other)?;
    let mut queue =
        HookQueue::open(&paths.database, Duration::from_secs(30)).map_err(io::Error::other)?;
    let mut guard = match queue.claim() {
        Ok(g) => Some(g),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
            let _ = detached::discard_own_log(
                &paths
                    .database
                    .with_file_name(format!("hook-flush-{log_id}.log")),
            );
            return Ok(0);
        }
        Err(e) => return Err(e),
    };
    let mut storage = Storage::open_hook_flusher(&paths.database).map_err(io::Error::other)?;
    loop {
        let Some(event) = queue.next().map_err(io::Error::other)? else {
            if queue
                .finish_if_empty(&mut guard)
                .map_err(io::Error::other)?
            {
                break;
            }
            continue; // A producer committed real work before the empty handoff.
        };
        let result = storage
            .ingest_hook_capture(&event)
            .map_err(io::Error::other)?;
        // Consent, source and launch-owner checks remain with their existing CLI
        // owners. Their failures stay silent and cannot undo a retained final.
        crate::provider_hook_command::flush_capture_follow_up(
            &paths,
            &event,
            result,
            Instant::now() + Duration::from_secs(30),
        );
        queue.acknowledge(&event).map_err(io::Error::other)?;
    }
    storage.close().map_err(io::Error::other)?;
    let _ = detached::discard_own_log(
        &paths
            .database
            .with_file_name(format!("hook-flush-{log_id}.log")),
    );
    Ok(0)
}
