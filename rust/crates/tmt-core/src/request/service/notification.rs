use super::super::notification::{
    HintKind, NoticeContext, NotificationPolicy, NotificationRecord, OriginatorHint,
};
use super::*;

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    /// Notice context is a read: the original prompt, indexed ID selection and,
    /// on request, the retained final body through the same lookup as `tmt result`.
    /// It never acknowledges attention or changes retention. A final that cannot
    /// be read yields no body, so a notice degrades to its preview instead of
    /// losing the rest of its context.
    pub fn notice_context(
        &mut self,
        request_id: &str,
        with_reply: bool,
    ) -> Result<Option<NoticeContext>, RequestError<R::Error>> {
        nonempty(request_id)?;
        self.read(|records, now| {
            let Some(context) = context(records, request_id, now)? else {
                return Ok(None);
            };
            let mut result_id = request_id.to_owned();
            if let Some(uuid) = request_id
                .strip_prefix("req_")
                .filter(|id| crate::dispatch::canonical_id(id))
            {
                let short = &uuid[..8];
                let (lower, upper) =
                    responses::prefix_range(short)?.expect("canonical UUID prefix");
                let matches = records.retained_request_ids(&lower, &upper, now, 2)?;
                // Result selection preserves exact legacy IDs before prefix lookup.
                // A same-spelled retained exact ID must not shadow this command.
                let exact = records.find_request(short)?;
                let shadowed = exact.is_some_and(|attempt| now < attempt.retention_expires_at_ms);
                if !shadowed && matches.ids.as_slice() == [request_id] {
                    result_id = short.into();
                }
            }
            let reply = if with_reply {
                match responses::response_lookup(records, request_id, now) {
                    Ok(ResponseLookup::Available(response)) => Some(response.body),
                    _ => None,
                }
            } else {
                None
            };
            Ok(Some(NoticeContext {
                recipient_id: context.attempt.recipient_identity_id,
                prompt: match context.prompt {
                    RequestPrompt::Retained(prompt) => Some(prompt.message),
                    RequestPrompt::Expired { .. } | RequestPrompt::Unavailable => None,
                },
                reply,
                result_id,
            }))
        })
    }

    /// Called before external delivery. Missing rows mean explicit queue-only,
    /// anonymous or pre-migration requests, and never acquire notification rights.
    pub fn enable_notifications(
        &mut self,
        request_id: &str,
        policy: NotificationPolicy,
    ) -> Result<(), RequestError<R::Error>> {
        nonempty(request_id)?;
        if policy.timeout_ms == 0 || policy.timeout_ms > 86_400_000 {
            return Err(RequestError::Invalid("Invalid notification timeout."));
        }
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            // A valid short deadline may elapse while acquiring the writer
            // lock. It is a timeout to observe, not a rejected request.
            if policy.deadline_ms == 0 || policy.deadline_ms > MAX_JS_SAFE_INTEGER {
                return Err(RequestError::Invalid("Invalid notification deadline."));
            }
            let attempt = records
                .find_request(request_id)?
                .ok_or(RequestError::NotFound)?;
            if attempt.originator.identity_id().is_none()
                || attempt.kind != RequestKind::Request
                || !matches!(
                    attempt.status,
                    AttemptStatus::Prepared | AttemptStatus::Queued
                )
                || attempt.response_submitted_at_ms.is_some()
                || now >= attempt.retention_expires_at_ms
            {
                return Err(RequestError::StateInvalid);
            }
            if let Some(existing) = records.notification(request_id)? {
                return if existing.policy == policy {
                    Ok(())
                } else {
                    Err(RequestError::StateInvalid)
                };
            }
            records.create_notification(request_id, &policy)?;
            Ok(())
        })
    }

    pub fn notification(
        &mut self,
        request_id: &str,
    ) -> Result<Option<NotificationRecord>, RequestError<R::Error>> {
        nonempty(request_id)?;
        self.repository.with_request_observation(|records| {
            records.notification(request_id).map_err(Into::into)
        })
    }

    /// Releasing an unsuccessful waiter transfers a racing final to the hint
    /// path. A delivered full response consumes that path without acknowledging X.
    pub fn finish_wait(
        &mut self,
        attempt_id: &str,
        delivered: bool,
    ) -> Result<Option<OriginatorHint>, RequestError<R::Error>> {
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            let mut attempt = records
                .find_attempt(attempt_id)?
                .ok_or(RequestError::NotFound)?;
            if !attempt.wait_active {
                return Ok(None);
            }
            records.release_wait(attempt_id, now)?;
            attempt.wait_active = false;
            if delivered {
                if let Some(mut value) = records.notification(&attempt.request_id)? {
                    value.observed = true;
                    records.set_notification(&attempt.request_id, &value)?;
                }
                return Ok(None);
            }
            if records.find_response(&attempt.request_id)?.is_some() {
                claim(records, &attempt, HintKind::Reply, now)
            } else {
                Ok(None)
            }
        })
    }

    pub fn claim_timeout_hint(
        &mut self,
        request_id: &str,
    ) -> Result<Option<OriginatorHint>, RequestError<R::Error>> {
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            let attempt = records
                .find_request(request_id)?
                .ok_or(RequestError::NotFound)?;
            if records.find_response(request_id)?.is_some() {
                return Ok(None);
            }
            claim(records, &attempt, HintKind::Timeout, now)
        })
    }

    pub fn settle_hint(
        &mut self,
        hint: &OriginatorHint,
        outcome: WakeState,
    ) -> Result<(), RequestError<R::Error>> {
        if !matches!(
            outcome,
            WakeState::Sent | WakeState::Unavailable | WakeState::Uncertain
        ) {
            return Err(RequestError::StateInvalid);
        }
        self.repository.with_request_transaction(|records| {
            let mut value = records
                .notification(&hint.request_id)?
                .ok_or(RequestError::NotFound)?;
            let state = match hint.kind {
                HintKind::Reply => &mut value.reply,
                HintKind::Timeout => &mut value.timeout,
            };
            if *state != WakeState::Claimed {
                return Err(RequestError::StateInvalid);
            }
            *state = outcome;
            records.set_notification(&hint.request_id, &value)?;
            Ok(())
        })
    }
}

pub(super) fn claim<E>(
    records: &mut dyn RequestRecords<Error = E>,
    attempt: &RequestAttempt,
    kind: HintKind,
    now: u64,
) -> Result<Option<OriginatorHint>, RequestError<E>> {
    let Some(originator) = attempt.originator.identity_id() else {
        return Ok(None);
    };
    let Some(mut value) = records.notification(&attempt.request_id)? else {
        return Ok(None);
    };
    if now >= attempt.retention_expires_at_ms || value.observed || attempt.wait_active {
        return Ok(None);
    }
    if kind == HintKind::Timeout && now < value.policy.deadline_ms {
        return Ok(None);
    }
    let state = match kind {
        HintKind::Reply => &mut value.reply,
        HintKind::Timeout => &mut value.timeout,
    };
    if *state != WakeState::NotAttempted {
        return Ok(None);
    }
    *state = if records.identity_is_active(originator)? {
        WakeState::Claimed
    } else {
        WakeState::Unavailable
    };
    let claimed = *state == WakeState::Claimed;
    records.set_notification(&attempt.request_id, &value)?;
    Ok(claimed.then(|| OriginatorHint {
        request_id: attempt.request_id.clone(),
        originator_id: originator.into(),
        recipient_id: attempt.recipient_identity_id.clone(),
        kind,
        timeout_ms: value.policy.timeout_ms,
    }))
}
