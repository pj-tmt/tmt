use super::*;

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    pub fn queue(&mut self, attempt_id: &str) -> Result<(), RequestError<R::Error>> {
        nonempty(attempt_id)?;
        let clock = &self.clock;
        let queued = self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            queue_records(records, attempt_id, now)
        })?;
        if !queued {
            return Err(RequestError::NotFound);
        }
        Ok(())
    }

    pub fn begin_send(&mut self, attempt_id: &str) -> Result<(), RequestError<R::Error>> {
        nonempty(attempt_id)?;
        let clock = &self.clock;
        let expired = self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            let attempt = records
                .find_attempt(attempt_id)?
                .ok_or(RequestError::NotFound)?;
            if now >= attempt.retention_expires_at_ms {
                return Err(RequestError::Expired);
            }
            if attempt.status != AttemptStatus::Prepared {
                return Err(RequestError::StateInvalid);
            }
            if now >= attempt.expires_at_ms {
                if attempt.wait_active {
                    records.release_wait(attempt_id, now)?;
                }
                if !fail_unsent(records, &attempt, now, None)? {
                    return Err(RequestError::StateInvalid);
                }
                // Commit refund/release before reporting expiry to the caller.
                return Ok(true);
            }
            if !records.update_state(
                &attempt,
                AttemptStatus::Sending,
                attempt.cadence_reserved,
                now,
                None,
            )? {
                return Err(RequestError::StateInvalid);
            }
            Ok(false)
        })?;
        if expired {
            return Err(RequestError::Expired);
        }
        Ok(())
    }

    pub fn settle(
        &mut self,
        attempt_id: &str,
        outcome: Settlement,
    ) -> Result<(), RequestError<R::Error>> {
        nonempty(attempt_id)?;
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            let attempt = records
                .find_attempt(attempt_id)?
                .ok_or(RequestError::NotFound)?;
            if now >= attempt.retention_expires_at_ms
                && matches!(
                    attempt.status,
                    AttemptStatus::Prepared | AttemptStatus::Sending
                )
            {
                return Err(RequestError::Expired);
            }
            let cannot_prove_unsent = (attempt.status == AttemptStatus::Sending
                && now >= attempt.expires_at_ms)
                || (attempt.response_submitted_at_ms.is_some()
                    && matches!(
                        attempt.status,
                        AttemptStatus::Sending | AttemptStatus::Uncertain
                    ));
            let effective = match outcome {
                Settlement::Sent => AttemptStatus::Sent,
                Settlement::Held if matches!(attempt.route, RequestRoute::Remote { .. }) => {
                    AttemptStatus::Queued
                }
                Settlement::Held => return Err(RequestError::StateInvalid),
                Settlement::Uncertain => AttemptStatus::Uncertain,
                Settlement::DefinitelyFailed if cannot_prove_unsent => AttemptStatus::Uncertain,
                Settlement::DefinitelyFailed => AttemptStatus::DefinitelyFailed,
            };
            if matches!(
                attempt.status,
                AttemptStatus::Sent
                    | AttemptStatus::Queued
                    | AttemptStatus::Uncertain
                    | AttemptStatus::DefinitelyFailed
            ) {
                return if attempt.status == effective {
                    Ok(())
                } else {
                    Err(RequestError::StateInvalid)
                };
            }
            if effective != AttemptStatus::DefinitelyFailed
                && attempt.status != AttemptStatus::Sending
            {
                return Err(RequestError::StateInvalid);
            }
            let horizon = Some(
                attempt
                    .retention_expires_at_ms
                    .max(deadline(now, METADATA_SETTLEMENT_FLOOR_MS)?),
            );
            let changed = if effective == AttemptStatus::DefinitelyFailed {
                fail_unsent(records, &attempt, now, horizon)?
            } else {
                records.update_state(&attempt, effective, attempt.cadence_reserved, now, horizon)?
            };
            if !changed {
                return Err(RequestError::StateInvalid);
            }
            Ok(())
        })
    }

    pub fn release_wait(&mut self, attempt_id: &str) -> Result<(), RequestError<R::Error>> {
        nonempty(attempt_id)?;
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            records.release_wait(attempt_id, positive(clock())?)?;
            Ok(())
        })
    }
}

pub(super) fn queue_records<E>(
    records: &mut dyn RequestRecords<Error = E>,
    attempt_id: &str,
    now: u64,
) -> Result<bool, RequestError<E>> {
    let attempt = records
        .find_attempt(attempt_id)?
        .ok_or(RequestError::NotFound)?;
    let RequestRoute::Inbox {
        recipient_identity_id,
    } = &attempt.route
    else {
        return Err(RequestError::StateInvalid);
    };
    if attempt.status != AttemptStatus::Prepared || now >= attempt.expires_at_ms {
        return Err(RequestError::StateInvalid);
    }
    if !records.identity_is_active(recipient_identity_id)? {
        if attempt.wait_active && !records.release_wait(attempt_id, now)? {
            return Err(RequestError::StateInvalid);
        }
        if !fail_unsent(records, &attempt, now, None)? {
            return Err(RequestError::StateInvalid);
        }
        return Ok(false);
    }
    // A passthrough is typed text, not a request to the agent: it publishes no attention.
    if records.delivery_policy(&attempt.request_id)?.mode
        != super::super::digest::DeliveryMode::Passthrough
    {
        let revision = reserve_recipient_revision(records, recipient_identity_id)?;
        records.set_recipient_attention_revision(&attempt.request_id, revision)?;
    }
    let horizon = Some(
        attempt
            .retention_expires_at_ms
            .max(deadline(now, METADATA_SETTLEMENT_FLOOR_MS)?),
    );
    if !records.update_state(
        &attempt,
        AttemptStatus::Queued,
        attempt.cadence_reserved,
        now,
        horizon,
    )? {
        return Err(RequestError::StateInvalid);
    }
    Ok(true)
}
