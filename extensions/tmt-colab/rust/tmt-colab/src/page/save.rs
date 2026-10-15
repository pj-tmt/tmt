//! The browser's whole-source Save over the owner-authenticated sync socket. The serve assembles
//! the source, prepares it natively from a snapshot of its own outside every lock, and commits
//! the frozen publication like `page write`, signing as the root-local writer. Nothing here
//! decodes, signs or stores: it names the pieces and the reply.
use super::{
    Fault, FrozenPublication, PublicationRecord, PublishOptions, SourceTooLarge,
    patch::{Hunk, PatchFault, apply_hunks, conflicting_hunks},
};
use crate::{
    decoder::{ContentEdit, Decoder},
    keyring::Keyring,
    publication::Outcome,
    store::{Store, owner::OwnerFault},
};
use serde::Serialize;
use std::sync::Arc;

/// A fresh read-only view for one preparation: its own store handle, keyring and decoder.
pub struct Source {
    pub store: Store,
    pub keyring: Keyring,
    pub decoder: Decoder,
}
/// How the serve opens a `Source`; supplied by whoever owns the data root.
pub type SourceOpener = Arc<dyn Fn() -> crate::Result<Source> + Send + Sync>;
/// The opener for a data root: the same read-only store, keyring and decoder `page write` uses.
pub fn open_source(
    root: &std::path::Path,
    decoder: crate::decoder::Config,
) -> crate::Result<Source> {
    let layout = crate::keyring::Layout::existing(root)?.ok_or(Fault::Missing)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    Ok(Source {
        keyring: Keyring::read(&layout)?,
        store,
        decoder: Decoder::with_config(decoder)?,
    })
}

/// One assembled save: the page, the caller's own operation ID, the SHA-256 of the source the
/// person edited from, and the replacement source.
pub struct Save {
    pub page: String,
    pub operation_id: String,
    pub base_sha256: [u8; 32],
    pub source: String,
    /// A typed change to the document's attachment list, bound to `source`.
    pub attachments: Option<tmt_colab_model::attachment::DocumentChange>,
}
/// A save after native preparation: nothing to publish, or a frozen publication.
pub enum Prepared {
    Unchanged(Box<super::Receipt>),
    Write(FrozenPublication),
}
/// Prepare outside every lock. The commit that follows rechecks every fence, so a page that
/// moved meanwhile turns this save into a stale-base refusal, never a silent overwrite.
///
/// The clock is read by `prepare_publication_with_clock` after its snapshot is taken, so a
/// certificate another writer issued while this save waited is never in the save's future.
pub fn prepare(
    open: &SourceOpener,
    save: &Save,
    clock: &dyn Fn() -> crate::Result<u64>,
) -> crate::Result<Prepared> {
    use super::PublicationPreparation;
    let mut source = open()?;
    let prepared = super::prepare_publication_with_clock(
        &source.store,
        &source.keyring,
        &save.page,
        ContentEdit {
            source: &save.source,
            publisher_agent: None,
            attachments: save.attachments.as_ref(),
        },
        PublishOptions {
            expected_revision: None,
            base_source_sha256: Some(save.base_sha256),
            operation_id: Some(&save.operation_id),
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
            // A no-op records nothing, so an operation ID that already has a recorded outcome
            // was used by another save: it must not answer "unchanged" for this one.
            if super::publication_status_by_operation(
                &source.store,
                &source.keyring,
                &save.page,
                &save.operation_id,
            )?
            .is_some()
            {
                return Err(OwnerFault::Conflict.into());
            }
            Prepared::Unchanged(Box::new(super::Receipt::unchanged(
                &source.keyring,
                &save.page,
                &save.source,
                epoch,
                membership_head,
                base_revision,
                memory_limit,
            )))
        }
        PublicationPreparation::Write(frozen) => Prepared::Write(frozen),
    })
}
/// A save's hunks could not be placed on the page as it now is: the text they anchor to moved or
/// changed. Nothing is written. `hunks` are the indices, as the editor sent them, that conflict.
#[derive(Debug)]
pub struct HunkConflict {
    pub hunks: Vec<usize>,
    fault: PatchFault,
}
impl std::fmt::Display for HunkConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fault.fmt(f)
    }
}
impl std::error::Error for HunkConflict {}

