//! Finite reply notice workers; durable bodies and notification claims stay in core.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    process::{
        SupervisedProbeRunner, detached,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    request_runtime::wall_time_ms,
    storage::Storage,
};
use tmt_core::{
    endpoint::ProcessIncarnation,
    operation::new_operation_id,
    request::{
        WakeState,
        notification::{
            NotificationOutcome, OriginatorHint,
            batch::{Batch, MAX_TYPING_WAIT_MS},
        },
    },
};

fn log_path(database: &Path, id: &str) -> io::Result<PathBuf> {
    // Both IDs are generated operation UUIDs, never caller-chosen paths.
    if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(io::Error::other("Invalid reply worker log ID"));
    }
    Ok(database
        .parent()
        .ok_or_else(|| io::Error::other("No data directory"))?
        .join("reply-notice-workers")
        .join(format!("{id}.log")))
}
fn start(database: &Path, batch: &Batch) -> io::Result<()> {
    if !batch.pending {
        return Ok(());
    }
    if let Some(owner) = &batch.worker {
        let observation = observe_runtime_process(
            &SupervisedProbeRunner,
            owner.pid(),
            Instant::now() + Duration::from_secs(1),
        )
        .map_err(io::Error::other)?;
        if observation.matches(owner) != tmt_core::binding::session::RuntimeLiveness::Gone {
            return Ok(());
        }
    }
    let log_id = new_operation_id();
    let path = log_path(database, &log_id)?;
    fs::create_dir_all(path.parent().expect("log directory"))?;
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    detached::start(
        std::env::current_exe()?.as_os_str(),
        &[
            "__reply-notice-worker".into(),
            batch.id.clone().into(),
            log_id.into(),
        ],
        log,
    )
}

/// A queued hint is never reported sent. Failure to schedule cannot invalidate
/// an accepted reply, and cannot authorize a second transport attempt.
pub fn notify(storage: &mut Storage, hint: &OriginatorHint) -> NotificationOutcome {
    match tmt_adapters::reply_notice::prepare(storage, hint) {
        Ok(tmt_adapters::reply_notice::Prepared::Immediate(state)) => {
            NotificationOutcome::Delivered(state)
        }
        Ok(tmt_adapters::reply_notice::Prepared::Queued(batch)) => {
            let scheduling = ConfigPaths::discover()
                .map_err(io::Error::other)
                .and_then(|paths| {
                    start(&paths.database, &batch)?;
                    // A later reply supplies the bounded opportunity to resume an
                    // exact dead worker's never-attempted channel frames.
                    for pending in storage
                        .pending_reply_notice_batches(&batch.binding_id)
                        .map_err(io::Error::other)?
                    {
                        if pending.id != batch.id {
                            start(&paths.database, &pending)?;
                        }
                    }
                    Ok(())
                });
            if let Err(error) = scheduling {
                let mut output = tmt_cli_style::stream::stderr();
                let terminal = output.terminal();
                let _ = tmt_cli_style::message::warning(
                    &mut output,
                    terminal,
                    &format!(
                        "Reply notice queued but worker unavailable: {error}; inspect tmt result {}.",
                        hint.request_id
                    ),
                    None,
                );
            }
            NotificationOutcome::Queued
        }
        Err(_) => NotificationOutcome::Delivered(WakeState::Unavailable),
    }
}

