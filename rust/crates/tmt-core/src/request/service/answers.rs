use super::{responses::first_final_refusal, *};
use crate::request::{
    attention::AttentionRejection,
    correlation,
    inbox::{AnswerRejection, INBOX_LIMIT, INBOX_MAX_LIMIT, OpenPage, OpenQuery, OpenRequest},
};

fn open_items<E>(
    records: &mut dyn RequestRecords<Error = E>,
    query: &OpenQuery,
    now: u64,
) -> Result<Vec<OpenRequest>, RequestError<E>> {
    Ok(records
        .list_open_requests(query, now)?
        .into_iter()
        // Storage only narrows; the acceptance rule decides.
        .filter(|row| first_final_refusal(&row.attention.attempt, now).is_none())
        .map(|row| {
            let attempt = row.attention.attempt;
            OpenRequest {
                request_id: attempt.request_id,
                room_id: attempt.room_id,
                originator: attempt.originator,
                prepared_at_ms: attempt.prepared_at_ms,
                delivery: attempt.status,
                preview: row.preview,
            }
        })
        .collect())
}

fn query(recipient: &str, originator: Option<&str>, limit: u64, now: u64) -> OpenQuery {
    OpenQuery {
        recipient_identity_id: recipient.into(),
        originator_identity_id: originator.map(str::to_owned),
        window_start_ms: now.saturating_sub(RESPONSE_ACCEPTANCE_WINDOW_MS),
        limit,
    }
}

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    /// Requests addressed to `recipient` that still accept a first final,
    /// oldest first. Reading never acknowledges.
    pub fn open_requests(
        &mut self,
        recipient: &str,
        originator: Option<&str>,
        limit: Option<u64>,
    ) -> Result<OpenPage, RequestError<R::Error>> {
        nonempty(recipient)?;
        let limit = limit.unwrap_or(INBOX_LIMIT);
        if limit == 0 || limit > INBOX_MAX_LIMIT || originator.is_some_and(str::is_empty) {
            return Err(RequestError::Invalid("Invalid inbox limit or originator."));
        }
        self.read(|records, now| {
            let mut items =
                open_items(records, &query(recipient, originator, limit + 1, now), now)?;
            let more = items.len() as u64 > limit;
            items.truncate(limit as usize);
            Ok(OpenPage { items, more })
        })
    }

    /// Selects the request `originator` is waiting on `recipient` for and
    /// derives its proof from the recorded attempt and route, as the receipt
    /// `talk` gave the recipient. The proof never leaves the process; the
    /// caller submits it through [`Self::submit_response_with_hint`].
    ///
    /// Without `request`, exactly one open request from `originator` must
    /// exist. An explicit `request` must be addressed to `recipient`, and by
    /// `originator` when one is given; it is the only way to answer an
    /// anonymous originator. Submission then decides between acceptance, an
    /// identical retry and a conflict. Returns the request's originator.
    pub fn answer_target(
        &mut self,
        recipient: &str,
        originator: Option<&str>,
        request: Option<&str>,
    ) -> Result<(String, ResponseProof, Originator), RequestError<R::Error>> {
        nonempty(recipient)?;
        originator.map(nonempty).transpose()?;
        request.map(nonempty).transpose()?;
        self.read(|records, now| {
            let request_id = match (request, originator) {
                (Some(id), _) => id.to_owned(),
                (None, None) => {
                    return Err(RequestError::Invalid(
                        "Name the originator or the request to answer.",
                    ));
                }
                (None, Some(originator)) => {
                    let query = query(recipient, Some(originator), INBOX_LIMIT, now);
                    let mut open = open_items(records, &query, now)?;
                    match open.len() {
                        0 => return Err(RequestError::Answer(AnswerRejection::NotWaiting)),
                        1 => open.remove(0).request_id,
                        _ => return Err(RequestError::Answer(AnswerRejection::Ambiguous(open))),
                    }
                }
            };
            let attempt = records
                .find_request(&request_id)?
                .filter(|attempt| {
                    now < attempt.retention_expires_at_ms
                        && attempt.recipient_identity_id.as_deref() == Some(recipient)
                        && originator.is_none_or(|id| attempt.originator.identity_id() == Some(id))
                })
                .ok_or(RequestError::Attention(AttentionRejection::NotFound))?;
            let token = correlation::response_token(
                &attempt.request_id,
                &attempt.attempt_id,
                &attempt.route,
            );
            Ok((
                attempt.request_id,
                ResponseProof::Compact(token),
                attempt.originator,
            ))
        })
    }
}
