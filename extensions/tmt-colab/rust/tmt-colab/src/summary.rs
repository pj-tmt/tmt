//! The owner list's display-only page summary: how many comments a page holds, whether a
//! proposal awaits the owner, and who created it. The values come from the owner fold, which is
//! too slow for a list read, so a serve-process cache holds them: a read replies with what is
//! fresh and schedules a small background refresh of what is not (contract: Page summary).
//! Nothing here is signed, stored or part of the owner log; a restart refills the cache lazily.
use crate::{
    Result, decoder, discussion, export::conversations::Conversations, page, page::save::Source,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Stale pages one read schedules for refresh; the worker takes them one at a time.
pub const REFRESH_PAGES: usize = 8;
/// The browser's answer to a question is an ordinary comment that starts this way; it is not a
/// comment about the page, so the count leaves it out.
const ANSWER_PREFIX: &str = "Answer to \u{201c}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// Non-deleted comments in the page's annotation threads, open and resolved, agent replies
    /// included; Chat messages, status actions, proposal decisions and question answers excluded.
    pub comments: u32,
    /// At least one proposal is neither decided, resolved nor deleted.
    pub waiting: bool,
    /// The page's creation label (`meta.originalAuthor`), when it has one.
    pub author: Option<String>,
}
impl Summary {
    pub fn json(&self) -> Value {
        let mut summary = json!({"comments": self.comments, "waiting": self.waiting});
        if let Some(author) = &self.author {
            summary["author"] = json!(author);
        }
        summary
    }
}

/// The counting rules, pure over the owner's projected conversations. An annotation thread is
/// one that is not deleted, carries no proposal and is anchored; the Chat thread has no anchor.
pub fn count(conversations: &Conversations, author: Option<&str>) -> Summary {
    let mut comments = 0u32;
    let mut waiting = false;
    for thread in conversations
        .threads
        .iter()
        .filter(|thread| !thread.deleted)
    {
        if thread.proposal.is_some() {
            waiting |= !thread.resolved && thread.decision.as_ref().is_none_or(|d| d.deleted);
        } else if thread.anchor.is_some() {
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
        waiting,
        author: author
            .filter(|label| decoder::valid_publisher_agent(label))
            .map(str::to_owned),
    }
}

/// One page through the owner fold: the same snapshot and materialize path `threads` reads.
pub fn summarize(source: &mut Source, page: &str) -> Result<Summary> {
    let snapshot = page::snapshot(&source.store, &source.keyring, page, false)?;
    let folded = snapshot.materialize(&source.keyring, page, &mut source.decoder)?;
    let conversations = discussion::project(&source.keyring, page, &snapshot, &folded);
    Ok(count(&conversations, folded.original_author.as_deref()))
}

#[derive(Default)]
struct Entry {
    fingerprint: String,
    /// `None` records that the fold failed at this fingerprint: the page is retried on a later
    /// read, after every page that has not failed.
    summary: Option<Summary>,
}
/// Fresh summaries by page, each valid for the fingerprint it was computed at. The fingerprint is
/// the page's epoch, the membership head and its receipt and checkpoint counts, read in the same
/// store transaction as the list, so any accepted write or membership change makes it stale.
#[derive(Default)]
pub struct SummaryCache {
    entries: Mutex<HashMap<String, Entry>>,
    refreshing: AtomicBool,
}
impl SummaryCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    pub fn get(&self, page: &str, fingerprint: &str) -> Option<Summary> {
        self.lock()
            .get(page)
            .filter(|entry| entry.fingerprint == fingerprint)
            .and_then(|entry| entry.summary.clone())
    }
    pub fn put(&self, page: &str, fingerprint: &str, summary: Option<Summary>) {
        self.lock().insert(
            page.to_owned(),
            Entry {
                fingerprint: fingerprint.to_owned(),
                summary,
            },
        );
    }
    /// At most `limit` pages without a fresh summary, as (page, fingerprint): pages that have not
    /// failed at their current fingerprint first, then the failed ones, each in page order. Pages
    /// that no longer exist are dropped from the cache.
    pub fn stale(
        &self,
        fingerprints: &BTreeMap<String, String>,
        limit: usize,
    ) -> Vec<(String, String)> {
        let mut entries = self.lock();
        entries.retain(|page, _| fingerprints.contains_key(page));
        let mut fresh_failures = Vec::new();
        let mut stale = Vec::new();
        for (page, fingerprint) in fingerprints {
            match entries
                .get(page)
                .filter(|entry| entry.fingerprint == *fingerprint)
            {
                Some(Entry {
                    summary: Some(_), ..
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
    mut summarize: impl FnMut(&str) -> Result<Summary>,
) {
    for (page, fingerprint) in pages {
        cache.put(page, fingerprint, summarize(page).ok());
    }
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
            run(&cache, &pages, |page| summarize(&mut source, page));
        });
    // A thread that could not start drops its closure, and with it the worker claim.
    drop(spawned);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::conversations::{Anchor, Comment, Conversations, Head, Thread};

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
    }

    fn fingerprints(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(page, fp)| ((*page).to_owned(), (*fp).to_owned()))
            .collect()
    }
    fn summary(comments: u32) -> Summary {
        Summary {
            comments,
            waiting: false,
            author: None,
        }
    }

    #[test]
    fn a_summary_is_fresh_only_at_the_fingerprint_it_was_computed_at() {
        let cache = SummaryCache::default();
        cache.put("a", "1:5", Some(summary(2)));
        assert_eq!(cache.get("a", "1:5"), Some(summary(2)));
        // A new receipt changes the fingerprint: the old entry is stale, not served.
        assert_eq!(cache.get("a", "1:6"), None);
        assert_eq!(
            cache.stale(&fingerprints(&[("a", "1:6")]), 8),
            vec![("a".into(), "1:6".into())]
        );
        assert!(cache.stale(&fingerprints(&[("a", "1:5")]), 8).is_empty());
    }

    #[test]
    fn a_refresh_takes_at_most_the_limit_and_forgets_deleted_pages() {
        let cache = SummaryCache::default();
        cache.put("gone", "1", Some(summary(1)));
        let map: BTreeMap<_, _> = (b'a'..=b'k')
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
        run(&cache, &pages, |page| {
            calls.push(page.to_owned());
            if page == "a" {
                Err(crate::page::Fault::Unavailable.into())
            } else {
                Ok(summary(4))
            }
        });
        // One attempt each, in order, and no retry within the run.
        assert_eq!(calls, ["a", "b"]);
        assert_eq!(cache.get("a", "1"), None);
        assert_eq!(cache.get("b", "1"), Some(summary(4)));
        // The failed page is still stale (retried on a later read), and ordered last.
        assert_eq!(
            cache.stale(&fingerprints(&[("a", "1"), ("b", "1")]), 8),
            vec![("a".to_owned(), "1".to_owned())]
        );
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
