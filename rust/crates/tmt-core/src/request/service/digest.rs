use super::super::focus::{
    self, FocusChecklist, FocusOpportunity, FocusPolicy, FocusPolicyView, FocusPolicyWrite,
    FocusRejection, FocusState,
};
use super::*;

fn focus_error<E>(reason: FocusRejection) -> RequestError<E> {
    RequestError::Focus(reason)
}

fn focus_identity<E>(
    records: &dyn RequestRecords<Error = E>,
    id: &str,
) -> Result<(), RequestError<E>> {
    if !crate::dispatch::canonical_id(id) {
        return Err(focus_error(FocusRejection::Invalid));
    }
    if !records.identity_is_active(id)? {
        return Err(focus_error(FocusRejection::IdentityUnavailable));
    }
    Ok(())
}

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    /// Uses the same first-final eligibility as reply admission, without renewing
    /// retention or acknowledging attention.
    pub fn focus_reply_context(
        &mut self,
        id: &str,
    ) -> Result<Option<(RequestContext, bool)>, RequestError<R::Error>> {
        let clock = &self.clock;
        self.repository.with_request_observation(|records| {
            let now = positive(clock())?;
            Ok(context(records, id, now)?.map(|context| {
                let replyable = responses::first_final_refusal(&context.attempt, now).is_none();
                (context, replyable)
            }))
        })
    }

    pub fn focus_result_context(
        &mut self,
        id: &str,
    ) -> Result<Option<FinalResponse>, RequestError<R::Error>> {
        let clock = &self.clock;
        self.repository.with_request_observation(|records| {
            let now = positive(clock())?;
            Ok(records
                .find_response(id)?
                .filter(|r| now < r.response_expires_at_ms))
        })
    }
    pub fn write_focus(
        &mut self,
        input: FocusPolicyWrite,
    ) -> Result<FocusPolicyView, RequestError<R::Error>> {
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            focus_identity(records, &input.identity_id)?;
            focus_identity(records, &input.owner_identity_id)?;
            focus_identity(records, &input.setter_identity_id)?;
            if input.until_ms > MAX_JS_SAFE_INTEGER
                || (input.until_ms != 0 && input.until_ms <= now)
            {
                return Err(focus_error(FocusRejection::Invalid));
            }
            let old = records.focus_policy(&input.identity_id)?;
            let revision = old.as_ref().map_or(0, |p| p.revision);
            if revision != input.expected_revision {
                return Err(focus_error(FocusRejection::Conflict));
            }
            if revision >= MAX_JS_SAFE_INTEGER {
                return Err(RequestError::RevisionExhausted);
            }
            let policy = FocusPolicy {
                identity_id: input.identity_id,
                revision: revision + 1,
                until_ms: input.until_ms,
                owner_identity_id: input.owner_identity_id,
                setter_identity_id: input.setter_identity_id,
            };
            records.write_focus_policy(&policy)?;
            policy_view(records, &policy.identity_id.clone(), Some(policy), now)
        })
    }

    pub fn focus_policies(
        &mut self,
        ids: &[String],
    ) -> Result<Vec<FocusPolicyView>, RequestError<R::Error>> {
        if ids.is_empty() || ids.len() > 256 {
            return Err(focus_error(FocusRejection::Invalid));
        }
        let clock = &self.clock;
        self.repository.with_request_observation(|records| {
            let now = positive(clock())?;
            ids.iter()
                .map(|id| {
                    focus_identity(records, id)?;
                    let policy = records.focus_policy(id)?;
                    policy_view(records, id, policy, now)
                })
                .collect()
        })
    }

    pub fn focus_checklist_items(
        &mut self,
        identity: &str,
        checklist: Option<&str>,
        after: u64,
        limit: u64,
    ) -> Result<(Vec<focus::FocusItem>, u64), RequestError<R::Error>> {
        if after > MAX_JS_SAFE_INTEGER || !(1..=128).contains(&limit) {
            return Err(focus_error(FocusRejection::Invalid));
        }
        let clock = &self.clock;
        self.repository.with_request_observation(|records| {
            let now = positive(clock())?;
            focus_identity(records, identity)?;
            if let Some(id) = checklist {
                let batch = records
                    .focus_checklist(id)?
                    .ok_or_else(|| focus_error(FocusRejection::StateInvalid))?;
                if batch.identity_id != identity {
                    return Err(focus_error(FocusRejection::AttemptMismatch));
                }
            }
            let (count, _) = records.focus_inventory(identity, checklist, after, now)?;
            let items = records.focus_items(identity, checklist, after, limit, now)?;
            Ok((items, count))
        })
    }

    /// The caller admits an opportunity; policy and sealed membership are rechecked
    /// in the writer transaction. A claimed record is never a replay lease.
    pub fn claim_focus_checklist(
        &mut self,
        identity: &str,
        id: String,
        token: String,
        opportunity: FocusOpportunity,
    ) -> Result<Option<FocusChecklist>, RequestError<R::Error>> {
        if !crate::dispatch::canonical_id(&id) || !crate::dispatch::canonical_id(&token) {
            return Err(focus_error(FocusRejection::Invalid));
        }
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            focus_identity(records, identity)?;
            if let Some(existing) = records.focus_checklist(&id)? {
                if existing.identity_id != identity || existing.attempt_token != token {
                    return Err(focus_error(FocusRejection::AttemptMismatch));
                }
                // Repeated claims return no transport permission, including a
                // crash whose external effect cannot be established.
                return Ok(None);
            }
            if records.active_focus_checklist(identity)?.is_some() {
                return Ok(None);
            }
            if opportunity == FocusOpportunity::Idle
                && records
                    .focus_policy(identity)?
                    .is_some_and(|p| p.active(now))
            {
                return Ok(None);
            }
            let (count, through_sequence) = records.focus_inventory(identity, None, 0, now)?;
            if count == 0 {
                return Ok(None);
            }
            let checklist = FocusChecklist {
                id,
                identity_id: identity.into(),
                attempt_token: token,
                through_sequence,
                state: FocusState::Claimed,
                created_at_ms: now,
            };
            records.create_focus_checklist(&checklist, now)?;
            Ok(Some(checklist))
        })
    }

    pub fn settle_focus_checklist(
        &mut self,
        identity: &str,
        id: &str,
        token: &str,
        state: FocusState,
    ) -> Result<bool, RequestError<R::Error>> {
        if state == FocusState::Claimed {
            return Err(focus_error(FocusRejection::Invalid));
        }
        self.repository.with_request_transaction(|records| {
            focus_identity(records, identity)?;
            let batch = records
                .focus_checklist(id)?
                .ok_or_else(|| focus_error(FocusRejection::StateInvalid))?;
            if batch.identity_id != identity || batch.attempt_token != token {
                return Err(focus_error(FocusRejection::AttemptMismatch));
            }
            if batch.state == state {
                return Ok(false);
            }
            if batch.state != FocusState::Claimed {
                return Err(focus_error(FocusRejection::StateInvalid));
            }
            records.settle_focus_checklist(&batch, state)?;
            Ok(true)
        })
    }

    pub fn request_delivery_policy(
        &mut self,
        id: &str,
    ) -> Result<focus::DeliveryPolicy, RequestError<R::Error>> {
        self.repository
            .with_request_observation(|records| records.delivery_policy(id).map_err(Into::into))
    }
}

