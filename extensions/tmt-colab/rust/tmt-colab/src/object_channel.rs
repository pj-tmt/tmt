//! Colab's consumer of the neutral object bus. One generation owns correlations,
//! lifecycle notices and bounded callback workers; it grants no Colab permission.
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_extension_objects::{
    Admission, Admit, Budgets, Bus, Call, Caps, Checkpoint, Context, Counter, Decision, Fault,
    Frame, Link, Origin, OriginPhase, Outcome, Request, Role, Uuid4,
};

mod binding;
mod client;
pub(crate) use client::CommittedReader;
#[cfg(test)]
mod tests;

/// This owner rechecks the actual peer/root-local capture at EACH callback. It
/// cannot perform a backend bus request: that would recursively wait on admission.
pub(crate) trait CallbackOwner: Send + Sync {
    fn decide(&self, admit: &Admit, deadline: Instant) -> Decision;
}
#[derive(Clone, Debug)]
pub(crate) enum Error {
    Unavailable,
    Wire(Fault),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("Object channel unavailable"),
            Self::Wire(fault) => write!(f, "Object channel: {fault:?}"),
        }
    }
}
impl std::error::Error for Error {}
type Reply = std::result::Result<Outcome, Error>;
struct Pending {
    origin: Origin,
    owner: Arc<dyn CallbackOwner>,
    deadline: Instant,
    reply: mpsc::SyncSender<Reply>,
}
struct Callback {
    admit: Admit,
    pending: Arc<Pending>,
    deadline: Instant,
}
struct State {
    pending: BTreeMap<Counter, Arc<Pending>>,
    origins: HashSet<Uuid4>,
    callbacks: VecDeque<Callback>,
}
struct Shared {
    generation: Uuid4,
    bus: Mutex<Option<Arc<Bus>>>,
    interrupt: UnixStream,
    stopped: AtomicBool,
    state: Mutex<State>,
    changed: Condvar,
    /// Counter allocation and its wire send have one order, not just atomic IDs.
    next: Mutex<u64>,
}
fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
impl Shared {
    fn bus(&self) -> std::result::Result<Arc<Bus>, Error> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::Unavailable);
        }
        locked(&self.bus).clone().ok_or(Error::Unavailable)
    }
    fn stop(&self) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            let _ = self.interrupt.shutdown(Shutdown::Both);
            let mut state = locked(&self.state);
            for (_, pending) in std::mem::take(&mut state.pending) {
                let _ = pending.reply.try_send(Err(Error::Unavailable));
            }
            state.origins.clear();
            state.callbacks.clear();
            self.changed.notify_all();
        }
    }
    fn standing(&self, origin: Origin) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        match origin {
            Origin::LocalExtension => true,
            Origin::Mounted(id) => locked(&self.state).origins.contains(&id),
        }
    }
}
/// The serve/socket owner closes and joins this generation even if a consumer
/// retains a Client. Such a client then refuses; it cannot keep a reader alive.
pub(crate) struct ObjectChannel {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}
#[derive(Clone)]
pub(crate) struct Client(Arc<Shared>);
impl ObjectChannel {
    /// `interrupt` is cloned from the accepted socket BEFORE leaf handshake.
    /// It is used only for shutdown, never to read/write beside the leaf's Bus.
    pub(crate) fn start(link: Link, interrupt: UnixStream) -> std::result::Result<Self, Error> {
        if link.role() != Role::Extension {
            return Err(Error::Unavailable);
        }
        let bus = Arc::new(
            Bus::start(link, Budgets::contract(), Caps::contract(), None).map_err(Error::Wire)?,
        );
        let shared = Arc::new(Shared {
            generation: bus.generation(),
            bus: Mutex::new(Some(bus)),
            interrupt,
            stopped: AtomicBool::new(false),
            state: Mutex::new(State {
                pending: BTreeMap::new(),
                origins: HashSet::new(),
                callbacks: VecDeque::new(),
            }),
            changed: Condvar::new(),
            next: Mutex::new(1),
        });
        let mut channel = Self {
            shared,
            threads: Vec::new(),
        };
        let dispatcher = Arc::clone(&channel.shared);
        channel.threads.push(
            thread::Builder::new()
                .name("colab-object-dispatch".into())
                .spawn(move || dispatch(&dispatcher))
                .map_err(|_| Error::Unavailable)?,
        );
        for _ in 0..crate::limits::OBJECT_CALLBACK_WORKERS {
            let worker = Arc::clone(&channel.shared);
            channel.threads.push(
                thread::Builder::new()
                    .name("colab-object-admit".into())
                    .spawn(move || callbacks(&worker))
                    .map_err(|_| Error::Unavailable)?,
            );
        }
        Ok(channel)
    }
    pub(crate) fn client(&self) -> Client {
        Client(Arc::clone(&self.shared))
    }
    pub(crate) fn active(&self) -> bool {
        !self.shared.stopped.load(Ordering::Acquire)
    }
}
impl Drop for ObjectChannel {
    fn drop(&mut self) {
        self.shared.stop();
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
        // Only owned workers ever retain a Bus across a wait. They are joined,
        // and request senders release it before waiting for a result.
        if let Some(bus) = locked(&self.shared.bus).take() {
            // A concurrent sender may still be leaving its bounded write. The
            // interrupt already ended the leaf's reader; its last Arc joins it.
            drop(bus);
        }
    }
}
impl Client {
    pub(crate) fn generation(&self) -> Uuid4 {
        self.0.generation
    }
    pub(crate) fn standing(&self, origin: Origin) -> bool {
        self.0.standing(origin)
    }
    pub(crate) fn request(
        &self,
        origin: Origin,
        call: Call,
        owner: Arc<dyn CallbackOwner>,
        deadline: Instant,
    ) -> Reply {
        if !self.standing(origin) || Instant::now() >= deadline {
            return Err(Error::Unavailable);
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let pending = Arc::new(Pending {
            origin,
            owner,
            deadline,
            reply: sender,
        });
        {
            let mut next = locked(&self.0.next);
            let request_id = Counter::new(*next).map_err(|_| Error::Unavailable)?;
            *next = next.checked_add(1).ok_or(Error::Unavailable)?;
            let bus = self.0.bus()?;
            {
                let mut state = locked(&self.0.state);
                if state.pending.len() >= Caps::contract().requests {
                    return Err(Error::Unavailable);
                }
                state.pending.insert(request_id, pending);
            }
            let frame = Frame::Request(Request {
                generation: self.generation(),
                request_id,
                origin,
                call,
            });
            if let Err(fault) = bus.send_until(&frame, deadline) {
                locked(&self.0.state).pending.remove(&request_id);
                if bus.fault().is_some() {
                    self.0.stop();
                }
                return Err(Error::Wire(fault));
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(reply) => reply,
            Err(_) => {
                // Do not forget a still-outstanding ledger entry or let a late
                // callback/result satisfy another request. End this generation.
                self.0.stop();
                Err(Error::Unavailable)
            }
        }
    }
}
fn context_matches(origin: Origin, context: Context) -> bool {
    match (origin, context) {
        (Origin::LocalExtension, Context::LocalExtension) => true,
        (
            Origin::Mounted(wanted),
            Context::OwnerSession { origin_id, .. } | Context::Mounted { origin_id },
        ) => wanted == origin_id,
        _ => false,
    }
}
fn dispatch(shared: &Arc<Shared>) {
    let Ok(bus) = shared.bus() else {
        return;
    };
    while !shared.stopped.load(Ordering::Acquire) {
        let Ok(received) = bus.recv_stamped(None) else {
            break;
        };
        match received.frame {
            Frame::Result(result) => {
                let pending = locked(&shared.state).pending.remove(&result.request_id);
                let Some(pending) = pending else {
                    break;
                };
                let answer = if Instant::now() < pending.deadline && shared.standing(pending.origin)
                {
                    Ok(result.outcome)
                } else {
                    Err(Error::Unavailable)
                };
                let _ = pending.reply.try_send(answer);
            }
            Frame::OriginState(notice) => {
                let mut state = locked(&shared.state);
                match notice.phase {
                    OriginPhase::Established => {
                        if state.origins.len() >= crate::limits::TUNNELS {
                            break;
                        }
                        state.origins.insert(notice.origin_id);
                    }
                    OriginPhase::Closed => {
                        state.origins.remove(&notice.origin_id);
                    }
                }
                shared.changed.notify_all();
            }
            Frame::Admit(admit) => {
                let mut state = locked(&shared.state);
                let Some(pending) = state.pending.get(&admit.request_id).cloned() else {
                    break;
                };
                let deadline = pending
                    .deadline
                    .min(received.first_prefix + crate::limits::OBJECT_CALLBACK);
                if state.callbacks.len() >= Caps::contract().callbacks {
                    break;
                }
                state.callbacks.push_back(Callback {
                    admit,
                    pending,
                    deadline,
                });
                shared.changed.notify_one();
            }
            _ => break,
        }
    }
    shared.stop();
}
fn callbacks(shared: &Arc<Shared>) {
    loop {
        let callback = {
            let mut state = locked(&shared.state);
            while state.callbacks.is_empty() && !shared.stopped.load(Ordering::Acquire) {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|p| p.into_inner());
            }
            if shared.stopped.load(Ordering::Acquire) {
                return;
            }
            state.callbacks.pop_front().expect("callback ready")
        };
        let decision = if Instant::now() >= callback.deadline {
            Decision::Unavailable
        } else if !shared.standing(callback.pending.origin)
            || !context_matches(callback.pending.origin, callback.admit.context)
        {
            Decision::Deny
        } else {
            callback
                .pending
                .owner
                .decide(&callback.admit, callback.deadline)
        };
        let decision = if !shared.standing(callback.pending.origin) {
            Decision::Deny
        } else if Instant::now() >= callback.deadline {
            Decision::Unavailable
        } else {
            decision
        };
        let Ok(bus) = shared.bus() else {
            return;
        };
        let answer = Frame::Admission(Admission {
            generation: shared.generation,
            callback_id: callback.admit.callback_id,
            request_id: callback.admit.request_id,
            decision,
        });
        // Never send a late allow. A spent callback bound ends the generation,
        // because its correlation cannot be safely abandoned.
        if Instant::now() >= callback.deadline
            || bus.send_until(&answer, callback.deadline).is_err()
        {
            shared.stop();
            return;
        }
    }
}
