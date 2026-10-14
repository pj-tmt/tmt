//! Remote's object service: the one installation-wide owner of local object storage and
//! of the private object channel to each extension that declares one. It borrows the
//! `Serving` lease through [`LocalFs`], so it cannot outlive it, and it names no
//! extension of its own: the extensions come only from the static declaration list.
//!
//! An extension has at most one active bus and one setup candidate, and the installation
//! at most [`ServiceBounds::buses`] buses in all. A channel is opened only by an
//! explicit [`ObjectService::activate`]; setup permits only the carrier’s one pre-reply
//! version fallback, with no operation retry or polling. Serve attempts activation once per Local declaration before
//! door readiness and shuts down the service before releasing its lease. The
//! static mount registry alone chooses enabled extensions; callbacks grant no cached authority.
//!
//! Locks are short and never held across I/O, a connect, a handshake or a close.
use crate::{
    error::RemoteError,
    mount::{Extension, Mounts, ObjectDeclaration},
    objects::{Clock, ExtensionId, IoBudget, LocalFs, LocalHandle, ObjectBackend, Quotas},
    state::Serving,
    store::uuid_v4,
};
use std::{
    io,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use tmt_extension_objects::{
    Budget, Budgets, Bus, Caps, Fault, Offer, Uuid4, negotiate, negotiate_sync,
};

mod activation;
mod readiness;
pub use activation::{NativeActivation, Reactivation};
pub use readiness::ObjectReadiness;

mod config;
mod dispatch;
mod namespace;
mod observe;
mod original;
mod origins;
pub mod sync;
mod upload;
use config::ConfigSource;
pub use dispatch::ChannelEnd;
use dispatch::{Launch, Running};
pub use origins::Origins;

/// Bounds of the service; [`ServiceBounds::contract`] is the proposed contract set and
/// tests inject smaller values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceBounds {
    /// The whole setup of one channel, from connect to the end of the handshake.
    pub setup: Duration,
    /// Buses in all, counting every active bus and every setup candidate.
    pub buses: usize,
    /// Bounds of every frame on every bus.
    pub frames: Budgets,
    /// Outstanding requests and callbacks of one bus.
    pub caps: Caps,
    /// Outstanding requests and callbacks of all buses together.
    pub installation: Caps,
    /// One admission callback is answered within this.
    pub callback: Duration,
    /// One request is answered within this, callbacks included.
    pub request: Duration,
}
impl ServiceBounds {
    pub const fn contract() -> Self {
        Self {
            setup: Duration::from_secs(15),
            buses: 8,
            frames: Budgets::contract(),
            caps: Caps::contract(),
            installation: Caps {
                requests: 32,
                callbacks: 32,
            },
            callback: Duration::from_secs(5),
            request: Duration::from_secs(30),
        }
    }
}

/// Why a channel was not established. None of them changes another extension's bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivateError {
    /// No extension of that name declares objects.
    NotDeclared,
    /// The service was stopped.
    Stopped,
    /// Native sync has no registered project-owner factory.
    Unavailable,
    /// A setup candidate for the extension is already in progress.
    Busy,
    /// The installation already has its most buses.
    Capacity,
    /// The extension's socket is missing or unsafe, or refused the connection.
    Connect(io::ErrorKind),
    /// The handshake or the bus did not start.
    Channel(Fault),
}

struct Setup {
    index: usize,
    id: ExtensionId,
    source: ConfigSource,
    tunnels: usize,
    native: Option<Arc<sync::Registration>>,
}
struct Declared {
    name: &'static str,
    id: ExtensionId,
    /// What `objects.config` projects for this extension, fixed when the service opens.
    source: ConfigSource,
    /// The tunnel cap the extension is mounted with, which bounds its origins.
    tunnels: usize,
    /// A candidate is being set up outside the lock.
    setup: bool,
    active: Option<Running>,
    failure: Option<ActivateError>,
    native: Option<Arc<sync::Registration>>,
}
impl Declared {
    /// Whether the extension has a channel that has not ended.
    fn running(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|running| running.ended().is_none())
    }
}
struct State {
    stopped: bool,
    slots: Vec<Declared>,
}

