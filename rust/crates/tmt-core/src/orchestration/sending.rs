//! Sending rules and talk limits for one new agent-to-agent request. The admission
//! transaction supplies the resolved values, the grant and the pair count at its captured
//! instant; it owns storage and the journal append. Exempt traffic never reaches here.
use super::*;

/// Talk limits count requests over this sliding window.
pub const ORCH_TALK_WINDOW_MS: u64 = 3_600_000;
/// The longest a route grant (`route allow`) may last.
pub const ORCH_MAX_GRANT_MS: u64 = 86_400_000;

/// `route allow`: one ordered pair passes its sending rule until `until_ms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchRouteGrant {
    pub from_id: String,
    pub to_id: String,
    pub until_ms: u64,
    /// The verified caller who allowed it; `None` for a person at a terminal.
    pub granted_by: Option<String>,
}

/// What the sender's effective `sending` value says about one recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchRoute {
    Allowed,
    /// The rule restricts this route, but a live grant lets it through until `until_ms`.
    Granted {
        until_ms: u64,
    },
    /// Refused before anything is stored.
    Restricted {
        sending: OrchSending,
        source: OrchSource,
    },
}

/// A refused route, with what the caller needs to explain it and to journal it after the
/// preparation transaction rolled back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchRouteRefusal {
    pub sender_id: String,
    pub recipient_id: String,
    pub sending: OrchSending,
    pub source: OrchSource,
    pub configuration_revision: u64,
    pub time_zone: String,
}

/// The preparation transaction's decision for one agent-to-agent request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchTalkAdmission {
    Allowed,
    /// Refused before anything is stored.
    Restricted(Box<OrchRouteRefusal>),
    /// Stored and held until a person resumes the pair.
    NeedsPerson {
        counted: u64,
        limit: u32,
    },
}

/// `grant_until_ms` is the end of the grant stored for this exact ordered pair, if any.
pub fn route(
    sending: &OrchValue<OrchSending>,
    recipient_is_lead: bool,
    grant_until_ms: Option<u64>,
    now: u64,
) -> OrchRoute {
    let allowed = match sending.value {
        OrchSending::Anyone => true,
        OrchSending::Leads => recipient_is_lead,
        OrchSending::Paused => false,
    };
    if allowed {
        return OrchRoute::Allowed;
    }
    match grant_until_ms.filter(|until| now < *until) {
        Some(until_ms) => OrchRoute::Granted { until_ms },
        None => OrchRoute::Restricted {
            sending: sending.value,
            source: sending.source.clone(),
        },
    }
}

/// A pair's limit is the smaller of its members' effective limits; `None` is unlimited.
pub fn pair_limit(first: Option<u32>, second: Option<u32>) -> Option<u32> {
    match (first, second) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (limit, None) | (None, limit) => limit,
    }
}

/// The pair window starts an hour back, or at the last resume when that is later, so
/// released requests are not counted again.
pub fn talk_window_start(now: u64, resumed_at_ms: Option<u64>) -> u64 {
    now.saturating_sub(ORCH_TALK_WINDOW_MS)
        .max(resumed_at_ms.unwrap_or(0))
}

/// Whether one more request, after `counted` in the window, must wait for a person.
pub fn needs_person(counted: u64, limit: Option<u32>) -> bool {
    limit.is_some_and(|limit| counted >= u64::from(limit))
}

/// A grant ends after now and within the bound.
pub fn valid_grant(now: u64, until_ms: u64) -> bool {
    until_ms > now && until_ms - now <= ORCH_MAX_GRANT_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sending(value: OrchSending) -> OrchValue<OrchSending> {
        OrchValue {
            value,
            source: OrchSource {
                kind: OrchSourceKind::Plan,
                plan_id: Some("night".into()),
                identity_id: None,
                revision: 3,
                slot: Some(OrchSlot {
                    start_ms: 0,
                    end_ms: 100,
                }),
                defaulted: false,
            },
        }
    }

    #[test]
    fn each_sending_value_decides_the_route_and_only_a_live_grant_lets_a_restricted_one_through() {
        assert_eq!(
            route(&sending(OrchSending::Anyone), false, None, 10),
            OrchRoute::Allowed
        );
        assert_eq!(
            route(&sending(OrchSending::Leads), true, None, 10),
            OrchRoute::Allowed
        );
        for value in [OrchSending::Leads, OrchSending::Paused] {
            let restricted = route(&sending(value), false, None, 10);
            assert_eq!(
                restricted,
                OrchRoute::Restricted {
                    sending: value,
                    source: sending(value).source
                }
            );
            assert_eq!(
                route(&sending(value), false, Some(50), 10),
                OrchRoute::Granted { until_ms: 50 }
            );
            assert_eq!(
                route(&sending(value), false, Some(10), 10),
                restricted,
                "ended"
            );
        }
        assert!(matches!(
            route(&sending(OrchSending::Paused), true, None, 10),
            OrchRoute::Restricted { .. }
        ));
    }

    #[test]
    fn the_pair_limit_is_the_smaller_one_and_unlimited_never_holds() {
        assert_eq!(pair_limit(Some(20), Some(5)), Some(5));
        assert_eq!(pair_limit(None, Some(7)), Some(7));
        assert_eq!(pair_limit(Some(7), None), Some(7));
        assert_eq!(pair_limit(None, None), None);
        assert!(!needs_person(u64::MAX, None));
        assert!(!needs_person(19, Some(20)));
        assert!(needs_person(20, Some(20)));
    }

    #[test]
    fn a_resume_restarts_the_window() {
        let now = 10 * ORCH_TALK_WINDOW_MS;
        assert_eq!(talk_window_start(now, None), now - ORCH_TALK_WINDOW_MS);
        assert_eq!(talk_window_start(now, Some(now - 5)), now - 5);
        assert_eq!(
            talk_window_start(now, Some(1)),
            now - ORCH_TALK_WINDOW_MS,
            "an old resume changes nothing"
        );
        assert_eq!(talk_window_start(5, None), 0);
    }

    #[test]
    fn a_grant_ends_in_the_future_within_a_day() {
        assert!(valid_grant(10, 11));
        assert!(valid_grant(10, 10 + ORCH_MAX_GRANT_MS));
        assert!(!valid_grant(10, 10));
        assert!(!valid_grant(10, 11 + ORCH_MAX_GRANT_MS));
    }
}
