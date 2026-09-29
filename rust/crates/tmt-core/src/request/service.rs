//! Short transactional use cases. The clock is sampled after acquiring the
//! write lock; callers generate IDs and freeze configuration before entry.

mod answers;
mod attention;
mod history;
mod lifecycle;
mod notification;
mod responses;
mod wake;

use super::*;
use crate::{exact_text::validate_exact_text, limits::MAX_JS_SAFE_INTEGER, retention::*};

pub struct RequestService<'a, R, C> {
    repository: &'a mut R,
    clock: C,
}

impl<'a, R: RequestRepository, C: Fn() -> u64> RequestService<'a, R, C> {
    pub fn new(repository: &'a mut R, clock: C) -> Self {
        Self { repository, clock }
    }

    pub fn prepare(
        &mut self,
        input: PrepareRequest,
        attempt_id: String,
        retention_days: u64,
    ) -> Result<PreparedRequest, RequestError<R::Error>> {
        validate_prepare(&input, &attempt_id, retention_days)?;
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            prepare_records(
                records,
                positive(clock())?,
                input,
                attempt_id,
                retention_days,
            )
        })
    }

    /// Prepare and publish inbox attention in one transaction. Pane sends retain
    /// their separate prepare/send/settle lifecycle for external tmux effects.
    pub fn enqueue(
        &mut self,
        input: PrepareRequest,
        attempt_id: String,
        retention_days: u64,
    ) -> Result<PreparedRequest, RequestError<R::Error>> {
        validate_prepare(&input, &attempt_id, retention_days)?;
        if !matches!(&input.route, RequestRoute::Inbox { .. }) {
            return Err(RequestError::StateInvalid);
        }
        let clock = &self.clock;
        let (prepared, queued) = self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            let prepared = prepare_records(records, now, input, attempt_id, retention_days)?;
            let queued = lifecycle::queue_records(records, &prepared.attempt_id, now)?;
            Ok::<_, RequestError<R::Error>>((prepared, queued))
        })?;
        if queued {
            Ok(prepared)
        } else {
            Err(RequestError::NotFound)
        }
    }

    pub fn cleanup(&mut self) -> Result<(), RequestError<R::Error>> {
        self.read(|_, _| Ok(()))
    }

    pub fn get_attempt(
        &mut self,
        attempt_id: &str,
    ) -> Result<Option<RequestAttempt>, RequestError<R::Error>> {
        nonempty(attempt_id)?;
        self.read(|records, now| {
            Ok(records
                .find_attempt(attempt_id)?
                .filter(|a| now < a.retention_expires_at_ms))
        })
    }

    pub fn get_context(
        &mut self,
        request_id: &str,
    ) -> Result<Option<RequestContext>, RequestError<R::Error>> {
        nonempty(request_id)?;
        self.read(|records, now| context(records, request_id, now))
    }

    fn read<T>(
        &mut self,
        operation: impl FnOnce(
            &mut dyn RequestRecords<Error = R::Error>,
            u64,
        ) -> Result<T, RequestError<R::Error>>,
    ) -> Result<T, RequestError<R::Error>> {
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            cleanup_records(records, now)?;
            operation(records, now)
        })
    }
}

fn validate_prepare<E>(
    input: &PrepareRequest,
    attempt_id: &str,
    retention_days: u64,
) -> Result<(), RequestError<E>> {
    nonempty(&input.request_id)?;
    nonempty(attempt_id)?;
    validate_exact_text(input.message.as_bytes()).map_err(|_| RequestError::InputTooLarge)?;
    route_valid(&input.route)?;
    if input
        .room_id
        .as_deref()
        .is_some_and(|id| !crate::dispatch::canonical_id(id))
    {
        return Err(RequestError::Invalid("Invalid request room UUID."));
    }
    if input.kind == RequestKind::Announcement
        && (input.wait
            || input.preamble.is_some()
            || !matches!(input.route, RequestRoute::Inbox { .. }))
    {
        return Err(RequestError::Invalid(
            "Announcements require an inbox route without a response wait or pane preamble.",
        ));
    }
    positive(input.expires_at_ms)?;
    if !valid_retention_days(retention_days) {
        return Err(RequestError::Invalid("Invalid retention days."));
    }
    if let Some(id) = input.originator.identity_id() {
        nonempty(id)?;
    }
    if let Some(id) = &input.recipient_identity_id {
        nonempty(id)?;
    }
    if let RequestRoute::Inbox {
        recipient_identity_id,
    } = &input.route
        && input.recipient_identity_id.as_deref() != Some(recipient_identity_id)
    {
        return Err(RequestError::Invalid(
            "Inbox route must match the recipient identity.",
        ));
    }
    if let Some(preamble) = &input.preamble {
        nonempty(&preamble.identity_id)?;
        positive(preamble.every)?;
    }
    Ok(())
}

