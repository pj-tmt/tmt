//! The serve's one object-maintenance worker: re-seal document attachments after epoch
//! advances, repair their page references with a second Save, and remove namespaces behind
//! retained committed deletions. Serve start, management
//! changes, a new channel generation and bounded retries wake it. Work is derived from signed
//! page state, so reconnects recover unfinished cleanup without delaying an owner transition.
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
    active: AtomicBool,
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
            active: AtomicBool::new(false),
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
    pub fn is_idle(&self) -> bool {
        !self.shared.active.load(Ordering::Acquire)
    }
    /// Stop between pages without interrupting the pass that is already publishing.
    pub fn request_stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake.notify_all();
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
    /// End the worker. The caller closes the object channel first, so a pass blocked on the
    /// backend ends quickly.
    pub fn stop(mut self) {
        self.request_stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct ActivePass<'a>(&'a AtomicBool);
impl Drop for ActivePass<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
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
    let mut deletion_after = None;
    let mut history_after = None;
    let mut history_at = Instant::now();
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
            deletion_after = None;
            due = true;
            backoff.reset();
        }
        due |= retry_at.is_some_and(|at| Instant::now() >= at);
        // Local history maintenance needs no object channel. Reuse this worker's bounded wake
        // and visit one eligible page per minute, rotating so a failing page cannot starve others.
        if Instant::now() >= history_at && !shared.stop.load(Ordering::Acquire) {
            history_at = Instant::now() + RETRY;
            shared.active.store(true, Ordering::Release);
            let active = ActivePass(&shared.active);
            let source = locked(registration).save_source();
            if let Some(source) = source {
                let _ = history_pass(
                    &source,
                    sync,
                    &mut history_after,
                    crate::page::compact::Trigger::default(),
                );
            }
            drop(active);
        }
        // Without a channel there is nothing to upload through: stay due until one is established.
        if !due || shared.stop.load(Ordering::Acquire) || objects.client().is_none() {
            continue;
        }
        due = false;
        retry_at = None;
        shared.active.store(true, Ordering::Release);
        let active = ActivePass(&shared.active);
        let source = locked(registration).save_source();
        let unfinished = match source {
            Some(source) => {
                pass(shared, objects, slots, &source, sync, &mut deletion_after).unwrap_or(true)
            }
            None => true,
        };
        drop(active);
        if unfinished {
            retry_at = Some(Instant::now() + backoff.next());
        } else {
            backoff.reset();
        }
    }
}