/// The service is bound to the held serve lease for its lifetime: the lease cannot be
/// released while it or any handle it gave out exists.
/// It must be dropped or shut down, never forgotten: scoped ownership joins all
/// mutating workers before its Serving borrow ends. Production serve owns it by
/// scope.
///
/// ```compile_fail,E0505
/// use std::{sync::atomic::AtomicBool, time::Instant};
/// use tmt_remote::{
///     mount::EXTENSIONS,
///     object_service::{ObjectService, Origins, ServiceBounds},
///     objects::{IoBudget, Quotas, system_clock},
///     state::Layout,
/// };
/// fn escape(root: &std::path::Path) {
///     let serving = Layout::open(root).unwrap().serve_lock().unwrap();
///     let cancelled = AtomicBool::new(false);
///     let io = IoBudget { deadline: Instant::now(), cancelled: &cancelled };
///     let service = ObjectService::open(
///         &serving, &EXTENSIONS, Quotas::contract(), system_clock(), ServiceBounds::contract(),
///         Origins::default(), &io,
///     ).unwrap();
///     drop(serving);
///     let _ = service.is_some();
/// }
/// ```
pub struct ObjectService<'s> {
    storage: LocalFs<'s>,
    bounds: ServiceBounds,
    /// Outstanding entries shared by every bus of the installation.
    budget: Budget,
    /// Where the origins of this service's channels are kept, shared with the mounts.
    origins: Origins,
    state: Arc<Mutex<State>>,
    /// Pauses a worker between a request's two admissions, so a test can spend its time.
    #[cfg(test)]
    hook: Mutex<dispatch::Hook>,
}

impl<'s> ObjectService<'s> {
    /// Prepare object storage for the extensions that declare it, or nothing at all when
    /// none does: no ledger or directory is created for a disabled declaration. Storage
    /// that cannot settle its accounting refuses here, so readiness never hides it.
    pub fn open(
        serving: &'s Serving,
        extensions: &'static [Extension],
        quotas: Quotas,
        clock: Clock,
        bounds: ServiceBounds,
        origins: Origins,
        io: &IoBudget<'_>,
    ) -> Result<Option<Self>, RemoteError> {
        let mut declared = Vec::new();
        for extension in extensions
            .iter()
            .filter(|extension| extension.objects == ObjectDeclaration::Local)
        {
            let id = ExtensionId::new(extension.name).map_err(|_| {
                RemoteError::new(
                    "REMOTE_OBJECTS_UNAVAILABLE",
                    "An extension declares object storage under an invalid name.",
                )
            })?;
            declared.push((extension.name, id, extension.tunnels));
        }
        if declared.is_empty() {
            return Ok(None);
        }
        let storage = LocalFs::open(serving, quotas, clock, io)?;
        let slots = declared
            .into_iter()
            .map(|(name, id, tunnels)| {
                let backend = storage.handle(id.clone());
                let source = ConfigSource {
                    backend_id: backend.id(),
                    caps: backend.capabilities(),
                    quotas,
                };
                Declared {
                    name,
                    id,
                    source,
                    tunnels,
                    setup: false,
                    active: None,
                    failure: None,
                    native: None,
                }
            })
            .collect();
        Ok(Some(Self {
            storage,
            bounds,
            budget: Budget::new(bounds.installation.requests, bounds.installation.callbacks),
            origins,
            state: Arc::new(Mutex::new(State {
                stopped: false,
                slots,
            })),
            #[cfg(test)]
            hook: Mutex::new(None),
        }))
    }