fn prepare_records<E>(
    records: &mut dyn RequestRecords<Error = E>,
    now: u64,
    input: PrepareRequest,
    attempt_id: String,
    retention_days: u64,
) -> Result<PreparedRequest, RequestError<E>> {
    if let Some(room_id) = &input.room_id {
        let recipient = input
            .recipient_identity_id
            .as_deref()
            .ok_or(RequestError::Invalid(
                "A room-scoped request requires an identified recipient.",
            ))?;
        if !records.room_has_recipient(room_id, recipient)? {
            return Err(RequestError::RoomRecipientNotMember);
        }
    }
    let expires = input
        .expires_at_ms
        .max(deadline(now, REQUEST_MIN_EXPIRY_MS)?);
    let prompt_expiry = retention_deadline(now, retention_days)
        .ok_or(RequestError::Invalid("Invalid prompt deadline."))?;
    let horizon = prompt_expiry
        .max(deadline(now, RESPONSE_ACCEPTANCE_WINDOW_MS)?)
        .max(deadline(expires, METADATA_SETTLEMENT_FLOOR_MS)?);
    cleanup_records(records, now)?;
    if records.find_response(&input.request_id)?.is_some() {
        return Err(RequestError::AlreadyExists);
    }
    let revision = match input.originator.identity_id() {
        Some(id) => reserve_revision(records, id)?,
        None => 0,
    };
    let previous_request_id = records.find_active_request(&input.route)?;
    let mut inject = false;
    if let Some(preamble) = &input.preamble {
        let count = records.preamble_count(&preamble.identity_id)?;
        if count >= MAX_JS_SAFE_INTEGER {
            return Err(RequestError::CounterExhausted);
        }
        inject = count % preamble.every == 0;
        records.set_preamble_count(&preamble.identity_id, count + 1, now)?;
    }
    let attempt = RequestAttempt {
        kind: input.kind,
        room_id: input.room_id,
        attempt_id: attempt_id.clone(),
        request_id: input.request_id.clone(),
        originator: input.originator,
        recipient_identity_id: input.recipient_identity_id,
        nonce: None,
        identity_id: input.preamble.as_ref().map(|p| p.identity_id.clone()),
        route: input.route,
        wait_active: input.wait,
        status: AttemptStatus::Prepared,
        preamble_every: input.preamble.as_ref().map(|p| p.every),
        inject_preamble: inject,
        cadence_reserved: input.preamble.is_some(),
        prepared_at_ms: now,
        sending_at_ms: None,
        settled_at_ms: None,
        wait_released_at_ms: None,
        response_submitted_at_ms: None,
        expires_at_ms: expires,
        retention_days,
        retention_expires_at_ms: horizon,
    };
    let prompt = StoredPrompt {
        message_bytes: input.message.len() as u64,
        message: input.message,
        expires_at_ms: prompt_expiry,
    };
    records.create_attempt(&attempt, &prompt, revision)?;
    Ok(PreparedRequest {
        attempt_id,
        request_id: input.request_id,
        inject_preamble: inject,
        previous_request_id,
    })
}

fn context<E>(
    records: &mut dyn RequestRecords<Error = E>,
    request_id: &str,
    now: u64,
) -> Result<Option<RequestContext>, RequestError<E>> {
    let Some(raw) = records.find_context(request_id)? else {
        return Ok(None);
    };
    if now >= raw.attempt.retention_expires_at_ms {
        return Ok(None);
    }
    let prompt = match (raw.expires_at_ms, raw.message, raw.message_bytes) {
        (None, _, _) => RequestPrompt::Unavailable,
        (Some(expiry), Some(message), Some(message_bytes)) if now < expiry => {
            RequestPrompt::Retained(StoredPrompt {
                message,
                message_bytes,
                expires_at_ms: expiry,
            })
        }
        (Some(expires_at_ms), _, _) => RequestPrompt::Expired { expires_at_ms },
    };
    Ok(Some(RequestContext {
        attempt: raw.attempt,
        prompt,
    }))
}

