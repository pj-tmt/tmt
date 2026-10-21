//! The owner list's display-only page summary: how many comments a page holds, how many
//! items await the owner, and who created it. The values come from the owner fold, which is
//! too slow for a list read, so a serve-process cache holds them: a read replies with what is
//! fresh and schedules a background drain of every page that is not, newest first (contract:
//! Page summary).
//! Nothing here is signed, stored or part of the owner log; a restart refills the cache lazily.
use crate::{
    Result, decoder, discussion, export::conversations::Conversations, page, page::save::Source,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Stale pages the drain takes before it reads the catalog again, so a page that changed meanwhile
/// is served next; the worker folds them one at a time.
pub const REFRESH_PAGES: usize = 8;
/// The browser's answer to a question is an ordinary comment that starts this way; it is not a
/// comment about the page, so the count leaves it out.
const ANSWER_PREFIX: &str = "Answer to \u{201c}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// Non-deleted comments in the page's annotation threads, open and resolved, agent replies
    /// included; Chat messages, status actions, proposal decisions and question answers excluded.
    pub comments: u32,
    /// At least one proposal, reading, answer block or review needs the reader.
    pub waiting: bool,
    /// Pending proposals, readings, answer groups and reviews, deduplicated by target ID.
    pub needs_you: u32,
    /// The page's creation label (`meta.originalAuthor`), when it has one.
    pub author: Option<String>,
    /// The page's own work, before any page it follows is resolved; `None` without work. It stays
    /// out of `json()`: the list read resolves it across pages (`work_links::Graph`).
    pub work: Option<Box<crate::work_index::PageWork>>,
}
impl Summary {
    pub fn json(&self) -> Value {
        let mut summary =
            json!({"comments": self.comments, "waiting": self.waiting, "needsYou": self.needs_you});
        if let Some(author) = &self.author {
            summary["author"] = json!(author);
        }
        summary
    }
}

/// Comment counting, pure over projected conversations; pending state has one owner in
/// `summary_pending`. An annotation thread is
/// one that is not deleted, carries no proposal and is anchored; the Chat thread has no anchor.
fn count(conversations: &Conversations, author: Option<&str>, needs_you: u32) -> Summary {
    let mut comments = 0u32;
    for thread in conversations
        .threads
        .iter()
        .filter(|thread| !thread.deleted)
    {
        if thread.proposal.is_none() && thread.anchor.is_some() {
            let own = thread
                .comments
                .iter()
                .filter(|comment| !comment.deleted && !comment.body.starts_with(ANSWER_PREFIX))
                .count();
            comments = comments.saturating_add(u32::try_from(own).unwrap_or(u32::MAX));
        }
    }
    Summary {
        comments,
        needs_you,
        waiting: needs_you > 0,
        author: author
            .filter(|label| decoder::valid_publisher_agent(label))
            .map(str::to_owned),
        work: None,
    }
}

/// The count, reading descriptors and passages are one projection and share one cache fingerprint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryProjection {
    pub summary: Summary,
    pub readings: Vec<crate::summary_readings::ReadingDescriptor>,
    /// The pending items by rank, at most [`crate::limits::NEEDS_PASSAGES`].
    pub needs: Vec<crate::summary_passages::Need>,
    /// The page's newest comment, when it has one.
    pub latest: Option<crate::summary_passages::Latest>,
}

#[cfg(test)]
impl From<Summary> for SummaryProjection {
    fn from(summary: Summary) -> Self {
        Self {
            summary,
            readings: Vec::new(),
            needs: Vec::new(),
            latest: None,
        }
    }
}

pub enum CachedDetails {
    Pending,
    Unavailable,
    Ready(SummaryProjection),
}

/// Numeric callers use the same complete fold; there is no second reading-state owner.
pub fn summarize(source: &mut Source, page: &str) -> Result<Summary> {
    summarize_details(source, page).map(|projection| projection.summary)
}

