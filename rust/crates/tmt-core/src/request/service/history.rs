use super::*;
use crate::{
    dispatch::canonical_id,
    request::{
        attention::{AttentionRecord, FinalState},
        history::*,
    },
};

fn valid_query(query: &HistoryQuery) -> bool {
    let scope = match &query.scope {
        HistoryScope::Recipient {
            identity_id,
            room_id,
        } => canonical_id(identity_id) && room_id.as_deref().is_none_or(canonical_id),
        HistoryScope::Room(room_id) => canonical_id(room_id),
        HistoryScope::OriginatorResults(identity_id) => canonical_id(identity_id),
    };
    scope
        && (1..=HISTORY_MAX_LIMIT).contains(&query.limit)
        && query.before.as_ref().is_none_or(|cursor| {
            matches!(
                (&query.scope, cursor),
                (
                    HistoryScope::OriginatorResults(_),
                    HistoryCursor::Submitted { .. }
                ) | (
                    HistoryScope::Recipient { .. } | HistoryScope::Room(_),
                    HistoryCursor::Prepared { .. }
                )
            ) && cursor.timestamp() > 0
                && cursor.timestamp() <= MAX_JS_SAFE_INTEGER
                && !cursor.request_id().is_empty()
                && cursor.request_id().len() <= 80
                && !cursor.request_id().chars().any(char::is_control)
        })
}

fn item<T>(
    record: AttentionRecord,
    final_state: FinalState<T>,
    delivery_policy: super::super::digest::DeliveryPolicy,
) -> HistoryItem<T> {
    let acknowledged = matches!(record.attempt.route, RequestRoute::Inbox { .. }).then_some(
        record.revision > 0
            && (record.acknowledged_revision >= record.revision
                || record.acknowledged_through >= record.revision),
    );
    HistoryItem {
        delivery_policy,
        request_id: record.attempt.request_id,
        room_id: record.attempt.room_id,
        recipient_identity_id: record.attempt.recipient_identity_id,
        originator: record.attempt.originator,
        kind: record.attempt.kind,
        prepared_at_ms: record.attempt.prepared_at_ms,
        delivery: record.attempt.status,
        recipient_acknowledged: acknowledged,
        final_state,
    }
}

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    /// Includes acknowledged work and unknown-owner sends. Reading never acknowledges.
    pub fn request_history(
        &mut self,
        query: HistoryQuery,
    ) -> Result<HistoryPage, RequestError<R::Error>> {
        if !valid_query(&query) {
            return Err(RequestError::Invalid("Invalid request history query."));
        }
        let results = matches!(query.scope, HistoryScope::OriginatorResults(_));
        let observe = |records: &mut dyn RequestRecords<Error = R::Error>, now| {
            let mut rows = records.list_request_history(&query, query.limit + 1, now)?;
            let more = rows.len() as u64 > query.limit;
            rows.truncate(query.limit as usize);
            let next_before = more.then(|| {
                let row = &rows.last().expect("positive page limit").attention.attempt;
                if results {
                    HistoryCursor::Submitted {
                        submitted_at_ms: row
                            .response_submitted_at_ms
                            .expect("submitted results only"),
                        request_id: row.request_id.clone(),
                    }
                } else {
                    HistoryCursor::Prepared {
                        prepared_at_ms: row.prepared_at_ms,
                        request_id: row.request_id.clone(),
                    }
                }
            });
            let items = rows
                .into_iter()
                .map(|row| {
                    let delivery_policy =
                        records.delivery_policy(&row.attention.attempt.request_id)?;
                    let final_state = attention::final_state(&row.attention, now, Some(()))?;
                    Ok(HistorySummary {
                        item: item(row.attention, final_state, delivery_policy),
                        preview: row.preview,
                        response_preview: row.response_preview,
                    })
                })
                .collect::<Result<_, RequestError<R::Error>>>()?;
            Ok(HistoryPage { items, next_before })
        };
        if results {
            let clock = &self.clock;
            self.repository
                .with_request_observation(|records| observe(records, positive(clock())?))
        } else {
            self.read(observe)
        }
    }

    /// Owner inspection exposes exact retained text, not reply credentials or pane paths.
    pub fn request_detail(
        &mut self,
        request_id: &str,
    ) -> Result<HistoryDetail, RequestError<R::Error>> {
        nonempty(request_id)?;
        self.read(|records, now| {
            let record = records
                .find_request_history(request_id)?
                .filter(|record| now < record.attempt.retention_expires_at_ms)
                .ok_or(RequestError::NotFound)?;
            let delivery_policy = records.delivery_policy(request_id)?;
            let context = context(records, request_id, now)?.ok_or(RequestError::NotFound)?;
            let response = records
                .find_response(request_id)?
                .filter(|response| now < response.response_expires_at_ms);
            let final_state =
                attention::final_state(&record, now, response.map(|response| response.body))?;
            Ok(HistoryDetail {
                item: item(record, final_state, delivery_policy),
                prompt: context.prompt,
            })
        })
    }
}
