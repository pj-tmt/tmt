use super::super::flow::{self as values, FlowKind, FlowReport, FlowRow};
use super::*;

pub(super) fn record<E>(
    records: &mut dyn RequestRecords<Error = E>,
    attempt: &RequestAttempt,
    kind: FlowKind,
    reverse: bool,
    bytes: u64,
    now: u64,
) -> Result<(), RequestError<E>> {
    let (origin, target) = (
        attempt.originator.identity_id().map(str::to_owned),
        attempt.recipient_identity_id.clone(),
    );
    let (sender_identity_id, recipient_identity_id) = if reverse {
        (target, origin)
    } else {
        (origin, target)
    };
    records.increment_flow(&FlowRow {
        sender_identity_id,
        recipient_identity_id,
        kind,
        bucket_start_ms: values::bucket(now),
        count: 1,
        bytes,
    })?;
    Ok(())
}
pub(super) fn record_ack<E>(
    records: &mut dyn RequestRecords<Error = E>,
    attempt: &RequestAttempt,
    identity: &str,
    now: u64,
) -> Result<(), RequestError<E>> {
    let other = if attempt.originator.identity_id() == Some(identity) {
        attempt.recipient_identity_id.clone()
    } else {
        attempt.originator.identity_id().map(str::to_owned)
    };
    records.increment_flow(&FlowRow {
        sender_identity_id: Some(identity.into()),
        recipient_identity_id: other,
        kind: FlowKind::Ack,
        bucket_start_ms: values::bucket(now),
        count: 1,
        bytes: 0,
    })?;
    Ok(())
}
impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    pub fn exchange_flow(&mut self, window_ms: u64) -> Result<FlowReport, RequestError<R::Error>> {
        let clock = &self.clock;
        self.repository.with_request_observation(|records| {
            let now = positive(clock())?;
            let since_ms = values::since(now, window_ms).ok_or(RequestError::Invalid(
                "Flow window must be positive and at most 30 days.",
            ))?;
            Ok(FlowReport {
                observed_at_ms: now,
                since_ms,
                rows: records.flow_rows(since_ms, now)?,
            })
        })
    }
}
