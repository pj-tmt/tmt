//! After `tmt run` or `tmt resume` starts an agent, gives it the identity's
//! name as the host's agent name without a user step (#3238).
//!
//! One thread per launch, started only after this exact child was admitted.
//! It tries a few times while the host has no agent to name yet, then stops
//! without output. It stores nothing: each attempt reads the binding again
//! and stops unless the same launch still holds it with both processes
//! alive. It never runs in the wait tick, so signal forwarding is unchanged.
//! A rename that races the thread is best effort (driver-protocol-v1,
//! `name-agent`).

use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tmt_adapters::{
    host::{AgentNameRefusal, AgentNaming, Host, HostError, OperationOptions},
    process::{UnixCommandRunner, runtime::observe_runtime_process},
    storage::Storage,
};
use tmt_core::{
    binding::{
        Binding, BindingEntry, BindingEvidence, evaluate_binding,
        session::{RuntimeLiveness, RuntimeState},
    },
    endpoint::{EndpointProbe, ProcessIncarnation},
    identity::Identity,
};

/// When each attempt is due, counted from the thread's start. Herdr takes a
/// few seconds to recognize a new agent; these are provisional (#3238).
pub(super) const ATTEMPTS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(7),
    Duration::from_secs(15),
    Duration::from_secs(31),
];

/// The deadline of each storage, process or host step of an attempt.
const STEP: Duration = Duration::from_secs(2);

/// How long `tmt run` waits for the thread after its child ended: one step
/// in flight plus the driver's process cleanup.
pub(super) const FINISH_WITHIN: Duration = Duration::from_secs(3);

/// The exact launch the thread names: the binding it holds and its two
/// processes, `tmt run` itself (`owner`) and the admitted child.
pub(super) struct Launch {
    pub database: PathBuf,
    pub identity_id: String,
    pub binding_id: String,
    pub owner: ProcessIncarnation,
    pub child: ProcessIncarnation,
}

impl Launch {
    /// Whether `entry` is still this launch's running binding.
    fn holds(&self, entry: &BindingEntry) -> bool {
        entry.binding.as_ref().is_some_and(|binding| {
            binding.id == self.binding_id
                && binding.session.launch_owner.as_ref() == Some(&self.owner)
                && binding.session.key.as_ref().map(|key| &key.incarnation) == Some(&self.child)
                && binding.session.state == RuntimeState::Running
        })
    }
}

/// How the thread ended, as far as `tmt run` needs to know: every other
/// outcome is silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Outcome {
    pub cleanup_failed: bool,
}

const ENDED: Outcome = Outcome {
    cleanup_failed: false,
};

/// A failed host or process call as the attempts classify it. A
/// `HostError` itself carries a cleanup failure that only the adapters can
/// build, so the effects classify it here and tests inject this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CallError {
    pub refusal: AgentNameRefusal,
    pub cleanup_failed: bool,
}

impl From<HostError> for CallError {
    fn from(error: HostError) -> Self {
        Self {
            refusal: AgentNameRefusal::of(&error),
            cleanup_failed: error.cleanup_failed(),
        }
    }
}

impl From<CallError> for Outcome {
    fn from(error: CallError) -> Self {
        Self {
            cleanup_failed: error.cleanup_failed,
        }
    }
}

/// What the agent name of the bound pane is, as one probe saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Probe {
    /// The pane is this binding's, and the host reports this agent name, or
    /// none.
    Active(Option<String>),
    /// The pane is not this binding's, or its evidence is unknown.
    NotActive,
}

/// The effects one attempt needs. Each call is bounded by its own deadline.
pub(super) trait Ports {
    /// The identity's stored binding entry, or `None` when it can't be read.
    fn entry(&mut self) -> Option<BindingEntry>;
    /// Whether `process` is alive as that exact incarnation; unknown is not.
    fn alive(&mut self, process: &ProcessIncarnation) -> Result<bool, CallError>;
    fn probe(&mut self, entry: &BindingEntry) -> Result<Probe, CallError>;
    fn name_agent(
        &mut self,
        binding: &Binding,
        identity: &Identity,
    ) -> Result<AgentNaming, CallError>;
}

