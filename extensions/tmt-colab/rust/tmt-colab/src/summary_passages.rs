//! Display-only passages of a page that need its owner, from the same admitted projection as the
//! numeric summary: the pending items in the order a reader should meet them, and the page's latest
//! comment as the fallback for a page with nothing pending. Nothing here is stored or signed.
use crate::{
    export::conversations::Conversations,
    limits::{LATEST_POINTS, NEEDS_PASSAGES, QUESTION_AGENT_POINTS, QUESTION_TITLE_POINTS},
    page_check::Report,
    summary_pending::PendingItem,
};
use serde::Serialize;

/// The comment forms the page's own controls write; they are bookkeeping, not a reply.
const CONTROL_COMMENTS: [&str; 5] = [
    "Answer to \u{201c}",
    "Answered \u{201c}",
    "Confirmed \u{201c}",
    "Needs change to \u{201c}",
    "Read \u{201c}",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NeedKind {
    Review,
    Question,
    Reading,
    Proposal,
}
impl NeedKind {
    /// The order a reader should meet them: a decision first, a plain read last.
    fn rank(self) -> u8 {
        match self {
            Self::Review => 0,
            Self::Question => 1,
            Self::Reading => 2,
            Self::Proposal => 3,
        }
    }
}

/// One thing that waits for the owner. `id` is the pending-target ID the count uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Need {
    pub kind: NeedKind,
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// The newest live comment of the page, as an excerpt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Latest {
    /// The device label the comment was written under; display-only, never proof of who wrote it.
    /// Absent when the label is empty once made inert.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who: Option<String>,
    pub at_ms: u64,
    pub text: String,
}

/// One line of inert text: control characters and runs of whitespace become single spaces, then
/// it is clipped to `points` code points. The browser refuses control characters and empty text,
/// so one odd label must not hide the whole reply.
fn excerpt(text: &str, points: usize) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let folded = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    folded
        .chars()
        .take(points)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn some(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// A named item, or none when its title is empty once made inert (the browser needs one).
fn need(
    kind: NeedKind,
    id: &str,
    title: &str,
    agent: Option<&str>,
    why: Option<&str>,
) -> Option<Need> {
    Some(Need {
        kind,
        id: id.to_owned(),
        title: some(excerpt(title, QUESTION_TITLE_POINTS))?,
        agent: agent.and_then(|agent| some(excerpt(agent, QUESTION_AGENT_POINTS))),
        why: why.and_then(|why| some(excerpt(why, QUESTION_TITLE_POINTS))),
    })
}

/// The pending items with what a reader needs to recognise each, review first, then question,
/// reading and proposal, each in page order; at most [`NEEDS_PASSAGES`] (the count stays
/// `summary.needsYou`). An item the report no longer lists, or with no usable title, is left out.
pub(crate) fn needs(report: &Report, view: &Conversations, pending: &[PendingItem]) -> Vec<Need> {
    let mut needs: Vec<Need> = pending
        .iter()
        .filter_map(|item| match item.kind {
            "review" => {
                let review = report.reviews.iter().find(|review| review.id == item.id)?;
                need(
                    NeedKind::Review,
                    &item.id,
                    &review.title,
                    Some(&review.agent),
                    None,
                )
            }
            "answer" => {
                let group = report.groups.iter().find(|group| group.id == item.id)?;
                let first = group
                    .questions
                    .iter()
                    .find_map(|id| report.questions.iter().find(|question| &question.id == id))?;
                need(
                    NeedKind::Question,
                    &item.id,
                    group.title.as_deref().unwrap_or(&first.title),
                    Some(&first.agent),
                    None,
                )
            }
            "reading" => {
                let reading = report
                    .readings
                    .iter()
                    .find(|reading| reading.id == item.id)?;
                need(
                    NeedKind::Reading,
                    &item.id,
                    &reading.title,
                    Some(&reading.agent),
                    Some(&reading.why),
                )
            }
            "proposal" => {
                let proposal = view
                    .threads
                    .iter()
                    .filter_map(|thread| thread.proposal.as_ref())
                    .find(|proposal| proposal.proposal_id == item.id)?;
                need(
                    NeedKind::Proposal,
                    &item.id,
                    &proposal.title,
                    Some(&proposal.proposer.label),
                    None,
                )
            }
            _ => None,
        })
        .collect();
    // A stable sort keeps page order inside a kind.
    needs.sort_by_key(|need| need.kind.rank());
    needs.truncate(NEEDS_PASSAGES);
    needs
}

/// The newest non-deleted comment that is not one of the page controls' own records, by the
/// device clock it was written under (display-only, so a skewed clock only picks another excerpt).
pub(crate) fn latest(view: &Conversations) -> Option<Latest> {
    view.threads
        .iter()
        .filter(|thread| !thread.deleted && thread.proposal.is_none())
        .flat_map(|thread| &thread.comments)
        .filter(|comment| {
            !comment.deleted
                && !CONTROL_COMMENTS
                    .iter()
                    .any(|prefix| comment.body.starts_with(prefix))
        })
        .filter_map(|comment| {
            let at_ms = comment.at.parse::<u64>().ok()?;
            let text = excerpt(&comment.body, LATEST_POINTS);
            (!text.is_empty()).then_some((at_ms, comment, text))
        })
        .max_by(|a, b| (a.0, &a.1.writer, &a.1.id).cmp(&(b.0, &b.1.writer, &b.1.id)))
        .map(|(at_ms, comment, text)| Latest {
            who: some(excerpt(&comment.device_name, QUESTION_AGENT_POINTS)),
            at_ms,
            text,
        })
}

#[cfg(test)]
#[path = "summary_passages_tests.rs"]
mod tests;
