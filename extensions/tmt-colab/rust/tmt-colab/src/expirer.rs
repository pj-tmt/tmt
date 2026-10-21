//! The serve's scheduled archive (#2600): a page that has sat without activity for as many days
//! as apply to it (its own `retention.set`, else the space's page expiry setting) is archived by
//! signing the ordinary `page.archive`, so it is frozen and never deleted. [`judge`] decides which
//! pages are idle, [`plan`] applies the rules that need the page itself, and the worker signs what
//! the plan names. `tmt colab expire --dry-run` prints the same plan.
//!
//! Nothing here is a statement of its own. The worker runs only while `serve` runs, looks soon
//! after it starts and then every hour, signs at most [`BATCH`] pages a pass, and judges each page
//! again inside the signing transaction, so an edit that lands between the look and the signature
//! wins. A clock that is behind the recorded activity archives nothing. Shell pages are left alone
//! (a collection of links is not an idle document). A restart is safe: an archived page is no
//! longer idle.
use crate::{
    Result,
    cloud_sync::{ActivePage, PageRef},
    page::{self, save::SourceOpener},
    page_expiry::SpaceExpiry,
    registration::{self, OwnerAdmission, Registration},
    sync::Server,
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// The most pages one pass archives; the next pass continues.
pub const BATCH: usize = 20;
/// How soon after the serve starts the first pass looks, and how long between passes.
const FIRST_PASS: Duration = Duration::from_secs(30);
const PASS_EVERY: Duration = Duration::from_secs(3600);
/// How often the worker checks whether it was asked to stop.
const TICK: Duration = Duration::from_secs(1);

/// A page judged idle, and what it was judged on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Idle {
    pub page: String,
    /// The page's last activity, in UTC milliseconds.
    pub last_activity_ms: u64,
    /// The days without activity that applied to it.
    pub days: u64,
    /// When it became due: the last activity plus those days.
    pub due_ms: u64,
}

/// The pages of a catalog (see `inspection::catalog_for`) that are due at `now`, most overdue
/// first, and whether the clock is behind what the catalog records, which makes every judgement
/// untrustworthy and so empties the list.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Judged {
    pub idle: Vec<Idle>,
    pub clock_behind: bool,
}
pub fn judge(catalog: &Value, now: u64) -> Judged {
    let pages = catalog["pages"].as_array().map_or(&[][..], Vec::as_slice);
    if pages
        .iter()
        .any(|page| page["lastActivityAtMs"].as_u64().is_some_and(|at| at > now))
    {
        return Judged {
            idle: Vec::new(),
            clock_behind: true,
        };
    }
    let mut idle: Vec<Idle> = pages
        .iter()
        .filter(|page| page["archived"] != true)
        .filter_map(|page| {
            let due_ms = page["archiveAtMs"].as_u64().filter(|due| *due <= now)?;
            Some(Idle {
                page: page["pageId"].as_str()?.to_owned(),
                last_activity_ms: page["lastActivityAtMs"].as_u64()?,
                days: page["archiveAfterDays"].as_u64()?,
                due_ms,
            })
        })
        .collect();
    idle.sort_by(|a, b| (a.due_ms, &a.page).cmp(&(b.due_ms, &b.page)));
    Judged {
        idle,
        clock_behind: false,
    }
}

/// What the next pass would do.
#[derive(Debug, Default)]
pub struct Plan {
    /// The pages it archives, at most [`BATCH`], most overdue first.
    pub archive: Vec<Idle>,
    /// Idle pages left alone because they are shell pages.
    pub shells: usize,
    /// More idle pages than one pass archives.
    pub more: bool,
    pub clock_behind: bool,
}

/// The plan for what `judge` found. `is_shell` reads a page to tell; `shells` remembers the pages
/// already found to be shells, which never change into anything else, so a worker reads each once.
pub fn plan(
    judged: Judged,
    shells: &mut BTreeSet<String>,
    mut is_shell: impl FnMut(&str) -> Result<bool>,
) -> Result<Plan> {
    let mut plan = Plan {
        clock_behind: judged.clock_behind,
        ..Plan::default()
    };
    for idle in judged.idle {
        if shells.contains(&idle.page) || is_shell(&idle.page)? {
            shells.insert(idle.page);
            plan.shells += 1;
        } else if plan.archive.len() == BATCH {
            plan.more = true;
            break;
        } else {
            plan.archive.push(idle);
        }
    }
    Ok(plan)
}

