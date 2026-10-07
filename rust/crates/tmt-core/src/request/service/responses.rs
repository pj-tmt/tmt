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
            if let Some(refusal) = first_final_refusal(&attempt, now) {
                return Err(RequestError::Response(refusal));
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
        self.read(|records, now| response_lookup(records, request_id, now))
    }

    /// CLI selection only: observers and receipt-based replies keep exact IDs.
    /// Selection and body retrieval use one retention sample and transaction.
    pub fn get_response_by_prefix(
        &mut self,
        input: &str,
    ) -> Result<(String, ResponseLookup), RequestError<R::Error>> {
        if input.is_empty() {
            return Err(RequestError::Response(ResponseRejection::InputInvalid));
        }
        self.read(|records, now| {
            // Historical service clients may have stored non-UUID IDs. Exact
            // retained IDs take precedence and keep their original spelling.
            if records.find_request(input)?.is_some() || records.find_response(input)?.is_some() {
                return Ok((input.into(), response_lookup(records, input, now)?));
            }
            let Some((lower, upper)) = prefix_range(input)? else {
                return Ok((input.into(), ResponseLookup::Unavailable));
            };
            let matches = records.retained_request_ids(&lower, &upper, now, 5)?;
            match matches.ids.as_slice() {
                [] => Ok((input.into(), ResponseLookup::Unavailable)),
                [id] => Ok((id.clone(), response_lookup(records, id, now)?)),
                _ => Err(RequestError::ResultSelection(
                    ResultSelectionRejection::Ambiguous(matches),
                )),
            }
        })
    }
}

pub(super) fn response_lookup<E>(
    records: &mut dyn RequestRecords<Error = E>,
    request_id: &str,
    now: u64,
) -> Result<ResponseLookup, RequestError<E>> {
    if let Some(response) = records
        .find_response(request_id)?
        .filter(|response| now < response.response_expires_at_ms)
    {
        return Ok(ResponseLookup::Available(Box::new(response)));
    }
    if let Some(attempt) = records
        .find_request(request_id)?
        .filter(|attempt| now < attempt.retention_expires_at_ms)
    {
        if let Some(withdrawal) = attempt.withdrawal {
            return Ok(ResponseLookup::Withdrawn(withdrawal));
        }
        if attempt.kind == RequestKind::Announcement {
            return Ok(ResponseLookup::NotRequired);
        }
    }
    Ok(ResponseLookup::Unavailable)
}

pub(super) fn prefix_range<E>(input: &str) -> Result<Option<(String, String)>, RequestError<E>> {
    let prefix = input.strip_prefix("req_").unwrap_or(input);
    // A complete prefixed ID retains exact lookup semantics, including case.
    if input.starts_with("req_") && prefix.len() == 36 {
        return Ok(None);
    }
    if prefix.is_empty() {
        return Err(RequestError::ResultSelection(
            ResultSelectionRejection::PrefixTooShort,
        ));
    }
    // Non-UUID identifiers remain exact-only for historical service clients.
    if prefix.len() > 36
        || !prefix.bytes().enumerate().all(|(i, byte)| {
            if [8, 13, 18, 23].contains(&i) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Ok(None);
    }
    if prefix.bytes().filter(u8::is_ascii_hexdigit).count() < 8 {
        return Err(RequestError::ResultSelection(
            ResultSelectionRejection::PrefixTooShort,
        ));
    }
    let lower = format!("req_{}", prefix.to_ascii_lowercase());
    // Valid prefixes are ASCII; advancing the last byte gives the exclusive
    // lexicographic successor, including a trailing hyphen or an f nibble.
    let mut upper = lower.as_bytes().to_vec();
    *upper.last_mut().expect("nonempty prefix") += 1;
    Ok(Some((
        lower,
        String::from_utf8(upper).expect("ASCII successor"),
    )))
}

/// Why `attempt` cannot take a first final at `now`, if it cannot. Final
/// acceptance and the open-request read share this one rule, so the inbox
/// never offers a request that an answer would refuse.
pub(super) fn first_final_refusal(attempt: &RequestAttempt, now: u64) -> Option<ResponseRejection> {
    if attempt.withdrawal.is_some() {
        return Some(ResponseRejection::Withdrawn);
    }
    if attempt.kind == RequestKind::Announcement {
        return Some(ResponseRejection::NotRequired);
    }
    if attempt.response_submitted_at_ms.is_some()
        || response_deadline_passed(now, attempt.prepared_at_ms, attempt.expires_at_ms)
    {
        return Some(ResponseRejection::Expired);
    }
    if !crate::request::inbox::ANSWERABLE.contains(&attempt.status) {
        return Some(ResponseRejection::StateInvalid);
    }
    None
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

#[cfg(test)]
mod tests {
    use super::prefix_range;

    #[test]
    fn uuid_prefix_range_has_an_exclusive_ascii_successor() {
        for (input, lower, upper) in [
            ("12345678", "req_12345678", "req_12345679"),
            ("req_FFFFFFFF", "req_ffffffff", "req_fffffffg"),
            ("ffffffff-", "req_ffffffff-", "req_ffffffff."),
            ("12345678-9abc", "req_12345678-9abc", "req_12345678-9abd"),
        ] {
            assert_eq!(
                prefix_range::<()>(input).unwrap(),
                Some((lower.into(), upper.into()))
            );
        }
        for input in [
            "request-old",
            "req_12345678-0000-4000-8000-000000000000",
            "req_xyz",
            "12345678%",
        ] {
            assert_eq!(prefix_range::<()>(input).unwrap(), None);
        }
        assert!(prefix_range::<()>("1234567").is_err());
        assert!(prefix_range::<()>("req_").is_err());
    }
}
