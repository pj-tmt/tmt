//! The stored facts behind one message's delivery state. Everything here is a
//! row read inside the request transaction: no process, pane or file is touched.
use super::*;
use tmt_core::{
    binding::BindingRecords,
    request::delivery::{DeliveryFacts, DigestHold, RecipientActivity},
};

pub(super) fn facts(
    db: &Connection,
    attempt: &RequestAttempt,
    now: u64,
) -> Result<DeliveryFacts, StorageError> {
    let recipient = attempt.recipient_identity_id.as_deref();
    Ok(DeliveryFacts {
        status: attempt.status,
        inbox: matches!(attempt.route, RequestRoute::Inbox { .. }),
        wake: RequestRows(db)
            .wake_state(&attempt.request_id)?
            .unwrap_or(WakeState::NotAttempted),
        prepared_at_ms: attempt.prepared_at_ms,
        settled_at_ms: attempt.settled_at_ms,
        hold: recipient
            .map(|id| hold(db, id, &attempt.request_id))
            .transpose()?
            .flatten(),
        max_hold_ms: recipient
            .map(|id| digest::policy(db, id))
            .transpose()?
            .flatten()
            .and_then(|policy| policy.max_hold_ms),
        oldest_held_at_ms: recipient
            .map(|id| digest::oldest_held(db, id, now))
            .transpose()?
            .flatten(),
        recipient: recipient.map(|id| activity(db, id)).transpose()?.flatten(),
    })
}

fn hold(
    db: &Connection,
    identity: &str,
    request: &str,
) -> Result<Option<DigestHold>, StorageError> {
    db.query_row(
        "SELECT i.created_at_ms, c.state FROM focus_items i
         LEFT JOIN focus_checklists c ON c.id = i.checklist_id
         WHERE i.identity_id = ?1 AND i.request_id = ?2 AND i.source = 'incoming'",
        params![identity, request],
        |row| {
            Ok(match row.get::<_, Option<String>>(1)?.as_deref() {
                None | Some("definitely_unsent") => DigestHold::Open {
                    since_ms: rows::u64_at(row, 0)?,
                },
                Some("claimed") => DigestHold::Claimed,
                Some("delivered") => DigestHold::Delivered,
                Some(_) => DigestHold::Uncertain,
            })
        },
    )
    .optional()
    .map_err(|error| classify(error, "Read a message's digest hold"))
}

/// The driver's last admitted event for the exact session and process the
/// recipient is bound to now. It is not liveness evidence: a stopped process
/// keeps its last phase until a new binding replaces it.
pub(super) fn activity(
    db: &Connection,
    identity: &str,
) -> Result<Option<RecipientActivity>, StorageError> {
    let rows = crate::storage::bindings::BindingRows(db);
    let Some(key) = rows
        .entry_by_id(identity)?
        .and_then(|entry| entry.binding)
        .and_then(|binding| binding.session.key)
    else {
        return Ok(None);
    };
    let preferences = rows.session_preferences(identity)?;
    Ok(preferences
        .remembered
        .as_ref()
        .filter(|remembered| key.provider_session.as_ref() == Some(&remembered.provider_session))
        .and_then(|remembered| {
            let activity = remembered
                .state
                .as_ref()
                .and_then(crate::runtime::driver_state::state_activity)?;
            activity
                .matches(&remembered.provider_session, &key.incarnation)
                .then_some(RecipientActivity {
                    phase: activity.phase,
                    since_ms: activity.since_ms,
                })
        }))
}