/// The pause between attempts and the stop request.
pub(super) trait Pacing {
    /// Waits until `at`: `false` when a stop was requested first.
    fn until(&self, at: Instant) -> bool;
    fn stopped(&self) -> bool;
}

/// The attempts. Once it observes a stop it starts no new step; a set
/// already started runs to its own deadline. A cleanup failure is returned
/// even when a stop came meanwhile, so `tmt run` can report it.
pub(super) fn name_launched_agent(
    launch: &Launch,
    ports: &mut impl Ports,
    pacing: &impl Pacing,
    started: Instant,
) -> Outcome {
    match attempts(launch, ports, pacing, started) {
        Ok(()) => ENDED,
        Err(outcome) => outcome,
    }
}

/// `Err` ends the attempts with that outcome.
fn attempts(
    launch: &Launch,
    ports: &mut impl Ports,
    pacing: &impl Pacing,
    started: Instant,
) -> Result<(), Outcome> {
    for due in ATTEMPTS {
        if !pacing.until(started + due) {
            return Err(ENDED);
        }
        let entry = held(launch, ports, pacing)?;
        match ports.probe(&entry)? {
            // An agent with any name keeps it: only an explicit bind replaces one.
            Probe::Active(None) => {}
            Probe::Active(Some(_)) | Probe::NotActive => return Err(ENDED),
        }
        // The probe may have taken its whole deadline: check the exact launch
        // again right before the set, and set the name stored now.
        stopped(pacing)?;
        let entry = held(launch, ports, pacing)?;
        let binding = entry.binding.as_ref().expect("a held entry has a binding");
        match ports.name_agent(binding, &entry.identity) {
            Err(error)
                if !error.cleanup_failed
                    && matches!(
                        error.refusal,
                        AgentNameRefusal::NoAgent | AgentNameRefusal::NotReady
                    ) => {}
            Err(error) => return Err(error.into()),
            Ok(_) => return Err(ENDED),
        }
    }
    Ok(())
}

fn stopped(pacing: &impl Pacing) -> Result<(), Outcome> {
    if pacing.stopped() { Err(ENDED) } else { Ok(()) }
}

/// The entry while this exact launch still holds its binding and both of
/// its processes are alive, checking the stop after each read.
fn held(
    launch: &Launch,
    ports: &mut impl Ports,
    pacing: &impl Pacing,
) -> Result<BindingEntry, Outcome> {
    let entry = ports.entry();
    stopped(pacing)?;
    let entry = entry.filter(|entry| launch.holds(entry)).ok_or(ENDED)?;
    for process in [&launch.owner, &launch.child] {
        let alive = ports.alive(process)?;
        stopped(pacing)?;
        if !alive {
            return Err(ENDED);
        }
    }
    Ok(entry)
}

/// The real effects: a fresh storage read and a fresh host for each call, so
/// a removed or changed driver is seen at the next attempt.
pub(super) struct Effects {
    database: PathBuf,
    identity_id: String,
}

impl Ports for Effects {
    fn entry(&mut self) -> Option<BindingEntry> {
        let now = tmt_adapters::request_runtime::wall_time_ms();
        Storage::context_by_identity(&self.database, &self.identity_id, now)
            .ok()
            .flatten()
            .map(|context| context.entry)
    }

    fn alive(&mut self, process: &ProcessIncarnation) -> Result<bool, CallError> {
        match observe_runtime_process(&UnixCommandRunner, process.pid(), Instant::now() + STEP) {
            Ok(observation) => Ok(observation.matches(process) == RuntimeLiveness::Alive),
            Err(error) => Err(CallError {
                refusal: AgentNameRefusal::Uncertain,
                cleanup_failed: error.cleanup_failed(),
            }),
        }
    }

    fn probe(&mut self, entry: &BindingEntry) -> Result<Probe, CallError> {
        let binding = entry.binding.as_ref().expect("a held entry has a binding");
        // `UnixCommandRunner` gives each driver process its own group. Never
        // `SupervisedProbeRunner` here: it kills its caller's own group, and
        // `tmt run` may lead the group that holds the agent.
        let snapshot = Host::for_server(&binding.server).observe_snapshot(OperationOptions {
            deadline: Some(Instant::now() + STEP),
            pane_ids: Some(std::slice::from_ref(&binding.pane_id)),
        })?;
        Ok(
            match evaluate_binding(entry, &EndpointProbe::Live(snapshot)) {
                BindingEvidence::Active(pane) => Probe::Active(pane.agent_name),
                _ => Probe::NotActive,
            },
        )
    }