pub(super) fn held_until<E>(
    records: &dyn RequestRecords<Error = E>,
    identity: Option<&str>,
    sender: Option<&str>,
    urgent: bool,
    now: u64,
) -> Result<Option<u64>, RequestError<E>> {
    let Some(id) = identity else {
        return Ok(None);
    };
    if !records.identity_is_active(id)? {
        return Ok(None);
    }
    Ok(records
        .focus_policy(id)?
        .filter(|p| p.holds(sender, urgent, now))
        .map(|p| p.until_ms))
}

pub(super) fn hold_incoming<E>(
    records: &mut dyn RequestRecords<Error = E>,
    attempt: &RequestAttempt,
    now: u64,
) -> Result<Option<u64>, RequestError<E>> {
    let policy = records.delivery_policy(&attempt.request_id)?;
    if !policy.automatic || !matches!(attempt.route, RequestRoute::Inbox { .. }) {
        return Ok(None);
    }
    let until = held_until(
        records,
        attempt.recipient_identity_id.as_deref(),
        attempt.originator.identity_id(),
        policy.urgent,
        now,
    )?;
    if until.is_some()
        && let Some(identity) = &attempt.recipient_identity_id
    {
        records.hold_focus_item(
            identity,
            &attempt.request_id,
            policy.kind,
            focus::FocusSource::Incoming,
            now,
        )?;
    }
    Ok(until)
}

fn policy_view<E>(
    records: &dyn RequestRecords<Error = E>,
    identity: &str,
    policy: Option<FocusPolicy>,
    now: u64,
) -> Result<FocusPolicyView, RequestError<E>> {
    let (mut held_count, _) = records.focus_inventory(identity, None, 0, now)?;
    let active_checklist = records.active_focus_checklist(identity)?;
    if let Some(batch) = &active_checklist {
        held_count += records
            .focus_inventory(identity, Some(&batch.id), 0, now)?
            .0;
    }
    Ok(FocusPolicyView {
        policy,
        active_checklist,
        held_count,
        observed_at_ms: now,
    })
}
