//! The reader thread: one observation read in flight while the saved TOK range
//! is wanted by Agents or Digest. It never runs inside the refresh worker, so a slow supplier
//! cannot hold up a reload, and the board never waits for it.

use super::{
    Request,
    model::{Report, Since},
};
use crate::{core::Core, runner::Cancellation};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, RecvTimeoutError},
    },
    time::Duration,
};

/// How the reader paces itself.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Between reads while observations remain wanted.
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
    Changed,
    Stop,
}

#[derive(Default)]
struct Demand {
    request: Option<Request>,
    cancellation: Cancellation,
    stopped: bool,
}

/// Owns both the current demand and cancellation of its in-flight read.
#[derive(Clone)]
pub struct Wants {
    signals: mpsc::Sender<Signal>,
    demand: Arc<Mutex<Demand>>,
}

impl Wants {
    pub fn set(&self, want: Option<Request>) {
        let mut demand = self.demand.lock().expect("reader demand");
        if demand.stopped || demand.request == want {
            return;
        }
        demand.cancellation.cancel();
        demand.cancellation = Cancellation::default();
        demand.request = want;
        let _ = self.signals.send(Signal::Changed);
    }
}

pub struct Reader {
    wants: Wants,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn read(core: &Core, request: &Request, deadline: Duration) -> Result<Report, String> {
    super::acquire::read(core, request, deadline)
}

impl Reader {
    /// Delivers each finished read to `deliver`, which returns `false` once
    /// nothing is listening. A read whose window was changed meanwhile is dropped.
    pub fn spawn(
        core: &Core,
        timing: Timing,
        mut deliver: impl FnMut(Delivery) -> bool + Send + 'static,
    ) -> Self {
        let core = core.clone();
        let (signals, wake) = mpsc::channel::<Signal>();
        let demand = Arc::new(Mutex::new(Demand::default()));
        let wants = Wants {
            signals,
            demand: Arc::clone(&demand),
        };
        let thread = std::thread::spawn(move || {
            let mut wait = timing.every;
            loop {
                let (request, cancellation) = {
                    let demand = demand.lock().expect("reader demand");
                    if demand.stopped {
                        return;
                    }
                    (demand.request.clone(), demand.cancellation.clone())
                };
                let Some(request) = request else {
                    // Not on screen: no reads until the panel is wanted again.
                    match wake.recv() {
                        Ok(Signal::Changed) => {}
                        Ok(Signal::Stop) | Err(_) => return,
                    }
                    wait = timing.every;
                    continue;
                };
                // Signals already represented by this demand need no further wake.
                for signal in wake.try_iter() {
                    if matches!(signal, Signal::Stop) {
                        return;
                    }
                }
                let result = read(
                    &core.cancellable(cancellation.clone()),
                    &request,
                    timing.deadline,
                );
                let current = demand.lock().expect("reader demand");
                if cancellation.cancelled() {
                    wait = timing.every;
                    continue;
                }
                drop(current);
                wait = if result.is_err() {
                    (wait * 2).min(timing.slowest)
                } else {
                    timing.every
                };
                if !deliver(Delivery {
                    since: request.since(),
                    result,
                }) {
                    return;
                }
                match wake.recv_timeout(wait) {
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                    Ok(Signal::Stop) => return,
                    Ok(Signal::Changed) => {
                        wait = timing.every;
                    }
                }
            }
        });
        Self {
            wants,
            thread: Some(thread),
        }
    }

    pub fn wants(&self) -> Wants {
        self.wants.clone()
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        {
            let mut demand = self.wants.demand.lock().expect("reader demand");
            demand.stopped = true;
            demand.cancellation.cancel();
        }
        let _ = self.wants.signals.send(Signal::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
