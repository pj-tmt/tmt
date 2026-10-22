//! The turn output a passthrough earned (#4336). The canonical request transaction creates and
//! reads a capture; the hook's runtime-observation transaction binds it to the turn that the
//! typed text started and settles it when that turn ends. The text carries no request id, so a
//! pending capture is matched to a turn by the prompt the provider reported.
use super::*;
use tmt_core::{
    request::passthrough::{
        BIND_WINDOW_MS, CaptureState, PassthroughAnswer, PassthroughCapture, same_prompt,
    },
    responses::FinalText,
};

pub(super) fn start_capture(
    db: &Connection,
    request_id: &str,
    delivered_at_ms: u64,
) -> Result<(), StorageError> {
    db.execute(
        "INSERT INTO request_passthrough(request_id,delivered_at_ms,state) VALUES(?1,?2,'pending')
         ON CONFLICT(request_id) DO NOTHING",
        params![
            request_id,
            checked_i64(delivered_at_ms, "Passthrough delivery time")?
        ],
    )
    .map_err(|error| classify(error, "Start passthrough capture"))?;
    Ok(())
}

pub(in crate::storage) fn capture(
    db: &Connection,
    request_id: &str,
) -> Result<Option<PassthroughCapture>, StorageError> {
    db.query_row(
        "SELECT delivered_at_ms,state,bound_session,bound_turn,bound_at_ms,answer_text,
                answer_truncated,answer_provider,answer_ended_ms,settled_at_ms
         FROM request_passthrough WHERE request_id=?",
        [request_id],
        |row| {
            let state = CaptureState::parse(&row.get::<_, String>(1)?)
                .ok_or(rusqlite::Error::InvalidQuery)?;
            let answer = match row.get::<_, Option<String>>(5)? {
                Some(text) => Some(PassthroughAnswer {
                    text,
                    truncated: row.get::<_, i64>(6)? == 1,
                    provider: row.get(7)?,
                    ended_ms: rows::u64_at(row, 8)?,
                }),
                None => None,
            };
            Ok(PassthroughCapture {
                delivered_at_ms: rows::u64_at(row, 0)?,
                state,
                bound_session: row.get(2)?,
                bound_turn: row.get(3)?,
                bound_at_ms: rows::optional_u64_at(row, 4)?,
                answer,
                settled_at_ms: rows::optional_u64_at(row, 9)?,
            })
        },
    )
    .optional()
    .map_err(|error| classify(error, "Read passthrough capture"))
}

/// A capture bound to a session that is no longer the agent's can never see its turn end.
fn release_other_sessions(
    db: &Connection,
    identity: &str,
    session: &str,
    now: u64,
) -> Result<(), StorageError> {
    db.execute(
        "UPDATE request_passthrough SET state='no_answer',settled_at_ms=?3
         WHERE state='bound' AND bound_session<>?2
           AND request_id IN (SELECT request_id FROM request_attempts WHERE recipient_identity_id=?1)",
        params![identity, session, checked_i64(now, "Passthrough settlement time")?],
    )
    .map_err(|error| classify(error, "Release passthrough captures of an earlier session"))?;
    Ok(())
}

/// A main turn started with `prompt`: the oldest pending passthrough that `identity` was typed,
/// still inside its binding window, whose text is that prompt, is bound to this turn.
/// Identical pending texts bind first in, first out. Returns whether one was bound.
pub(in crate::storage) fn bind(
    db: &Connection,
    identity: &str,
    session: &str,
    turn: Option<&str>,
    prompt: &str,
    now: u64,
) -> Result<bool, StorageError> {
    release_other_sessions(db, identity, session, now)?;
    let now_i64 = checked_i64(now, "Passthrough binding time")?;
    if let Some(turn) = turn {
        // The same provider turn is never bound twice.
        let bound: i64 = db
            .query_row(
                "SELECT count(*) FROM request_passthrough
                 WHERE state IN ('bound','answered','no_answer') AND bound_session=?1 AND bound_turn=?2",
                params![session, turn],
                |row| row.get(0),
            )
            .map_err(|error| classify(error, "Check the bound passthrough turn"))?;
        if bound > 0 {
            return Ok(false);
        }
    }
    let floor = checked_i64(
        now.saturating_sub(BIND_WINDOW_MS),
        "Passthrough binding window",
    )?;
    let mut pending = db
        .prepare(
            "SELECT p.request_id,a.message_text FROM request_passthrough p
             JOIN request_attempts a ON a.request_id=p.request_id
             WHERE a.recipient_identity_id=?1 AND p.state='pending' AND p.delivered_at_ms>=?2
             ORDER BY p.delivered_at_ms,p.request_id",
        )
        .map_err(|error| classify(error, "Prepare pending passthrough captures"))?;
    let mut candidates = pending
        .query(params![identity, floor])
        .map_err(|error| classify(error, "Read pending passthrough captures"))?;
    while let Some(row) = candidates
        .next()
        .map_err(|error| classify(error, "Read pending passthrough captures"))?
    {
        let (request, text): (String, Option<String>) = (
            row.get(0)
                .map_err(|error| classify(error, "Read pending passthrough captures"))?,
            row.get(1)
                .map_err(|error| classify(error, "Read pending passthrough captures"))?,
        );
        if text.is_some_and(|text| same_prompt(&text, prompt)) {
            db.execute(
                "UPDATE request_passthrough SET state='bound',bound_session=?2,bound_turn=?3,bound_at_ms=?4
                 WHERE request_id=?1 AND state='pending'",
                params![request, session, turn, now_i64],
            )
            .map_err(|error| classify(error, "Bind passthrough capture"))?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// A main turn ended. Every capture bound to it settles: with the turn's final message when
/// one was read, otherwise without an answer. A final is never guessed from elsewhere.
pub(in crate::storage) fn settle_turn(
    db: &Connection,
    identity: &str,
    session: &str,
    turn: Option<&str>,
    final_text: Option<(&FinalText, &str)>,
    now: u64,
) -> Result<bool, StorageError> {
    release_other_sessions(db, identity, session, now)?;
    let now_i64 = checked_i64(now, "Passthrough settlement time")?;
    let in_turn = "state='bound' AND bound_session=?2 AND (?3 IS NULL OR bound_turn IS NULL OR bound_turn=?3)
         AND request_id IN (SELECT request_id FROM request_attempts WHERE recipient_identity_id=?1)";
    // The provider is whichever driver read the final; `recent_responses` bounds it the same way.
    let changed = match final_text {
        Some((text, provider)) => db.execute(
            &format!(
                "UPDATE request_passthrough SET state='answered',answer_text=?4,answer_truncated=?5,
                 answer_provider=?6,answer_ended_ms=?7,settled_at_ms=?7 WHERE {in_turn}"
            ),
            params![
                identity,
                session,
                turn,
                text.text(),
                i64::from(text.truncated()),
                provider,
                now_i64
            ],
        ),
        None => db.execute(
            &format!(
                "UPDATE request_passthrough SET state='no_answer',settled_at_ms=?4 WHERE {in_turn}"
            ),
            params![identity, session, turn, now_i64],
        ),
    }
    .map_err(|error| classify(error, "Settle passthrough capture"))?;
    Ok(changed > 0)
}