/// What the next pass would do for the space at `root`'s store, read the way the worker reads it.
pub fn plan_for(
    source: &mut page::save::Source,
    space: SpaceExpiry,
    now: u64,
    shells: &mut BTreeSet<String>,
) -> Result<Plan> {
    let catalog = source.store.owner_read(
        &source.keyring.space_id,
        &source.keyring.owner_public(),
        |tx| tx.page_list_for_cli(space),
    )?;
    let page::save::Source {
        store,
        keyring,
        decoder,
    } = source;
    plan(judge(&catalog, now), shells, |id| {
        page::is_shell(store, keyring, id, decoder)
    })
}

struct Shared {
    stop: AtomicBool,
    active: AtomicBool,
    wake: Condvar,
    idle: Mutex<()>,
}
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The serve's one scheduled-archive worker.
pub struct Expirer {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl Expirer {
    pub fn start(
        registration: Arc<Mutex<Registration>>,
        sync: Server<OwnerAdmission>,
        source: SourceOpener,
        cloud: Option<ActivePage>,
        space: String,
        report: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            active: AtomicBool::new(false),
            wake: Condvar::new(),
            idle: Mutex::new(()),
        });
        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("colab-expire".into())
            .spawn(move || {
                work(
                    &worker,
                    &registration,
                    &sync,
                    &source,
                    cloud.as_ref(),
                    &space,
                    &report,
                )
            })?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }
    pub fn is_idle(&self) -> bool {
        !self.shared.active.load(Ordering::Acquire)
    }
    /// Stop between pages without interrupting the signature that is already in progress.
    pub fn request_stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake.notify_all();
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
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
    registration: &Mutex<Registration>,
    sync: &Server<OwnerAdmission>,
    source: &SourceOpener,
    cloud: Option<&ActivePage>,
    space: &str,
    report: &Arc<dyn Fn(&str) + Send + Sync>,
) {
    let mut next = Instant::now() + FIRST_PASS;
    let mut shells = BTreeSet::new();
    while !shared.stop.load(Ordering::Acquire) {
        {
            let guard = locked(&shared.idle);
            let _ = shared
                .wake
                .wait_timeout(guard, TICK)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        if shared.stop.load(Ordering::Acquire) || Instant::now() < next {
            continue;
        }
        next = Instant::now() + PASS_EVERY;
        shared.active.store(true, Ordering::Release);
        let _active = ActivePass(&shared.active);
        if let Err(error) = pass(
            shared,
            registration,
            sync,
            source,
            cloud,
            space,
            report,
            &mut shells,
        ) {
            report(&format!("The scheduled archive could not run: {error}"));
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn pass(
    shared: &Shared,
    registration: &Mutex<Registration>,
    sync: &Server<OwnerAdmission>,
    source: &SourceOpener,
    cloud: Option<&ActivePage>,
    space: &str,
    report: &Arc<dyn Fn(&str) + Send + Sync>,
    shells: &mut BTreeSet<String>,
) -> Result<()> {
    let Some(expiry) = locked(registration).page_expiry() else {
        report("The scheduled archive is paused: settings.json could not be read.");
        return Ok(());
    };
    let now = registration::now_ms().map_err(|_| "The clock could not be read.")?;
    let mut view = source()?;
    let plan = plan_for(&mut view, expiry, now, shells)?;
    if plan.clock_behind {
        report("The scheduled archive is paused: the clock is behind the recorded page activity.");
        return Ok(());
    }
    drop(view);
    for idle in plan.archive {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        if let Some(done) = archive_one(sync, &idle.page)? {
            report(&format!(
                "Archived page {} after {} days without activity (last activity {}).",
                done.page, done.days, done.last_activity_ms
            ));
            if let Some(cloud) = cloud {
                let epoch = locked(registration).page_epoch(&done.page);
                if let Some(epoch) = epoch {
                    cloud.touch(PageRef {
                        space: space.to_owned(),
                        page: done.page,
                        epoch,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Archive one page if it is still idle, judged in the signing transaction under the sync lock:
/// what it was judged on comes back, `None` when an update or the clock changed its mind.
fn archive_one(sync: &Server<OwnerAdmission>, page: &str) -> Result<Option<Idle>> {
    use crate::management::{Applied, Code};
    let mut archived = None;
    sync.manage(|admission| {
        let mut service = admission.0.lock().map_err(|_| Code::Unavailable)?;
        let now = registration::now_ms().map_err(|_| Code::Unavailable)?;
        archived = service.archive_if_idle(page, now)?;
        let page = archived.as_ref().map(|done| done.page.clone());
        Ok(Applied {
            body: Vec::new(),
            touched: page.iter().cloned().collect(),
            archived_page: page,
            deleted_page: None,
        })
    })
    .map_err(|code| format!("A page could not be archived: {}.", code.text()))?;
    Ok(archived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DAY: u64 = 86_400_000;
    fn page(id: &str, active: Option<u64>, days: Option<u64>, archived: bool) -> Value {
        let due = active.zip(days).map(|(at, days)| at + days * DAY);
        json!({"pageId": id, "archived": archived, "lastActivityAtMs": active,
            "archiveAfterDays": days, "archiveAtMs": if archived { None } else { due }})
    }

    #[test]
    fn the_due_pages_are_listed_most_overdue_first_and_unknown_activity_is_never_due() {
        let now = 1_000 * DAY;
        let catalog = json!({"pages": [
            page("c", Some(now - 190 * DAY), Some(180), false),
            page("a", Some(now - 300 * DAY), Some(180), false),
            page("fresh", Some(now - 10 * DAY), Some(180), false),
            page("never", Some(now - 900 * DAY), None, false),
            page("unknown", None, Some(180), false),
            page("done", Some(now - 400 * DAY), Some(180), true),
            page("b", Some(now - 190 * DAY), Some(180), false),
        ]});
        let judged = judge(&catalog, now);
        assert!(!judged.clock_behind);
        let ids: Vec<&str> = judged.idle.iter().map(|idle| idle.page.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"], "ties break by page id");
        assert_eq!(
            judged.idle[0],
            Idle {
                page: "a".into(),
                last_activity_ms: now - 300 * DAY,
                days: 180,
                due_ms: now - 120 * DAY,
            }
        );
    }

    #[test]
    fn the_plan_leaves_shells_alone_reads_each_once_and_stops_at_the_batch() {
        let now = 1_000 * DAY;
        let pages: Vec<Value> = (0..30)
            .map(|n| {
                page(
                    &format!("p{n:02}"),
                    Some(now - (400 - n) * DAY),
                    Some(180),
                    false,
                )
            })
            .collect();
        let judged = judge(&json!({ "pages": pages }), now);
        assert_eq!(judged.idle.len(), 30);
        let mut shells = BTreeSet::new();
        let mut reads = Vec::new();
        let planned = plan(judged, &mut shells, |id| {
            reads.push(id.to_owned());
            Ok(id == "p00" || id == "p03")
        })
        .unwrap();
        assert_eq!(planned.shells, 2);
        assert_eq!(planned.archive.len(), BATCH);
        assert!(planned.more && !planned.clock_behind);
        assert!(
            planned
                .archive
                .iter()
                .all(|idle| idle.page != "p00" && idle.page != "p03")
        );
        assert_eq!(planned.archive[0].page, "p01", "most overdue first");
        // The next pass reads no shell again.
        reads.clear();
        let judged = judge(&json!({ "pages": pages }), now);
        let again = plan(judged, &mut shells, |id| {
            reads.push(id.to_owned());
            Ok(false)
        })
        .unwrap();
        assert_eq!(again.shells, 2);
        assert!(!reads.contains(&"p00".to_owned()) && !reads.contains(&"p03".to_owned()));
        // A page that cannot be read fails the plan, never archives on a guess.
        let judged = judge(&json!({ "pages": pages }), now);
        assert!(plan(judged, &mut BTreeSet::new(), |_| Err("unreadable".into())).is_err());
    }

    #[test]
    fn a_page_is_archived_once_and_an_update_or_the_clock_that_changed_its_mind_wins() {
        use crate::readers::test_support::{Fixture, OTHER, PAGE};
        let fixture = Fixture::new();
        let db = rusqlite::Connection::open(fixture.root.join("colab/space.db")).unwrap();
        let set = |page: &str, ms: i64| {
            db.execute(
                "UPDATE pages SET last_activity_at_ms=?1 WHERE page=?2",
                rusqlite::params![ms, page],
            )
            .unwrap();
        };
        let now = i64::try_from(registration::now_ms().unwrap()).unwrap();
        let archived = |page: &str| {
            let catalog = crate::inspection::catalog(&fixture.store, &fixture.key).unwrap();
            catalog["pages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["pageId"] == page)
                .unwrap()["archived"]
                .clone()
        };
        set(PAGE, 1);
        set(OTHER, now);
        let done = archive_one(&fixture.server, PAGE).unwrap().unwrap();
        assert_eq!(
            (done.page.as_str(), done.last_activity_ms, done.days),
            (PAGE, 1, 180)
        );
        assert_eq!(
            (archived(PAGE), archived(OTHER)),
            (json!(true), json!(false))
        );
        // Archived is not idle: a second look, or a restart, signs nothing.
        assert!(archive_one(&fixture.server, PAGE).unwrap().is_none());
        // A page that looked idle and then saw an update is left alone.
        set(OTHER, 1);
        let idle = judge(
            &crate::inspection::catalog(&fixture.store, &fixture.key).unwrap(),
            registration::now_ms().unwrap(),
        );
        assert_eq!(idle.idle.len(), 1);
        set(OTHER, now);
        assert!(archive_one(&fixture.server, OTHER).unwrap().is_none());
        assert_eq!(archived(OTHER), json!(false));
        // A clock behind the recorded activity archives nothing.
        set(OTHER, now + 3 * i64::try_from(DAY).unwrap());
        assert!(archive_one(&fixture.server, OTHER).unwrap().is_none());
        assert_eq!(archived(OTHER), json!(false));
    }

    #[test]
    fn a_setting_that_cannot_be_read_archives_nothing() {
        use crate::{
            keyring::{Keyring, Layout},
            readers::test_support::{Fixture, PAGE, decoder_config},
            store::Store,
        };
        let fixture = Fixture::new();
        let db = rusqlite::Connection::open(fixture.root.join("colab/space.db")).unwrap();
        db.execute(
            "UPDATE pages SET last_activity_at_ms=1 WHERE page=?1",
            [PAGE],
        )
        .unwrap();
        let layout = Layout::existing(&fixture.root).unwrap().unwrap();
        let build = |reader: crate::page_expiry::Reader| {
            Registration::with_decoder_config(
                Store::open(&layout).unwrap(),
                Keyring::read(&layout).unwrap(),
                decoder_config(std::env::current_exe().unwrap()),
            )
            .unwrap()
            .with_page_expiry(reader)
        };
        let archived = || {
            crate::inspection::catalog(&fixture.store, &fixture.key).unwrap()["pages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["pageId"] == PAGE)
                .unwrap()["archived"]
                .clone()
        };
        let now = registration::now_ms().unwrap();
        // The page is long idle, but a damaged settings.json must not let the default stand in
        // for an owner who may have said `off`.
        let mut unreadable = build(Arc::new(|| None));
        assert!(unreadable.page_expiry().is_none());
        assert!(unreadable.archive_if_idle(PAGE, now).unwrap().is_none());
        assert_eq!(archived(), json!(false));
        let mut readable = build(Arc::new(|| Some(SpaceExpiry::Default)));
        assert!(readable.archive_if_idle(PAGE, now).unwrap().is_some());
        assert_eq!(archived(), json!(true));
    }

    #[test]
    fn a_clock_behind_the_recorded_activity_archives_nothing() {
        let now = 1_000 * DAY;
        let catalog = json!({"pages": [
            page("old", Some(now - 300 * DAY), Some(180), false),
            page("ahead", Some(now + DAY), Some(180), false),
        ]});
        assert_eq!(
            judge(&catalog, now),
            Judged {
                idle: Vec::new(),
                clock_behind: true
            }
        );
        assert!(!judge(&catalog, now + 2 * DAY).clock_behind);
        assert_eq!(judge(&json!({}), now), Judged::default());
    }
}
