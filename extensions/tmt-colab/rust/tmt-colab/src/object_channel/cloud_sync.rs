//! Native cloud page synchronization on the existing neutral bus. One worker owns each
//! generation: it asks Remote to activate sync, binds, holds the interests of the one active
//! page and keeps that page's local originals published. Notices are wakeups only; the verified
//! Store owns every publication position.
use crate::{SealedClient, keyring::Layout, object_channel::ChannelOwner, store::Store};
use std::{
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tmt_extension_objects::Uuid4;

mod activation;
#[cfg(test)]
mod fixture;
#[cfg(test)]
mod interests;
mod local;
mod publisher;
mod reader;
#[cfg(test)]
mod tests;
mod worker;

#[cfg(test)]
pub(crate) use interests::PageInterests;
pub(crate) use local::{Bound, StoreLocal};
pub(crate) use publisher::Local;

/// Remote allows 30 seconds for provider work. The sealed caller leaves room
/// for that work and cleanup rather than expiring the shared attachment bus.
pub const REQUEST_BOUND: Duration = Duration::from_secs(60);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// One page of one space at its current epoch: the unit a watched group follows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PageRef {
    pub space: String,
    pub page: String,
    pub epoch: u64,
}

struct Pulse {
    state: Mutex<PulseState>,
    wake: Condvar,
}
struct PulseState {
    page: Option<PageRef>,
    version: u64,
    stopped: bool,
}
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}
/// What woke the worker.
pub struct Wake {
    pub version: u64,
    pub page: Option<PageRef>,
    pub stopped: bool,
}
/// The most recently active page: the one a browser opened or showed, or the CLI wrote to.
/// Touching it is cheap and never blocks; the worker alone decides what to do about it.
#[derive(Clone)]
pub struct ActivePage(Arc<Pulse>);
impl Default for ActivePage {
    fn default() -> Self {
        Self(Arc::new(Pulse {
            state: Mutex::new(PulseState {
                page: None,
                version: 0,
                stopped: false,
            }),
            wake: Condvar::new(),
        }))
    }
}
impl ActivePage {
    pub fn touch(&self, page: PageRef) {
        let mut state = locked(&self.0.state);
        state.page = Some(page);
        state.version += 1;
        self.0.wake.notify_all();
    }
    /// Wake the worker for a reason that is not a page: a local write, or a retry that is due.
    pub fn nudge(&self) {
        locked(&self.0.state).version += 1;
        self.0.wake.notify_all();
    }
    fn stop(&self) {
        locked(&self.0.state).stopped = true;
        self.0.wake.notify_all();
    }
    /// The next state after `seen`, or the unchanged one when `timeout` passes first.
    pub fn wait(&self, seen: u64, timeout: Duration) -> Wake {
        let deadline = Instant::now() + timeout;
        let mut state = locked(&self.0.state);
        while state.version == seen && !state.stopped {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            state = self
                .0
                .wake
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        Wake {
            version: state.version,
            page: state.page.clone(),
            stopped: state.stopped,
        }
    }
}

/// Where the worker finds the current object-channel generation. The serve's `ChannelOwner`
/// in production; a fixed peer in tests.
pub(crate) trait Channels: Send + Sync {
    fn client(&self) -> Option<SealedClient>;
    /// Counts accepted channels, so a generation nobody activated is noticed.
    fn started(&self) -> u64;
}
impl Channels for ChannelOwner {
    fn client(&self) -> Option<SealedClient> {
        ChannelOwner::client(self)
    }
    fn started(&self) -> u64 {
        self.generation()
    }
}

/// What Remote's single activation exchange returned: its public project and the generation of
/// the channel it installed. Neither carries Firebase credentials or authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activation {
    pub project: String,
    pub generation: Uuid4,
}
impl Activation {
    /// The activation Remote's reply describes, or none for a generation that is not an ID.
    pub fn from_reply(project: String, generation: &str) -> Option<Self> {
        Some(Self {
            project,
            generation: Uuid4::parse(generation).ok()?,
        })
    }
}
/// A typed refusal exactly as Remote wrote it (`state` is only ever `deadline`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: String,
    pub state: Option<String>,
}
/// Why one activation gave no generation: Remote said no, or the exchange itself did not
/// complete (no executable, deadline, unreadable reply).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationFailure {
    Remote(Refusal),
    Local,
}
pub type Activate = dyn Fn(Instant) -> Result<Activation, ActivationFailure> + Send + Sync;
/// Everything the worker needs, injected: the real control exchange and a one-line sink for
/// a page that cloud sync cannot continue.
pub(crate) struct Parts {
    pub channels: Arc<dyn Channels>,
    pub activity: ActivePage,
    pub activate: Arc<Activate>,
    pub report: Arc<dyn Fn(&str) + Send + Sync>,
    pub declaration_digest: String,
    pub space: String,
    /// The publisher's local side for one bound client; its `Bound` is what Remote returned.
    pub local: Arc<dyn Fn(Bound) -> Option<Box<dyn Local + Send>> + Send + Sync>,
}

/// What the serve supplies to run cloud sync: Remote's activation exchange, the one-line sink
/// for a page that cannot continue, and the digest of the declaration this executable is
/// frozen to.
pub struct Config {
    pub activate: Arc<Activate>,
    pub report: Arc<dyn Fn(&str) + Send + Sync>,
    pub declaration_digest: String,
}

/// The serve's one cloud-sync worker.
pub struct CloudSync {
    activity: ActivePage,
    thread: Option<JoinHandle<()>>,
}
impl CloudSync {
    /// The serve's worker: it reads the space owned by `owner` through its own Store handle.
    pub(crate) fn serve(
        objects: Arc<ChannelOwner>,
        activity: ActivePage,
        config: Config,
        layout: Layout,
        owner: [u8; 32],
        space: String,
    ) -> io::Result<Self> {
        let Config {
            activate,
            report,
            declaration_digest,
        } = config;
        let local_space = space.clone();
        Self::start(Parts {
            channels: objects,
            activity,
            activate,
            report,
            declaration_digest,
            space,
            local: Arc::new(move |bound| {
                let store = Store::open(&layout).ok()?;
                Some(Box::new(StoreLocal::new(
                    store,
                    owner,
                    local_space.clone(),
                    bound,
                    Arc::new(now_ms),
                )))
            }),
        })
    }
    pub(crate) fn start(parts: Parts) -> io::Result<Self> {
        let activity = parts.activity.clone();
        let thread = thread::Builder::new()
            .name("colab-cloud-sync".into())
            .spawn(move || worker::run(parts))?;
        Ok(Self {
            activity,
            thread: Some(thread),
        })
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
    /// End the worker. The caller closes the object channel first, so a request blocked on the
    /// bus ends quickly; the worker cancels its interests and joins before this returns.
    pub fn stop(mut self) {
        self.activity.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for CloudSync {
    fn drop(&mut self) {
        self.activity.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
