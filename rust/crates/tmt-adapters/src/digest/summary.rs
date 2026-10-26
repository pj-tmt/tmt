//! One finite invocation. Readiness and transport remain the ordinary Digest owners.
use super::*;
use std::time::Instant;

pub fn wake_step(
    storage: &mut Storage,
    delay: Duration,
    budget_ms: u64,
    limit: u64,
) -> Result<Value, RequestError<StorageError>> {
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    let (candidates, clock) =
        RequestService::new(&mut *storage, wall_time_ms).digest_wake_candidates(limit)?;
    let response = |state: &str,
                    identity: Option<&str>,
                    batch: Option<&DigestChecklist>,
                    next: Option<u64>| {
        let mut result = json!({"schemaVersion":1,"state":state,"identityId":identity,"projection":"incoming_summary","nextAtMs":next});
        if let Some(DigestChecklist {
            projection:
                DigestProjection::IncomingSummary {
                    incoming_count,
                    urgent_count,
                },
            ..
        }) = batch
        {
            result["incomingCount"] = json!(incoming_count);
            result["urgentCount"] = json!(urgent_count);
        }
        result
    };
    if wall_time_ms() < clock.next_admission_ms {
        return Ok(response(
            "nothing_due",
            None,
            None,
            Some(clock.next_admission_ms),
        ));
    }
    let mut after = clock.scan_after_identity_id.as_deref();
    for identity in &candidates {
        if Instant::now() >= deadline {
            break;
        }
        let released = RequestService::new(&mut *storage, wall_time_ms)
            .digest_wake_released(identity, || Instant::now() < deadline)?;
        if released && let Some(entry) = idle_entry(storage, identity)? {
            let batch = RequestService::new(&mut *storage, wall_time_ms).claim_digest_summary(
                identity,
                tmt_core::operation::new_operation_id(),
                tmt_core::operation::new_operation_id(),
                DigestOpportunity::Idle,
                || Instant::now() < deadline,
            )?;
            if let Some(batch) = batch {
                let line = text(storage, &batch)?;
                let state = send_claim(storage, &entry, &batch, &line, delay, Readiness::Idle)?;
                return Ok(response(
                    match state {
                        DigestState::Delivered => "delivered",
                        DigestState::Unsent => "unavailable",
                        _ => "uncertain",
                    },
                    Some(identity),
                    Some(&batch),
                    Some(batch.created_at_ms + 1000),
                ));
            }
        }
        after = Some(identity.as_str());
    }
    if candidates.is_empty()
        || (after == candidates.last().map(String::as_str) && candidates.len() < limit as usize)
    {
        after = None;
    }
    RequestService::new(&mut *storage, wall_time_ms).advance_digest_wake_scan(after)?;
    Ok(response(
        if candidates.is_empty() {
            "nothing_due"
        } else {
            "unavailable"
        },
        None,
        None,
        None,
    ))
}