/// Prepare a save that also carries the editor's change as anchored hunks. While the page is
/// still the source the editor opened, this is exactly `prepare`. When it moved, the hunks are
/// evaluated on the page as it now is, with the same rules as `page patch --diff`: each must match
/// exactly once and all apply or none does, and the result is prepared against that source, so a
/// page that moves again turns it into a stale-base refusal for the caller to evaluate once more.
/// When the page the editor saw had changed, the result is a merge and its digest says which source
/// the commit holds.
pub fn prepare_merging(
    open: &SourceOpener,
    save: &Save,
    hunks: &[Hunk],
    clock: &dyn Fn() -> crate::Result<u64>,
) -> crate::Result<(Prepared, Option<[u8; 32]>)> {
    match prepare(open, save, clock) {
        Err(error) if matches!(error.downcast_ref::<Fault>(), Some(Fault::StaleBase)) => {
            let mut view = open()?;
            let current =
                super::read(&view.store, &view.keyring, &save.page, &mut view.decoder)?.source;
            let merged = match apply_hunks(&current, hunks) {
                Ok(merged) => merged,
                Err(fault @ PatchFault::Conflict(_)) => {
                    return Err(Box::new(HunkConflict {
                        hunks: conflicting_hunks(&current, hunks),
                        fault,
                    }));
                }
                Err(other) => return Err(Box::new(other)),
            };
            let merged_digest = tmt_colab_model::crypto::digest(merged.as_bytes());
            let merged_save = Save {
                page: save.page.clone(),
                operation_id: save.operation_id.clone(),
                base_sha256: tmt_colab_model::crypto::digest(current.as_bytes()),
                source: merged,
                attachments: None,
            };
            Ok((prepare(open, &merged_save, clock)?, Some(merged_digest)))
        }
        other => Ok((other?, None)),
    }
}
/// The one reply to a save or a status request. `absent` means the operation never reached a
/// terminal outcome, so nothing was committed and the person may save again from a fresh view.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub operation_id: String,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Committed only: the page had changed since the editor opened it and the save was merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged: Option<bool>,
    /// Merged only: the digest of the source the commit holds, which the editor never had.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    /// Rejected only: the hunks (as sent, 0-based) whose anchors moved or changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflicts: Option<Vec<usize>>,
}
impl SaveResult {
    fn state(operation_id: &str, state: &'static str) -> Self {
        Self {
            operation_id: operation_id.into(),
            state,
            revision: None,
            code: None,
            message: None,
            merged: None,
            source_sha256: None,
            conflicts: None,
        }
    }
    pub fn committed(operation_id: &str, revision: String) -> Self {
        Self {
            revision: Some(revision),
            ..Self::state(operation_id, "committed")
        }
    }
    /// A commit whose source is the editor's change merged into a page that had moved.
    pub fn merged(operation_id: &str, revision: String, source_sha256: String) -> Self {
        Self {
            merged: Some(true),
            source_sha256: Some(source_sha256),
            ..Self::committed(operation_id, revision)
        }
    }
    pub fn unchanged(operation_id: &str, revision: String) -> Self {
        Self {
            revision: Some(revision),
            ..Self::state(operation_id, "unchanged")
        }
    }
    /// Status only: the save is still preparing, so its outcome is not final yet.
    pub fn pending(operation_id: &str) -> Self {
        Self::state(operation_id, "pending")
    }
    /// How a request that asked for this save is settled by it: applied, in conflict with a page
    /// that moved, or refused. `None` for a result a later try could change, or one that is not
    /// final.
    pub fn settles(&self) -> Option<crate::store::EditState> {
        use crate::store::EditState;
        match (self.state, self.code) {
            ("committed" | "unchanged", _) => Some(EditState::Applied),
            ("rejected", Some("COLAB_STALE_BASE" | "COLAB_PATCH_CONFLICT")) => {
                Some(EditState::Conflict)
            }
            ("rejected", Some("COLAB_UNAVAILABLE" | "COLAB_STREAM_GAP") | None) => None,
            ("rejected", Some(_)) => Some(EditState::Refused),
            _ => None,
        }
    }
    pub fn absent(operation_id: &str) -> Self {
        Self::state(operation_id, "absent")
    }
    /// An expected refusal, as a reply: a stable code and a person-readable message.
    pub fn rejected(operation_id: &str, error: &(dyn std::error::Error + 'static)) -> Self {
        let (code, message) = refusal(error);
        Self {
            code: Some(code),
            message: Some(message),
            conflicts: error
                .downcast_ref::<HunkConflict>()
                .map(|conflict| conflict.hunks.clone()),
            ..Self::state(operation_id, "rejected")
        }
    }
    /// What the root-local stream recorded for the operation, if anything.
    pub fn recorded(operation_id: &str, record: Option<PublicationRecord>) -> crate::Result<Self> {
        Ok(match record.map(|record| record.outcome) {
            None => Self::absent(operation_id),
            Some(Outcome::Committed {
                committed_revision, ..
            }) => Self::committed(operation_id, committed_revision),
            Some(Outcome::Rejected { code, .. }) => Self {
                code: Some(Fault::from(code).code()),
                ..Self::state(operation_id, "rejected")
            },
            Some(Outcome::Unknown { .. }) => return Err(Fault::Invalid.into()),
        })
    }
}
/// A stable code and a message a person can read, for any failure of a save.
fn refusal(error: &(dyn std::error::Error + 'static)) -> (&'static str, String) {
    if let Some(large) = error.downcast_ref::<SourceTooLarge>() {
        return (large.code(), error.to_string());
    }
    if let Some(fault) = error.downcast_ref::<Fault>() {
        return (fault.code(), error.to_string());
    }
    if let Some(conflict) = error.downcast_ref::<HunkConflict>() {
        return (conflict.fault.code(), error.to_string());
    }
    // The operation ID was used before with different bytes: that save is not this one.
    if error.downcast_ref::<OwnerFault>() == Some(&OwnerFault::Conflict) {
        return ("COLAB_OPERATION_CONFLICT", error.to_string());
    }
    (Fault::Unavailable.code(), Fault::Unavailable.to_string())
}
