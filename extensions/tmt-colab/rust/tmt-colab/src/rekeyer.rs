//! The serve's background re-seal of document attachments after an epoch advance (#2293). One
//! worker waits for a reason to look (serve start, a management change, a freshly established
//! object channel, or the retry of an unfinished pass), then runs one bounded rekey pass over the
//! pages whose epoch has advanced. What is left to do is read from each page, never remembered,
//! so a stopped serve resumes on its next start and a failure never blocks the advance.
use crate::{
    Result,
    attachments::slots::StagingSlots,
    object_channel::ChannelOwner,
    page::save::SourceOpener,
    registration::{self, OwnerAdmission, Registration},
    socket::ServePublish,
    sync::Server,
};
use std::{
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How often the worker looks for a reason to run.
const TICK: Duration = Duration::from_secs(1);
/// When an unfinished pass first runs again, and the longest it ever waits: a pass that keeps
/// failing backs off instead of materializing every advanced page once a minute forever.
const RETRY: Duration = Duration::from_secs(60);
const RETRY_CAP: Duration = Duration::from_secs(3600);
/// The most one page's pass may take.
const PAGE_BUDGET: Duration = Duration::from_secs(120);

/// The wait before the next retry of an unfinished pass: it doubles up to [`RETRY_CAP`] and starts
/// over when something new happens (a nudge, a new channel, or a pass that finished).
struct Backoff(Duration);
impl Backoff {
    fn new() -> Self {
        Self(RETRY)
    }
    fn next(&mut self) -> Duration {
        let wait = self.0;
        self.0 = (self.0 * 2).min(RETRY_CAP);
        wait
    }
    fn reset(&mut self) {
        self.0 = RETRY;
    }
}

struct Shared {
    due: Mutex<bool>,
    wake: Condvar,
    stop: AtomicBool,
}
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
/// Ask the worker to look again; cheap enough to call after any management change.
#[derive(Clone)]
pub struct Nudge(Arc<Shared>);
impl Nudge {
    pub fn now(&self) {
        *locked(&self.0.due) = true;
        self.0.wake.notify_all();
    }
}
pub struct Rekeyer {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl Rekeyer {
    pub fn start(
        objects: Arc<ChannelOwner>,
        slots: Arc<StagingSlots>,
        registration: Arc<Mutex<Registration>>,
        sync: Server<OwnerAdmission>,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            due: Mutex::new(true),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("colab-rekey".into())
            .spawn(move || work(&worker, &objects, &slots, &registration, &sync))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }
    pub fn nudge(&self) -> Nudge {
        Nudge(Arc::clone(&self.shared))
    }
    /// End the worker. The caller closes the object channel first, so a pass blocked on the
    /// backend ends quickly.
    pub fn stop(mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn work(
    shared: &Shared,
    objects: &ChannelOwner,
    slots: &StagingSlots,
    registration: &Mutex<Registration>,
    sync: &Server<OwnerAdmission>,
) {
    let mut seen = objects.generation();
    let mut due = true;
    let mut retry_at: Option<Instant> = None;
    let mut backoff = Backoff::new();
    while !shared.stop.load(Ordering::Acquire) {
        {
            let mut flag = locked(&shared.due);
            if !*flag {
                flag = shared
                    .wake
                    .wait_timeout(flag, TICK)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0;
            }
            if std::mem::take(&mut *flag) {
                due = true;
                backoff.reset();
            }
        }
        let generation = objects.generation();
        if generation != seen {
            seen = generation;
            due = true;
            backoff.reset();
        }
        due |= retry_at.is_some_and(|at| Instant::now() >= at);
        // Without a channel there is nothing to upload through: stay due until one is established.
        if !due || shared.stop.load(Ordering::Acquire) || objects.client().is_none() {
            continue;
        }
        due = false;
        retry_at = None;
        let source = locked(registration).save_source();
        let unfinished = match source {
            Some(source) => pass(shared, objects, slots, &source, sync).unwrap_or(true),
            None => true,
        };
        if unfinished {
            retry_at = Some(Instant::now() + backoff.next());
        } else {
            backoff.reset();
        }
    }
}
/// One round over every page whose epoch has advanced; whether anything is still waiting.
fn pass(
    shared: &Shared,
    objects: &ChannelOwner,
    slots: &StagingSlots,
    source: &SourceOpener,
    sync: &Server<OwnerAdmission>,
) -> Result<bool> {
    objects.sweep_slots(slots, source, registration::now_ms()?);
    let view = source()?;
    let advanced =
        view.store
            .owner_read(&view.keyring.space_id, &view.keyring.owner_public(), |tx| {
                let mut pages = Vec::new();
                for page in tx.pages()? {
                    if tx.current_epoch(&page)? > 1 {
                        pages.push(page);
                    }
                }
                Ok(pages)
            })?;
    drop(view);
    let publisher = ServePublish(sync);
    let mut unfinished = false;
    for page in advanced {
        if shared.stop.load(Ordering::Acquire) {
            return Ok(true);
        }
        unfinished |= objects
            .rekey_page(&page, slots, source, &publisher, PAGE_BUDGET)
            .map_or(true, |pass| pass.waiting > 0);
    }
    Ok(unfinished)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_pass_backs_off_to_the_cap_and_starts_over_when_something_new_happens() {
        let mut backoff = Backoff::new();
        let waits: Vec<u64> = (0..9).map(|_| backoff.next().as_secs()).collect();
        assert_eq!(waits, [60, 120, 240, 480, 960, 1920, 3600, 3600, 3600]);
        backoff.reset();
        assert_eq!(backoff.next(), RETRY);
    }
}
