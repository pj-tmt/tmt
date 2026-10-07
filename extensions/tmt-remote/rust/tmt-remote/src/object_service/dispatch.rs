//! One running channel: a dispatcher that alone reads the bus, and a small fixed pool of
//! workers that answer requests. The dispatcher keeps reading while workers wait for a
//! callback decision, so a decision is never stuck behind the request that needs it.
//! Nothing here holds a lock across a bus send, a wait for the extension or a join.
//!
//! The channel is bounded and final: when the bus ends, an extension leaves a callback
//! unanswered past its bound, or the service stops, every thread ends and the last one
//! drops the bus, which shuts the socket down and joins its reader. No frame spawns a
//! thread, and nothing is retried.
use super::{ServiceBounds, config::ConfigSource};
#[cfg(test)]
use std::sync::Weak;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_extension_objects::{
    Admit, AdmitInput, Bus, Call, Checkpoint, Context, Counter, Decision, Disclosure, ErrorCode,
    Fault, Frame, Operation, Origin, Outcome, Projection, Reason, Request, ResultFrame, Stage,
    Success, Uuid4,
};

/// Workers per channel. A request waits for its callbacks, so more than one is needed for
/// requests not to queue behind each other; the bus holds at most eight outstanding.
const WORKERS: usize = 2;
/// How long a waiting thread goes before it looks at the stop flag again.
const SLICE: Duration = Duration::from_millis(50);

/// Why a channel is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelEnd {
    /// The bus ended: the extension closed it, or a frame, write or correlation failed.
    Bus(Fault),
    /// A callback was not answered within its bound, so the request could not be answered
    /// either: a result is refused while a callback is outstanding. Never an allow.
    CallbackTimeout,
    /// The service stopped or replaced the channel.
    Stopped,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Work {
    request: Request,
    received: Instant,
}
struct Hub {
    generation: Uuid4,
    source: ConfigSource,
    callback: Duration,
    request: Duration,
    stop: AtomicBool,
    ended: Mutex<Option<ChannelEnd>>,
    queue: Mutex<VecDeque<Work>>,
    queued: Condvar,
    /// Decisions delivered by the dispatcher and not yet taken by their worker.
    decisions: Mutex<BTreeMap<u64, Decision>>,
    decided: Condvar,
    /// The last callback identifier issued. Held across issuing one and writing its frame,
    /// so identifiers reach the wire in the strictly increasing order the ledger requires.
    issuer: Mutex<u64>,
}
impl Hub {
    /// Record the first reason and wake every thread.
    fn end(&self, why: ChannelEnd) {
        locked(&self.ended).get_or_insert(why);
        self.stop.store(true, Ordering::Release);
        self.queued.notify_all();
        self.decided.notify_all();
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}

pub(super) struct Running {
    hub: Arc<Hub>,
    #[cfg(test)]
    bus: Weak<Bus>,
    threads: Vec<JoinHandle<()>>,
}
impl Running {
    /// Start the dispatcher and workers of an established `bus`.
    pub(super) fn start(
        bus: Bus,
        source: ConfigSource,
        bounds: &ServiceBounds,
    ) -> Result<Self, Fault> {
        let hub = Arc::new(Hub {
            generation: bus.generation(),
            source,
            callback: bounds.callback,
            request: bounds.request,
            stop: AtomicBool::new(false),
            ended: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
            queued: Condvar::new(),
            decisions: Mutex::new(BTreeMap::new()),
            decided: Condvar::new(),
            issuer: Mutex::new(0),
        });
        let bus = Arc::new(bus);
        let mut threads = Vec::new();
        for index in 0..=WORKERS {
            let (thread_hub, thread_bus) = (Arc::clone(&hub), Arc::clone(&bus));
            let spawned = thread::Builder::new()
                .name(if index == 0 {
                    "tmt-object-dispatch".into()
                } else {
                    format!("tmt-object-worker-{index}")
                })
                .spawn(move || {
                    if index == 0 {
                        dispatch(&thread_hub, &thread_bus);
                    } else {
                        work(&thread_hub, &thread_bus);
                    }
                });
            match spawned {
                Ok(thread) => threads.push(thread),
                Err(error) => {
                    hub.end(ChannelEnd::Stopped);
                    for thread in threads {
                        let _ = thread.join();
                    }
                    return Err(Fault::Io(error.kind()));
                }
            }
        }
        Ok(Self {
            hub,
            #[cfg(test)]
            bus: Arc::downgrade(&bus),
            threads,
        })
    }

