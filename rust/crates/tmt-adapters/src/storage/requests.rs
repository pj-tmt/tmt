//! SQLite-backed durable request state.
//!
//! Request policy stays in `tmt-core`; this module only decodes rows and applies
//! the bounded SQL mutations while the invocation-owned immediate transaction is
//! held.

mod attention;
mod focus;
mod history;
mod notification;
mod reply_batch;
mod rows;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use tmt_core::request::{
    AttemptStatus, FinalResponse, RawRequestContext, RequestAttempt, RequestEndpoint,
    RequestIdMatches, RequestRecords, RequestRepository, RequestRoute, StoredPrompt, WakeState,
};

use super::{
    Storage, StorageError, StorageErrorCode, errors::classify,
    identities::with_immediate_transaction,
};

struct RequestRows<'a>(&'a Connection);

/// A caller-owned SQLite transaction can compose canonical request operations
/// with its own receipt. This adapter never opens or commits a nested transaction.
pub(super) struct TransactionRequests<'a, 'connection>(pub &'a Transaction<'connection>);

impl RequestRepository for TransactionRequests<'_, '_> {
    type Error = StorageError;

    fn with_request_transaction<T, E: From<Self::Error>>(
        &mut self,
        operation: impl FnOnce(&mut dyn RequestRecords<Error = Self::Error>) -> Result<T, E>,
    ) -> Result<T, E> {
        operation(&mut RequestRows(self.0))
    }
}

const DELETE_RETAINED_SQL: &str = "DELETE FROM request_attempts
             WHERE attempt_id IN (
                 SELECT attempt_id
                 FROM request_attempts INDEXED BY request_attempts_retention_horizon
                 WHERE wait_active = 0
                   AND status IN ('sent', 'queued', 'uncertain', 'definitely_failed')
                   AND settled_at_ms IS NOT NULL
                   AND settled_at_ms <= ?
                   AND retention_expires_at_ms <= ?
                   AND NOT EXISTS (
                       SELECT 1 FROM request_responses
                       WHERE request_responses.attempt_id = request_attempts.attempt_id
                         AND request_responses.response_expires_at_ms > ?
                   )
                 ORDER BY retention_expires_at_ms, attempt_id
                 LIMIT ?
             )";

const RETAINED_REQUEST_IDS_SQL: &str = "SELECT request_id FROM request_attempts
    WHERE request_id >= ?1 AND request_id < ?2 AND retention_expires_at_ms > ?3
    ORDER BY request_id LIMIT ?4";
const RETAINED_REQUEST_IDS_COUNT_SQL: &str = "SELECT COUNT(*) FROM request_attempts
    WHERE request_id >= ?1 AND request_id < ?2 AND retention_expires_at_ms > ?3";

fn incoming_watermark_sql(scoped: bool) -> String {
    let (recipient_index, response_index, scope) = if scoped {
        (
            "request_attempts_room_recipient_attention",
            "request_attempts_room_response_attention",
            "room_id = ?3",
        )
    } else {
        (
            "request_attempts_recipient_attention",
            "request_attempts_response_attention",
            "?3 IS NULL",
        )
    };
    format!(
        "SELECT MAX(
             COALESCE((
               SELECT recipient_attention_revision FROM request_attempts
               INDEXED BY {recipient_index}
               JOIN request_recipient_attention_identities AS state
                 ON state.identity_id = request_attempts.recipient_identity_id
               WHERE route_kind = 'inbox' AND status = 'queued'
                 AND {scope}
                 AND recipient_identity_id = ?1 AND retention_expires_at_ms > ?2
                 AND recipient_attention_revision > recipient_attention_acknowledged_revision
                 AND recipient_attention_revision > state.acknowledged_through
               ORDER BY recipient_attention_revision DESC, request_id DESC
               LIMIT 1
             ), 0),
             COALESCE((
               SELECT attention_revision FROM request_attempts
               INDEXED BY {response_index}
               JOIN request_attention_identities AS state
                 ON state.identity_id = request_attempts.originator_identity_id
               WHERE originator_identity_id = ?1 AND retention_expires_at_ms > ?2
                 AND {scope}
                 AND response_submitted_at_ms IS NOT NULL
                 AND attention_revision > attention_acknowledged_revision
                 AND attention_revision > state.acknowledged_through
               ORDER BY attention_revision DESC, request_id DESC
               LIMIT 1
             ), 0)
           )"
    )
}

fn endpoint_args(endpoint: &RequestEndpoint) -> Result<[rusqlite::types::Value; 6], StorageError> {
    Ok([
        endpoint.server.server_id.clone().into(),
        endpoint.server.socket_path.clone().into(),
        checked_i64(endpoint.server.server_pid, "Server PID")?.into(),
        endpoint.server.server_start_time.clone().into(),
        endpoint.pane_id.clone().into(),
        checked_i64(endpoint.pane_pid, "Pane PID")?.into(),
    ])
}