pub fn execute(id: &str, log_id: &str) -> io::Result<u8> {
    let result = (|| -> io::Result<PathBuf> {
        detached::enter()?;
        let mut log = tmt_cli_style::stream::stderr();
        writeln!(
            log,
            "reply_worker_pid={}\nreply_batch_id={id}",
            std::process::id()
        )?;
        let paths = ConfigPaths::discover().map_err(io::Error::other)?;
        let mut storage = Storage::open(&paths.database).map_err(io::Error::other)?;
        let batch = storage.reply_notice_batch(id).map_err(io::Error::other)?;
        let Some(batch) = batch else {
            detached::ready()?;
            storage.close().map_err(io::Error::other)?;
            return Ok(paths.database);
        };
        let owner = match observe_runtime_process(
            &SupervisedProbeRunner,
            u64::from(std::process::id()),
            Instant::now() + Duration::from_secs(1),
        )
        .map_err(io::Error::other)?
        {
            ProcessObservation::Live(owner) => owner,
            _ => return Err(io::Error::other("Could not identify reply worker")),
        };
        let eligible = match &batch.worker {
            None => true,
            Some(previous) => {
                observe_runtime_process(
                    &SupervisedProbeRunner,
                    previous.pid(),
                    Instant::now() + Duration::from_secs(1),
                )
                .map_err(io::Error::other)?
                .matches(previous)
                    == tmt_core::binding::session::RuntimeLiveness::Gone
            }
        };
        let claimed = eligible
            && batch.pending
            && storage
                .claim_reply_notice_worker(&batch, &owner)
                .map_err(io::Error::other)?;
        // Losers acknowledge and exit cleanly; only the CAS winner owns work.
        detached::ready()?;
        if claimed {
            run_batch(&mut storage, &batch, &owner)?;
        }
        storage.close().map_err(io::Error::other)?;
        Ok(paths.database)
    })();
    match result {
        Ok(database) => {
            log_path(&database, log_id).and_then(|p| detached::discard_own_log(&p))?;
            Ok(0)
        }
        Err(error) => {
            let mut output = tmt_cli_style::stream::stderr();
            let terminal = output.terminal();
            let _ = tmt_cli_style::message::warning(
                &mut output,
                terminal,
                &format!("Reply notice worker unavailable: {error}; results remain retrievable."),
                None,
            );
            Ok(1)
        }
    }
}
fn run_batch(storage: &mut Storage, batch: &Batch, worker: &ProcessIncarnation) -> io::Result<()> {
    let wall = wall_time_ms();
    let monotonic = Instant::now();
    let remaining = batch.due_ms.saturating_sub(wall).min(batch.window_ms);
    let window = monotonic + Duration::from_millis(remaining);
    let bounded = batch
        .due_ms
        .saturating_add(MAX_TYPING_WAIT_MS)
        .saturating_sub(wall)
        .min(remaining.saturating_add(MAX_TYPING_WAIT_MS));
    let limit = monotonic + Duration::from_millis(bounded);
    let transport_limit = limit + Duration::from_secs(3);
    loop {
        let now = Instant::now();
        if now >= window {
            let input = if batch.quiet_ms == 0 || now >= limit {
                tmt_core::driver::InputState::Unknown
            } else {
                match tmt_adapters::reply_notice::matching_entry(storage, batch)
                    .map_err(io::Error::other)?
                {
                    Some(entry) => tmt_adapters::reply_notice::input_state(&entry, batch.quiet_ms)
                        .map_err(io::Error::other)?,
                    None => tmt_core::driver::InputState::Unknown,
                }
            };
            if tmt_core::request::notification::batch::notice_is_ready(
                input,
                true,
                Instant::now() >= limit,
            ) {
                match tmt_adapters::reply_notice::flush(storage, batch, worker)
                    .map_err(io::Error::other)?
                {
                    tmt_adapters::reply_notice::FlushOutcome::Finished => return Ok(()),
                    tmt_adapters::reply_notice::FlushOutcome::Waiting(active) => {
                        let owner = active.worker.as_ref().ok_or_else(|| {
                            io::Error::other("Pane reply sender has no process owner")
                        })?;
                        let observation = observe_runtime_process(
                            &SupervisedProbeRunner,
                            owner.pid(),
                            Instant::now() + Duration::from_secs(1),
                        )
                        .map_err(io::Error::other)?;
                        if observation.matches(owner)
                            == tmt_core::binding::session::RuntimeLiveness::Gone
                        {
                            storage
                                .release_reply_notice_send(&active)
                                .map_err(io::Error::other)?;
                            continue;
                        }
                        if Instant::now() >= transport_limit {
                            return Err(io::Error::other(
                                "Pane reply sender still busy; untouched notices remain queued",
                            ));
                        }
                    }
                }
            }
        }
        std::thread::sleep(
            Duration::from_millis(250)
                .min(transport_limit.saturating_duration_since(Instant::now())),
        );
    }
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "Reply notice queued but worker unavailable: {error}; inspect tmt result {}.",
        &["."],
        &[],
    )];
