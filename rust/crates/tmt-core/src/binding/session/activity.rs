//! Last authoritative main-turn event, never a utilization or silence heuristic.
use super::ProviderSessionId;
use crate::{endpoint::ProcessIncarnation, limits::is_valid_js_safe_integer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityPhase {
    Working,
    Idle,
}
impl ActivityPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Idle => "idle",
        }
    }
}

/// The event ordering contract belongs to the driver. `turn` is a provider ID,
/// never a locally manufactured sequence. None requires synchronous hook ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub phase: ActivityPhase,
    pub turn: Option<ProviderSessionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub session: ProviderSessionId,
    pub process: ProcessIncarnation,
    pub phase: ActivityPhase,
    pub turn: Option<ProviderSessionId>,
    pub since_ms: u64,
    pub last_activity_ms: u64,
}

impl Activity {
    pub fn matches(&self, session: &ProviderSessionId, process: &ProcessIncarnation) -> bool {
        &self.session == session && &self.process == process
    }

    /// None means no admitted transition, including duplicates, stale stops and
    /// clock rollback. Rejected events cannot renew timestamps.
    pub fn observe(
        previous: Option<&Self>,
        session: &ProviderSessionId,
        process: &ProcessIncarnation,
        event: &Event,
        now: u64,
    ) -> Option<Self> {
        if now == 0 || !is_valid_js_safe_integer(now) {
            return None;
        }
        let previous = previous.filter(|value| value.matches(session, process));
        if let Some(previous) = previous {
            if now < previous.last_activity_ms {
                return None;
            }
            if event.turn == previous.turn {
                if event.phase == previous.phase
                    || event.phase == ActivityPhase::Working && event.turn.is_some()
                {
                    return None;
                }
            } else if event.phase == ActivityPhase::Idle {
                return None;
            }
        } else if event.phase == ActivityPhase::Idle {
            // An end needs an admitted start in this exact session/incarnation.
            return None;
        }
        Some(Self {
            session: session.clone(),
            process: process.clone(),
            phase: event.phase,
            turn: event.turn.clone(),
            since_ms: now,
            last_activity_ms: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(phase: ActivityPhase, turn: Option<&str>) -> Event {
        Event {
            phase,
            turn: turn.map(|id| ProviderSessionId::new(id).unwrap()),
        }
    }
    #[test]
    fn correlated_turns_reject_duplicates_old_stops_and_clock_rollback() {
        let session = ProviderSessionId::new("session").unwrap();
        let process = ProcessIncarnation::new(12, "start").unwrap();
        let start = event(ActivityPhase::Working, Some("a"));
        let a = Activity::observe(None, &session, &process, &start, 10).unwrap();
        assert!(Activity::observe(Some(&a), &session, &process, &start, 11).is_none());
        let idle = Activity::observe(
            Some(&a),
            &session,
            &process,
            &event(ActivityPhase::Idle, Some("a")),
            12,
        )
        .unwrap();
        assert!(Activity::observe(Some(&idle), &session, &process, &start, 13).is_none());
        let b = Activity::observe(
            Some(&idle),
            &session,
            &process,
            &event(ActivityPhase::Working, Some("b")),
            14,
        )
        .unwrap();
        assert!(
            Activity::observe(
                Some(&b),
                &session,
                &process,
                &event(ActivityPhase::Idle, Some("a")),
                15
            )
            .is_none()
        );
        assert!(
            Activity::observe(
                Some(&b),
                &session,
                &process,
                &event(ActivityPhase::Idle, Some("b")),
                13
            )
            .is_none()
        );
        assert!(
            Activity::observe(
                None,
                &session,
                &process,
                &event(ActivityPhase::Idle, Some("b")),
                15
            )
            .is_none()
        );
    }
    #[test]
    fn synchronous_events_do_not_inherit_other_sessions_or_processes() {
        let session = ProviderSessionId::new("session").unwrap();
        let process = ProcessIncarnation::new(12, "start").unwrap();
        let working = event(ActivityPhase::Working, None);
        let a = Activity::observe(None, &session, &process, &working, 10).unwrap();
        assert!(Activity::observe(Some(&a), &session, &process, &working, 11).is_none());
        let idle = Activity::observe(
            Some(&a),
            &session,
            &process,
            &event(ActivityPhase::Idle, None),
            12,
        )
        .unwrap();
        assert!(Activity::observe(Some(&idle), &session, &process, &working, 13).is_some());
        let replacement = ProcessIncarnation::new(12, "new-start").unwrap();
        let fresh = Activity::observe(Some(&a), &session, &replacement, &working, 11).unwrap();
        assert_eq!(fresh.process, replacement);
        assert_eq!(fresh.since_ms, 11);
    }
}
