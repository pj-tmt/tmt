//! One UI's public API channel. The neutral leaf owns bytes and child lifecycle;
//! this owner schedules API requests and interprets capability/stale resources.

use crate::runner::{self, Cancellation, RunError};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tmt_invoke::{LineChannel, LineChannelOptions};

const CANCEL_CHECK: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Priority {
    #[default]
    Background,
    Interaction,
}

struct Call {
    payload: Vec<u8>,
    deadline: Instant,
    priority: Priority,
    abandoned: Cancellation,
    parent: Option<Cancellation>,
    reply: Sender<Result<Option<Value>, RunError>>,
}

impl Call {
    fn stopped(&self) -> bool {
        self.abandoned.cancelled() || self.parent.as_ref().is_some_and(Cancellation::cancelled)
    }
}

pub(super) struct Session {
    calls: Option<Sender<Call>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub(super) fn new(executable: PathBuf) -> Result<Self, RunError> {
        let (calls, pending) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("ops-api-session".into())
            .spawn(move || serve(&executable, &pending, &stopped))
            .map_err(|_| RunError::Spawn)?;
        Ok(Self {
            calls: Some(calls),
            stop,
            thread: Some(thread),
        })
    }

    pub(super) fn call(
        &self,
        payload: &[u8],
        deadline: Instant,
        cancellation: Option<&Cancellation>,
        priority: Priority,
    ) -> Result<Option<Value>, RunError> {
        // Enqueuing and waiting are forbidden on the paint/input thread too.
        runner::assert_commands_allowed("API session");
        let abandoned = Cancellation::default();
        struct Abandon(Cancellation);
        impl Drop for Abandon {
            fn drop(&mut self) {
                self.0.cancel();
            }
        }
        let _abandon = Abandon(abandoned.clone());
        let (reply, result) = mpsc::channel();
        self.calls
            .as_ref()
            .expect("live session")
            .send(Call {
                payload: payload.to_vec(),
                deadline,
                priority,
                abandoned,
                parent: cancellation.cloned(),
                reply,
            })
            .map_err(|_| RunError::Io)?;
        loop {
            if cancellation.is_some_and(Cancellation::cancelled) {
                return Err(RunError::Cancelled);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(RunError::Timeout);
            }
            match result.recv_timeout(left.min(CANCEL_CHECK)) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(RunError::Io),
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.calls.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Transport {
    Unknown,
    Legacy,
    Session(Option<LineChannel>),
}

fn serve(executable: &Path, pending: &Receiver<Call>, stopped: &AtomicBool) {
    let mut queued = VecDeque::new();
    let mut transport = Transport::Unknown;
    loop {
        if queued.is_empty() {
            match pending.recv() {
                Ok(call) => queued.push_back(call),
                Err(_) => return,
            }
        }
        queued.extend(pending.try_iter());
        // A background acquisition queues at most its current request, not its
        // entire scan. At every response boundary interactions take priority.
        let next = queued
            .iter()
            .position(|call| call.priority == Priority::Interaction)
            .unwrap_or(0);
        let call = queued.remove(next).expect("one pending call");
        let result = if stopped.load(Ordering::Acquire) || call.stopped() {
            Err(RunError::Cancelled)
        } else if call.deadline <= Instant::now() {
            Err(RunError::Timeout)
        } else {
            request(executable, &mut transport, &call, stopped)
        };
        let _ = call.reply.send(result);
    }
}

fn request(
    executable: &Path,
    transport: &mut Transport,
    call: &Call,
    stopped: &AtomicBool,
) -> Result<Option<Value>, RunError> {
    if matches!(transport, Transport::Unknown) {
        let input = json!({"version":1,"operation":"capabilities","input":{}}).to_string();
        let left = call.deadline.saturating_duration_since(Instant::now());
        let capability = runner::run_cancellable(
            executable,
            &["api".into()],
            input.as_bytes(),
            left,
            super::OUTPUT_LIMIT,
            Some(&call.abandoned),
        )?;
        if !capability.success {
            return Err(RunError::Io);
        }
        let capability = serde_json::from_slice::<Value>(&capability.stdout)
            .ok()
            .filter(Value::is_object)
            .ok_or(RunError::Io)?;
        *transport = if capability["session"]["framing"] == "ndjson" {
            Transport::Session(None)
        } else {
            // The API mode is additive; older invoking CLIs keep their existing
            // one-request behavior. Admission is checked once per UI, not per poll.
            Transport::Legacy
        };
    }
    let Transport::Session(channel) = transport else {
        return Ok(None);
    };
    let stop = || stopped.load(Ordering::Acquire) || call.stopped();
    if channel.is_none() {
        *channel = Some(
            LineChannel::start(
                LineChannelOptions {
                    program: executable,
                    args: &["api".into(), "--session".into()],
                    deadline: call.deadline,
                    max_frame_bytes: super::OUTPUT_LIMIT,
                    launch: Default::default(),
                },
                stop,
            )
            .map_err(|error| runner::run_error(error.kind))?,
        );
    }
    let result =
        channel
            .as_mut()
            .expect("opened channel")
            .exchange(&call.payload, call.deadline, stop);
    match result {
        Ok(bytes) => {
            let document = serde_json::from_slice::<Value>(&bytes).map_err(|_| {
                channel.take();
                RunError::Io
            })?;
            if document["error"]["code"] == "API_SESSION_STALE" {
                // This resource belongs to this request. Reopen for later work;
                // neither a known stale response nor missing bytes replays it.
                channel.take();
            }
            Ok(Some(document))
        }
        Err(error) => {
            channel.take();
            Err(runner::run_error(error.kind))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tests::{Fixture, session_fixture};

    #[test]
    fn queued_interaction_precedes_background_and_abandoned_work_never_starts() {
        let fixture = Fixture::new();
        let core = session_fixture(&fixture);
        let reader = core.clone();
        let first = std::thread::spawn(move || reader.api("gate", json!({})));
        let deadline = Instant::now() + Duration::from_secs(30);
        while !fixture.0.join("admitted").exists() {
            assert!(Instant::now() < deadline, "fixture request admission");
            std::thread::sleep(Duration::from_millis(10));
        }
        let (reply, replies) = mpsc::channel();
        let session = core.session.as_ref().unwrap();
        for (operation, priority, abandoned) in [
            ("background", Priority::Background, false),
            ("interaction", Priority::Interaction, false),
            ("abandoned", Priority::Interaction, true),
        ] {
            let cancellation = Cancellation::default();
            if abandoned {
                cancellation.cancel();
            }
            session
                .calls
                .as_ref()
                .unwrap()
                .send(Call {
                    payload: json!({"version":1,"operation":operation,"input":{}})
                        .to_string()
                        .into_bytes(),
                    deadline,
                    priority,
                    abandoned: cancellation,
                    parent: None,
                    reply: reply.clone(),
                })
                .unwrap();
        }
        std::fs::write(fixture.0.join("release"), b"").unwrap();
        assert_eq!(first.join().unwrap().unwrap()["operation"], "gate");
        for _ in 0..3 {
            let _ = replies.recv_timeout(Duration::from_secs(30)).unwrap();
        }
        drop(core);
        let requests: Vec<Value> = std::fs::read_to_string(fixture.0.join("requests"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            requests
                .iter()
                .map(|request| request["operation"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["gate", "interaction", "background"]
        );
    }
}
