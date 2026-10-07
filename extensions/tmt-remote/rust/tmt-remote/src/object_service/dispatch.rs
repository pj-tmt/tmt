//! One running channel: a dispatcher that alone reads the bus, and a small fixed pool of
//! workers that answer requests. The dispatcher keeps reading while workers wait for a
//! callback decision, so a decision is never stuck behind the request that needs it.
//! Nothing here holds a lock across a bus send, a wait for the extension or a join.
//!
//! The channel is bounded and final: when the bus ends, an extension leaves a callback
//! unanswered past its bound, or the service stops, every thread ends and the last one
//! drops the bus, which shuts the socket down and joins its reader. No frame spawns a
//! thread, and nothing is retried.
use super::{
    ServiceBounds,
    config::ConfigSource,
    origins::{Events, Next, Origins},
};
use crate::mount::Sessions;
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
    Fault, Frame, Operation, Origin, OriginState, Outcome, Projection, Reason, Request,
    ResultFrame, Stage, Success, Uuid4,
};

/// Where a test-only pause runs in a `config` request.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pause {
    /// After the acquire decision, before the disclose callback.
    BetweenAdmissions,
    /// After the disclose decision, before the result is sent.
    BeforeResult,
}
/// A test-only pause; it receives the request's deadline.
#[cfg(test)]
pub(super) type Hook = Option<Arc<dyn Fn(Pause, Instant) + Send + Sync>>;
#[cfg(not(test))]
pub(super) type Hook = ();

/// Everything a channel needs to start besides its bus.
pub(super) struct Launch<'a> {
    pub(super) source: ConfigSource,
    pub(super) bounds: &'a ServiceBounds,
    pub(super) hook: Hook,
    pub(super) origins: &'a Origins,
    pub(super) extension: &'a str,
    pub(super) tunnels: usize,
    /// The owner sessions mounted tunnels were admitted under.
    pub(super) sessions: Arc<dyn Sessions>,
}

