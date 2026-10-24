//! The stored facts behind one message's delivery state. Everything here is a
//! row read inside the request transaction: no process, pane or file is touched.
use super::*;
use tmt_core::{
    binding::BindingRecords,
    request::{
        delivery::{DeliveryFacts, DigestHold, RecipientActivity, ReturnedAnswer},
        passthrough::CaptureState,
    },
};

pub(super) fn facts(
    db: &Connection,
    attempt: &RequestAttempt,
    now: u64,
) -> Result<DeliveryFacts, StorageError> {
    let recipient = attempt.recipient_identity_id.as_deref();
    let context = recipient
        .map(|id| {
            let rows = crate::storage::bindings::BindingRows(db);
            let entry = rows.entry_by_id(id)?;
            let refusal = entry
                .as_ref()
                .and_then(|entry| entry.binding.as_ref())
                .and_then(|binding| binding.session.start_refusal.as_ref())
                .map(|refusal| refusal.reason);
            let mut refusal = refusal;
            let mut recovery = None;
            if refusal.is_none()
                && let Some(entry) = entry.as_ref()
                && let Some(binding) = entry.binding.as_ref()
                && let Some(observed) =
                    crate::storage::launch_observations::latest(db, &binding.id, now)?
                && observed.status != tmt_core::binding::session::launch::LaunchStatus::Admitted
                && observed.diagnostic.is_some()
            {
                refusal = Some(tmt_core::binding::session::StartRefusalReason::SessionNotLearned);
                let preferences = rows.session_preferences(&entry.identity.id)?;
                let fix = tmt_core::binding::session::launch::session_learning_recovery(
                    &entry.identity,
                    &observed.driver,
                    &preferences,
                );
                recovery = Some(if fix.len() + "session not bound · ".len() <= 256 {
                    fix
                } else {
                    format!(
                        "Inspect the recipient and its next step: tmt identity show {}",
                        entry.identity.id
                    )
                });
            }
            Ok::<_, StorageError>((refusal, entry.map(|entry| entry.identity.name), recovery))
        })
        .transpose()?
        .unwrap_or_default();
    Ok(DeliveryFacts {
        recipient_start_refusal: context.0,
        recipient_name: context.1,
        recipient_session_recovery: context.2,
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
        answer: answer(db, attempt, now)?,
    })
}

/// A submitted reply, else the passthrough capture as it reads at `now`.
fn answer(
    db: &Connection,
    attempt: &RequestAttempt,
    now: u64,
) -> Result<ReturnedAnswer, StorageError> {
    if attempt.response_submitted_at_ms.is_some() {
        return Ok(ReturnedAnswer::Final);
    }
    Ok(match passthrough::capture(db, &attempt.request_id)? {
        Some(capture) => match capture.state_at(now) {
            CaptureState::Answered => ReturnedAnswer::Turn,
            CaptureState::NoAnswer => ReturnedAnswer::NoCapture {
                bound: capture.bound_at_ms.is_some(),
            },
            CaptureState::Pending | CaptureState::Bound => ReturnedAnswer::None,
        },
        None => ReturnedAnswer::None,
    })
}

fn hold(
    db: &Connection,
    identity: &str,
    request: &str,
) -> Result<Option<DigestHold>, StorageError> {
    db.query_row(
        "SELECT i.created_at_ms, c.state, c.opportunity, c.confirmed_at_ms FROM focus_items i
         LEFT JOIN focus_checklists c ON c.id = i.checklist_id
         WHERE i.identity_id = ?1 AND i.request_id = ?2 AND i.source = 'incoming'",
        params![identity, request],
        |row| {
            Ok(match row.get::<_, Option<String>>(1)?.as_deref() {
                None | Some("definitely_unsent") => DigestHold::Open {
                    since_ms: rows::u64_at(row, 0)?,
                },
                Some("claimed") => DigestHold::Claimed,
                Some("delivered")
                    if row.get::<_, Option<String>>(2)?.as_deref() == Some("idle")
                        && row.get::<_, Option<i64>>(3)?.is_none() =>
                {
                    DigestHold::Unconfirmed
                }
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
