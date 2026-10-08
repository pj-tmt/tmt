use super::*;

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    pub fn withdraw_request(
        &mut self,
        originator: &str,
        request_id: &str,
        reason: &str,
    ) -> Result<WithdrawnRequest, RequestError<R::Error>> {
        if originator.is_empty()
            || request_id.is_empty()
            || reason.is_empty()
            || reason.len() > MAX_WITHDRAWAL_REASON_BYTES
        {
            return Err(RequestError::Withdrawal(WithdrawalRejection::InputInvalid));
        }
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            // Refusals do no housekeeping or acknowledgment. The submitted marker
            // remains authoritative even after retention prunes the final body.
            let attempt = records
                .find_request(request_id)?
                .filter(|attempt| now < attempt.retention_expires_at_ms)
                .ok_or(RequestError::Withdrawal(WithdrawalRejection::NotFound))?;
            if attempt.originator.identity_id() != Some(originator) {
                return Err(RequestError::Withdrawal(WithdrawalRejection::NotOriginator));
            }
            if attempt.kind != RequestKind::Request {
                return Err(RequestError::Withdrawal(WithdrawalRejection::NotRequired));
            }
            if attempt.response_submitted_at_ms.is_some()
                || records.find_response(request_id)?.is_some()
            {
                return Err(RequestError::Withdrawal(WithdrawalRejection::AlreadyFinal));
            }
            if let Some(withdrawal) = attempt.withdrawal {
                if withdrawal.reason != reason {
                    return Err(RequestError::Withdrawal(WithdrawalRejection::Conflict));
                }
                return Ok(WithdrawnRequest {
                    request_id: request_id.into(),
                    withdrawal,
                    changed: false,
                });
            }
            let withdrawal = Withdrawal {
                reason: reason.into(),
                withdrawn_at_ms: now,
            };
            if !records.withdraw_request(&attempt.attempt_id, &withdrawal)? {
                return Err(RequestError::StateInvalid);
            }
            if attempt.wait_active {
                records.release_wait(&attempt.attempt_id, now)?;
            }
            Ok(WithdrawnRequest {
                request_id: request_id.into(),
                withdrawal,
                changed: true,
            })
        })
    }
}
