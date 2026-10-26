//! Freeze admitted checklist evidence and append outcomes in its owner transaction.
use super::*;
use crate::orchestration::wire;
use serde::{Deserialize, Serialize};
use tmt_core::{
    orchestration::{receiving::OrchReceivingPolicy, *},
    request::digest::{DigestChecklist, DigestState},
};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Context {
    configuration_revision: u64,
    time_zone: String,
    source: serde_json::Value,
}
pub(super) fn claim(
    db: &Connection,
    batch: &DigestChecklist,
    policy: &OrchReceivingPolicy,
    now: u64,
) -> Result<(), StorageError> {
    let frozen = Context {
        configuration_revision: policy.configuration_revision,
        time_zone: policy.time_zone.clone(),
        source: wire::source_value(&policy.receiving.source),
    };
    let document = serde_json::to_string(&frozen).map_err(|_| invalid())?;
    db.execute(
        "UPDATE focus_checklists SET orchestration_context=?1 WHERE id=?2",
        params![document, batch.id],
    )
    .map_err(|e| classify(e, "Freeze checklist receiving provenance"))?;
    append(
        db,
        batch,
        frozen,
        OrchDecisionOutcome::Allowed,
        "digest_claim",
        now,
    )
}
pub(super) fn settle(
    db: &Connection,
    batch: &DigestChecklist,
    state: DigestState,
    now: u64,
) -> Result<(), StorageError> {
    let document: Option<String> = db
        .query_row(
            "SELECT orchestration_context FROM focus_checklists WHERE id=?",
            [&batch.id],
            |r| r.get(0),
        )
        .map_err(|e| classify(e, "Read frozen checklist receiving provenance"))?;
    let Some(document) = document else {
        return Ok(());
    };
    if document.len() > 8192 {
        return Err(invalid());
    }
    let frozen: Context = serde_json::from_str(&document).map_err(|_| invalid())?;
    let (outcome, reason) = match state {
        DigestState::Delivered => (OrchDecisionOutcome::Sent, "digest_delivered"),
        DigestState::Unsent => (OrchDecisionOutcome::Failed, "digest_unsent"),
        DigestState::Uncertain => (OrchDecisionOutcome::Uncertain, "digest_uncertain"),
        DigestState::Claimed => return Err(invalid()),
    };
    append(db, batch, frozen, outcome, reason, now)
}
fn append(
    db: &Connection,
    batch: &DigestChecklist,
    frozen: Context,
    outcome: OrchDecisionOutcome,
    reason: &str,
    now: u64,
) -> Result<(), StorageError> {
    let source = wire::decode_source(&frozen.source.to_string()).map_err(|_| invalid())?;
    crate::storage::orchestration::append(
        db,
        &OrchDecision {
            recorded_at_ms: now,
            subject_kind: OrchSubjectKind::Identity,
            subject_id: batch.identity_id.clone(),
            actor_id: None,
            recipient_id: Some(batch.identity_id.clone()),
            action: OrchDecisionAction::Send,
            outcome,
            reason: reason.into(),
            configuration_revision: frozen.configuration_revision,
            time_zone: frozen.time_zone,
            sources: vec![OrchDecisionSource {
                field: OrchPolicyField::Receiving,
                source,
            }],
        },
    )?;
    Ok(())
}
fn invalid() -> StorageError {
    StorageError::new(
        StorageErrorCode::Corrupt,
        "Invalid frozen checklist receiving provenance",
    )
}