fn nonempty<E>(value: &str) -> Result<(), RequestError<E>> {
    if value.is_empty() {
        return Err(RequestError::Invalid(
            "Request identifiers must be non-empty.",
        ));
    }
    Ok(())
}

fn positive<E>(value: u64) -> Result<u64, RequestError<E>> {
    if value == 0 || value > MAX_JS_SAFE_INTEGER {
        return Err(RequestError::Invalid("Expected a positive safe integer."));
    }
    Ok(value)
}

fn deadline<E>(anchor: u64, delta: u64) -> Result<u64, RequestError<E>> {
    checked_deadline(anchor, delta).ok_or(RequestError::Invalid(
        "Request deadline is outside the supported range.",
    ))
}

fn endpoint_valid<E>(endpoint: &RequestEndpoint) -> Result<(), RequestError<E>> {
    // Historical correlation fences are not fresh tmux observations. Preserve
    // the service's non-empty/safe-integer contract instead of imposing today's
    // server UUID or wire-format rules on retained requests during upgrade.
    nonempty(&endpoint.server.server_id)?;
    nonempty(&endpoint.server.socket_path)?;
    nonempty(&endpoint.server.server_start_time)?;
    nonempty(&endpoint.pane_id)?;
    positive(endpoint.server.server_pid)?;
    positive(endpoint.pane_pid)?;
    Ok(())
}

fn route_valid<E>(route: &RequestRoute) -> Result<(), RequestError<E>> {
    match route {
        RequestRoute::Pane(endpoint) => endpoint_valid(endpoint),
        RequestRoute::Inbox {
            recipient_identity_id,
        } => nonempty(recipient_identity_id),
    }
}

fn reserve_revision<E>(
    records: &mut dyn RequestRecords<Error = E>,
    id: &str,
) -> Result<u64, RequestError<E>> {
    let current = records.attention_counter(id)?;
    let value = current.unwrap_or(0);
    if value >= MAX_JS_SAFE_INTEGER {
        return Err(RequestError::RevisionExhausted);
    }
    records.set_attention_counter(id, current, value + 1)?;
    Ok(value + 1)
}

fn reserve_recipient_revision<E>(
    records: &mut dyn RequestRecords<Error = E>,
    id: &str,
) -> Result<u64, RequestError<E>> {
    let recipient_current = records.recipient_attention_counter(id)?;
    let revision = reserve_revision(records, id)?;
    records.set_recipient_attention_counter(id, recipient_current, revision)?;
    Ok(revision)
}

fn fail_unsent<E>(
    records: &mut dyn RequestRecords<Error = E>,
    attempt: &RequestAttempt,
    now: u64,
    horizon: Option<u64>,
) -> Result<bool, RequestError<E>> {
    let changed = records.update_state(
        attempt,
        AttemptStatus::DefinitelyFailed,
        false,
        now,
        horizon,
    )?;
    if changed
        && attempt.cadence_reserved
        && let Some(id) = &attempt.identity_id
    {
        let count = records.preamble_count(id)?;
        if count > MAX_JS_SAFE_INTEGER {
            return Err(RequestError::Invalid("Invalid cadence count."));
        }
        records.set_preamble_count(id, count.saturating_sub(1), now)?;
    }
    Ok(changed)
}

fn cleanup_records<E>(
    records: &mut dyn RequestRecords<Error = E>,
    now: u64,
) -> Result<(), RequestError<E>> {
    for attempt in records.expired_attempts(now, CLEANUP_BATCH_SIZE)? {
        if attempt.wait_active {
            records.release_wait(&attempt.attempt_id, now)?;
        }
        match attempt.status {
            AttemptStatus::Prepared => {
                fail_unsent(records, &attempt, now, None)?;
            }
            AttemptStatus::Sending => {
                records.update_state(
                    &attempt,
                    AttemptStatus::Uncertain,
                    attempt.cadence_reserved,
                    now,
                    None,
                )?;
            }
            _ => {}
        }
    }
    records.clear_expired_prompts(now, CLEANUP_BATCH_SIZE)?;
    records.delete_expired_responses(now, CLEANUP_BATCH_SIZE)?;
    records.delete_retained(
        now,
        now.saturating_sub(METADATA_SETTLEMENT_FLOOR_MS),
        CLEANUP_BATCH_SIZE,
    )?;
    Ok(())
}
