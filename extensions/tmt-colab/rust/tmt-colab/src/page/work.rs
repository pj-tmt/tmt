//! A browser's work action over the owner-authenticated sync socket. Like a whole-source save,
//! the serve prepares it natively from a snapshot of its own outside every lock and commits the
//! frozen publication, signing as the root-local writer; unlike a save it names an action, not a
//! source, and runs the write command's admission (the limit, the link rules) first. It applies to
//! the revision the serve reads, so the action's own preconditions fence it: a task that is gone or
//! a state that cannot follow is refused, never overwritten.
use super::{
    PublicationPreparation, StructuredAction,
    save::{Prepared, SourceOpener},
};
use crate::{
    decoder::StructuredKind,
    work_admission::{self, Space},
};
use serde_json::Value;

/// One work action of a page, with the operation ID the browser chose so its outcome can be read
/// back (`savestatus`).
pub struct WorkEdit {
    pub page: String,
    pub operation_id: String,
    pub action: String,
    pub body: Value,
}

/// The closed actions of a work value (contract: Work); anything else is refused before any read.
pub fn known(action: &str) -> bool {
    matches!(
        action,
        "summary.set"
            | "summary.clear"
            | "task.add"
            | "task.set"
            | "task.state"
            | "task.assign"
            | "task.link"
            | "task.unlink"
            | "task.remove"
    )
}

/// Prepare outside every lock. Also names the task a `task.add` makes, which is the page's counter
/// at the revision the action applies to; the commit's fences turn a page that moved meanwhile into
/// a stale-base refusal.
/// A browser's convert: a page made from a task, then the link, as one operation.
pub fn is_convert(action: &str) -> bool {
    action == "task.convert"
}

pub fn prepare(
    open: &SourceOpener,
    edit: &WorkEdit,
    clock: &dyn Fn() -> crate::Result<u64>,
) -> crate::Result<(Prepared, Option<String>)> {
    prepare_expecting(open, edit, clock, None)
}
/// As `prepare`, but only while the named task is still exactly `expect`: a convert links the task
/// it copied, never one that changed while its page was being made.
pub(super) fn prepare_expecting(
    open: &SourceOpener,
    edit: &WorkEdit,
    clock: &dyn Fn() -> crate::Result<u64>,
    expect: Option<(&str, &Value)>,
) -> crate::Result<(Prepared, Option<String>)> {
    let mut source = open()?;
    let read = super::read_work(
        &source.store,
        &source.keyring,
        &edit.page,
        &mut source.decoder,
    )?;
    if let Some((task, copied)) = expect
        && read.work["tasks"].get(task) != Some(copied)
    {
        return Err(super::Fault::StaleBase.into());
    }
    work_admission::admit(
        &Space {
            root: &source.root,
            layout: &source.layout,
            store: &source.store,
            key: &source.keyring,
        },
        &mut source.decoder,
        &edit.page,
        &read.work,
        &edit.action,
        &edit.body,
    )?;
    let made = (edit.action == "task.add")
        .then(|| read.work["next"].as_u64().map(|next| format!("T-{next}")))
        .flatten();
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
    Ok(match prepared {
        PublicationPreparation::Noop {
            epoch,
            membership_head,
            base_revision,
            memory_limit,
        } => {
            // A no-op records nothing, so an operation ID that already has a recorded outcome was
            // used by another action: it must not answer "unchanged" for this one.
            if super::publication_status_by_operation(
                &source.store,
                &source.keyring,
                &edit.page,
                &edit.operation_id,
            )?
            .is_some()
            {
                return Err(crate::store::owner::OwnerFault::Conflict.into());
            }
            (
                Prepared::Unchanged(Box::new(super::Receipt::unchanged(
                    &source.keyring,
                    &edit.page,
                    "",
                    epoch,
                    membership_head,
                    base_revision,
                    memory_limit,
                ))),
                None,
            )
        }
        PublicationPreparation::Write(frozen) => (Prepared::Write(frozen), made),
    })
}