/// One page through the owner fold: the same snapshot and materialize path `threads` reads.
pub fn summarize_details(source: &mut Source, page: &str) -> Result<SummaryProjection> {
    let snapshot = page::snapshot(&source.store, &source.keyring, page, false)?;
    let folded = snapshot.materialize(&source.keyring, page, &mut source.decoder)?;
    let conversations = discussion::project(&source.keyring, page, &snapshot, &folded);
    let (report, anchors) = crate::page_check::summary(&folded.source, &conversations.threads)
        .map_err(std::io::Error::other)?;
    let pending =
        crate::summary_pending::items(&report, &anchors, &conversations, &folded.status_writers);
    if pending.len() > crate::limits::NEEDS_YOU_ITEMS {
        return Err(std::io::Error::other("summary item budget").into());
    }
    let readings = crate::summary_readings::descriptors(&report, &anchors, &pending);
    if readings.len() > crate::limits::READING_PAGE {
        return Err(std::io::Error::other("summary reading budget").into());
    }
    let mut summary = count(
        &conversations,
        folded.original_author.as_deref(),
        u32::try_from(pending.len())?,
    );
    summary.work = crate::work_index::page_work(page, &folded.meta).map(Box::new);
    Ok(SummaryProjection {
        summary,
        readings,
        needs: crate::summary_passages::needs(&report, &conversations, &pending),
        latest: crate::summary_passages::latest(&conversations),
    })
}

#[derive(Default)]
struct CachedSummary {
    fingerprint: String,
    /// `None` records that the fold failed at this fingerprint: the page is retried on a later
    /// read, after every page that has not failed.
    projection: Option<SummaryProjection>,
}
/// Fresh summaries by page, each valid for the fingerprint it was computed at. The fingerprint is
/// the page's epoch, the membership head and its receipt and checkpoint counts, read in the same
/// store transaction as the list, so any accepted write or membership change makes it stale.
#[derive(Default)]
pub struct SummaryCache {
    entries: Mutex<HashMap<String, CachedSummary>>,
    refreshing: AtomicBool,
}
impl SummaryCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CachedSummary>> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    pub fn get(&self, page: &str, fingerprint: &str) -> Option<Summary> {
        self.lock()
            .get(page)
            .filter(|entry| entry.fingerprint == fingerprint)
            .and_then(|entry| {
                entry
                    .projection
                    .as_ref()
                    .map(|projection| projection.summary.clone())
            })
    }
    /// Fresh failure differs from a cache miss. Neither becomes an empty complete list.
    pub fn details(&self, page: &str, fingerprint: &str) -> CachedDetails {
        let entries = self.lock();
        match entries
            .get(page)
            .filter(|entry| entry.fingerprint == fingerprint)
        {
            None if !entries.contains_key(page) && entries.len() >= crate::limits::PAGES => {
                CachedDetails::Unavailable
            }
            None => CachedDetails::Pending,
            Some(entry) => match &entry.projection {
                None => CachedDetails::Unavailable,
                Some(projection) => CachedDetails::Ready(projection.clone()),
            },
        }
    }
    pub fn put(&self, page: &str, fingerprint: &str, projection: Option<SummaryProjection>) {
        let mut entries = self.lock();
        // Direct details reads need not list the catalog, which normally prunes deleted pages.
        if !entries.contains_key(page) && entries.len() >= crate::limits::PAGES {
            return;
        }
        entries.insert(
            page.to_owned(),
            CachedSummary {
                fingerprint: fingerprint.to_owned(),
                projection,
            },
        );
    }
    /// At most `limit` of `pages` (page, fingerprint) without a fresh summary, in the order given:
    /// those that have not failed at their current fingerprint first, then the failed ones. Pages
    /// that are not in `pages` any more are dropped from the cache.
    pub fn stale(&self, pages: &[(String, String)], limit: usize) -> Vec<(String, String)> {
        let mut entries = self.lock();
        let known: HashSet<&str> = pages.iter().map(|(page, _)| page.as_str()).collect();
        entries.retain(|page, _| known.contains(page.as_str()));
        let mut fresh_failures = Vec::new();
        let mut stale = Vec::new();
        for (page, fingerprint) in pages {
            match entries
                .get(page)
                .filter(|entry| entry.fingerprint == *fingerprint)
            {
                Some(CachedSummary {
                    projection: Some(_),
                    ..
                }) => {}
                Some(_) => fresh_failures.push((page.clone(), fingerprint.clone())),
                None => stale.push((page.clone(), fingerprint.clone())),
            }
        }
        stale.extend(fresh_failures);
        stale.truncate(limit);
        stale
    }
    /// Claims the single refresh worker; `None` while another is running, so reads never stack.
    fn claim(self: &Arc<Self>) -> Option<Worker> {
        self.refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Worker(Arc::clone(self)))
    }
}
struct Worker(Arc<SummaryCache>);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.refreshing.store(false, Ordering::Release);
    }
}