/// The most origin-state notices a channel may hold: an `established` and a `closed` for
/// every tunnel the extension may have.
pub(super) const fn notice_limit(tunnels: usize) -> usize {
    2 * tunnels
}

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
    /// The extension stopped reading origin-state notices and the backlog hit its bound.
    Backlog,
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
    extension: String,
    origins: Origins,
    sessions: Arc<dyn Sessions>,
    /// Origin-state notices for the announcer, the only sender of them.
    events: Arc<Events>,
    source: ConfigSource,
    callback: Duration,
    request: Duration,
    #[cfg_attr(not(test), allow(dead_code))]
    hook: Hook,
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
        // Every origin of this channel is gone with it, so none can reach a successor; this
        // happens before the end is visible, so whoever sees the end sees no origins.
        self.origins.detach(&self.extension, self.generation);
        locked(&self.ended).get_or_insert(why);
        self.stop.store(true, Ordering::Release);
        self.queued.notify_all();
        self.decided.notify_all();
        self.events.wake();
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
    pub(super) fn start(bus: Bus, launch: Launch<'_>) -> Result<Self, Fault> {
        let Launch {
            source,
            bounds,
            hook,
            origins,
            extension,
            tunnels,
            sessions,
        } = launch;
        let hub = Arc::new(Hub {
            generation: bus.generation(),
            extension: extension.to_owned(),
            origins: origins.clone(),
            sessions,
            events: Arc::new(Events::new(notice_limit(tunnels))),
            source,
            callback: bounds.callback,
            request: bounds.request,
            hook,
            stop: AtomicBool::new(false),
            ended: Mutex::new(None),
            queue: Mutex::new(VecDeque::new()),
            queued: Condvar::new(),
            decisions: Mutex::new(BTreeMap::new()),
            decided: Condvar::new(),
            issuer: Mutex::new(0),
        });
        let bus = Arc::new(bus);
        // Registered before any thread runs, so a channel that ends right away detaches
        // itself (`Hub::end`) instead of being registered after it is over.
        origins.attach(extension, hub.generation, Arc::clone(&hub.events));
        let mut threads = Vec::new();
        for index in 0..=WORKERS + 1 {
            let (thread_hub, thread_bus) = (Arc::clone(&hub), Arc::clone(&bus));
            let spawned = thread::Builder::new()
                .name(match index {
                    0 => "tmt-object-dispatch".into(),
                    i if i > WORKERS => "tmt-object-announce".into(),
                    i => format!("tmt-object-worker-{i}"),
                })
                .spawn(move || match index {
                    0 => dispatch(&thread_hub, &thread_bus),
                    i if i > WORKERS => announce(&thread_hub, &thread_bus),
                    _ => work(&thread_hub, &thread_bus),
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

/// The only sender of origin-state notices, so each origin's `established` precedes its
/// `closed` on the wire. A write that fails or stalls ends the channel and every record.
fn announce(hub: &Hub, bus: &Bus) {
    while !hub.stopped() {
        match hub.events.next(SLICE) {
            Next::Notice(origin, phase) => {
                let notice = Frame::OriginState(OriginState {
                    generation: hub.generation,
                    origin_id: origin,
                    phase,
                });
                if let Err(fault) = bus.send(&notice) {
                    hub.end(ChannelEnd::Bus(fault));
                }
            }
            Next::Overflow => hub.end(ChannelEnd::Backlog),
            Next::Idle => {}
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
    let denied = || Outcome::Failure(ErrorCode::Denied);
    match (request.origin, &request.call) {
        (Origin::LocalExtension, Call::Config(_)) => config(hub, bus, &request, deadline),
        // The other six methods belong to later slices: no callback and no effect.
        (Origin::LocalExtension, _) => {
            send_result(hub, bus, &request, Outcome::Failure(ErrorCode::Unavailable))
        }
        // A mounted request is only as good as its origin: established on this channel and,
        // for an owner session, still current. Anything else is denied without a callback.
        (Origin::Mounted(id), call) => match (standing(hub, id), call) {
            (None, _) => send_result(hub, bus, &request, denied()),
            (Some(_), Call::Config(_)) => config(hub, bus, &request, deadline),
            (Some(_), _) => {
                send_result(hub, bus, &request, Outcome::Failure(ErrorCode::Unavailable))
            }
        },
    }
}

/// The context a request arrived in, if its origin still stands: Remote's registry says the
/// origin is established on this channel, and for an owner session the session owner says it
/// is the current one with the same device and grant revision. `None` denies, whatever the
/// extension might say. A local extension always stands.
fn standing(hub: &Hub, origin: Uuid4) -> Option<Context> {
    let known = hub
        .origins
        .established(origin, &hub.extension, hub.generation)?;
    let Some(owner) = known.owner else {
        return Some(Context::Mounted { origin_id: origin });
    };
    // A revision outside the frame grammar cannot be told to the extension, so it denies.
    let revision = owner.grant_revision;
    let device = Uuid4::parse(&owner.device_id).ok()?;
    let grammar = 1..=tmt_extension_objects::limits::MAX_SAFE_INTEGER;
    (grammar.contains(&revision) && hub.sessions.current(&owner)).then_some(Context::OwnerSession {
        origin_id: origin,
        device_id: device,
        grant_revision: revision,
    })
}
fn stands(hub: &Hub, origin: Origin) -> Option<Context> {
    match origin {
        Origin::LocalExtension => Some(Context::LocalExtension),
        Origin::Mounted(id) => standing(hub, id),
    }
}

/// `objects.config`: current acquire admission, the delivered backend's projection, then
/// current disclose admission. Each request asks afresh and nothing is remembered between
/// requests. A local extension sees the owner's limits; every mounted origin, owner session
/// or not, sees the reduced browser projection. The origin is checked before each callback
/// and again before the result leaves, so a session that ended meanwhile discloses nothing.
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
    let spent = || Outcome::Failure(ErrorCode::Unavailable);
    let denied = || Outcome::Failure(ErrorCode::Denied);
    let Some(context) = stands(hub, request.origin) else {
        return send_result(hub, bus, request, denied());
    };
    let Some(acquire) = ask(
        hub,
        bus,
        request,
        context,
        Checkpoint::Acquire,
        operation(None),
        deadline,
    )?
    else {
        return send_result(hub, bus, request, spent());
    };
    if let Some(code) = refusal(acquire) {
        return send_result(hub, bus, request, Outcome::Failure(code));
    }
    let projection = match request.origin {
        Origin::LocalExtension => Projection::Local,
        Origin::Mounted(_) => Projection::Browser,
    };
    let config = hub.source.project(projection);
    let disclosure = Some(Disclosure::Config { projection });
    #[cfg(test)]
    if let Some(hook) = &hub.hook {
        hook(Pause::BetweenAdmissions, deadline);
    }
    let Some(context) = stands(hub, request.origin) else {
        return send_result(hub, bus, request, denied());
    };
    let Some(disclose) = ask(
        hub,
        bus,
        request,
        context,
        Checkpoint::Disclose,
        operation(disclosure),
        deadline,
    )?
    else {
        return send_result(hub, bus, request, spent());
    };
    if let Some(code) = refusal(disclose) {
        return send_result(hub, bus, request, Outcome::Failure(code));
    }
    #[cfg(test)]
    if let Some(hook) = &hub.hook {
        hook(Pause::BeforeResult, deadline);
    }
    // Fenced again before the result leaves: a revoked or closed origin gets no bytes.
    if stands(hub, request.origin).is_none() {
        return send_result(hub, bus, request, denied());
    }
    send_result(hub, bus, request, Outcome::Success(Success::Config(config)))
}

/// Send one admission callback and wait for its decision, within the callback bound and
/// the request's own deadline. `None` means the request's time was already spent, so no
/// callback was sent and the request is simply unavailable; a callback that is sent and
/// not answered in time ends the channel.
fn ask(
    hub: &Hub,
    bus: &Bus,
    request: &Request,
    context: Context,
    boundary: Checkpoint,
    operation: Operation,
    deadline: Instant,
) -> Result<Option<Decision>, ChannelEnd> {
    let now = Instant::now();
    if now >= deadline {
        return Ok(None);
    }
    let limit = deadline.min(now + hub.callback);
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
            context,
            operation,
        });
        bus.send(&admit).map_err(ChannelEnd::Bus)?;
        *last
    };
    let mut decisions = locked(&hub.decisions);
    loop {
        if let Some(decision) = decisions.remove(&id) {
            return Ok(Some(decision));
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
