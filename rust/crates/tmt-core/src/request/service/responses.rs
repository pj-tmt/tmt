use super::*;

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    pub fn submit_response(
        &mut self,
        input: SubmitResponse,
    ) -> Result<FinalResponse, RequestError<R::Error>> {
        self.accept_response(input, None, false)
            .map(|(response, _)| response)
    }

    pub fn submit_response_with_hint(
        &mut self,
        input: SubmitResponse,
        gone_waiter: Option<&super::super::notification::NotificationPolicy>,
    ) -> Result<
        (
            FinalResponse,
            Option<super::super::notification::OriginatorHint>,
        ),
        RequestError<R::Error>,
    > {
        self.accept_response(input, gone_waiter, true)
    }

    fn accept_response(
        &mut self,
        input: SubmitResponse,
        gone_waiter: Option<&super::super::notification::NotificationPolicy>,
        claim_hint: bool,
    ) -> Result<
        (
            FinalResponse,
            Option<super::super::notification::OriginatorHint>,
        ),
        RequestError<R::Error>,
    > {
        let invalid_proof = match &input.proof {
            ResponseProof::Recorded {
                attempt_id,
                endpoint,
            } => attempt_id.is_empty() || endpoint_valid::<R::Error>(endpoint).is_err(),
            ResponseProof::Compact(_) => false,
        };
        if input.request_id.is_empty() || invalid_proof {
            return Err(RequestError::Response(ResponseRejection::InputInvalid));
        }
        validate_exact_text(input.body.as_bytes())
            .map_err(|_| RequestError::Response(ResponseRejection::InputTooLarge))?;
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            // No housekeeping on a rejected submission: even unrelated rows
            // must remain unchanged. Retained finals are authoritative for retries.
            if let Some(existing) = records.find_response(&input.request_id)? {
                validate_proof(
                    &existing.request_id,
                    &existing.attempt_id,
                    &existing.route,
                    &input.proof,
                )?;
                if now >= existing.response_expires_at_ms {
                    return Err(RequestError::Response(ResponseRejection::Expired));
                }
                if input.body != existing.body {
                    return Err(RequestError::Response(ResponseRejection::Conflict));
                }
                return Ok((existing, None));
            }
            let mut attempt = records
                .find_request(&input.request_id)?
                .ok_or(RequestError::Response(ResponseRejection::RequestNotFound))?;
            validate_proof(
                &attempt.request_id,
                &attempt.attempt_id,
                &attempt.route,
                &input.proof,
            )?;
            if attempt.kind == RequestKind::Announcement {
                return Err(RequestError::Response(ResponseRejection::NotRequired));
            }
            if attempt.response_submitted_at_ms.is_some()
                || response_deadline_passed(now, attempt.prepared_at_ms, attempt.expires_at_ms)
            {
                return Err(RequestError::Response(ResponseRejection::Expired));
            }
            if !matches!(
                attempt.status,
                AttemptStatus::Sending
                    | AttemptStatus::Sent
                    | AttemptStatus::Queued
                    | AttemptStatus::Uncertain
            ) {
                return Err(RequestError::Response(ResponseRejection::StateInvalid));
            }
            // Only an accepted first final may release a proven-dead waiter.
            // Bad receipts, conflicting retries and observation reads never mutate it.
            if attempt.wait_active
                && let Some(expected) = gone_waiter
                && expected.waiter.is_some()
                && records
                    .notification(&input.request_id)?
                    .is_some_and(|value| value.policy == *expected)
            {
                records.release_wait(&attempt.attempt_id, now)?;
                attempt.wait_active = false;
            }
            let response = FinalResponse {
                request_id: input.request_id,
                attempt_id: attempt.attempt_id.clone(),
                route: attempt.route.clone(),
                body_bytes: input.body.len() as u64,
                body: input.body,
                submitted_at_ms: now,
                response_expires_at_ms: retention_deadline(now, attempt.retention_days)
                    .ok_or(RequestError::Invalid("Invalid response deadline."))?,
            };
            records.create_response(&response)?;
            if let Some(id) = attempt.originator.identity_id() {
                let revision = reserve_revision(records, id)?;
                records.set_attention_revision(&response.request_id, revision)?;
            }
            let hint = if claim_hint {
                notification::claim(
                    records,
                    &attempt,
                    super::super::notification::HintKind::Reply,
                    now,
                )?
            } else {
                None
            };
            Ok((response, hint))
        })
    }

    pub fn get_response(
        &mut self,
        request_id: &str,
    ) -> Result<ResponseLookup, RequestError<R::Error>> {
        if request_id.is_empty() {
            return Err(RequestError::Response(ResponseRejection::InputInvalid));
        }
        self.read(|records, now| {
            if let Some(response) = records
                .find_response(request_id)?
                .filter(|response| now < response.response_expires_at_ms)
            {
                return Ok(ResponseLookup::Available(Box::new(response)));
            }
            if records.find_request(request_id)?.is_some_and(|attempt| {
                attempt.kind == RequestKind::Announcement && now < attempt.retention_expires_at_ms
            }) {
                return Ok(ResponseLookup::NotRequired);
            }
            Ok(ResponseLookup::Unavailable)
        })
    }
}

fn validate_proof<E>(
    request_id: &str,
    attempt_id: &str,
    route: &RequestRoute,
    proof: &ResponseProof,
) -> Result<(), RequestError<E>> {
    match proof {
        ResponseProof::Recorded {
            attempt_id: supplied_id,
            endpoint: supplied_endpoint,
        } => {
            if attempt_id != supplied_id {
                return Err(RequestError::Response(ResponseRejection::AttemptMismatch));
            }
            if !matches!(route, RequestRoute::Pane(endpoint) if endpoint == supplied_endpoint) {
                return Err(RequestError::Response(ResponseRejection::RecipientMismatch));
            }
        }
        ResponseProof::Compact(token) => {
            if *token != correlation::response_token(request_id, attempt_id, route) {
                return Err(RequestError::Response(ResponseRejection::ReceiptMismatch));
            }
        }
    }
    Ok(())
}
