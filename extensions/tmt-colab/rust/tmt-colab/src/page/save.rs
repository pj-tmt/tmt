//! The browser's whole-source Save over the owner-authenticated sync socket. The serve assembles
//! the source, prepares it natively from a snapshot of its own outside every lock, and commits
//! the frozen publication like `page write`, signing as the root-local writer. Nothing here
//! decodes, signs or stores: it names the pieces and the reply.
use super::{Fault, FrozenPublication, PublicationRecord, PublishOptions, SourceTooLarge};
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
}
/// A save after native preparation: nothing to publish, or a frozen publication.
pub enum Prepared {
    Unchanged(Box<super::Receipt>),
    Write(FrozenPublication),
}
/// Prepare outside every lock. The commit that follows rechecks every fence, so a page that
/// moved meanwhile turns this save into a stale-base refusal, never a silent overwrite.
///
/// The clock is read only after the snapshot is open: a certificate another writer issued before
/// that snapshot was taken is issued no later than this reading, so a chain renewed while this
/// save was waiting is never "from the future" and cannot turn the save into a denial.
pub fn prepare(
    open: &SourceOpener,
    save: &Save,
    clock: &dyn Fn() -> crate::Result<u64>,
) -> crate::Result<Prepared> {
    use super::PublicationPreparation;
    let mut source = open()?;
    let now = clock()?;
    let prepared = super::prepare_publication(
        &source.store,
        &source.keyring,
        &save.page,
        ContentEdit {
            source: &save.source,
            publisher_agent: None,
        },
        PublishOptions {
            expected_revision: None,
            base_source_sha256: Some(save.base_sha256),
            operation_id: Some(&save.operation_id),
        },
        &mut source.decoder,
        now,
    )?;
    Ok(match prepared {
        PublicationPreparation::Noop {
            epoch,
            membership_head,
            base_revision,
            memory_limit,
        } => Prepared::Unchanged(Box::new(super::Receipt::unchanged(
            &source.keyring,
            &save.page,
            &save.source,
            epoch,
            membership_head,
            base_revision,
            memory_limit,
        ))),
        PublicationPreparation::Write(frozen) => Prepared::Write(frozen),
    })
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
}
impl SaveResult {
    fn state(operation_id: &str, state: &'static str) -> Self {
        Self {
            operation_id: operation_id.into(),
            state,
            revision: None,
            code: None,
            message: None,
        }
    }
    pub fn committed(operation_id: &str, revision: String) -> Self {
        Self {
            revision: Some(revision),
            ..Self::state(operation_id, "committed")
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
    pub fn absent(operation_id: &str) -> Self {
        Self::state(operation_id, "absent")
    }
    /// An expected refusal, as a reply: a stable code and a person-readable message.
    pub fn rejected(operation_id: &str, error: &(dyn std::error::Error + 'static)) -> Self {
        let (code, message) = refusal(error);
        Self {
            code: Some(code),
            message: Some(message),
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
    // The operation ID was used before with different bytes: that save is not this one.
    if error.downcast_ref::<OwnerFault>() == Some(&OwnerFault::Conflict) {
        return ("COLAB_OPERATION_CONFLICT", error.to_string());
    }
    (Fault::Unavailable.code(), Fault::Unavailable.to_string())
}
