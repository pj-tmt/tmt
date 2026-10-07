use super::*;

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    /// Claim exactly one advisory wake while leaving the inbox request queued.
    /// A retained claim is an unknown outcome after process loss, never a retry lease.
    pub fn claim_wake(&mut self, request_id: &str) -> Result<WakeClaim, RequestError<R::Error>> {
        nonempty(request_id)?;
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let attempt = records
                .find_request(request_id)?
                .ok_or(RequestError::NotFound)?;
            let state = records
                .wake_state(request_id)?
                .ok_or(RequestError::NotFound)?;
            if state != WakeState::NotAttempted {
                return Ok(WakeClaim {
                    state,
                    claimed: false,
                    focus_until_ms: None,
                });
            }
            let RequestRoute::Inbox {
                recipient_identity_id,
            } = &attempt.route
            else {
                return Ok(WakeClaim {
                    state,
                    claimed: false,
                    focus_until_ms: None,
                });
            };
            if !records.delivery_policy(request_id)?.automatic
                || attempt.kind != RequestKind::Request
                || attempt.status != AttemptStatus::Queued
            {
                return Ok(WakeClaim {
                    state,
                    claimed: false,
                    focus_until_ms: None,
                });
            }
            let now = positive(clock())?;
            let focus_until_ms = if records.has_focus_item(
                recipient_identity_id,
                request_id,
                super::super::focus::FocusSource::Incoming,
            )? {
                Some(
                    records
                        .focus_policy(recipient_identity_id)?
                        .map_or(0, |p| p.until_ms),
                )
            } else {
                super::focus::hold_incoming(records, &attempt, now)?
            };
            if focus_until_ms.is_some() {
                return Ok(WakeClaim {
                    state,
                    claimed: false,
                    focus_until_ms,
                });
            }
            if !records.claim_wake(request_id)? {
                return Err(RequestError::StateInvalid);
            }
            let eligible = attempt.response_submitted_at_ms.is_none()
                && records.identity_is_active(recipient_identity_id)?
                && match attempt.room_id.as_deref() {
                    Some(room) => records.room_has_recipient(room, recipient_identity_id)?,
                    None => true,
                };
            if !eligible {
                if !records.settle_wake(request_id, WakeState::Unavailable)? {
                    return Err(RequestError::StateInvalid);
                }
                refund_unsent_preamble(records, &attempt, positive(clock())?)?;
                return Ok(WakeClaim {
                    state: WakeState::Unavailable,
                    claimed: false,
                    focus_until_ms: None,
                });
            }
            Ok(WakeClaim {
                state: WakeState::Claimed,
                claimed: true,
                focus_until_ms: None,
            })
        })
    }

    /// Recheck the accepted UUID and room after endpoint work, before pane input.
    pub fn wake_recipient_is_eligible(
        &mut self,
        request_id: &str,
        recipient_id: &str,
    ) -> Result<bool, RequestError<R::Error>> {
        nonempty(request_id)?;
        nonempty(recipient_id)?;
        self.repository.with_request_observation(|records| {
            let Some(attempt) = records.find_request(request_id)? else {
                return Ok(false);
            };
            if attempt.kind != RequestKind::Request
                || attempt.status != AttemptStatus::Queued
                || attempt.response_submitted_at_ms.is_some()
                || records.wake_state(request_id)? != Some(WakeState::Claimed)
                || !matches!(&attempt.route, RequestRoute::Inbox { recipient_identity_id } if recipient_identity_id == recipient_id)
                || !records.identity_is_active(recipient_id)?
            {
                return Ok(false);
            }
            match attempt.room_id.as_deref() {
                Some(room) => records.room_has_recipient(room, recipient_id).map_err(Into::into),
                None => Ok(true),
            }
        })
    }

    pub fn settle_wake(
        &mut self,
        request_id: &str,
        state: WakeState,
    ) -> Result<(), RequestError<R::Error>> {
        nonempty(request_id)?;
        if !matches!(
            state,
            WakeState::Sent | WakeState::Unavailable | WakeState::Uncertain
        ) {
            return Err(RequestError::StateInvalid);
        }
        self.repository.with_request_transaction(|records| {
            if records.settle_wake(request_id, state)? {
                Ok(())
            } else {
                Err(RequestError::StateInvalid)
            }
        })
    }

    /// A full request paste, unlike an advisory Office wake, satisfies recipient
    /// attention. It does not acknowledge the originator's eventual response.
    pub fn settle_request_delivery(
        &mut self,
        request_id: &str,
        state: WakeState,
    ) -> Result<(), RequestError<R::Error>> {
        nonempty(request_id)?;
        if !matches!(
            state,
            WakeState::Sent | WakeState::Unavailable | WakeState::Uncertain
        ) {
            return Err(RequestError::StateInvalid);
        }
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let attempt = records
                .find_request(request_id)?
                .ok_or(RequestError::NotFound)?;
            let RequestRoute::Inbox {
                recipient_identity_id,
            } = &attempt.route
            else {
                return Err(RequestError::StateInvalid);
            };
            if !records.settle_wake(request_id, state)? {
                return Err(RequestError::StateInvalid);
            }
            if state == WakeState::Sent {
                let attention = records
                    .find_recipient_attention(recipient_identity_id, request_id)?
                    .ok_or(RequestError::StateInvalid)?;
                records.acknowledge_recipient_revision(
                    recipient_identity_id,
                    request_id,
                    attention.revision,
                )?;
            } else if state == WakeState::Unavailable {
                refund_unsent_preamble(records, &attempt, positive(clock())?)?;
            }
            Ok(())
        })
    }
}

fn refund_unsent_preamble<E>(
    records: &mut dyn RequestRecords<Error = E>,
    attempt: &RequestAttempt,
    now: u64,
) -> Result<(), RequestError<E>> {
    if attempt.cadence_reserved {
        if !records.update_state(attempt, AttemptStatus::Queued, false, now, None)? {
            return Err(RequestError::StateInvalid);
        }
        if let Some(id) = &attempt.identity_id {
            let count = records.preamble_count(id)?;
            records.set_preamble_count(id, count.saturating_sub(1), now)?;
        }
    }
    Ok(())
}
