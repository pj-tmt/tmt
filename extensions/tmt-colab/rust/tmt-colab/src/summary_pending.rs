//! The owner's pending-item projection. It consumes admitted markup, resolved quote associations
//! and verified conversation provenance; it never publishes records or interprets a display label.
use std::collections::BTreeSet;

use crate::{
    export::conversations::{Comment, Conversations, Thread},
    page_check::{Report, SummaryAnchors},
};

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct PendingItem {
    pub kind: &'static str,
    pub id: String,
}

fn device<'a>(comments: &'a [Comment], writers: &BTreeSet<String>) -> Vec<&'a Comment> {
    let mut values: Vec<_> = comments
        .iter()
        .filter(|comment| !comment.deleted && writers.contains(&comment.writer))
        .collect();
    values.sort_by(|a, b| {
        (a.sequence.parse::<u64>().unwrap_or(0), &a.writer, &a.id).cmp(&(
            b.sequence.parse::<u64>().unwrap_or(0),
            &b.writer,
            &b.id,
        ))
    });
    values
}

fn closed(thread: &Thread) -> bool {
    thread.resolved
        && thread
            .status
            .as_ref()
            .is_some_and(|status| status.action.resolved && status.action.actor == "person")
}

fn section_action(title: &str, body: &str) -> Option<&'static str> {
    for (action, head) in [
        ("confirmed", "Confirmed"),
        ("changes", "Needs change to"),
        ("answered", "Answered"),
    ] {
        if let Some(rest) = body.strip_prefix(&format!("{head} “{title}”"))
            && (rest.is_empty() || rest.starts_with(':') || rest.starts_with('\n'))
        {
            return Some(action);
        }
    }
    None
}

fn reading_comment(id: &str, title: &str, body: &str) -> bool {
    let Some(rest) = body.strip_prefix(&format!("Read “{title}”\nReading: {id}\nPage: "))
    else {
        return false;
    };
    let lines: Vec<_> = rest.split('\n').collect();
    lines.len() == 1
        || (lines.len() == 2
            && lines[1].starts_with('@')
            && lines[1].len() > 1
            && !lines[1].contains('\r'))
}

fn associated<'a>(view: &'a Conversations, anchors: &SummaryAnchors, id: &str) -> Vec<&'a Thread> {
    let mut threads: Vec<_> = view
        .threads
        .iter()
        .filter(|thread| {
            !thread.deleted
                && thread.proposal.is_none()
                && anchors
                    .threads(id)
                    .iter()
                    .any(|key| key == &format!("{}:{}", thread.writer, thread.id))
        })
        .collect();
    threads.sort_by(|a, b| (&a.writer, &a.id).cmp(&(&b.writer, &b.id)));
    threads
}

/// Matches the browser's canonical selection: a recorded device action wins over an empty
/// sibling; otherwise writer/thread identity order wins. Wall clocks never decide.
fn canonical<'a>(
    threads: Vec<&'a Thread>,
    writers: &BTreeSet<String>,
    matches: impl Fn(&str) -> bool,
) -> Option<&'a Thread> {
    threads
        .iter()
        .copied()
        .find(|thread| {
            device(&thread.comments, writers)
                .iter()
                .any(|comment| matches(&comment.body))
        })
        .or_else(|| threads.first().copied())
}

pub(crate) fn items(
    report: &Report,
    anchors: &SummaryAnchors,
    view: &Conversations,
    writers: &BTreeSet<String>,
) -> Vec<PendingItem> {
    let mut pending = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |kind, id: &str| {
        if seen.insert(id.to_owned()) {
            pending.push(PendingItem {
                kind,
                id: id.to_owned(),
            });
        }
    };
    for thread in &view.threads {
        if !thread.deleted
            && !thread.resolved
            && thread
                .decision
                .as_ref()
                .is_none_or(|decision| decision.deleted)
            && let Some(proposal) = &thread.proposal
        {
            add("proposal", &proposal.proposal_id);
        }
    }
    for reading in &report.readings {
        if !anchors.contains(&reading.id) {
            continue;
        }
        // Reading's canonical thread requires its exact fixed comment, not merely the heading.
        let thread = associated(view, anchors, &reading.id)
            .into_iter()
            .find(|thread| {
                device(&thread.comments, writers)
                    .iter()
                    .any(|comment| reading_comment(&reading.id, &reading.title, &comment.body))
            });
        if !thread.is_some_and(closed) {
            add("reading", &reading.id);
        }
    }
    for group in &report.groups {
        if !group.questions.iter().all(|id| anchors.contains(id)) {
            continue;
        }
        let unanswered = group.questions.iter().any(|id| {
            let Some(question) = report.questions.iter().find(|question| &question.id == id) else {
                return true;
            };
            let prefix = format!("Answer to “{}”: ", question.title);
            let thread = canonical(associated(view, anchors, id), writers, |body| {
                body.starts_with(&prefix)
            });
            !thread.is_some_and(|thread| {
                device(&thread.comments, writers)
                    .iter()
                    .any(|comment| comment.body.starts_with(&prefix))
            })
        });
        if unanswered {
            add("answer", &group.id);
        }
    }
    for review in &report.reviews {
        if !anchors.contains(&review.id) {
            continue;
        }
        let thread = canonical(associated(view, anchors, &review.id), writers, |body| {
            section_action(&review.title, body).is_some()
        });
        let waiting = match thread {
            Some(thread) if closed(thread) => false,
            Some(thread) => {
                let latest = device(&thread.comments, writers)
                    .into_iter()
                    .rev()
                    .find(|comment| section_action(&review.title, &comment.body).is_some());
                match latest {
                    Some(comment)
                        if section_action(&review.title, &comment.body) == Some("changes") =>
                    {
                        view.asks.iter().any(|ask| {
                            ask.reply.is_some()
                                && ask.message_ids.contains(&comment.id)
                                && (ask.thread == thread.id
                                    || (ask.message_ids.len() > 1 && ask.writer == comment.writer))
                        })
                    }
                    _ => true,
                }
            }
            None => true,
        };
        if waiting {
            add("review", &review.id);
        }
    }
    pending
}

#[cfg(test)]
#[path = "summary_pending_tests.rs"]
mod tests;
