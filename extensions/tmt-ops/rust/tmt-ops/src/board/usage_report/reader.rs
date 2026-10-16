//! The reader thread: one report read in flight, started only while the panel
//! is on screen. It never runs inside the refresh worker, so a slow supplier
//! cannot hold up a reload, and the board never waits for it.

use super::model::{Report, Since};
use crate::{core::Core, runner::Cancellation};
use std::{
    sync::mpsc::{self, RecvTimeoutError},
    time::{Duration, Instant},
};

/// How the reader paces itself.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Between reads while the panel stays on screen.
    pub every: Duration,
    /// One read's limit.
    pub deadline: Duration,
    /// Failing reads back off to this.
    pub slowest: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            every: Duration::from_secs(60),
            deadline: Duration::from_secs(45),
            slowest: Duration::from_secs(300),
        }
    }
}

/// One finished read for one window.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    pub since: Since,
    pub result: Result<Report, String>,
}

enum Signal {
    Want(Option<Since>),
    Stop,
}

/// Tells the reader which window the panel shows, or that it is not on screen.
#[derive(Clone)]
pub struct Wants(mpsc::Sender<Signal>);

impl Wants {
    pub fn set(&self, want: Option<Since>) {
        let _ = self.0.send(Signal::Want(want));
    }
}

pub struct Reader {
    cancellation: Cancellation,
    signals: mpsc::Sender<Signal>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Applies queued signals; `None` once the reader is told to stop.
fn drain(
    wake: &mpsc::Receiver<Signal>,
    first: Option<Signal>,
    want: &mut Option<Since>,
) -> Option<bool> {
    let mut changed = false;
    for signal in first.into_iter().chain(wake.try_iter()) {
        match signal {
            Signal::Stop => return None,
            Signal::Want(next) if next != *want => {
                *want = next;
                changed = true;
            }
            Signal::Want(_) => {}
        }
    }
    Some(changed)
}

fn read(core: &Core, since: Since, deadline: Duration) -> Result<Report, String> {
    let document = core
        .json_within(
            &["digest", "usage", "report", "--since", since.text()],
            deadline,
        )
        .map_err(|error| error.message)?;
    Report::parse(&document)
}

impl Reader {
    /// Delivers each finished read to `deliver`, which returns `false` once
    /// nothing is listening. A read whose window was changed meanwhile is dropped.
    pub fn spawn(
        core: &Core,
        timing: Timing,
        mut deliver: impl FnMut(Delivery) -> bool + Send + 'static,
    ) -> Self {
        let cancellation = Cancellation::default();
        let core = core.cancellable(cancellation.clone());
        let (signals, wake) = mpsc::channel::<Signal>();
        let thread = std::thread::spawn(move || {
            let mut want: Option<Since> = None;
            let mut wait = timing.every;
            loop {
                let Some(since) = want else {
                    // Not on screen: no reads until the panel is wanted again.
                    let Ok(signal) = wake.recv() else { return };
                    if drain(&wake, Some(signal), &mut want).is_none() {
                        return;
                    }
                    wait = timing.every;
                    continue;
                };
                let result = read(&core, since, timing.deadline);
                match drain(&wake, None, &mut want) {
                    None => return,
                    Some(true) => {
                        wait = timing.every;
                        continue;
                    }
                    Some(false) => {}
                }
                wait = if result.is_err() {
                    (wait * 2).min(timing.slowest)
                } else {
                    timing.every
                };
                if !deliver(Delivery { since, result }) {
                    return;
                }
                let end = Instant::now() + wait;
                loop {
                    match wake.recv_timeout(end.saturating_duration_since(Instant::now())) {
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                        Ok(signal) => match drain(&wake, Some(signal), &mut want) {
                            None => return,
                            Some(true) => {
                                wait = timing.every;
                                break;
                            }
                            Some(false) => {}
                        },
                    }
                }
            }
        });
        Self {
            cancellation,
            signals,
            thread: Some(thread),
        }
    }

    pub fn wants(&self) -> Wants {
        Wants(self.signals.clone())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let _ = self.signals.send(Signal::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
