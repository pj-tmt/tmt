//! The registry of origins: one record per websocket upgrade to an extension that has an
//! object channel, from the moment Remote forwards the upgrade until its tunnel ends.
//! `Origins` is the one type that moves a record between phases:
//!
//! Pending -> Established -> gone, or Pending -> gone. A record is Pending while the upgrade
//! is forwarded, Established once the session is attached, the browser has its 101 and the
//! tunnel runs, and gone at the first close. A close wins over a later establish, and an
//! origin of one channel generation never moves to another: ending a channel removes all of
//! its records, and a replacement channel starts with none.
//!
//! Origin-state frames are lifecycle notices queued in order for the channel's announcer;
//! the registry, not a frame, is what Remote asks at request and disclose time, so a late
//! `closed` notice is safe.
//!
//! Lock order: `Origins` before a channel's [`Events`]. Neither is held while a frame is
//! written, and nothing here calls the sessions: closing a ticket never blocks on a socket
//! or on a session.
use crate::{
    mount::{OriginSink, OriginTicket, OwnerBinding},
    store::uuid_v4,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tmt_extension_objects::{OriginPhase, Uuid4};

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The origin-state notices of one channel, in the order they must reach the wire. Each
/// origin yields at most an `established` and a `closed`, and the tunnel cap bounds the
/// origins, so a backlog past `limit` means the extension has stopped reading: it ends the
/// channel instead of growing.
pub(super) struct Events {
    queue: Mutex<VecDeque<(Uuid4, OriginPhase)>>,
    ready: Condvar,
    limit: usize,
    overflowed: AtomicBool,
}
/// What the announcer does next.
pub(super) enum Next {
    Notice(Uuid4, OriginPhase),
    Overflow,
    Idle,
}
impl Events {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
            limit,
            overflowed: AtomicBool::new(false),
        }
    }
    fn push(&self, origin: Uuid4, phase: OriginPhase) {
        let mut queue = locked(&self.queue);
        if queue.len() >= self.limit {
            self.overflowed.store(true, Ordering::Release);
        } else {
            queue.push_back((origin, phase));
        }
        drop(queue);
        self.ready.notify_all();
    }
    /// The next notice within `wait`.
    pub(super) fn next(&self, wait: Duration) -> Next {
        let mut queue = locked(&self.queue);
        if self.overflowed.load(Ordering::Acquire) {
            return Next::Overflow;
        }
        if let Some((origin, phase)) = queue.pop_front() {
            return Next::Notice(origin, phase);
        }
        queue = self
            .ready
            .wait_timeout(queue, wait)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
        match queue.pop_front() {
            Some((origin, phase)) => Next::Notice(origin, phase),
            None if self.overflowed.load(Ordering::Acquire) => Next::Overflow,
            None => Next::Idle,
        }
    }
    /// Wake the announcer, for a stop.
    pub(super) fn wake(&self) {
        self.ready.notify_all();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Pending,
    Established,
}
struct Record {
    extension: String,
    generation: Uuid4,
    phase: Phase,
    /// The owner session the upgrade was admitted under, if any.
    owner: Option<OwnerBinding>,
    events: Arc<Events>,
}
struct Attached {
    generation: Uuid4,
    events: Arc<Events>,
}
#[derive(Default)]
struct Table {
    /// The newest channel of each extension, which new origins attach to.
    channels: HashMap<String, Attached>,
    records: HashMap<Uuid4, Record>,
}

/// What Remote knows about an established origin when it is asked.
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct Established {
    pub(super) owner: Option<OwnerBinding>,
}

/// A shared handle; clones are the same registry.
#[derive(Clone, Default)]
pub struct Origins {
    table: Arc<Mutex<Table>>,
}
impl Origins {
    /// Make a channel the one new origins of its extension attach to.
    pub(super) fn attach(&self, extension: &str, generation: Uuid4, events: Arc<Events>) {
        locked(&self.table)
            .channels
            .insert(extension.to_owned(), Attached { generation, events });
    }
    /// A channel ended: every origin of that generation is gone, with no notice (the bus is
    /// over) and nothing for a successor to inherit.
    pub(super) fn detach(&self, extension: &str, generation: Uuid4) {
        let mut table = locked(&self.table);
        if table
            .channels
            .get(extension)
            .is_some_and(|channel| channel.generation == generation)
        {
            table.channels.remove(extension);
        }
        table
            .records
            .retain(|_, record| record.generation != generation);
    }
    /// The established origin `id` of `extension` on channel `generation`; a Pending,
    /// closed, unknown, other-extension or other-generation origin is `None`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn established(
        &self,
        id: Uuid4,
        extension: &str,
        generation: Uuid4,
    ) -> Option<Established> {
        let table = locked(&self.table);
        let record = table.records.get(&id)?;
        (record.phase == Phase::Established
            && record.extension == extension
            && record.generation == generation)
            .then(|| Established {
                owner: record.owner.clone(),
            })
    }
    #[cfg(test)]
    pub(super) fn count(&self) -> usize {
        locked(&self.table).records.len()
    }

    fn establish(&self, id: Uuid4) {
        let mut table = locked(&self.table);
        if let Some(record) = table.records.get_mut(&id)
            && record.phase == Phase::Pending
        {
            record.phase = Phase::Established;
            record.events.push(id, OriginPhase::Established);
        }
    }
    fn close(&self, id: Uuid4) {
        let mut table = locked(&self.table);
        if let Some(record) = table.records.remove(&id)
            && record.phase == Phase::Established
        {
            record.events.push(id, OriginPhase::Closed);
        }
    }
}

impl OriginSink for Origins {
    fn pending(
        &self,
        extension: &str,
        owner: Option<OwnerBinding>,
    ) -> Option<Arc<dyn OriginTicket>> {
        let id = uuid_v4().ok().and_then(|text| Uuid4::parse(&text).ok())?;
        let mut table = locked(&self.table);
        let channel = table.channels.get(extension)?;
        let (generation, events) = (channel.generation, Arc::clone(&channel.events));
        table.records.insert(
            id,
            Record {
                extension: extension.to_owned(),
                generation,
                phase: Phase::Pending,
                owner,
                events,
            },
        );
        Some(Arc::new(Ticket {
            origins: self.clone(),
            id,
        }))
    }
}

/// A tunnel's handle on its record; dropping the last one closes it.
struct Ticket {
    origins: Origins,
    id: Uuid4,
}
impl OriginTicket for Ticket {
    fn id(&self) -> String {
        self.id.to_string()
    }
    fn establish(&self) {
        self.origins.establish(self.id);
    }
    fn close(&self) {
        self.origins.close(self.id);
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.origins.close(self.id);
    }
}
