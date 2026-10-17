//! Display-only reading descriptors from the same admitted projection as the numeric summary.
use crate::{
    page_check::{Report, SummaryAnchors},
    summary_pending::PendingItem,
};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadingState {
    Unread,
    Read,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReadingDescriptor {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub why: String,
    pub state: ReadingState,
}

/// The pending projection already owns canonical device provenance and read/Resolve semantics.
/// Its absence means read only after this exact reading's anchor has been admitted and resolved.
/// A pending target of another kind with this ID wins the shared cross-kind deduplication.
pub(crate) fn descriptors(
    report: &Report,
    anchors: &SummaryAnchors,
    pending: &[PendingItem],
) -> Vec<ReadingDescriptor> {
    report
        .readings
        .iter()
        .filter(|reading| {
            anchors.contains(&reading.id)
                && !pending
                    .iter()
                    .any(|item| item.id == reading.id && item.kind != "reading")
        })
        .map(|reading| ReadingDescriptor {
            id: reading.id.clone(),
            title: reading.title.clone(),
            agent: reading.agent.clone(),
            why: reading.why.clone(),
            state: if pending
                .iter()
                .any(|item| item.id == reading.id && item.kind == "reading")
            {
                ReadingState::Unread
            } else {
                ReadingState::Read
            },
        })
        .collect()
}