/// Recomputes `pages` one at a time with `summarize`. A failure leaves the page without a fresh
/// entry (marked failed at that fingerprint); there is no retry inside the run.
pub fn run(
    cache: &SummaryCache,
    pages: &[(String, String)],
    mut summarize: impl FnMut(&str, &str) -> Result<SummaryProjection>,
) {
    for (page, fingerprint) in pages {
        cache.put(page, fingerprint, summarize(page, fingerprint).ok());
    }
}

/// Counts and membership revisions are monotonic. Matching before and after the fold
/// proves it belongs to the scheduled fingerprint, even when the worker started late.
fn require_fingerprint(source: &Source, page: &str, expected: &str) -> Result<()> {
    let current = source.store.owner_read(
        &source.keyring.space_id,
        &source.keyring.owner_public(),
        |tx| tx.page_fingerprint(page),
    )?;
    if current.as_deref() != Some(expected) {
        return Err(std::io::Error::other("summary fingerprint changed").into());
    }
    Ok(())
}

/// Schedules `pages` for refresh on one background worker with its own read-only source, unless a
/// worker is already running. Never blocks the caller.
pub fn refresh(
    cache: &Arc<SummaryCache>,
    open: &page::save::SourceOpener,
    pages: Vec<(String, String)>,
) {
    if pages.is_empty() {
        return;
    }
    let Some(worker) = cache.claim() else { return };
    let (cache, open) = (Arc::clone(cache), Arc::clone(open));
    let spawned = std::thread::Builder::new()
        .name("colab-summary".into())
        .spawn(move || {
            let _worker = worker;
            let Ok(mut source) = open() else {
                // No source: mark every page failed at this fingerprint so the next read orders it last.
                for (page, fingerprint) in &pages {
                    cache.put(page, fingerprint, None);
                }
                return;
            };
            run(&cache, &pages, |page, fingerprint| {
                require_fingerprint(&source, page, fingerprint)?;
                let projection = summarize_details(&mut source, page)?;
                require_fingerprint(&source, page, fingerprint)?;
                Ok(projection)
            });
        });
    // A thread that could not start drops its closure, and with it the worker claim.
    drop(spawned);
}