    fn name_agent(
        &mut self,
        binding: &Binding,
        identity: &Identity,
    ) -> Result<AgentNaming, CallError> {
        Ok(Host::for_server(&binding.server).name_agent(
            binding,
            identity,
            Some(Instant::now() + STEP),
        )?)
    }
}

/// The stop request. Its lock is held only while waiting between attempts,
/// never during storage, process or host work.
#[derive(Default)]
pub(super) struct StopSignal {
    stopped: Mutex<bool>,
    wake: Condvar,
}

impl StopSignal {
    fn request(&self) {
        *self
            .stopped
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = true;
        self.wake.notify_all();
    }
}

impl Pacing for StopSignal {
    fn until(&self, at: Instant) -> bool {
        let mut stopped = self
            .stopped
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        loop {
            if *stopped {
                return false;
            }
            let Some(left) = at
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
            else {
                return true;
            };
            stopped = self
                .wake
                .wait_timeout(stopped, left)
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
    }

    fn stopped(&self) -> bool {
        *self
            .stopped
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// The running thread. Dropping it on any path out of `tmt run` asks the
/// thread to stop; `finish` also waits for it, within a bound.
pub(super) struct Naming {
    stop: Arc<StopSignal>,
    done: Receiver<Outcome>,
    handle: Option<JoinHandle<()>>,
}

type Work = Box<dyn FnOnce() + Send>;

impl Naming {
    /// Starts naming `launch` when its host offers agent names; `None` when
    /// it does not, or when no thread could be started (silent: best effort).
    pub(super) fn start(launch: Launch, binding: &Binding) -> Option<Self> {
        if !Host::for_server(&binding.server).offers_agent_names(binding) {
            return None;
        }
        let effects = Effects {
            database: launch.database.clone(),
            identity_id: launch.identity_id.clone(),
        };
        Self::start_with(launch, effects, |work| {
            std::thread::Builder::new()
                .name("tmt-agent-name".into())
                .spawn(work)
        })
    }

    pub(super) fn start_with<P: Ports + Send + 'static>(
        launch: Launch,
        mut ports: P,
        spawn: impl FnOnce(Work) -> io::Result<JoinHandle<()>>,
    ) -> Option<Self> {
        let stop = Arc::new(StopSignal::default());
        let (sender, done) = mpsc::channel();
        let pacing = stop.clone();
        let started = Instant::now();
        let handle = spawn(Box::new(move || {
            let outcome = name_launched_agent(&launch, &mut ports, &*pacing, started);
            // The last act; `finish` still waits for the thread to end.
            let _ = sender.send(outcome);
        }))
        .ok()?;
        Some(Self {
            stop,
            done,
            handle: Some(handle),
        })
    }

    /// Asks the thread to stop; it starts no new step after it sees this.
    pub(super) fn request_stop(&self) {
        self.stop.request();
    }

    /// Asks the thread to stop and waits at most `within` for it to end. The
    /// outcome comes only from a thread that ended; when the bound runs out
    /// the thread is left to finish on its own (detached) and its outcome,
    /// cleanup failure included, is lost. A panic gives `None`.
    pub(super) fn finish(mut self, within: Duration) -> Option<Outcome> {
        let deadline = Instant::now() + within;
        self.request_stop();
        let outcome = match self.done.recv_timeout(within) {
            Ok(outcome) => Some(outcome),
            Err(RecvTimeoutError::Timeout) => return None,
            Err(RecvTimeoutError::Disconnected) => None,
        };
        let handle = self.handle.take()?;
        // A sent outcome is not proof that the thread ended.
        while !handle.is_finished() {
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        handle.join().ok().and(outcome)
    }
}

impl Drop for Naming {
    fn drop(&mut self) {
        self.request_stop();
    }
}

#[cfg(test)]
mod tests;
