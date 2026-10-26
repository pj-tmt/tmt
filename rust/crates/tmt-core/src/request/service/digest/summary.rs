//! Release claims over canonical held Incoming. The marker contains no membership.
use super::*;
use digest::{DigestPendingWake, DigestProjection, DigestSource, DigestWakeClock};

pub(super) fn restricted_hold<E>(
    records: &mut dyn RequestRecords<Error = E>,
    identity: &str,
    urgent: bool,
    now: u64,
) -> Result<(), RequestError<E>> {
    use crate::orchestration::OrchReceiving;
    if records
        .orchestration_receiving_policy(identity, now)?
        .is_none_or(|p| {
            !matches!(
                p.receiving.value,
                OrchReceiving::Urgent | OrchReceiving::Hold
            )
        })
    {
        return Ok(());
    }
    let mut marker = records
        .digest_pending_wake(identity)?
        .unwrap_or(DigestPendingWake {
            classes: 0,
            held_at_ms: now,
            checklist_id: None,
        });
    marker.classes |= if urgent { 2 } else { 1 };
    records.write_digest_pending_wake(identity, Some(&marker))?;
    Ok(())
}

#[derive(Default)]
struct EligibleIncoming {
    ordinary: Vec<u64>,
    urgent: Vec<u64>,
}
impl EligibleIncoming {
    fn classes(&self) -> u8 {
        u8::from(!self.ordinary.is_empty()) | (u8::from(!self.urgent.is_empty()) << 1)
    }
    fn selected(self, classes: u8) -> (Vec<u64>, u64) {
        let mut members = if classes & 1 != 0 {
            self.ordinary
        } else {
            Vec::new()
        };
        let urgent = if classes & 2 != 0 {
            self.urgent
        } else {
            Vec::new()
        };
        let urgent_count = urgent.len() as u64;
        members.extend(urgent);
        members.sort_unstable();
        (members, urgent_count)
    }
}
fn released_classes(policy: &crate::orchestration::receiving::OrchReceivingPolicy, now: u64) -> u8 {
    if policy.legacy_window.as_ref().is_some_and(|w| w.active(now)) {
        u8::from(policy.permits_held_item(false)) | (u8::from(policy.permits_held_item(true)) << 1)
    } else {
        3
    }
}

/// None means the budget ended before a complete count. No partial membership may publish.
fn eligible<E>(
    records: &mut dyn RequestRecords<Error = E>,
    identity: &str,
    classes: u8,
    now: u64,
    within_budget: &impl Fn() -> bool,
) -> Result<Option<EligibleIncoming>, RequestError<E>> {
    let mut eligible = EligibleIncoming::default();
    let mut after = 0;
    loop {
        if !within_budget() {
            return Ok(None);
        }
        let page = records.digest_items(identity, None, after, 128, now)?;
        let Some(last) = page.last() else { break };
        after = last.sequence;
        let full = page.len() == 128;
        for item in page {
            if item.source != DigestSource::Incoming {
                continue;
            }
            let Some(attempt) = records.find_request(&item.request_id)? else {
                continue;
            };
            if now >= attempt.retention_expires_at_ms
                || responses::first_final_refusal(&attempt, now).is_some()
                || !matches!(&attempt.route, RequestRoute::Inbox { recipient_identity_id } if recipient_identity_id == identity)
                || attempt
                    .originator
                    .identity_id()
                    .map(|sender| records.identity_is_user(sender))
                    .transpose()?
                    .unwrap_or(false)
            {
                continue;
            }
            let policy = records.delivery_policy(&item.request_id)?;
            let class = if policy.urgent { 2 } else { 1 };
            if !policy.automatic
                || policy.mode != digest::DeliveryMode::Request
                || classes & class == 0
            {
                continue;
            }
            if policy.urgent {
                eligible.urgent.push(item.sequence);
            } else {
                eligible.ordinary.push(item.sequence);
            }
        }
        if !full {
            break;
        }
    }
    Ok(Some(eligible))
}

pub(super) fn settled_marker<E>(
    records: &mut dyn RequestRecords<Error = E>,
    batch: &DigestChecklist,
    now: u64,
) -> Result<(), RequestError<E>> {
    let Some(mut marker) = records.digest_pending_wake(&batch.identity_id)? else {
        return Ok(());
    };
    if marker.checklist_id.as_deref() != Some(&batch.id) {
        return Ok(());
    }
    // Settlement released only proven-unsent members. Sent/uncertain members remain sealed;
    // later arrivals can keep the marker but cannot resurrect this attempt.
    let remaining = eligible(records, &batch.identity_id, marker.classes, now, &|| true)?
        .expect("unbounded settlement scan completes");
    marker.classes &= remaining.classes();
    marker.checklist_id = None;
    records
        .write_digest_pending_wake(&batch.identity_id, (marker.classes != 0).then_some(&marker))?;
    Ok(())
}

