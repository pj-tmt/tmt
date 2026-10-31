//! A browser's sub-page action over the owner-authenticated sync socket. Like a whole-source save,
//! the serve prepares it natively from a snapshot of its own outside every lock and commits the
//! frozen publication, signing as the root-local writer; unlike a save it names an action, not a
//! source, and runs the write command's admission (the link rules) first. It applies to the
//! revision the serve reads, so the action's own preconditions fence it: a page that is no longer
//! listed, or one already listed, is refused, never overwritten.
use super::{
    StructuredAction,
    save::{Prepared, SourceOpener},
};
use crate::{
    decoder::StructuredKind,
    work_admission::{self, Space},
};
use serde_json::Value;

/// One sub-page action of a page, with the operation ID the browser chose so its outcome can be
/// read back (`savestatus`).
pub struct SubpageEdit {
    pub page: String,
    pub operation_id: String,
    pub action: String,
    pub body: Value,
}

/// The closed list actions (contract: Sub-pages); anything else is refused before any read.
pub fn known(action: &str) -> bool {
    crate::work::SUBPAGE_ACTIONS.contains(&action)
}

/// A browser's create: a page made from a title, then attached, as one operation.
pub fn is_create(action: &str) -> bool {
    action == crate::work::CREATE
}

/// Prepare outside every lock; the commit's fences turn a page that moved meanwhile into a
/// stale-base refusal.
pub fn prepare(
    open: &SourceOpener,
    edit: &SubpageEdit,
    clock: &dyn Fn() -> crate::Result<u64>,
) -> crate::Result<Prepared> {
    let mut source = open()?;
    let read = super::read_work(
        &source.store,
        &source.keyring,
        &edit.page,
        &mut source.decoder,
    )?;
    let before = serde_json::json!({"version":2,"order":read.children});
    work_admission::admit(
        &Space {
            root: &source.root,
            layout: &source.layout,
            store: &source.store,
            key: &source.keyring,
        },
        &mut source.decoder,
        &edit.page,
        &before,
        &edit.action,
        &edit.body,
    )?;
    let prepared = super::prepare_structured_publication(
        &source.store,
        &source.keyring,
        &edit.page,
        StructuredAction {
            kind: StructuredKind::Work,
            action: &edit.action,
            body: &edit.body,
            display_agent: None,
            expected_revision: &read.revision,
            operation_id: Some(&edit.operation_id),
        },
        &mut source.decoder,
        clock,
    )?;
    super::save::structured(&source, &edit.page, &edit.operation_id, prepared)
}