fn checked_i64(value: u64, label: &str) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| {
        StorageError::new(
            StorageErrorCode::Unknown,
            format!("{label} is outside the supported range"),
        )
    })
}

fn checked_limit(value: u64) -> Result<i64, StorageError> {
    checked_i64(value, "Cleanup batch limit").and_then(|value| {
        if value > 0 {
            Ok(value)
        } else {
            Err(StorageError::new(
                StorageErrorCode::Unknown,
                "Cleanup batch limit must be positive",
            ))
        }
    })
}

fn checked_now(value: u64, label: &str) -> Result<i64, StorageError> {
    checked_i64(value, label).and_then(|value| {
        if value > 0 {
            Ok(value)
        } else {
            Err(StorageError::new(
                StorageErrorCode::Unknown,
                format!("{label} is outside the supported range"),
            ))
        }
    })
}

impl RequestRecords for RequestRows<'_> {
    type Error = StorageError;

    fn focus_policy(
        &self,
        id: &str,
    ) -> Result<Option<tmt_core::request::focus::FocusPolicy>, Self::Error> {
        focus::policy(self.0, id)
    }
    fn write_focus_policy(
        &mut self,
        p: &tmt_core::request::focus::FocusPolicy,
    ) -> Result<(), Self::Error> {
        focus::write_policy(self.0, p)
    }
    fn delivery_policy(
        &self,
        id: &str,
    ) -> Result<tmt_core::request::focus::DeliveryPolicy, Self::Error> {
        focus::delivery_policy(self.0, id)
    }
    fn write_delivery_policy(
        &mut self,
        id: &str,
        p: &tmt_core::request::focus::DeliveryPolicy,
    ) -> Result<(), Self::Error> {
        focus::write_delivery_policy(self.0, id, p)
    }
    fn hold_focus_item(
        &mut self,
        id: &str,
        request: &str,
        kind: tmt_core::request::focus::FocusKind,
        source: tmt_core::request::focus::FocusSource,
        now: u64,
    ) -> Result<(), Self::Error> {
        focus::hold(self.0, id, request, kind, source, now)
    }
    fn has_focus_item(
        &self,
        id: &str,
        request: &str,
        source: tmt_core::request::focus::FocusSource,
    ) -> Result<bool, Self::Error> {
        focus::has_item(self.0, id, request, source)
    }
    fn focus_inventory(
        &self,
        id: &str,
        batch: Option<&str>,
        after: u64,
        now: u64,
    ) -> Result<(u64, u64), Self::Error> {
        focus::inventory(self.0, id, batch, after, now)
    }
    fn focus_items(
        &self,
        id: &str,
        batch: Option<&str>,
        after: u64,
        limit: u64,
        now: u64,
    ) -> Result<Vec<tmt_core::request::focus::FocusItem>, Self::Error> {
        focus::items(self.0, id, batch, after, limit, now)
    }
    fn focus_checklist(
        &self,
        id: &str,
    ) -> Result<Option<tmt_core::request::focus::FocusChecklist>, Self::Error> {
        focus::checklist(self.0, id)
    }
    fn active_focus_checklist(
        &self,
        id: &str,
    ) -> Result<Option<tmt_core::request::focus::FocusChecklist>, Self::Error> {
        focus::active(self.0, id)
    }
    fn create_focus_checklist(
        &mut self,
        b: &tmt_core::request::focus::FocusChecklist,
        now: u64,
    ) -> Result<(), Self::Error> {
        focus::create_checklist(self.0, b, now)
    }
    fn settle_focus_checklist(
        &mut self,
        b: &tmt_core::request::focus::FocusChecklist,
        state: tmt_core::request::focus::FocusState,
    ) -> Result<(), Self::Error> {
        focus::settle(self.0, b, state)
    }

    fn notification(
        &self,
        request_id: &str,
    ) -> Result<Option<tmt_core::request::notification::NotificationRecord>, Self::Error> {
        notification::read(self.0, request_id)
    }

    fn create_notification(
        &mut self,
        request_id: &str,
        policy: &tmt_core::request::notification::NotificationPolicy,
    ) -> Result<(), Self::Error> {
        notification::insert_notification(self.0, request_id, policy)
    }

    fn set_notification(
        &mut self,
        request_id: &str,
        value: &tmt_core::request::notification::NotificationRecord,
    ) -> Result<(), Self::Error> {
        notification::write(self.0, request_id, value)
    }

    fn list_request_history(
        &self,
        query: &tmt_core::request::history::HistoryQuery,
        limit: u64,
        now_ms: u64,
    ) -> Result<Vec<tmt_core::request::history::HistoryRecord>, Self::Error> {
        history::list_request_history(self.0, query, limit, now_ms)
    }

    fn find_request_history(
        &self,
        request_id: &str,
    ) -> Result<Option<tmt_core::request::attention::AttentionRecord>, Self::Error> {
        history::find_request_history(self.0, request_id)
    }

    fn list_open_requests(
        &self,
        query: &tmt_core::request::inbox::OpenQuery,
        now_ms: u64,
    ) -> Result<Vec<tmt_core::request::history::HistoryRecord>, Self::Error> {
        history::list_open_requests(self.0, query, now_ms)
    }

    fn room_has_recipient(&self, room_id: &str, identity_id: &str) -> Result<bool, Self::Error> {
        Ok(super::room::read_room(self.0, room_id)?
            .is_some_and(|room| room.member_ids.iter().any(|id| id == identity_id)))
    }

    fn find_attempt(&self, attempt_id: &str) -> Result<Option<RequestAttempt>, Self::Error> {
        self.0
            .query_row(
                &format!(
                    "SELECT {} FROM request_attempts WHERE attempt_id = ?",
                    rows::ATTEMPT_COLUMNS
                ),
                [attempt_id],
                rows::attempt_row,
            )
            .optional()
            .map_err(|error| classify(error, "Find request attempt"))
    }

    fn find_request(&self, request_id: &str) -> Result<Option<RequestAttempt>, Self::Error> {
        self.0
            .query_row(
                &format!(
                    "SELECT {} FROM request_attempts WHERE request_id = ?",
                    rows::ATTEMPT_COLUMNS
                ),
                [request_id],
                rows::attempt_row,
            )
            .optional()
            .map_err(|error| classify(error, "Find request"))
    }

    fn retained_request_ids(
        &self,
        lower: &str,
        upper: &str,
        now_ms: u64,
        limit: u64,
    ) -> Result<RequestIdMatches, Self::Error> {
        let now = checked_now(now_ms, "Request-ID lookup clock")?;
        let limit = checked_limit(limit)?;
        let sample_limit = limit.checked_add(1).ok_or_else(|| {
            StorageError::new(
                StorageErrorCode::Unknown,
                "Request-ID sample limit overflow",
            )
        })?;
        let mut statement = self
            .0
            .prepare(RETAINED_REQUEST_IDS_SQL)
            .map_err(|error| classify(error, "Prepare retained request-ID range"))?;
        let mut ids = statement
            .query_map(params![lower, upper, now, sample_limit], |row| row.get(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<String>>>())
            .map_err(|error| classify(error, "Read retained request-ID range"))?;
        let total = if ids.len() as u64 > limit as u64 {
            self.0
                .query_row(
                    RETAINED_REQUEST_IDS_COUNT_SQL,
                    params![lower, upper, now],
                    |row| row.get::<_, i64>(0),
                )
                // SQLite COUNT is nonnegative and fits its signed integer.
                .map(|count| count as u64)
                .map_err(|error| classify(error, "Count retained request-ID range"))?
        } else {
            ids.len() as u64
        };
        ids.truncate(limit as usize);
        Ok(RequestIdMatches { ids, total })
    }

    fn wake_state(&self, request_id: &str) -> Result<Option<WakeState>, Self::Error> {
        self.0
            .query_row(
                "SELECT wake_state FROM request_attempts WHERE request_id = ?",
                [request_id],
                |row| match row.get::<_, String>(0)?.as_str() {
                    "not_attempted" => Ok(WakeState::NotAttempted),
                    "claimed" => Ok(WakeState::Claimed),
                    "sent" => Ok(WakeState::Sent),
                    "unavailable" => Ok(WakeState::Unavailable),
                    "uncertain" => Ok(WakeState::Uncertain),
                    _ => Err(rusqlite::Error::InvalidQuery),
                },
            )
            .optional()
            .map_err(|error| classify(error, "Read request wake state"))
    }

    fn claim_wake(&mut self, request_id: &str) -> Result<bool, Self::Error> {
        self.0
            .execute(
                "UPDATE request_attempts SET wake_state = 'claimed'
                 WHERE request_id = ? AND route_kind = 'inbox' AND status = 'queued'
                   AND request_kind = 'request' AND wake_state = 'not_attempted'",
                [request_id],
            )
            .map(|count| count == 1)
            .map_err(|error| classify(error, "Claim request wake"))
    }

    fn settle_wake(&mut self, request_id: &str, state: WakeState) -> Result<bool, Self::Error> {
        self.0
            .execute(
                "UPDATE request_attempts SET wake_state = ?
                 WHERE request_id = ? AND wake_state = 'claimed'",
                params![state.as_str(), request_id],
            )
            .map(|count| count == 1)
            .map_err(|error| classify(error, "Settle request wake"))
    }

    fn find_context(&self, request_id: &str) -> Result<Option<RawRequestContext>, Self::Error> {
        self.0
            .query_row(
                &format!(
                    "SELECT {}, message_text, message_bytes, message_expires_at_ms
                     FROM request_attempts WHERE request_id = ?",
                    rows::ATTEMPT_COLUMNS
                ),
                [request_id],
                rows::context_row,
            )
            .optional()
            .map_err(|error| classify(error, "Find request context"))
    }

    fn find_response(&self, request_id: &str) -> Result<Option<FinalResponse>, Self::Error> {
        self.0
            .query_row(
                &format!(
                    "SELECT {} FROM request_responses WHERE request_id = ?",
                    rows::RESPONSE_COLUMNS
                ),
                [request_id],
                rows::response_row,
            )
            .optional()
            .map_err(|error| classify(error, "Find request response"))
    }

    fn find_attention(
        &self,
        identity_id: &str,
        request_id: &str,
    ) -> Result<Option<tmt_core::request::attention::AttentionRecord>, Self::Error> {
        attention::find_attention(self.0, identity_id, request_id)
    }

    fn list_attention(
        &self,
        identity_id: &str,
        after: u64,
        limit: u64,
        now_ms: u64,
    ) -> Result<Vec<tmt_core::request::attention::AttentionRecord>, Self::Error> {
        attention::list_attention(self.0, identity_id, after, limit, now_ms)
    }

    fn list_response_attention(
        &self,
        identity_id: &str,
        room_id: Option<&str>,
        after: u64,
        limit: u64,
        now_ms: u64,
    ) -> Result<Vec<tmt_core::request::attention::AttentionRecord>, Self::Error> {
        attention::list_response_attention(self.0, identity_id, room_id, after, limit, now_ms)
    }

    fn identity_is_active(&self, identity_id: &str) -> Result<bool, Self::Error> {
        self.0
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM identities WHERE id = ? AND retired_at_ms IS NULL
                 )",
                [identity_id],
                |row| row.get(0),
            )
            .map_err(|error| classify(error, "Check active request recipient"))
    }

    fn acknowledge_revision(
        &mut self,
        identity_id: &str,
        request_id: &str,
        revision: u64,
    ) -> Result<bool, Self::Error> {
        attention::acknowledge_revision(self.0, identity_id, request_id, revision)
    }

    fn acknowledge_through(&mut self, identity_id: &str, latest: u64) -> Result<bool, Self::Error> {
        attention::acknowledge_through(self.0, identity_id, latest)
    }

    fn find_recipient_attention(
        &self,
        identity_id: &str,
        request_id: &str,
    ) -> Result<Option<tmt_core::request::attention::AttentionRecord>, Self::Error> {
        attention::find_recipient_attention(self.0, identity_id, request_id)
    }

    fn list_recipient_attention(
        &self,
        identity_id: &str,
        room_id: Option<&str>,
        after: u64,
        limit: u64,
        now_ms: u64,
    ) -> Result<Vec<tmt_core::request::attention::AttentionRecord>, Self::Error> {
        attention::list_recipient_attention(self.0, identity_id, room_id, after, limit, now_ms)
    }

    fn acknowledge_recipient_revision(
        &mut self,
        identity_id: &str,
        request_id: &str,
        revision: u64,
    ) -> Result<bool, Self::Error> {
        attention::acknowledge_recipient_revision(self.0, identity_id, request_id, revision)
    }

    fn acknowledge_recipient_through(
        &mut self,
        identity_id: &str,
        latest: u64,
    ) -> Result<bool, Self::Error> {
        attention::acknowledge_recipient_through(self.0, identity_id, latest)
    }

    fn find_active_request(&self, route: &RequestRoute) -> Result<Option<String>, Self::Error> {
        let (sql, args) = match route {
            RequestRoute::Pane(endpoint) => (
                "SELECT request_id FROM request_attempts WHERE route_kind = 'pane'
                 AND server_id = ? AND socket_path = ? AND server_pid = ?
                 AND server_start_time = ? AND pane_id = ? AND pane_pid = ?
                 AND COALESCE(host, 'tmux') = ?
                 AND wait_active = 1 ORDER BY prepared_at_ms, attempt_id LIMIT 1",
                endpoint_args(endpoint)?
                    .into_iter()
                    .chain([endpoint.server.host.as_str().to_owned().into()])
                    .collect(),
            ),
            RequestRoute::Inbox {
                recipient_identity_id,
            } => (
                "SELECT request_id FROM request_attempts WHERE route_kind = 'inbox'
                 AND recipient_identity_id = ? AND wait_active = 1
                 ORDER BY prepared_at_ms, attempt_id LIMIT 1",
                vec![recipient_identity_id.clone().into()],
            ),
        };
        self.0
            .query_row(sql, rusqlite::params_from_iter(args), |row| row.get(0))
            .optional()
            .map_err(|error| classify(error, "Find active request"))
    }

    fn create_attempt(
        &mut self,
        attempt: &RequestAttempt,
        prompt: &StoredPrompt,
        originator_revision: u64,
    ) -> Result<(), Self::Error> {
        let (route_kind, endpoint) = match &attempt.route {
            RequestRoute::Pane(endpoint) => ("pane", Some(endpoint)),
            RequestRoute::Inbox { .. } => ("inbox", None),
        };
        let server_pid = endpoint
            .map(|e| checked_i64(e.server.server_pid, "Server PID"))
            .transpose()?;
        let pane_pid = endpoint
            .map(|e| checked_i64(e.pane_pid, "Pane PID"))
            .transpose()?;
        let preamble_every = attempt
            .preamble_every
            .map(|value| checked_i64(value, "Preamble cadence"))
            .transpose()?;
        let sending_at_ms = attempt
            .sending_at_ms
            .map(|value| checked_i64(value, "Sending timestamp"))
            .transpose()?;
        let settled_at_ms = attempt
            .settled_at_ms
            .map(|value| checked_i64(value, "Settlement timestamp"))
            .transpose()?;
        let wait_released_at_ms = attempt
            .wait_released_at_ms
            .map(|value| checked_i64(value, "Wait release timestamp"))
            .transpose()?;
        let response_submitted_at_ms = attempt
            .response_submitted_at_ms
            .map(|value| checked_i64(value, "Response timestamp"))
            .transpose()?;
        let prepared_at_ms = checked_i64(attempt.prepared_at_ms, "Preparation timestamp")?;
        let expires_at_ms = checked_i64(attempt.expires_at_ms, "Expiry timestamp")?;
        let retention_days = checked_i64(attempt.retention_days, "Retention days")?;
        let retention_expires_at_ms = checked_i64(
            attempt.retention_expires_at_ms,
            "Retention expiry timestamp",
        )?;
        let originator_revision = checked_i64(originator_revision, "Attention revision")?;
        let prompt_bytes = checked_i64(prompt.message_bytes, "Prompt bytes")?;
        let prompt_expires_at_ms = checked_i64(prompt.expires_at_ms, "Prompt expiry timestamp")?;
        self.0
            .execute(
                "INSERT INTO request_attempts (
                attempt_id, request_id, originator_kind, originator_identity_id,
                recipient_identity_id, nonce, identity_id, route_kind, server_id, socket_path,
                server_pid, server_start_time, pane_id, pane_pid, wait_active, status,
                preamble_every, inject_preamble, cadence_reserved, prepared_at_ms,
                sending_at_ms, settled_at_ms, wait_released_at_ms,
                response_submitted_at_ms, expires_at_ms, retention_days,
                retention_expires_at_ms, attention_revision,
                attention_acknowledged_revision, recipient_attention_revision,
                recipient_attention_acknowledged_revision, message_text, message_bytes,
                message_expires_at_ms, request_kind, room_id, host
            ) VALUES (
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                ?, ?, ?, ?, ?, ?, ?
            )",
                params![
                    attempt.attempt_id,
                    attempt.request_id,
                    attempt.originator.as_str(),
                    attempt.originator.identity_id(),
                    attempt.recipient_identity_id,
                    attempt.nonce,
                    attempt.identity_id,
                    route_kind,
                    endpoint.map(|e| e.server.server_id.as_str()),
                    endpoint.map(|e| e.server.socket_path.as_str()),
                    server_pid,
                    endpoint.map(|e| e.server.server_start_time.as_str()),
                    endpoint.map(|e| e.pane_id.as_str()),
                    pane_pid,
                    if attempt.wait_active { 1 } else { 0 },
                    attempt.status.as_str(),
                    preamble_every,
                    if attempt.inject_preamble { 1 } else { 0 },
                    if attempt.cadence_reserved { 1 } else { 0 },
                    prepared_at_ms,
                    sending_at_ms,
                    settled_at_ms,
                    wait_released_at_ms,
                    response_submitted_at_ms,
                    expires_at_ms,
                    retention_days,
                    retention_expires_at_ms,
                    originator_revision,
                    0_i64,
                    0_i64,
                    0_i64,
                    prompt.message,
                    prompt_bytes,
                    prompt_expires_at_ms,
                    attempt.kind.as_str(),
                    attempt.room_id,
                    endpoint.map(|e| e.server.host.as_str()),
                ],
            )
            .map_err(|error| classify(error, "Create request attempt"))?;
        Ok(())
    }

    fn withdraw_request(
        &mut self,
        attempt_id: &str,
        withdrawal: &tmt_core::request::Withdrawal,
    ) -> Result<bool, Self::Error> {
        let changed = self
            .0
            .execute(
                "UPDATE request_attempts SET withdrawn_at_ms = ?1, withdrawal_reason = ?2
             WHERE attempt_id = ?3 AND withdrawn_at_ms IS NULL
               AND response_submitted_at_ms IS NULL
               AND NOT EXISTS (SELECT 1 FROM request_responses WHERE attempt_id = ?3)",
                params![
                    checked_now(withdrawal.withdrawn_at_ms, "Withdrawal time")?,
                    withdrawal.reason,
                    attempt_id
                ],
            )
            .map_err(|error| classify(error, "Withdraw unanswered request"))?;
        Ok(changed == 1)
    }

    fn create_response(&mut self, response: &FinalResponse) -> Result<(), Self::Error> {
        let (route_kind, route_recipient, endpoint) = match &response.route {
            RequestRoute::Pane(endpoint) => ("pane", None, Some(endpoint)),
            RequestRoute::Inbox {
                recipient_identity_id,
            } => ("inbox", Some(recipient_identity_id.as_str()), None),
        };
        let server_pid = endpoint
            .map(|e| checked_i64(e.server.server_pid, "Server PID"))
            .transpose()?;
        let pane_pid = endpoint
            .map(|e| checked_i64(e.pane_pid, "Pane PID"))
            .transpose()?;
        let body_bytes = checked_i64(response.body_bytes, "Response bytes")?;
        let submitted_at_ms = checked_i64(response.submitted_at_ms, "Submission timestamp")?;
        let response_expires_at_ms =
            checked_i64(response.response_expires_at_ms, "Response expiry timestamp")?;
        self.0
            .execute(
                "INSERT INTO request_responses (
                request_id, attempt_id, route_kind, route_recipient_identity_id,
                server_id, socket_path, server_pid,
                server_start_time, pane_id, pane_pid, body, body_bytes,
                submitted_at_ms, response_expires_at_ms, host
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    response.request_id,
                    response.attempt_id,
                    route_kind,
                    route_recipient,
                    endpoint.map(|e| e.server.server_id.as_str()),
                    endpoint.map(|e| e.server.socket_path.as_str()),
                    server_pid,
                    endpoint.map(|e| e.server.server_start_time.as_str()),
                    endpoint.map(|e| e.pane_id.as_str()),
                    pane_pid,
                    response.body,
                    body_bytes,
                    submitted_at_ms,
                    response_expires_at_ms,
                    endpoint.map(|e| e.server.host.as_str()),
                ],
            )
            .map_err(|error| classify(error, "Create request response"))?;
        let changed = self
            .0
            .execute(
                "UPDATE request_attempts
             SET response_submitted_at_ms = ?,
                 retention_expires_at_ms = MAX(retention_expires_at_ms, ?)
             WHERE attempt_id = ? AND request_id = ?
               AND response_submitted_at_ms IS NULL AND withdrawn_at_ms IS NULL",
                params![
                    submitted_at_ms,
                    response_expires_at_ms,
                    response.attempt_id,
                    response.request_id,
                ],
            )
            .map_err(|error| classify(error, "Record request response completion"))?;
        if changed != 1 {
            return Err(StorageError::new(
                StorageErrorCode::Unknown,
                format!(
                    "Request attempt '{}' could not record response completion",
                    response.attempt_id
                ),
            ));
        }
        Ok(())
    }

    fn update_state(
        &mut self,
        attempt: &RequestAttempt,
        status: AttemptStatus,
        reserved: bool,
        now_ms: u64,
        horizon: Option<u64>,
    ) -> Result<bool, Self::Error> {
        let now = checked_i64(now_ms, "Request state timestamp")?;
        let horizon = horizon
            .map(|value| checked_i64(value, "Request retention horizon"))
            .transpose()?;
        let changed = self
            .0
            .execute(
                "UPDATE request_attempts
             SET status = ?,
                 cadence_reserved = ?,
                 sending_at_ms = CASE
                     WHEN ? = 'sending' THEN ? ELSE sending_at_ms
                 END,
                 settled_at_ms = CASE
                     WHEN ? IN ('sent', 'queued', 'uncertain', 'definitely_failed')
                         THEN ? ELSE settled_at_ms
                 END,
                 retention_expires_at_ms = MAX(
                     retention_expires_at_ms,
                     COALESCE(?, retention_expires_at_ms)
                 )
             WHERE attempt_id = ? AND status = ?",
                params![
                    status.as_str(),
                    if reserved { 1 } else { 0 },
                    status.as_str(),
                    now,
                    status.as_str(),
                    now,
                    horizon,
                    attempt.attempt_id,
                    attempt.status.as_str(),
                ],
            )
            .map_err(|error| classify(error, "Update request state"))?;
        Ok(changed == 1)
    }

    fn release_wait(&mut self, attempt_id: &str, now_ms: u64) -> Result<bool, Self::Error> {
        let changed = self
            .0
            .execute(
                "UPDATE request_attempts SET wait_active = 0, wait_released_at_ms = ?
             WHERE attempt_id = ? AND wait_active = 1",
                params![checked_i64(now_ms, "Wait release timestamp")?, attempt_id],
            )
            .map_err(|error| classify(error, "Release request wait"))?;
        Ok(changed == 1)
    }

    fn preamble_count(&self, identity_id: &str) -> Result<u64, Self::Error> {
        self.0
            .query_row(
                "SELECT reserved_count FROM preamble_counters WHERE identity_id = ?",
                [identity_id],
                |row| rows::u64_at(row, 0),
            )
            .optional()
            .map_err(|error| classify(error, "Read preamble counter"))
            .map(|value| value.unwrap_or(0))
    }

    fn set_preamble_count(
        &mut self,
        identity_id: &str,
        count: u64,
        now_ms: u64,
    ) -> Result<(), Self::Error> {
        self.0
            .execute(
                "INSERT INTO preamble_counters (identity_id, reserved_count, updated_at_ms)
             VALUES (?, ?, ?) ON CONFLICT(identity_id) DO UPDATE SET
               reserved_count = excluded.reserved_count,
               updated_at_ms = excluded.updated_at_ms",
                params![
                    identity_id,
                    checked_i64(count, "Preamble counter")?,
                    checked_i64(now_ms, "Preamble counter timestamp")?
                ],
            )
            .map_err(|error| classify(error, "Set preamble counter"))?;
        Ok(())
    }

    fn attention_counter(&self, identity_id: &str) -> Result<Option<u64>, Self::Error> {
        self.0
            .query_row(
                "SELECT latest_revision FROM request_attention_identities WHERE identity_id = ?",
                [identity_id],
                |row| rows::u64_at(row, 0),
            )
            .optional()
            .map_err(|error| classify(error, "Read attention counter"))
    }

    fn set_attention_counter(
        &mut self,
        identity_id: &str,
        expected: Option<u64>,
        next: u64,
    ) -> Result<(), Self::Error> {
        let next = checked_i64(next, "Attention revision")?;
        let changed = match expected {
            None => self
                .0
                .execute(
                    "INSERT INTO request_attention_identities
                 (identity_id, latest_revision, acknowledged_through) VALUES (?, ?, 0)",
                    params![identity_id, next],
                )
                .map_err(|error| classify(error, "Create attention counter"))?,
            Some(expected) => self
                .0
                .execute(
                    "UPDATE request_attention_identities SET latest_revision = ?
                 WHERE identity_id = ? AND latest_revision = ?",
                    params![
                        next,
                        identity_id,
                        checked_i64(expected, "Expected attention revision")?
                    ],
                )
                .map_err(|error| classify(error, "Advance attention counter"))?,
        };
        if expected.is_some() && changed != 1 {
            return Err(StorageError::new(
                StorageErrorCode::Unknown,
                format!("Attention counter for identity '{identity_id}' changed unexpectedly"),
            ));
        }
        Ok(())
    }

    fn set_attention_revision(
        &mut self,
        request_id: &str,
        revision: u64,
    ) -> Result<(), Self::Error> {
        let changed = self
            .0
            .execute(
                "UPDATE request_attempts SET attention_revision = ? WHERE request_id = ?",
                params![checked_i64(revision, "Attention revision")?, request_id],
            )
            .map_err(|error| classify(error, "Set request attention revision"))?;
        if changed != 1 {
            return Err(StorageError::new(
                StorageErrorCode::Unknown,
                format!("Request '{request_id}' was not found"),
            ));
        }
        Ok(())
    }

    fn recipient_attention_counter(&self, identity_id: &str) -> Result<Option<u64>, Self::Error> {
        self.0.query_row(
            "SELECT latest_revision FROM request_recipient_attention_identities WHERE identity_id = ?",
            [identity_id], |row| rows::u64_at(row, 0),
        ).optional().map_err(|error| classify(error, "Read recipient attention counter"))
    }

    fn set_recipient_attention_counter(
        &mut self,
        identity_id: &str,
        expected: Option<u64>,
        next: u64,
    ) -> Result<(), Self::Error> {
        let next = checked_i64(next, "Recipient attention revision")?;
        let changed = match expected {
            None => self
                .0
                .execute(
                    "INSERT INTO request_recipient_attention_identities
                 (identity_id, latest_revision, acknowledged_through) VALUES (?, ?, 0)",
                    params![identity_id, next],
                )
                .map_err(|error| classify(error, "Create recipient attention counter"))?,
            Some(expected) => self
                .0
                .execute(
                    "UPDATE request_recipient_attention_identities SET latest_revision = ?
                 WHERE identity_id = ? AND latest_revision = ?",
                    params![
                        next,
                        identity_id,
                        checked_i64(expected, "Expected recipient attention revision")?
                    ],
                )
                .map_err(|error| classify(error, "Advance recipient attention counter"))?,
        };
        if expected.is_some() && changed != 1 {
            return Err(StorageError::new(
                StorageErrorCode::Unknown,
                format!(
                    "Recipient attention counter for identity '{identity_id}' changed unexpectedly"
                ),
            ));
        }
        Ok(())
    }

    fn set_recipient_attention_revision(
        &mut self,
        request_id: &str,
        revision: u64,
    ) -> Result<(), Self::Error> {
        let changed = self
            .0
            .execute(
                "UPDATE request_attempts SET recipient_attention_revision = ?
                 WHERE request_id = ? AND route_kind = 'inbox'
                   AND status = 'prepared' AND recipient_attention_revision = 0",
                params![
                    checked_i64(revision, "Recipient attention revision")?,
                    request_id
                ],
            )
            .map_err(|error| classify(error, "Set recipient attention revision"))?;
        if changed != 1 {
            return Err(StorageError::new(
                StorageErrorCode::Unknown,
                format!("Inbox request '{request_id}' could not receive attention"),
            ));
        }
        Ok(())
    }

    fn incoming_watermark(
        &self,
        identity_id: &str,
        room_id: Option<&str>,
        now_ms: u64,
    ) -> Result<u64, Self::Error> {
        self.0
            .query_row(
                &incoming_watermark_sql(room_id.is_some()),
                params![
                    identity_id,
                    checked_now(now_ms, "Incoming retention cutoff")?,
                    room_id,
                ],
                |row| rows::u64_at(row, 0),
            )
            .map_err(|error| classify(error, "Read incoming attention watermark"))
    }

    fn expired_attempts(
        &self,
        now_ms: u64,
        limit: u64,
    ) -> Result<Vec<RequestAttempt>, Self::Error> {
        let query = format!(
            "SELECT {} FROM request_attempts
             WHERE expires_at_ms <= ? AND (wait_active = 1 OR status IN ('prepared', 'sending'))
             ORDER BY expires_at_ms, attempt_id LIMIT ?",
            rows::ATTEMPT_COLUMNS
        );
        let mut statement = self
            .0
            .prepare(&query)
            .map_err(|error| classify(error, "Prepare expired request query"))?;
        let rows = statement
            .query_map(
                params![
                    checked_now(now_ms, "Request expiry cutoff")?,
                    checked_limit(limit)?
                ],
                rows::attempt_row,
            )
            .map_err(|error| classify(error, "List expired requests"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| classify(error, "Decode expired requests"))
    }

    fn clear_expired_prompts(&mut self, now_ms: u64, limit: u64) -> Result<(), Self::Error> {
        self.0
            .execute(
                "UPDATE request_attempts
             SET message_text = NULL, message_bytes = NULL
             WHERE attempt_id IN (
                 SELECT attempt_id FROM request_attempts
                 WHERE message_text IS NOT NULL
                   AND message_expires_at_ms <= ?
                 ORDER BY message_expires_at_ms, attempt_id
                 LIMIT ?
             )",
                params![
                    checked_now(now_ms, "Prompt retention cutoff")?,
                    checked_limit(limit)?
                ],
            )
            .map_err(|error| classify(error, "Clear expired request prompts"))?;
        Ok(())
    }

    fn delete_expired_responses(&mut self, now_ms: u64, limit: u64) -> Result<(), Self::Error> {
        self.0
            .execute(
                "DELETE FROM request_responses
             WHERE request_id IN (
                 SELECT request_id FROM request_responses
                 WHERE response_expires_at_ms <= ?
                 ORDER BY response_expires_at_ms, request_id
                 LIMIT ?
             )",
                params![
                    checked_now(now_ms, "Response retention cutoff")?,
                    checked_limit(limit)?
                ],
            )
            .map_err(|error| classify(error, "Delete expired request responses"))?;
        Ok(())
    }

    fn delete_retained(
        &mut self,
        now_ms: u64,
        settled_cutoff_ms: u64,
        limit: u64,
    ) -> Result<(), Self::Error> {
        let now = checked_now(now_ms, "Request retention cutoff")?;
        let settled_cutoff = checked_i64(settled_cutoff_ms, "Settlement retention cutoff")?;
        self.0
            .execute(
                DELETE_RETAINED_SQL,
                params![settled_cutoff, now, now, checked_limit(limit)?],
            )
            .map_err(|error| classify(error, "Delete retained requests"))?;
        Ok(())
    }
}

impl RequestRepository for Storage {
    type Error = StorageError;

    fn with_request_transaction<T, E: From<Self::Error>>(
        &mut self,
        operation: impl FnOnce(&mut dyn RequestRecords<Error = Self::Error>) -> Result<T, E>,
    ) -> Result<T, E> {
        with_immediate_transaction(self, "request", |transaction| {
            operation(&mut RequestRows(transaction))
        })
    }

    fn with_request_observation<T, E: From<Self::Error>>(
        &mut self,
        operation: impl FnOnce(&mut dyn RequestRecords<Error = Self::Error>) -> Result<T, E>,
    ) -> Result<T, E> {
        let connection = self.connection.as_mut().ok_or_else(|| {
            E::from(StorageError::new(
                StorageErrorCode::Closed,
                "Storage is already closed",
            ))
        })?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| E::from(classify(error, "Begin request observation transaction")))?;
        let result = operation(&mut RequestRows(&transaction))?;
        transaction
            .commit()
            .map_err(|error| E::from(classify(error, "Commit request observation transaction")))?;
        Ok(result)
    }
}

#[cfg(test)]
mod service_tests;
#[cfg(test)]
mod tests;
