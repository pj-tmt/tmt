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
    run_batch_with(storage, batch, worker, wall_time_ms(), &mut NativeWorker)
}

trait Worker {
    fn now(&self) -> Instant;
    fn wait(&mut self, duration: Duration);
    fn input(
        &mut self,
        storage: &mut Storage,
        batch: &Batch,
    ) -> io::Result<tmt_core::driver::InputState>;
    fn flush(
        &mut self,
        storage: &mut Storage,
        batch: &Batch,
        worker: &ProcessIncarnation,
    ) -> io::Result<tmt_adapters::reply_notice::FlushOutcome>;
    fn owner_gone(&mut self, owner: &ProcessIncarnation) -> io::Result<bool>;
}

struct NativeWorker;
impl Worker for NativeWorker {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn wait(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
    fn input(
        &mut self,
        storage: &mut Storage,
        batch: &Batch,
    ) -> io::Result<tmt_core::driver::InputState> {
        match tmt_adapters::reply_notice::matching_entry(storage, batch)
            .map_err(io::Error::other)?
        {
            Some(entry) => tmt_adapters::reply_notice::input_state(&entry, batch.quiet_ms)
                .map_err(io::Error::other),
            None => Ok(tmt_core::driver::InputState::Unknown),
        }
    }
    fn flush(
        &mut self,
        storage: &mut Storage,
        batch: &Batch,
        worker: &ProcessIncarnation,
    ) -> io::Result<tmt_adapters::reply_notice::FlushOutcome> {
        tmt_adapters::reply_notice::flush(storage, batch, worker).map_err(io::Error::other)
    }
    fn owner_gone(&mut self, owner: &ProcessIncarnation) -> io::Result<bool> {
        let observation = observe_runtime_process(
            &SupervisedProbeRunner,
            owner.pid(),
            Instant::now() + Duration::from_secs(1),
        )
        .map_err(io::Error::other)?;
        Ok(observation.matches(owner) == tmt_core::binding::session::RuntimeLiveness::Gone)
    }
}

fn run_batch_with(
    storage: &mut Storage,
    batch: &Batch,
    worker: &ProcessIncarnation,
    wall: u64,
    runtime: &mut impl Worker,
) -> io::Result<()> {
    let monotonic = runtime.now();
    let remaining = batch.due_ms.saturating_sub(wall).min(batch.window_ms);
    let window = monotonic + Duration::from_millis(remaining);
    let bounded = batch
        .due_ms
        .saturating_add(MAX_TYPING_WAIT_MS)
        .saturating_sub(wall)
        .min(remaining.saturating_add(MAX_TYPING_WAIT_MS));
    let limit = monotonic + Duration::from_millis(bounded);
    let transport_limit = limit + tmt_adapters::reply_notice::maximum_send_duration();
    loop {
        let now = runtime.now();
        if now >= window {
            let input = if batch.quiet_ms == 0 || now >= limit {
                tmt_core::driver::InputState::Unknown
            } else {
                runtime.input(storage, batch)?
            };
            if tmt_core::request::notification::batch::notice_is_ready(
                input,
                true,
                runtime.now() >= limit,
            ) {
                match runtime.flush(storage, batch, worker)? {
                    tmt_adapters::reply_notice::FlushOutcome::Finished => return Ok(()),
                    tmt_adapters::reply_notice::FlushOutcome::Waiting(active) => {
                        let owner = active.worker.as_ref().ok_or_else(|| {
                            io::Error::other("Pane reply sender has no process owner")
                        })?;
                        if runtime.owner_gone(owner)? {
                            storage
                                .release_reply_notice_send(&active)
                                .map_err(io::Error::other)?;
                            continue;
                        }
                        if runtime.now() >= transport_limit {
                            return Err(io::Error::other(
                                "Pane reply sender still busy; untouched notices remain queued",
                            ));
                        }
                    }
                }
            }
        }
        runtime.wait(
            Duration::from_millis(250)
                .min(transport_limit.saturating_duration_since(runtime.now())),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::{
        driver::InputState,
        endpoint::ServerEvidence,
        host::HostKind,
        identity::{Lifetime, create_or_resolve},
        request::{
            Originator, PrepareRequest, RequestEndpoint, RequestKind, RequestRoute, RequestService,
            ResponseProof, Settlement, SubmitResponse,
            notification::{NotificationPolicy, batch::SendClaim},
        },
    };

    const WALL: u64 = 1_700_000_000_000;

    struct Fixture {
        directory: PathBuf,
        storage: Storage,
        sender_storage: Storage,
        active: Batch,
        waiting: Batch,
        sender: ProcessIncarnation,
        waiter: ProcessIncarnation,
    }
    impl Fixture {
        fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("tmt-reply-grace-{}", new_operation_id()));
            let database = directory.join("state.db");
            let mut storage = Storage::open(&database).unwrap();
            let identity = create_or_resolve(&mut storage, "Notice owner", Lifetime::Saved)
                .unwrap()
                .identity;
            let endpoint = RequestEndpoint {
                server: ServerEvidence {
                    host: HostKind::Tmux,
                    server_id: "fixture-server".into(),
                    socket_path: "/tmp/fixture.sock".into(),
                    server_pid: 41,
                    server_start_time: "fixture-start".into(),
                },
                pane_id: "%1".into(),
                pane_pid: 42,
            };
            let batches = ["owner-notice", "waiting-notice"].map(|id| {
                let attempt = format!("{id}-attempt");
                let mut service = RequestService::new(&mut storage, || WALL);
                service
                    .prepare(
                        PrepareRequest {
                            room_id: None,
                            kind: RequestKind::Request,
                            request_id: id.into(),
                            message: format!("request {id}"),
                            route: RequestRoute::Pane(endpoint.clone()),
                            wait: false,
                            expires_at_ms: WALL + 60_000,
                            originator: Originator::Explicit(identity.id.clone()),
                            recipient_identity_id: Some(identity.id.clone()),
                            preamble: None,
                        },
                        attempt.clone(),
                        1,
                    )
                    .unwrap();
                service
                    .enable_notifications(
                        id,
                        NotificationPolicy {
                            deadline_ms: WALL + 1000,
                            timeout_ms: 1000,
                            waiter: None,
                        },
                    )
                    .unwrap();
                service.begin_send(&attempt).unwrap();
                service.settle(&attempt, Settlement::Sent).unwrap();
                let hint = service
                    .submit_response_with_hint(
                        SubmitResponse {
                            request_id: id.into(),
                            proof: ResponseProof::Recorded {
                                attempt_id: attempt,
                                endpoint: endpoint.clone(),
                            },
                            body: format!("final {id}"),
                        },
                        None,
                    )
                    .unwrap()
                    .1
                    .unwrap();
                // Start the competing observation at its exhausted typing limit.
                storage
                    .queue_reply_notice(&hint, "binding", id, 0, 0, WALL - MAX_TYPING_WAIT_MS)
                    .unwrap()
            });
            let [active, waiting] = batches;
            let sender = ProcessIncarnation::new(101, "sender-start").unwrap();
            let waiter = ProcessIncarnation::new(102, "waiter-start").unwrap();
            assert!(storage.claim_reply_notice_worker(&active, &sender).unwrap());
            assert!(
                storage
                    .claim_reply_notice_worker(&waiting, &waiter)
                    .unwrap()
            );
            let mut sender_storage = Storage::open(&database).unwrap();
            let SendClaim::Ready(notices) = sender_storage
                .claim_reply_notice_send(&active.id, &sender)
                .unwrap()
            else {
                panic!("sender must own its sole frame");
            };
            assert_eq!(notices.len(), 1);
            assert!(
                sender_storage
                    .mark_reply_notice_attempted(&active.id, &notices[0].request_id, &sender)
                    .unwrap()
            );
            Self {
                directory,
                storage,
                sender_storage,
                active,
                waiting,
                sender,
                waiter,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.storage.close().unwrap();
            self.sender_storage.close().unwrap();
            fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    struct GatedSender<'a> {
        now: Instant,
        release: Option<Instant>,
        sender_storage: &'a mut Storage,
        active: &'a Batch,
        sender: &'a ProcessIncarnation,
        dispatches: Vec<String>,
        waiting_observations: usize,
    }
    impl Worker for GatedSender<'_> {
        fn now(&self) -> Instant {
            self.now
        }
        fn wait(&mut self, duration: Duration) {
            assert!(!duration.is_zero(), "waiter must make bounded progress");
            self.now += duration;
        }
        fn input(&mut self, _: &mut Storage, _: &Batch) -> io::Result<InputState> {
            panic!("typing limit already elapsed");
        }
        fn flush(
            &mut self,
            storage: &mut Storage,
            batch: &Batch,
            worker: &ProcessIncarnation,
        ) -> io::Result<tmt_adapters::reply_notice::FlushOutcome> {
            if self.release.is_some_and(|release| self.now >= release) {
                self.sender_storage
                    .settle_reply_notice_member(&self.active.id, "owner-notice", WakeState::Sent)
                    .unwrap();
                self.sender_storage
                    .finish_reply_notice_batch(&self.active.id)
                    .unwrap();
                self.release = None;
            }
            Ok(
                match storage.claim_reply_notice_send(&batch.id, worker).unwrap() {
                    SendClaim::Waiting(active) => {
                        self.waiting_observations += 1;
                        assert_eq!(active.worker.as_ref(), Some(self.sender));
                        let waiting = self
                            .sender_storage
                            .reply_notice_batch(&batch.id)
                            .unwrap()
                            .unwrap();
                        assert!(
                            waiting.pending,
                            "untouched frame remains durable while owner sends"
                        );
                        assert!(
                            !waiting.sending,
                            "waiter cannot steal the live sending claim"
                        );
                        tmt_adapters::reply_notice::FlushOutcome::Waiting(active)
                    }
                    SendClaim::Ready(notices) => {
                        for notice in notices {
                            assert!(
                                storage
                                    .mark_reply_notice_attempted(
                                        &batch.id,
                                        &notice.request_id,
                                        worker
                                    )
                                    .unwrap()
                            );
                            self.dispatches.push(notice.request_id.clone());
                            storage
                                .settle_reply_notice_member(
                                    &batch.id,
                                    &notice.request_id,
                                    WakeState::Sent,
                                )
                                .unwrap();
                        }
                        storage.finish_reply_notice_batch(&batch.id).unwrap();
                        tmt_adapters::reply_notice::FlushOutcome::Finished
                    }
                    SendClaim::Lost => panic!("waiter's queued notice must not be lost"),
                },
            )
        }
        fn owner_gone(&mut self, owner: &ProcessIncarnation) -> io::Result<bool> {
            assert_eq!(owner, self.sender);
            Ok(false)
        }
    }

    #[test]
    fn competing_waiter_outlives_longest_declared_single_send() {
        let mut fixture = Fixture::new();
        let start = Instant::now();
        let budget = tmt_adapters::reply_notice::maximum_send_duration();
        assert!(
            budget > Duration::from_secs(3),
            "positive control must cross the old grace"
        );
        let mut runtime = GatedSender {
            now: start,
            release: Some(start + budget),
            sender_storage: &mut fixture.sender_storage,
            active: &fixture.active,
            sender: &fixture.sender,
            dispatches: vec!["owner-notice".into()],
            waiting_observations: 0,
        };
        run_batch_with(
            &mut fixture.storage,
            &fixture.waiting,
            &fixture.waiter,
            WALL,
            &mut runtime,
        )
        .unwrap();
        assert_eq!(runtime.now, start + budget);
        assert!(runtime.waiting_observations > 1);
        assert_eq!(runtime.dispatches, ["owner-notice", "waiting-notice"]);
        assert!(
            fixture
                .storage
                .reply_notice_batch(&fixture.active.id)
                .unwrap()
                .is_none()
        );
        assert!(
            fixture
                .storage
                .reply_notice_batch(&fixture.waiting.id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            fixture
                .storage
                .claim_reply_notice_send(&fixture.waiting.id, &fixture.waiter)
                .unwrap(),
            SendClaim::Lost
        );
    }

    #[test]
    fn expired_grace_preserves_live_sender_and_untouched_notice() {
        let mut fixture = Fixture::new();
        let start = Instant::now();
        let budget = tmt_adapters::reply_notice::maximum_send_duration();
        let mut runtime = GatedSender {
            now: start,
            release: None,
            sender_storage: &mut fixture.sender_storage,
            active: &fixture.active,
            sender: &fixture.sender,
            dispatches: vec!["owner-notice".into()],
            waiting_observations: 0,
        };
        let error = run_batch_with(
            &mut fixture.storage,
            &fixture.waiting,
            &fixture.waiter,
            WALL,
            &mut runtime,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("untouched notices remain queued")
        );
        assert_eq!(runtime.now, start + budget);
        assert_eq!(runtime.dispatches, ["owner-notice"]);
        let active = fixture
            .storage
            .reply_notice_batch(&fixture.active.id)
            .unwrap()
            .unwrap();
        assert!(active.sending);
        assert_eq!(active.worker, Some(fixture.sender.clone()));
        let waiting = fixture
            .storage
            .reply_notice_batch(&fixture.waiting.id)
            .unwrap()
            .unwrap();
        assert!(waiting.pending);
        assert!(!waiting.sending);
    }
}