/// The catalog's pages with their fingerprints, the most recently updated first (then by page ID),
/// so the pages a person is working on fill the list before old ones do. A page without a
/// fingerprint is left out.
pub fn recent_first(
    catalog: &Value,
    fingerprints: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut rows: Vec<(u64, &str)> = catalog["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|page| {
            Some((
                page["lastUpdateAtMs"].as_u64().unwrap_or(0),
                page["pageId"].as_str()?,
            ))
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    rows.into_iter()
        .filter_map(|(_, id)| Some((id.to_owned(), fingerprints.get(id)?.clone())))
        .collect()
}

/// Folds every stale page, `REFRESH_PAGES` at a time, until none is left that this drain has not
/// tried. `order` reads the catalog again before each pass (newest first), so a page that changed
/// during the drain is served next; a failure leaves a page without a fresh entry, and the drain
/// never tries one page twice, so it always ends.
fn drain_with(
    cache: &SummaryCache,
    mut order: impl FnMut() -> Result<Vec<(String, String)>>,
    mut summarize: impl FnMut(&str, &str) -> Result<SummaryProjection>,
) {
    let mut tried: HashSet<(String, String)> = HashSet::new();
    loop {
        let Ok(pages) = order() else { return };
        let batch: Vec<_> = cache
            .stale(&pages, usize::MAX)
            .into_iter()
            .filter(|page| !tried.contains(page))
            .take(REFRESH_PAGES)
            .collect();
        if batch.is_empty() {
            return;
        }
        tried.extend(batch.iter().cloned());
        run(cache, &batch, &mut summarize);
    }
}

/// Schedules a drain of every stale page on one background worker with its own read-only source,
/// unless a worker is already running. Never blocks the caller.
pub fn drain(cache: &Arc<SummaryCache>, open: &page::save::SourceOpener) {
    let Some(worker) = cache.claim() else { return };
    let (cache, open) = (Arc::clone(cache), Arc::clone(open));
    let spawned = std::thread::Builder::new()
        .name("colab-summary".into())
        .spawn(move || {
            let _worker = worker;
            let Ok(mut source) = open() else { return };
            let space = source.keyring.space_id.clone();
            let owner = source.keyring.owner_public();
            let order = |source: &Source| -> Result<Vec<(String, String)>> {
                let (catalog, fingerprints) = source.store.owner_read(&space, &owner, |tx| {
                    Ok((tx.page_list()?, tx.page_fingerprints()?))
                })?;
                Ok(recent_first(&catalog, &fingerprints))
            };
            // The fold needs the source mutably and the order reads it, so each pass takes turns.
            let source = std::cell::RefCell::new(&mut source);
            drain_with(
                &cache,
                || order(&source.borrow()),
                |page, fingerprint| {
                    let mut source = source.borrow_mut();
                    require_fingerprint(&source, page, fingerprint)?;
                    let projection = summarize_details(&mut source, page)?;
                    require_fingerprint(&source, page, fingerprint)?;
                    Ok(projection)
                },
            );
        });
    // A thread that could not start drops its closure, and with it the worker claim.
    drop(spawned);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::conversations::{Anchor, Comment, Conversations, Head, Thread};

    fn count(view: &Conversations, author: Option<&str>) -> Summary {
        let (report, anchors) = crate::page_check::summary("", &view.threads).unwrap();
        let pending = crate::summary_pending::items(&report, &anchors, view, &Default::default());
        super::count(view, author, pending.len() as u32)
    }

    fn comment(id: &str, deleted: bool, body: &str) -> Comment {
        Comment {
            writer: "w".into(),
            id: id.into(),
            revision: "1".into(),
            sequence: "1".into(),
            deleted,
            body: body.into(),
            device_name: "d".into(),
            at: "1".into(),
            attachments: Vec::new(),
        }
    }
    fn thread(id: &str, anchored: bool, comments: Vec<Comment>) -> Thread {
        Thread {
            writer: "w".into(),
            id: id.into(),
            revision: "1".into(),
            anchor: anchored.then(|| Anchor {
                exact: "quote".into(),
                prefix: String::new(),
                suffix: String::new(),
            }),
            resolved: false,
            deleted: false,
            device_name: "d".into(),
            at: "1".into(),
            comments,
            proposal: None,
            decision: None,
            status: None,
            notifications: Vec::new(),
        }
    }
    fn proposal() -> crate::threads::Proposal {
        crate::threads::Proposal {
            proposal_id: "p".into(),
            title: "t".into(),
            body: "b".into(),
            proposer: crate::threads::Proposer {
                machine_id: "m".into(),
                agent_id: "a".into(),
                label: "l".into(),
            },
        }
    }
    fn conversations(threads: Vec<Thread>) -> Conversations {
        Conversations {
            format: "test",
            version: 1,
            space_id: "s".into(),
            page_id: "p".into(),
            title: "t".into(),
            epoch: "1".into(),
            membership_head: Head {
                revision: "1".into(),
                statement_hash: String::new(),
            },
            threads,
            asks: Vec::new(),
        }
    }

    #[test]
    fn comments_count_live_annotation_threads_open_or_resolved_and_nothing_else() {
        let mut resolved = thread("resolved", true, vec![comment("c3", false, "closed")]);
        resolved.resolved = true;
        let mut deleted = thread("deleted", true, vec![comment("c4", false, "gone")]);
        deleted.deleted = true;
        let mut proposed = thread(
            "proposal",
            true,
            vec![comment("c5", false, "on a proposal")],
        );
        proposed.proposal = Some(proposal());
        let view = conversations(vec![
            // Two live comments, one deleted, one question answer.
            thread(
                "open",
                true,
                vec![
                    comment("c1", false, "a"),
                    comment("c2", false, "reply from an agent"),
                    comment("c9", true, "deleted"),
                    comment("c8", false, "Answer to \u{201c}Which?\u{201d}: this one"),
                ],
            ),
            resolved,
            deleted,
            proposed,
            // The Chat thread has no anchor.
            thread("chat", false, vec![comment("c6", false, "hello")]),
        ]);
        assert_eq!(count(&view, None).comments, 3);
    }

    #[test]
    fn a_page_waits_only_for_a_live_unresolved_undecided_proposal() {
        let pending = |edit: &dyn Fn(&mut Thread)| {
            let mut t = thread("p", false, Vec::new());
            t.proposal = Some(proposal());
            edit(&mut t);
            count(&conversations(vec![t]), None).waiting
        };
        assert!(pending(&|_| {}));
        assert!(!pending(&|t| t.resolved = true));
        assert!(!pending(&|t| t.deleted = true));
        assert!(!count(&conversations(vec![thread("q", true, Vec::new())]), None).waiting);
        assert!(!count(&conversations(Vec::new()), None).waiting);
    }

    #[test]
    fn the_author_is_the_creation_label_when_it_is_valid() {
        let view = conversations(Vec::new());
        assert_eq!(
            count(&view, Some("Studio agent")).author.as_deref(),
            Some("Studio agent")
        );
        assert_eq!(count(&view, None).author, None);
        assert_eq!(count(&view, Some("")).author, None);
        assert_eq!(count(&view, Some("bad\nlabel")).author, None);
        let json = count(&view, None).json();
        assert!(json.get("author").is_none());
        assert_eq!(json["comments"], 0);
        assert_eq!(json["waiting"], false);
        assert_eq!(json["needsYou"], 0);
    }

    fn fingerprints(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(page, fp)| ((*page).to_owned(), (*fp).to_owned()))
            .collect()
    }
    fn summary(comments: u32) -> Summary {
        Summary {
            comments,
            waiting: false,
            needs_you: 0,
            author: None,
            work: None,
        }
    }

    #[test]
    fn a_summary_is_fresh_only_at_the_fingerprint_it_was_computed_at() {
        let cache = SummaryCache::default();
        cache.put("a", "1:5", Some(summary(2).into()));
        assert_eq!(cache.get("a", "1:5"), Some(summary(2)));
        assert!(matches!(cache.details("a", "1:5"), CachedDetails::Ready(_)));
        // A new receipt changes the fingerprint: the old entry is stale, not served.
        assert_eq!(cache.get("a", "1:6"), None);
        assert!(matches!(cache.details("a", "1:6"), CachedDetails::Pending));
        assert_eq!(
            cache.stale(&fingerprints(&[("a", "1:6")]), 8),
            vec![("a".into(), "1:6".into())]
        );
        assert!(cache.stale(&fingerprints(&[("a", "1:5")]), 8).is_empty());
    }

    #[test]
    fn direct_details_reads_cannot_grow_the_cache_past_the_page_limit() {
        let cache = SummaryCache::default();
        for page in 0..crate::limits::PAGES {
            cache.put(&page.to_string(), "1", None);
        }
        cache.put("new", "1", Some(summary(7).into()));
        assert_eq!(cache.lock().len(), crate::limits::PAGES);
        assert_eq!(cache.get("new", "1"), None);
        assert!(matches!(
            cache.details("new", "1"),
            CachedDetails::Unavailable
        ));

        // Existing entries may be refreshed even while the cache is full.
        cache.put("0", "2", Some(summary(3).into()));
        assert_eq!(cache.get("0", "2"), Some(summary(3)));
        assert!(matches!(cache.details("0", "3"), CachedDetails::Pending));

        // A catalog refresh removes deleted pages and admits the new page again.
        assert_eq!(
            cache.stale(&fingerprints(&[("new", "1")]), 8),
            vec![("new".into(), "1".into())]
        );
        cache.put("new", "1", Some(summary(7).into()));
        assert_eq!(cache.lock().len(), 1);
        assert_eq!(cache.get("new", "1"), Some(summary(7)));
    }

    #[test]
    fn a_refresh_takes_at_most_the_limit_and_forgets_deleted_pages() {
        let cache = SummaryCache::default();
        cache.put("gone", "1", Some(summary(1).into()));
        let map: Vec<_> = (b'a'..=b'k')
            .map(|c| ((c as char).to_string(), "1".to_owned()))
            .collect();
        let stale = cache.stale(&map, REFRESH_PAGES);
        assert_eq!(stale.len(), REFRESH_PAGES);
        assert_eq!(stale.first().map(|(page, _)| page.as_str()), Some("a"));
        assert_eq!(stale.last().map(|(page, _)| page.as_str()), Some("h"));
        // A page that no longer exists leaves the cache.
        assert!(!cache.lock().contains_key("gone"));
    }

    #[test]
    fn a_page_that_failed_at_its_fingerprint_waits_behind_every_page_that_has_not() {
        let cache = SummaryCache::default();
        cache.put("a", "1", None);
        let map = fingerprints(&[("a", "1"), ("b", "1"), ("c", "1")]);
        let order: Vec<String> = cache
            .stale(&map, 8)
            .into_iter()
            .map(|(page, _)| page)
            .collect();
        assert_eq!(order, ["b", "c", "a"]);
        // A new fingerprint is a new attempt, so the page is no longer ordered as a failure.
        let moved = fingerprints(&[("a", "2"), ("b", "1"), ("c", "1")]);
        let order: Vec<String> = cache
            .stale(&moved, 8)
            .into_iter()
            .map(|(page, _)| page)
            .collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    fn a_failed_fold_leaves_the_page_absent_and_does_not_loop() {
        let cache = SummaryCache::default();
        let pages = vec![
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), "1".to_owned()),
        ];
        let mut calls = Vec::new();
        run(&cache, &pages, |page, _| {
            calls.push(page.to_owned());
            if page == "a" {
                Err(crate::page::Fault::Unavailable.into())
            } else {
                Ok(summary(4).into())
            }
        });
        // One attempt each, in order, and no retry within the run.
        assert_eq!(calls, ["a", "b"]);
        assert_eq!(cache.get("a", "1"), None);
        assert!(matches!(
            cache.details("a", "1"),
            CachedDetails::Unavailable
        ));
        assert_eq!(cache.get("b", "1"), Some(summary(4)));
        // The failed page is still stale (retried on a later read), and ordered last.
        assert_eq!(
            cache.stale(&fingerprints(&[("a", "1"), ("b", "1")]), 8),
            vec![("a".to_owned(), "1".to_owned())]
        );
    }

    fn fp(pages: &[&str]) -> Vec<(String, String)> {
        pages
            .iter()
            .map(|page| ((*page).to_owned(), "1".to_owned()))
            .collect()
    }

    #[test]
    fn the_catalog_is_served_newest_first_and_skips_a_page_without_a_fingerprint() {
        let catalog = json!({"pages":[
            {"pageId":"a","lastUpdateAtMs":10},
            {"pageId":"b","lastUpdateAtMs":30},
            {"pageId":"c","lastUpdateAtMs":30},
            {"pageId":"d"},
            {"pageId":"e","lastUpdateAtMs":50}
        ]});
        let fingerprints: BTreeMap<String, String> = ["a", "b", "c", "d"]
            .iter()
            .map(|page| ((*page).to_owned(), format!("fp-{page}")))
            .collect();
        let order: Vec<_> = recent_first(&catalog, &fingerprints)
            .into_iter()
            .map(|(page, fingerprint)| format!("{page}:{fingerprint}"))
            .collect();
        // Newest first, ties by page ID, a page with no update time last; `e` has no fingerprint.
        assert_eq!(order, ["b:fp-b", "c:fp-c", "a:fp-a", "d:fp-d"]);
        assert!(recent_first(&json!({}), &BTreeMap::new()).is_empty());
    }

    #[test]
    fn a_drain_folds_every_stale_page_newest_first_in_passes_of_the_refresh_size() {
        let cache = SummaryCache::default();
        let names: Vec<String> = (0..20).map(|n| format!("p{n:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let order = fp(&refs);
        let mut calls = Vec::new();
        let mut reads = 0;
        drain_with(
            &cache,
            || {
                reads += 1;
                Ok(order.clone())
            },
            |page, _| {
                calls.push(page.to_owned());
                Ok(summary(1).into())
            },
        );
        assert_eq!(calls, names, "in the order given, none twice");
        // 20 pages in passes of 8 are three folds' worth of reads, plus the one that finds none.
        assert_eq!(reads, 4);
        assert!(cache.stale(&order, usize::MAX).is_empty());
    }

    #[test]
    fn a_page_that_changes_during_a_drain_is_served_before_the_older_pages_left() {
        let cache = SummaryCache::default();
        let names: Vec<String> = (0..12).map(|n| format!("p{n:02}")).collect();
        let mut passes = 0;
        let mut calls = Vec::new();
        drain_with(
            &cache,
            || {
                passes += 1;
                let mut order: Vec<(String, String)> =
                    names.iter().map(|n| (n.clone(), "1".to_owned())).collect();
                // After the first pass the last (oldest) page is edited: a new fingerprint, and now
                // the newest, so the next pass starts with it.
                if passes > 1 {
                    order.retain(|(page, _)| page != "p11");
                    order.insert(0, ("p11".to_owned(), "2".to_owned()));
                }
                Ok(order)
            },
            |page, fingerprint| {
                calls.push(format!("{page}:{fingerprint}"));
                Ok(summary(1).into())
            },
        );
        // The oldest page changed after the first pass, so it is folded at its new fingerprint first
        // in the second, ahead of the pages left, and never at the old one.
        let mut expected: Vec<String> = (0..REFRESH_PAGES).map(|n| format!("p{n:02}:1")).collect();
        expected.extend(["p11:2", "p08:1", "p09:1", "p10:1"].map(str::to_owned));
        assert_eq!(calls, expected);
        assert!(cache.get("p11", "2").is_some());
    }

    #[test]
    fn a_drain_tries_a_failing_page_once_and_ends() {
        let cache = SummaryCache::default();
        let order = fp(&["a", "b", "c"]);
        let mut calls = Vec::new();
        drain_with(
            &cache,
            || Ok(order.clone()),
            |page, _| {
                calls.push(page.to_owned());
                if page == "b" {
                    Err(crate::page::Fault::Unavailable.into())
                } else {
                    Ok(summary(1).into())
                }
            },
        );
        assert_eq!(calls, ["a", "b", "c"]);
        // The failed page is still stale, ordered last, for a later read to retry.
        assert_eq!(
            cache.stale(&order, 8),
            vec![("b".to_owned(), "1".to_owned())]
        );
    }

    #[test]
    fn a_drain_ends_when_the_cache_refuses_a_page_and_when_the_catalog_cannot_be_read() {
        let cache = SummaryCache::default();
        let existing: Vec<String> = (0..crate::limits::PAGES).map(|n| n.to_string()).collect();
        for page in &existing {
            cache.put(page, "1", Some(summary(1).into()));
        }
        let mut order: Vec<(String, String)> = existing
            .iter()
            .map(|page| (page.clone(), "1".to_owned()))
            .collect();
        order.push(("new".to_owned(), "1".to_owned()));
        let mut calls = 0;
        drain_with(
            &cache,
            || Ok(order.clone()),
            |_, _| {
                calls += 1;
                Ok(summary(1).into())
            },
        );
        // The full cache drops the page, so it stays stale; the drain still tried it only once.
        assert_eq!(calls, 1);
        assert_eq!(cache.get("new", "1"), None);

        let mut folded = 0;
        drain_with(
            &SummaryCache::default(),
            || Err(crate::page::Fault::Unavailable.into()),
            |_, _| {
                folded += 1;
                Ok(summary(1).into())
            },
        );
        assert_eq!(folded, 0);
    }

    #[test]
    fn one_worker_runs_at_a_time() {
        let cache = Arc::new(SummaryCache::default());
        let first = cache.claim().expect("the first claim");
        assert!(cache.claim().is_none(), "a second worker must not start");
        drop(first);
        assert!(
            cache.claim().is_some(),
            "the claim is released when the worker ends"
        );
    }
}