    /// The backend of one declared extension.
    pub fn handle(&self, name: &str) -> Option<LocalHandle<'_>> {
        let state = self.locked();
        let slot = state.slots.iter().find(|slot| slot.name == name)?;
        Some(self.storage.handle(slot.id.clone()))
    }

    /// Register verified project-owner construction without opening a channel.
    /// The production provider registers only after its independent delivery gate.
    pub fn register_native(
        &self,
        name: &str,
        registration: sync::Registration,
    ) -> Result<(), ActivateError> {
        let mut state = self.locked();
        if state.stopped {
            return Err(ActivateError::Stopped);
        }
        let slot = state
            .slots
            .iter_mut()
            .find(|slot| slot.name == name)
            .ok_or(ActivateError::NotDeclared)?;
        if slot.setup || slot.native.is_some() {
            return Err(ActivateError::Busy);
        }
        if registration.extension() != name {
            return Err(ActivateError::NotDeclared);
        }
        slot.native = Some(Arc::new(registration));
        Ok(())
    }
    /// Explicit v3 activation; it never guesses a version or falls back.
    pub fn activate_native_until(
        &self,
        mounts: &Mounts,
        name: &str,
        deadline: Instant,
    ) -> Result<Uuid4, ActivateError> {
        self.activate_channel_until(mounts, name, deadline, true)
    }
    /// Open the channel to `name` and make it the extension's active bus; the bus it
    /// replaces is closed and joined. The extension's socket is connected through
    /// `mounts` with exactly the `Host` and mount values the door sets on every request
    /// it sends that extension.
    pub fn activate(&self, mounts: &Mounts, name: &str) -> Result<Uuid4, ActivateError> {
        self.activate_until(mounts, name, Instant::now() + self.bounds.setup)
    }
    /// Activate within the caller's absolute deadline; setup I/O also respects its service bound.
    pub fn activate_until(
        &self,
        mounts: &Mounts,
        name: &str,
        deadline: Instant,
    ) -> Result<Uuid4, ActivateError> {
        self.activate_channel_until(mounts, name, deadline, false)
    }
    fn activate_channel_until(
        &self,
        mounts: &Mounts,
        name: &str,
        deadline: Instant,
        native: bool,
    ) -> Result<Uuid4, ActivateError> {
        if Instant::now() >= deadline {
            return Err(ActivateError::Channel(Fault::Timeout(
                tmt_extension_objects::Stage::Head,
            )));
        }
        let prepared = self.begin_setup(name, native)?;
        let index = prepared.index;
        #[cfg(test)]
        let hook = self.hook.lock().unwrap_or_else(|p| p.into_inner()).clone();
        #[cfg(test)]
        if let Some(hook) = &hook {
            hook(dispatch::Pause::BeforeSetup, deadline);
        }
        let built = self.connect(
            mounts,
            name,
            prepared,
            deadline.min(Instant::now() + self.bounds.setup),
        );
        #[cfg(test)]
        if built.is_ok()
            && let Some(hook) = hook
        {
            hook(dispatch::Pause::BeforeInstall, deadline);
        }
        self.finish_setup(index, built, deadline)
    }
    fn begin_setup(&self, name: &str, native: bool) -> Result<Setup, ActivateError> {
        let mut state = self.locked();
        if state.stopped {
            return Err(ActivateError::Stopped);
        }
        let index = state
            .slots
            .iter()
            .position(|slot| slot.name == name)
            .ok_or(ActivateError::NotDeclared)?;
        let native = if native {
            Some(
                state.slots[index]
                    .native
                    .clone()
                    .ok_or(ActivateError::Unavailable)?,
            )
        } else {
            None
        };
        if state.slots[index].setup {
            return Err(ActivateError::Busy);
        }
        let buses: usize = state
            .slots
            .iter()
            .map(|slot| usize::from(slot.setup) + usize::from(slot.running()))
            .sum();
        if buses >= self.bounds.buses {
            state.slots[index].failure = Some(ActivateError::Capacity);
            return Err(ActivateError::Capacity);
        }
        state.slots[index].setup = true;
        Ok(Setup {
            index,
            id: state.slots[index].id.clone(),
            source: state.slots[index].source,
            tunnels: state.slots[index].tunnels,
            native,
        })
    }
    fn connect(
        &self,
        mounts: &Mounts,
        name: &str,
        prepared: Setup,
        setup: Instant,
    ) -> Result<Running, ActivateError> {
        let Setup {
            id,
            source,
            tunnels,
            native,
            ..
        } = prepared;
        let connect = || {
            let endpoint = mounts
                .open_object_channel(name, setup)
                .map_err(|error| Fault::Io(error.kind()))?;
            let generation = uuid_v4()
                .ok()
                .and_then(|text| Uuid4::parse(&text).ok())
                .ok_or(Fault::Io(io::ErrorKind::Other))?;
            Ok((
                endpoint.stream,
                Offer {
                    host: endpoint.host,
                    mount: endpoint.mount,
                    generation,
                },
            ))
        };
        let link = if native.is_some() {
            connect()
                .and_then(|(stream, offer)| negotiate_sync(stream, &offer, setup))
                .and_then(|link| {
                    if link.version() == 3 {
                        Ok(link)
                    } else {
                        Err(Fault::UnsupportedSync)
                    }
                })
        } else {
            negotiate(setup, connect)
        }
        .map_err(|fault| match fault {
            Fault::Io(kind) => ActivateError::Connect(kind),
            fault => ActivateError::Channel(fault),
        })?;
        if self.locked().stopped {
            return Err(ActivateError::Stopped);
        }
        if Instant::now() >= setup {
            return Err(ActivateError::Channel(Fault::Timeout(
                tmt_extension_objects::Stage::Head,
            )));
        }
        let bus = Bus::start(
            link,
            self.bounds.frames,
            self.bounds.caps,
            Some(self.budget.clone()),
        )
        .map_err(ActivateError::Channel)?;
        #[cfg(test)]
        let hook = self
            .hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        #[cfg(not(test))]
        let hook = ();
        let launch = Launch {
            source,
            bounds: &self.bounds,
            hook,
            origins: &self.origins,
            extension: name,
            tunnels,
            sessions: mounts.sessions(),
            backend: self.storage.reader(id.clone()),
            writer: self.storage.writer(id),
            native,
        };
        Running::start(bus, launch).map_err(ActivateError::Channel)
    }
    fn finish_setup(
        &self,
        index: usize,
        built: Result<Running, ActivateError>,
        deadline: Instant,
    ) -> Result<Uuid4, ActivateError> {
        // Channels are ended after the lock is released: ending joins threads.
        let (result, closing) = {
            let mut state = self.locked();
            let stopped = state.stopped;
            let slot = &mut state.slots[index];
            slot.setup = false;
            match built {
                Err(error) => {
                    slot.failure = Some(error);
                    (Err(error), None)
                }
                Ok(running) if stopped || Instant::now() >= deadline => {
                    let error = if stopped {
                        ActivateError::Stopped
                    } else {
                        ActivateError::Channel(Fault::Timeout(tmt_extension_objects::Stage::Head))
                    };
                    slot.failure = Some(error);
                    (Err(error), Some(running))
                }
                Ok(running) => {
                    let generation = running.generation();
                    slot.failure = None;
                    (Ok(generation), slot.active.replace(running))
                }
            }
        };
        if let Some(running) = closing {
            running.end();
        }
        result
    }

    /// The generation of the extension's active bus, if it has one.
    pub fn active(&self, name: &str) -> Option<Uuid4> {
        let state = self.locked();
        let slot = state.slots.iter().find(|slot| slot.name == name)?;
        slot.active
            .as_ref()
            .filter(|running| running.ended().is_none())
            .map(Running::generation)
    }
    /// Why the extension's last channel is over, if it has ended.
    pub fn ended(&self, name: &str) -> Option<ChannelEnd> {
        let state = self.locked();
        let slot = state.slots.iter().find(|slot| slot.name == name)?;
        slot.active.as_ref().and_then(Running::ended)
    }
    /// The bus of the extension's channel, which is gone once the channel's threads end.
    #[cfg(test)]
    fn bus(&self, name: &str) -> Option<std::sync::Weak<Bus>> {
        let state = self.locked();
        let slot = state.slots.iter().find(|slot| slot.name == name)?;
        slot.active.as_ref().map(Running::bus)
    }

    /// Install the request boundary pause on later channels.
    #[cfg(test)]
    fn set_hook(&self, hook: dispatch::Hook) {
        *self
            .hook
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = hook;
    }

    /// Stop: later activations are refused, and every channel is ended: its threads are
    /// joined and its bus closed.
    pub fn shutdown(&self) {
        let channels: Vec<Running> = {
            let mut state = self.locked();
            state.stopped = true;
            state
                .slots
                .iter_mut()
                .filter_map(|slot| slot.active.take())
                .collect()
        };
        for running in channels {
            running.end();
        }
    }

    fn locked(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
impl Drop for ObjectService<'_> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests;
