use super::*;
use crate::request::{
    delivery::{DeliveryState, SendNowOutcome, SendNowRejection, delivery_view},
    digest::{DeliveryMode, DeliveryPolicy, DigestSource},
};

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    /// Let the originator, or a person, send a held or waiting message now (#4378). Only the
    /// Digest hold is skipped: the caller still runs the ordinary wake, so readiness, approval
    /// and channel guards apply. A message that already reached the agent is reported, never
    /// resent, and one that may have reached it is refused.
    pub fn send_now(
        &mut self,
        request_id: &str,
        caller_id: &str,
    ) -> Result<SendNowOutcome, RequestError<R::Error>> {
        nonempty(request_id)?;
        nonempty(caller_id)?;
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            let attempt = records
                .find_request(request_id)?
                .filter(|attempt| now < attempt.retention_expires_at_ms)
                .ok_or(RequestError::NotFound)?;
            let reject = |reason| Err(RequestError::SendNow(reason));
            if attempt.originator.identity_id() != Some(caller_id)
                && !records.identity_is_user(caller_id)?
            {
                return reject(SendNowRejection::NotAllowed);
            }
            let RequestRoute::Inbox {
                recipient_identity_id,
            } = &attempt.route
            else {
                return reject(SendNowRejection::NotApplicable);
            };
            let policy = records.delivery_policy(request_id)?;
            if attempt.kind != RequestKind::Request
                || policy.mode != DeliveryMode::Request
                || !policy.automatic
            {
                return reject(SendNowRejection::NotApplicable);
            }
            if attempt.response_submitted_at_ms.is_some()
                || attempt.withdrawal.is_some()
                || now >= attempt.expires_at_ms
            {
                return reject(SendNowRejection::Closed);
            }
            let facts = records.delivery_facts(&attempt, now)?;
            let view = delivery_view(&facts, now);
            match view.state {
                DeliveryState::Delivered | DeliveryState::Working => {
                    return Ok(SendNowOutcome::Already(view.state));
                }
                DeliveryState::Unconfirmed => return reject(SendNowRejection::Unconfirmed),
                _ if !view.can_send_now => {
                    return reject(SendNowRejection::NotAvailable(view.reason));
                }
                _ => {}
            }
            records.release_digest_item(
                recipient_identity_id,
                request_id,
                DigestSource::Incoming,
            )?;
            if !policy.urgent {
                records.write_delivery_policy(
                    request_id,
                    &DeliveryPolicy {
                        urgent: true,
                        ..policy
                    },
                )?;
            }
            if facts.wake == WakeState::Unavailable {
                records.reopen_wake(request_id)?;
            }
            Ok(SendNowOutcome::Release {
                recipient_identity_id: recipient_identity_id.clone(),
            })
        })
    }
}