impl<R: RequestRepository, C: Fn() -> u64> RequestService<'_, R, C> {
    /// A bounded UUID pass. The clock's continuation is scan progress, never admission authority.
    pub fn digest_wake_candidates(
        &mut self,
        limit: u64,
    ) -> Result<(Vec<String>, DigestWakeClock), RequestError<R::Error>> {
        if !(1..=256).contains(&limit) {
            return Err(digest_error(DigestRejection::Invalid));
        }
        self.repository.with_request_observation(|records| {
            let clock = records.digest_wake_clock()?;
            let candidates =
                records.digest_wake_candidates(clock.scan_after_identity_id.as_deref(), limit)?;
            Ok((candidates, clock))
        })
    }
    pub fn advance_digest_wake_scan(
        &mut self,
        after: Option<&str>,
    ) -> Result<(), RequestError<R::Error>> {
        self.repository.with_request_transaction(|records| {
            let mut clock = records.digest_wake_clock()?;
            clock.scan_after_identity_id = match after {
                Some(id) if records.identity_is_active(id)? => Some(id.into()),
                _ => None,
            };
            records.write_digest_wake_clock(&clock)?;
            Ok(())
        })
    }
    pub fn digest_wake_pending(&mut self, identity: &str) -> Result<bool, RequestError<R::Error>> {
        self.repository.with_request_observation(|records| {
            Ok(records.digest_pending_wake(identity)?.is_some())
        })
    }
    /// Close dead opportunities before any host probe; a read/blocked scan logs no decision.
    pub fn digest_wake_released(
        &mut self,
        identity: &str,
        within_budget: impl Fn() -> bool,
    ) -> Result<bool, RequestError<R::Error>> {
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            let Some(marker) = records.digest_pending_wake(identity)? else {
                return Ok(false);
            };
            if marker.checklist_id.is_some() {
                return Ok(false);
            }
            if records.identity_is_user(identity)? {
                records.write_digest_pending_wake(identity, None)?;
                return Ok(false);
            }
            let Some(all) = eligible(records, identity, marker.classes, now, &within_budget)?
            else {
                return Ok(false);
            };
            if all.classes() == 0 {
                records.write_digest_pending_wake(identity, None)?;
                return Ok(false);
            }
            let Some(policy) = records.resolved_receiving_policy(identity, now)? else {
                return Ok(false);
            };
            Ok(all.classes() & released_classes(&policy, now) != 0)
        })
    }
    pub fn claim_digest_summary(
        &mut self,
        identity: &str,
        id: String,
        token: String,
        opportunity: DigestOpportunity,
        within_budget: impl Fn() -> bool,
    ) -> Result<Option<DigestChecklist>, RequestError<R::Error>> {
        if !crate::dispatch::canonical_id(&id) || !crate::dispatch::canonical_id(&token) {
            return Err(digest_error(DigestRejection::Invalid));
        }
        let clock = &self.clock;
        self.repository.with_request_transaction(|records| {
            let now = positive(clock())?;
            digest_identity(records, identity)?;
            if !within_budget() || records.identity_is_user(identity)? {
                return Ok(None);
            }
            if let Some(existing) = records.digest_checklist(&id)? {
                if existing.identity_id != identity || existing.attempt_token != token {
                    return Err(digest_error(DigestRejection::AttemptMismatch));
                }
                return Ok(None);
            }
            let Some(mut marker) = records.digest_pending_wake(identity)? else {
                return Ok(None);
            };
            if marker.checklist_id.is_some() || records.active_digest_checklist(identity)?.is_some()
            {
                return Ok(None);
            }
            let Some(policy) = records.resolved_receiving_policy(identity, now)? else {
                return Ok(None);
            };
            let Some(all) = eligible(records, identity, marker.classes, now, &within_budget)?
            else {
                return Ok(None);
            };
            if all.classes() == 0 {
                records.write_digest_pending_wake(identity, None)?;
                return Ok(None);
            }
            let classes = marker.classes & released_classes(&policy, now);
            if classes == 0 {
                return Ok(None);
            }
            let (members, urgent_count) = all.selected(classes);
            if members.is_empty() {
                return Ok(None);
            }
            let mut admission = records.digest_wake_clock()?;
            if now < admission.next_admission_ms || !within_budget() {
                return Ok(None);
            }
            let batch = DigestChecklist {
                id,
                identity_id: identity.into(),
                attempt_token: token,
                through_sequence: *members.last().expect("nonempty membership"),
                state: DigestState::Claimed,
                created_at_ms: now,
                projection: DigestProjection::IncomingSummary {
                    incoming_count: members.len() as u64,
                    urgent_count,
                },
            };
            records.create_digest_checklist(
                &batch,
                now,
                opportunity,
                members.len() as u64,
                digest::DigestSelection::Incoming,
                Some(&members),
            )?;
            marker.checklist_id = Some(batch.id.clone());
            records.write_digest_pending_wake(identity, Some(&marker))?;
            admission.next_admission_ms = now
                .checked_add(1000)
                .filter(|n| *n <= MAX_JS_SAFE_INTEGER)
                .ok_or(RequestError::RevisionExhausted)?;
            admission.scan_after_identity_id = None;
            records.write_digest_wake_clock(&admission)?;
            Ok(Some(batch))
        })
    }
}
