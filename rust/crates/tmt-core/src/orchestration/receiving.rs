//! Admission of one message under the resolved receiving value. This does not
//! choose plans or precedence, inspect a host, or grant a transport opportunity.
use super::*;
use crate::request::digest::DigestPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchReceivingPolicy {
    pub receiving: OrchValue<OrchReceiving>,
    pub configuration_revision: u64,
    pub time_zone: String,
    pub next_change_at_ms: Option<u64>,
    pub legacy_window: Option<DigestPolicy>,
}
impl OrchReceivingPolicy {
    /// A former auto/interval lease cannot promise a cap while the current
    /// receiving value withholds this traffic indefinitely.
    pub fn max_hold_ms(&self) -> Option<u64> {
        match self.receiving.value {
            OrchReceiving::Auto => self.legacy_window.as_ref().and_then(|p| p.max_hold_ms),
            OrchReceiving::Batch(ms) => Some(ms),
            OrchReceiving::Direct | OrchReceiving::Urgent | OrchReceiving::Hold => None,
        }
    }
    pub fn held_until(
        &self,
        sender: Option<&str>,
        urgent: bool,
        now: u64,
    ) -> Result<Option<u64>, OrchError> {
        let ordinary = matches!(
            self.receiving.value,
            OrchReceiving::Auto | OrchReceiving::Batch(_)
        );
        let legacy_base = ordinary && self.receiving.source.kind == OrchSourceKind::Base;
        if self.receiving.value == OrchReceiving::Direct
            || (urgent && self.receiving.value != OrchReceiving::Hold)
            || (legacy_base
                && self
                    .legacy_window
                    .as_ref()
                    .is_none_or(|p| !p.holds(sender, urgent, now)))
        {
            return Ok(None);
        }
        // Settings are durable input, not a resident scheduler. The existing
        // Digest tick must keep its finite delivery window active.
        let Some(lease) = self
            .legacy_window
            .as_ref()
            .filter(|p| p.active(now))
            .map(|p| p.until_ms)
        else {
            return Ok(None);
        };
        let until = self
            .next_change_at_ms
            .filter(|end| *end > now)
            .map_or(lease, |end| lease.min(end));
        if until <= now {
            return Err(OrchError::Limit);
        }
        Ok(Some(until))
    }
    /// Hard receiving restrictions apply at every opportunity, including a
    /// provider turn boundary; an idle/cap flag never changes this admission.
    pub fn permits_held_item(&self, urgent: bool) -> bool {
        match self.receiving.value {
            OrchReceiving::Hold => false,
            OrchReceiving::Urgent => urgent,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy(mode: OrchReceiving, kind: OrchSourceKind) -> OrchReceivingPolicy {
        OrchReceivingPolicy {
            receiving: OrchValue {
                value: mode,
                source: OrchSource {
                    kind,
                    plan_id: None,
                    identity_id: None,
                    revision: 1,
                    slot: None,
                    defaulted: false,
                },
            },
            configuration_revision: 1,
            time_zone: "UTC".into(),
            next_change_at_ms: Some(20_000),
            legacy_window: Some(DigestPolicy {
                identity_id: "seat".into(),
                revision: 1,
                until_ms: 30_000,
                owner_identity_id: "lead".into(),
                setter_identity_id: "lead".into(),
                max_hold_ms: None,
            }),
        }
    }
    #[test]
    fn scheduled_focus_and_sleep_have_no_owner_or_setter_exemption() {
        for sender in [None, Some("lead"), Some("other")] {
            let focus = policy(OrchReceiving::Urgent, OrchSourceKind::Focus);
            assert_eq!(focus.held_until(sender, false, 10_000), Ok(Some(20_000)));
            assert_eq!(focus.held_until(sender, true, 10_000), Ok(None));
            assert!(!focus.permits_held_item(false));
            assert!(focus.permits_held_item(true));
            let sleep = policy(OrchReceiving::Hold, OrchSourceKind::Plan);
            for urgent in [false, true] {
                assert_eq!(sleep.held_until(sender, urgent, 10_000), Ok(Some(20_000)));
                assert!(!sleep.permits_held_item(urgent));
            }
        }
    }
    #[test]
    fn ordinary_base_retains_legacy_owner_and_lease_behavior_but_a_plan_is_immediate() {
        let base = policy(OrchReceiving::Auto, OrchSourceKind::Base);
        assert_eq!(base.held_until(Some("lead"), false, 10_000), Ok(None));
        assert_eq!(
            base.held_until(Some("other"), false, 10_000),
            Ok(Some(20_000))
        );
        assert_eq!(base.held_until(Some("other"), false, 30_000), Ok(None));
        let mut plan = policy(OrchReceiving::Batch(1_800_000), OrchSourceKind::Plan);
        assert_eq!(
            plan.held_until(Some("lead"), false, 10_000),
            Ok(Some(20_000))
        );
        assert_eq!(plan.held_until(Some("lead"), true, 10_000), Ok(None));
        plan.legacy_window = None;
        assert_eq!(plan.held_until(Some("lead"), false, 10_000), Ok(None));
    }
}