    pub(super) fn generation(&self) -> Uuid4 {
        self.hub.generation
    }
    /// Why the channel is over, or `None` while it runs.
    pub(super) fn ended(&self) -> Option<ChannelEnd> {
        *locked(&self.hub.ended)
    }
    /// Stop and join every thread; the last of them closes the bus.
    pub(super) fn end(self) {
        self.hub.end(ChannelEnd::Stopped);
        for thread in self.threads {
            let _ = thread.join();
        }
    }
    /// The bus, which is gone once every thread has ended.
    #[cfg(test)]
    pub(super) fn bus(&self) -> Weak<Bus> {
        Weak::clone(&self.bus)
    }
}

/// The only reader of the bus.
fn dispatch(hub: &Hub, bus: &Bus) {
    while !hub.stopped() {
        match bus.recv(Some(Instant::now() + SLICE)) {
            Ok(Frame::Request(request)) => {
                locked(&hub.queue).push_back(Work {
                    request,
                    received: Instant::now(),
                });
                hub.queued.notify_one();
            }
            Ok(Frame::Admission(answer)) => {
                locked(&hub.decisions).insert(answer.callback_id.get(), answer.decision);
                hub.decided.notify_all();
            }
            // The ledger refuses every other kind in this direction before it is queued.
            Ok(_) => {}
            Err(Fault::Timeout(Stage::Idle)) => {}
            Err(fault) => hub.end(ChannelEnd::Bus(fault)),
        }
    }
}

fn work(hub: &Hub, bus: &Bus) {
    while let Some(next) = take(hub) {
        if let Err(why) = serve(hub, bus, next) {
            hub.end(why);
        }
    }
}
fn take(hub: &Hub) -> Option<Work> {
    let mut queue = locked(&hub.queue);
    loop {
        if hub.stopped() {
            return None;
        }
        if let Some(next) = queue.pop_front() {
            return Some(next);
        }
        queue = hub
            .queued
            .wait_timeout(queue, SLICE)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
}

/// Answer one request. An `Err` ends the channel.
fn serve(hub: &Hub, bus: &Bus, next: Work) -> Result<(), ChannelEnd> {
    let Work { request, received } = next;
    let deadline = received + hub.request;
    match (request.origin, &request.call) {
        // No mounted origin is established yet, so nothing mounted is admitted.
        (Origin::Mounted(_), _) => {
            send_result(hub, bus, &request, Outcome::Failure(ErrorCode::Denied))
        }
        (Origin::LocalExtension, Call::Config(_)) => config(hub, bus, &request, deadline),
        // The other six methods belong to later slices: no callback and no effect.
        (Origin::LocalExtension, _) => {
            send_result(hub, bus, &request, Outcome::Failure(ErrorCode::Unavailable))
        }
    }
}

/// `objects.config` for the local extension: current acquire admission, the delivered
/// backend's projection, then current disclose admission. Each request asks afresh and
/// nothing is remembered between requests.
fn config(hub: &Hub, bus: &Bus, request: &Request, deadline: Instant) -> Result<(), ChannelEnd> {
    let Call::Config(input) = &request.call else {
        return Ok(());
    };
    let operation = |disclosure| Operation {
        input: AdmitInput::Config(input.clone()),
        disclosure,
    };
    let refusal = |decision| match decision {
        Decision::Allow => None,
        Decision::Deny => Some(ErrorCode::Denied),
        Decision::Unavailable => Some(ErrorCode::Unavailable),
    };
    let acquire = ask(
        hub,
        bus,
        request,
        Checkpoint::Acquire,
        operation(None),
        deadline,
    )?;
    if let Some(code) = refusal(acquire) {
        return send_result(hub, bus, request, Outcome::Failure(code));
    }
    let projection = Projection::Local;
    let config = hub.source.project(projection);
    let disclosure = Some(Disclosure::Config { projection });
    let disclose = ask(
        hub,
        bus,
        request,
        Checkpoint::Disclose,
        operation(disclosure),
        deadline,
    )?;
    if let Some(code) = refusal(disclose) {
        return send_result(hub, bus, request, Outcome::Failure(code));
    }
    send_result(hub, bus, request, Outcome::Success(Success::Config(config)))
}

/// Send one admission callback and wait for its decision, within the callback bound and
/// the request's own deadline. No answer in time ends the channel.
fn ask(
    hub: &Hub,
    bus: &Bus,
    request: &Request,
    boundary: Checkpoint,
    operation: Operation,
    deadline: Instant,
) -> Result<Decision, ChannelEnd> {
    let limit = deadline.min(Instant::now() + hub.callback);
    let id = {
        let mut last = locked(&hub.issuer);
        *last += 1;
        let callback_id =
            Counter::new(*last).map_err(|_| ChannelEnd::Bus(Fault::Correlation(Reason::Order)))?;
        let admit = Frame::Admit(Admit {
            generation: hub.generation,
            callback_id,
            request_id: request.request_id,
            boundary,
            context: Context::LocalExtension,
            operation,
        });
        bus.send(&admit).map_err(ChannelEnd::Bus)?;
        *last
    };
    let mut decisions = locked(&hub.decisions);
    loop {
        if let Some(decision) = decisions.remove(&id) {
            return Ok(decision);
        }
        if hub.stopped() {
            return Err(ChannelEnd::Stopped);
        }
        let left = limit
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or(ChannelEnd::CallbackTimeout)?;
        decisions = hub
            .decided
            .wait_timeout(decisions, left.min(SLICE))
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
}

fn send_result(
    hub: &Hub,
    bus: &Bus,
    request: &Request,
    outcome: Outcome,
) -> Result<(), ChannelEnd> {
    let result = Frame::Result(ResultFrame {
        generation: hub.generation,
        request_id: request.request_id,
        method: request.call.method(),
        transfer_id: transfer_of(&request.call),
        outcome,
    });
    bus.send(&result).map_err(ChannelEnd::Bus)
}
/// The transfer a request names, which its result repeats.
fn transfer_of(call: &Call) -> Option<Uuid4> {
    match call {
        Call::Begin(input) => Some(input.transfer_id),
        Call::Part(input) => Some(input.transfer_id),
        Call::Commit(input) | Call::Discard(input) => Some(input.transfer_id),
        Call::Status(input) => Some(input.transfer_id),
        Call::Config(_) | Call::Read(_) => None,
    }
}