fn history_pass(
    source: &SourceOpener,
    sync: &Server<OwnerAdmission>,
    after: &mut Option<String>,
    trigger: crate::page::compact::Trigger,
) -> Result<Option<crate::page::compact::Report>> {
    let view = source()?;
    let pages =
        view.store
            .owner_read(&view.keyring.space_id, &view.keyring.owner_public(), |tx| {
                let revision = tx.head().ok_or(crate::page::Fault::Invalid)?.revision;
                let mut eligible = Vec::new();
                for page in tx.pages()? {
                    // A malformed page policy cannot authorize work or starve independent pages.
                    let Ok(policy) = tx.page_policy_at(&page, revision) else {
                        continue;
                    };
                    if policy.writable() && policy.history_current && !policy.public_mode {
                        eligible.push(page);
                    }
                }
                Ok(eligible)
            })?;
    let Some(page) = pages
        .iter()
        .find(|page| after.as_ref().is_none_or(|last| *page > last))
        .or_else(|| pages.first())
        .cloned()
    else {
        *after = None;
        return Ok(None);
    };
    *after = Some(page.clone());
    let due = crate::page::compact::history_due(&view.store, &view.keyring, &page, trigger)?;
    drop(view);
    if !due {
        return Ok(None);
    }
    // Reuse the serving writer's lock, equality checks, publication fence and notifications.
    Ok(Some(sync.compact_page(&page, Instant::now())?))
}
/// One round over every page whose epoch has advanced; whether anything is still waiting.
fn pass(
    shared: &Shared,
    objects: &ChannelOwner,
    slots: &StagingSlots,
    source: &SourceOpener,
    sync: &Server<OwnerAdmission>,
    deletion_after: &mut Option<String>,
) -> Result<bool> {
    // A pass may follow an epoch transition committed outside this process.
    sync.update_admission(|_| {})?;
    // Share the pass snapshot; each actual removal still rechecks fresh admission.
    let view = source()?;
    // Cleanup admission failure must not skip independent epoch work.
    let cleanup_waiting = objects
        .sweep_deletions(
            source,
            &view,
            deletion_after,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap_or(true);
    let mut advanced =
        view.store
            .owner_read(&view.keyring.space_id, &view.keyring.owner_public(), |tx| {
                let mut pages = Vec::new();
                for page in tx.pages()? {
                    if tx.current_epoch(&page)? > 1
                        && !tx
                            .page_policy_at(
                                &page,
                                tx.head().ok_or(crate::page::Fault::Invalid)?.revision,
                            )?
                            .deleted
                    {
                        pages.push(page);
                    }
                }
                Ok(pages)
            })?;
    drop(view);
    advanced.extend(slots.repair_pages()?);
    advanced.sort();
    advanced.dedup();
    let publisher = ServePublish(sync);
    let mut unfinished = cleanup_waiting;
    for page in advanced {
        if shared.stop.load(Ordering::Acquire) {
            return Ok(true);
        }
        unfinished |= objects
            .rekey_page(&page, slots, source, &publisher, PAGE_BUDGET)
            .map_or(true, |pass| pass.waiting > 0);
    }
    // Reconcile committed attachment swaps before age cleanup can remove their reference maps.
    objects.sweep_slots(slots, source, registration::now_ms()?);
    Ok(unfinished)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_history_maintenance_commits_without_an_object_channel_and_stops_cleanly() {
        use crate::{
            keyring::{Keyring, Layout},
            page::{self, PublicationPreparation},
            readers::test_support::{Fixture, PAGE, decoder_config},
            store::{Store, owner::Mutation},
        };
        let mut fixture = Fixture::new();
        let binary = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("tmt-colab");
        let config = decoder_config(binary);
        let mut decoder = crate::decoder::Decoder::with_config(config.clone()).unwrap();
        for index in 0..crate::page::compact::TRIGGER_UPDATES {
            let source = format!("<p>Retained source {index}</p>");
            let PublicationPreparation::Write(frozen) = page::prepare_publication(
                &fixture.store,
                &fixture.key,
                PAGE,
                crate::decoder::ContentEdit {
                    source: &source,
                    layout: None,
                    publisher_agent: None,
                    attachments: None,
                },
                None,
                &mut decoder,
                1000,
            )
            .unwrap() else {
                panic!("changed source must publish")
            };
            page::commit_publication(
                &mut fixture.store,
                &fixture.key,
                frozen.job(),
                frozen.packet(),
                frozen.chain(),
                1000,
            )
            .unwrap();
        }
        let head = fixture
            .store
            .owner_head(&fixture.key.space_id, &fixture.key.owner_public())
            .unwrap()
            .unwrap();
        fixture
            .store
            .owner_transaction(
                &fixture.key.space_id,
                &fixture.key.owner_public(),
                Mutation {
                    operation_id: "40000000-0000-4000-8000-000000000093",
                    digest: [93; 32],
                    expected_revision: head.revision,
                },
                |tx| {
                    tx.append_statement(&fixture.key.sign_statement(
                        tx.head(),
                        "page.history",
                        &serde_json::to_vec(
                            &serde_json::json!({"pageId": PAGE, "mode": "current"}),
                        )?,
                    )?)?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        let root = fixture.root.clone();
        let (started, observed) = std::sync::mpsc::channel();
        let source: SourceOpener = Arc::new(move || {
            let view = crate::page::save::open_source(&root, config.clone())?;
            let _ = started.send(());
            Ok(view)
        });
        let layout = Layout::existing(&fixture.root).unwrap().unwrap();
        let registration = Arc::new(Mutex::new(
            Registration::with_decoder_config(
                Store::open(&layout).unwrap(),
                Keyring::read(&layout).unwrap(),
                decoder_config(
                    std::env::current_exe()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .join("tmt-colab"),
                ),
            )
            .unwrap()
            .with_save_source(source),
        ));
        let server = Server::new(
            Store::open(&layout).unwrap(),
            OwnerAdmission(registration.clone()),
        );
        let objects = Arc::new(ChannelOwner::new(Arc::new(|| None)));
        assert!(objects.client().is_none());
        let slots = Arc::new(StagingSlots::open(&layout).unwrap());
        let worker = Rekeyer::start(objects, slots, registration, server).unwrap();
        let started = observed.recv_timeout(Duration::from_secs(10));
        worker.stop();
        assert!(started.is_ok(), "local maintenance never started");
        let after = page::read(&fixture.store, &fixture.key, PAGE, &mut decoder).unwrap();
        assert_eq!(
            after.source,
            format!(
                "<p>Retained source {}</p>",
                crate::page::compact::TRIGGER_UPDATES - 1
            )
        );
        assert_eq!(after.usage.updates, 0);
        assert!(after.usage.checkpoint_bytes > 0);
    }

    #[test]
    fn a_failing_pass_backs_off_to_the_cap_and_starts_over_when_something_new_happens() {
        let mut backoff = Backoff::new();
        let waits: Vec<u64> = (0..9).map(|_| backoff.next().as_secs()).collect();
        assert_eq!(waits, [60, 120, 240, 480, 960, 1920, 3600, 3600, 3600]);
        backoff.reset();
        assert_eq!(backoff.next(), RETRY);
    }
    #[test]
    fn failed_cleanup_policies_finish_the_sweep_and_do_not_skip_the_rekey_owner() {
        use crate::{
            keyring::Layout,
            readers::test_support::{Fixture, OTHER, PAGE, decoder_config},
            transitions::OwnerAction,
        };
        use std::sync::atomic::AtomicUsize;
        let mut fixture = Fixture::new();
        fixture.apply(OwnerAction::EpochAdvance { page: PAGE });
        // A signed statement with an inconsistent OTHER-page epoch fails only
        // that page's policy reduction. PAGE's committed epoch and rekey remain valid.
        let head = fixture
            .store
            .owner_head(&fixture.key.space_id, &fixture.key.owner_public())
            .unwrap()
            .unwrap();
        let envelope = fixture
            .key
            .sign_statement(
                Some(&head),
                "page.share",
                &serde_json::to_vec(
                    &serde_json::json!({"pageId":OTHER,"epoch":"99","mode":"private"}),
                )
                .unwrap(),
            )
            .unwrap();
        let db = rusqlite::Connection::open(fixture.root.join("colab/space.db")).unwrap();
        db.execute(
            "INSERT INTO membership_log(revision,hash,envelope) VALUES (?1,?2,?3)",
            rusqlite::params![
                format!("{:020}", head.revision + 1),
                envelope.hash().unwrap().as_slice(),
                envelope.to_json().unwrap()
            ],
        )
        .unwrap();
        let root = fixture.root.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let observed = reads.clone();
        let config = decoder_config(std::env::current_exe().unwrap());
        let source: SourceOpener = Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            crate::page::save::open_source(&root, config.clone())
        });
        let layout = Layout::existing(&fixture.root).unwrap().unwrap();
        let slots = StagingSlots::open(&layout).unwrap();
        let objects = ChannelOwner::new(Arc::new(|| None));
        let shared = Shared {
            due: Mutex::new(false),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            active: AtomicBool::new(false),
        };
        let mut after = None;
        assert!(
            pass(
                &shared,
                &objects,
                &slots,
                &source,
                &fixture.server,
                &mut after
            )
            .unwrap()
        );
        assert!(
            after.is_none(),
            "failed policies must not stop at the first page"
        );
        assert_eq!(
            reads.load(Ordering::SeqCst),
            2,
            "the rekey owner still opens its fresh read"
        );
    }
}
