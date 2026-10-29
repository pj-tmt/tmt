//! Per-recipient projection and monotonic scheduling; neither grants authority.
use super::{SquadObservation, Summary};
use crate::{authority::GrantAuthority, error::RemoteError, limits};
use sha2::{Digest, Sha256};

/// Provider storage is publication evidence, never receiver delivery.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Publication {
    Stored,
    Unobserved,
}

#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    pub bytes: Vec<u8>,
    pub fingerprint: [u8; 32],
}
impl Prepared {
    pub fn for_peer(summary: &Summary, authority: &GrantAuthority) -> Result<Self, RemoteError> {
        summary.validate()?;
        let mut filtered = summary.clone();
        if let SquadObservation::Available { squads } = &mut filtered.observation {
            for squad in squads.iter_mut() {
                if squad
                    .lead
                    .as_ref()
                    .is_some_and(|lead| !authority.permits(&lead.id))
                {
                    squad.lead = None;
                }
                squad.members.retain(|member| authority.permits(&member.id));
                squad.members.sort_by(|a, b| a.id.cmp(&b.id));
            }
            // A hidden-only squad must not leak its name through an empty group.
            squads.retain(|squad| squad.lead.is_some() || !squad.members.is_empty());
            squads.sort_by(|a, b| {
                a.squad.cmp(&b.squad).then_with(|| {
                    a.members
                        .iter()
                        .map(|m| &m.id)
                        .cmp(b.members.iter().map(|m| &m.id))
                })
            });
        }
        let bytes = filtered.bytes()?;
        let fingerprint = Self::published_fingerprint(&filtered)?;
        Ok(Self { bytes, fingerprint })
    }
    /// Completion identifies the exact immutable payload already published.
    /// Applying a newer grant here would mislabel old broad data as narrow data.
    pub fn published_fingerprint(summary: &Summary) -> Result<[u8; 32], RemoteError> {
        let mut exact = summary.clone();
        exact.observed_at = 0;
        Ok(Sha256::digest(exact.bytes()?).into())
    }
}

/// Only a verified capabilities response reaches this feature projection. It
/// never grants permission; current native grants remain the publication owner.
pub(crate) fn supports_summary(value: &serde_json::Value) -> Result<bool, RemoteError> {
    let invalid = || {
        RemoteError::new(
            "REMOTE_SUMMARY_CAPABILITY_UNAVAILABLE",
            "Summary capability is unavailable.",
        )
    };
    if value["version"] != 1 {
        return Err(invalid());
    }
    let operations = value["operations"]
        .as_array()
        .filter(|ops| ops.len() <= limits::SQUAD_SUMMARY_CAPABILITIES)
        .ok_or_else(invalid)?;
    if operations.iter().any(|op| op.as_str().is_none()) {
        return Err(invalid());
    }
    Ok(operations.iter().any(|op| op == "squads.summary"))
}

/// One completion-based cadence shared by all recipients. The Service holds
/// its sampling mutex through the read, so another caller cannot overlap it.
#[derive(Default)]
pub(crate) struct SummarySampling {
    finished: Option<u64>,
    generation: u64,
    latest: Option<(u64, Summary)>,
}
impl SummarySampling {
    pub fn due(&self, now: u64) -> bool {
        self.finished
            .is_none_or(|end| now.saturating_sub(end) >= limits::SQUAD_SUMMARY_SAMPLE_MS)
    }
    pub fn finished(&mut self, end: u64) {
        self.finished = Some(end);
    }
    pub fn record(&mut self, summary: Summary) {
        self.generation = self.generation.saturating_add(1);
        self.latest = Some((self.generation, summary));
    }
    pub fn latest(&self) -> Option<&(u64, Summary)> {
        self.latest.as_ref()
    }
}
#[derive(Debug, Default, PartialEq, Eq)]
enum SummaryFailure {
    #[default]
    None,
    Holding,
    Continuing,
    Ready,
}

/// Monotonic deadlines restored from persisted ages, never backdated into a new
/// process's zero-based clock. Restart does not reset an almost-due heartbeat.
#[derive(Default)]
pub(crate) struct Schedule {
    latest: Option<Prepared>,
    changed_due: Option<u64>,
    published: Option<[u8; 32]>,
    next_allowed: u64,
    heartbeat_due: Option<u64>,
    blocked_until: u64,
    sample_generation: Option<u64>,
    failure: SummaryFailure,
}
impl Schedule {
    pub fn restore(
        published_age: Option<([u8; 32], u64)>,
        changed_age: Option<u64>,
        blocked_remaining: u64,
        now: u64,
    ) -> Self {
        Self {
            published: published_age.map(|(hash, _)| hash),
            next_allowed: published_age.map_or(0, |(_, age)| {
                now.saturating_add(limits::SQUAD_SUMMARY_COALESCE_MS.saturating_sub(age))
            }),
            heartbeat_due: published_age.map(|(_, age)| {
                now.saturating_add(limits::SQUAD_SUMMARY_HEARTBEAT_MS.saturating_sub(age))
            }),
            changed_due: changed_age.map(|age| {
                now.saturating_add(limits::SQUAD_SUMMARY_COALESCE_MS.saturating_sub(age))
            }),
            blocked_until: now.saturating_add(blocked_remaining),
            ..Self::default()
        }
    }
    /// Publication certainty may be lost while its last confirmed time remains
    /// known. Failure grace must not be renewed by an uncertain newer attempt.
    pub fn restore_publication_age(&mut self, age: u64, now: u64) {
        self.heartbeat_due =
            Some(now.saturating_add(limits::SQUAD_SUMMARY_HEARTBEAT_MS.saturating_sub(age)));
    }
    /// Only fresh failed reads establish a continuing failure. Cached failures
    /// on five-second publication ticks are not additional observations.
    pub fn observe_sample(
        &mut self,
        next: Prepared,
        retained: Option<Prepared>,
        generation: u64,
        now: u64,
    ) -> Prepared {
        let fresh = self.sample_generation != Some(generation);
        self.sample_generation = Some(generation);
        let available = |p: &Prepared| Summary::read(&p.bytes).is_ok_and(|s| s.squads().is_some());
        let chosen = if !available(&next) && retained.as_ref().is_some_and(available) {
            match self.failure {
                SummaryFailure::None => self.failure = SummaryFailure::Holding,
                SummaryFailure::Holding if fresh => self.failure = SummaryFailure::Continuing,
                _ => {}
            }
            if self.failure == SummaryFailure::Continuing
                && self.heartbeat_due.is_some_and(|at| now >= at)
            {
                self.failure = SummaryFailure::Ready;
            }
            if matches!(
                self.failure,
                SummaryFailure::Holding | SummaryFailure::Continuing
            ) {
                retained.expect("available retained snapshot")
            } else {
                next
            }
        } else {
            self.failure = SummaryFailure::None;
            next
        };
        self.observe(chosen.clone(), now);
        chosen
    }
    pub fn observe(&mut self, next: Prepared, now: u64) {
        if self.published == Some(next.fingerprint) {
            self.changed_due = None;
        } else if self
            .latest
            .as_ref()
            .is_none_or(|old| old.fingerprint != next.fingerprint)
        {
            self.changed_due
                .get_or_insert(now.saturating_add(limits::SQUAD_SUMMARY_COALESCE_MS));
        }
        self.latest = Some(next);
    }
    pub fn due(&self, now: u64) -> Option<&Prepared> {
        let latest = self.latest.as_ref()?;
        if matches!(
            self.failure,
            SummaryFailure::Holding | SummaryFailure::Continuing
        ) && self.changed_due.is_none()
        {
            return None;
        }
        let due = match self.changed_due {
            Some(changed) => changed.max(self.next_allowed),
            None => self.heartbeat_due?,
        };
        (now >= due.max(self.blocked_until)).then_some(latest)
    }
    pub fn published(&mut self, fingerprint: [u8; 32], now: u64) {
        self.published = Some(fingerprint);
        self.next_allowed = now.saturating_add(limits::SQUAD_SUMMARY_COALESCE_MS);
        self.heartbeat_due = Some(now.saturating_add(limits::SQUAD_SUMMARY_HEARTBEAT_MS));
        if self
            .latest
            .as_ref()
            .is_some_and(|latest| latest.fingerprint == fingerprint)
        {
            self.changed_due = None;
        }
    }
    /// An attempted write may have changed the peer even when its reply was lost.
    /// Returning to the old content cannot cancel original-ID recovery or repair.
    pub fn unconfirmed(&mut self, now: u64) {
        self.published = None;
        self.changed_due.get_or_insert(now);
    }
    pub fn defer_until(&mut self, until: u64) {
        self.blocked_until = self.blocked_until.max(until);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authority::{DispatchMode, PermittedAgents},
        squads::{Member, MemberPresence, Squad, State},
    };
    const A: &str = "00000000-0000-4000-8000-000000000001";
    const B: &str = "00000000-0000-4000-8000-000000000002";
    fn summary(at: u64) -> Summary {
        let member = |id: &str| Member {
            id: id.into(),
            name: id.into(),
            state: State::Working,
            presence: MemberPresence::Active,
        };
        Summary {
            version: 1,
            machine_name: "Studio".into(),
            observed_at: at,
            observation: SquadObservation::Available {
                squads: vec![Squad {
                    squad: "Team".into(),
                    lead: Some(member(A)),
                    members: vec![member(B)],
                }],
            },
        }
    }
    fn authority(agents: PermittedAgents) -> GrantAuthority {
        GrantAuthority {
            agents,
            mode: DispatchMode::Hold,
        }
    }
    fn prepared(at: u64) -> Prepared {
        Prepared::for_peer(&summary(at), &authority(PermittedAgents::All)).unwrap()
    }
    #[test]
    fn recipient_projection_never_widens_read_only_allowlist() {
        let input = summary(1);
        let only_b = authority(PermittedAgents::Selected(vec![B.into()]));
        let prepared = Prepared::for_peer(&input, &only_b).unwrap();
        let output = Summary::read(&prepared.bytes).unwrap();
        assert!(output.squads().unwrap()[0].lead.is_none());
        assert_eq!(output.squads().unwrap()[0].members[0].id, B);
        assert!(input.squads().unwrap()[0].lead.is_some());
        let empty =
            Prepared::for_peer(&input, &authority(PermittedAgents::Selected(vec![]))).unwrap();
        assert!(
            Summary::read(&empty.bytes)
                .unwrap()
                .squads()
                .unwrap()
                .is_empty()
        );
        let mut hidden = input.clone();
        if let SquadObservation::Available { squads } = &mut hidden.observation {
            squads[0].lead.as_mut().unwrap().name = "PRIVATE".into();
        }
        assert_eq!(
            prepared.fingerprint,
            Prepared::for_peer(&hidden, &only_b).unwrap().fingerprint
        );
    }
    #[test]
    fn timestamps_do_not_publish_but_availability_and_visible_state_do() {
        assert_eq!(prepared(1).fingerprint, prepared(999).fingerprint);
        let unavailable = Summary::unavailable("Studio", 2).unwrap();
        assert_ne!(
            prepared(1).fingerprint,
            Prepared::for_peer(&unavailable, &authority(PermittedAgents::All))
                .unwrap()
                .fingerprint
        );
        let mut changed = summary(2);
        if let SquadObservation::Available { squads } = &mut changed.observation {
            squads[0].members[0].state = State::Blocked;
        }
        assert_ne!(
            prepared(1).fingerprint,
            Prepared::for_peer(&changed, &authority(PermittedAgents::All))
                .unwrap()
                .fingerprint
        );
    }
    #[test]
    fn coalesces_latest_without_extending_the_window_and_heartbeats() {
        let mut schedule = Schedule::default();
        assert!(schedule.due(u64::MAX).is_none());
        schedule.observe(prepared(1), 100);
        schedule.observe(prepared(2), 200);
        assert!(schedule.due(5099).is_none());
        let published = schedule.due(5100).unwrap().clone();
        assert_eq!(Summary::read(&published.bytes).unwrap().observed_at, 2);
        schedule.published(published.fingerprint, 5100);
        schedule.observe(prepared(3), 6000);
        assert!(schedule.due(305099).is_none());
        assert!(schedule.due(305100).is_some());
    }
    #[test]
    fn budget_hold_keeps_newest_pending_and_old_completion_cannot_clear_it() {
        let mut schedule = Schedule::default();
        let first = prepared(1);
        schedule.observe(first.clone(), 0);
        schedule.defer_until(100_000);
        let next = Prepared::for_peer(
            &Summary::unavailable("Studio", 2).unwrap(),
            &authority(PermittedAgents::All),
        )
        .unwrap();
        schedule.observe(next.clone(), 5001);
        schedule.published(first.fingerprint, 6000);
        assert!(schedule.due(99_999).is_none());
        assert_eq!(schedule.due(100_000).unwrap().fingerprint, next.fingerprint);
    }
    #[test]
    fn reverting_a_coalesced_change_cancels_it_without_delaying_heartbeat() {
        let mut schedule = Schedule::default();
        let original = prepared(1);
        schedule.observe(original.clone(), 0);
        schedule.published(original.fingerprint, 5000);
        let unavailable = Prepared::for_peer(
            &Summary::unavailable("Studio", 2).unwrap(),
            &authority(PermittedAgents::All),
        )
        .unwrap();
        schedule.observe(unavailable, 5100);
        schedule.observe(prepared(3), 5200);
        assert!(schedule.due(10_100).is_none());
        assert!(schedule.due(304_999).is_none());
        assert!(schedule.due(305_000).is_some());
    }
    #[test]
    fn ops_order_is_not_a_semantic_change() {
        let mut input = summary(1);
        if let SquadObservation::Available { squads } = &mut input.observation {
            let lead = squads[0].lead.take().unwrap();
            squads[0].members.push(lead);
        }
        let first = Prepared::for_peer(&input, &authority(PermittedAgents::All)).unwrap();
        if let SquadObservation::Available { squads } = &mut input.observation {
            squads[0].members.reverse();
        }
        assert_eq!(
            first.fingerprint,
            Prepared::for_peer(&input, &authority(PermittedAgents::All))
                .unwrap()
                .fingerprint
        );
    }
    #[test]
    fn restart_preserves_elapsed_heartbeat_and_change_ages_on_a_zero_based_clock() {
        let next = prepared(1);
        let mut schedule = Schedule::restore(Some((next.fingerprint, 299_999)), None, 0, 0);
        schedule.observe(next.clone(), 0);
        assert!(schedule.due(0).is_none());
        assert!(schedule.due(1).is_some());
        let mut overdue = Schedule::restore(Some((next.fingerprint, 400_000)), None, 0, 0);
        overdue.observe(next.clone(), 0);
        assert!(overdue.due(0).is_some());
        let mut pending = Schedule::restore(None, Some(5_000), 0, 0);
        pending.observe(next, 0);
        assert!(pending.due(0).is_some());
    }
    #[test]
    fn returning_to_old_content_does_not_cancel_an_uncertain_publication() {
        let a = prepared(1);
        let b = Prepared::for_peer(
            &Summary::unavailable("Studio", 2).unwrap(),
            &authority(PermittedAgents::All),
        )
        .unwrap();
        let mut schedule = Schedule::default();
        schedule.observe(a.clone(), 0);
        schedule.published(a.fingerprint, 5000);
        schedule.observe(b.clone(), 6000);
        assert!(schedule.due(11000).is_some());
        schedule.unconfirmed(11000);
        schedule.defer_until(71000);
        schedule.observe(a.clone(), 12000);
        assert!(schedule.due(70999).is_none());
        assert_eq!(schedule.due(71000).unwrap().fingerprint, a.fingerprint);
        // This completion belongs to the original B, not the newer desired A.
        schedule.published(b.fingerprint, 71000);
        assert!(schedule.due(75999).is_none());
        assert_eq!(schedule.due(76000).unwrap().fingerprint, a.fingerprint);
    }
    #[test]
    fn capability_support_is_explicit_and_never_inferred_from_labels_or_unknown_fields() {
        use serde_json::json;
        assert!(supports_summary(&json!({"version":1,"operations":["future.read","squads.summary"],"extra":"ignored"})).unwrap());
        assert!(!supports_summary(&json!({"version":1,"operations":["agents.list"],"name":"squads.summary","summarySupported":true})).unwrap());
        for value in [
            json!({"version":1,"operations":{}}),
            json!({"version":2,"operations":["squads.summary"]}),
            json!({"version":1,"operations":[null]}),
            json!({"error":{"code":"PRIVATE"}}),
        ] {
            assert!(supports_summary(&value).is_err());
        }
    }
    #[test]
    fn quiet_day_has_288_publications_despite_five_second_observations() {
        let mut schedule = Schedule::default();
        let mut publications = 0;
        for now in (0..86_400_000u64).step_by(limits::SQUAD_SUMMARY_COALESCE_MS as usize) {
            schedule.observe(prepared(now), now);
            if let Some(frame) = schedule.due(now) {
                let fingerprint = frame.fingerprint;
                publications += 1;
                schedule.published(fingerprint, now);
            }
        }
        assert_eq!(publications, 288);
    }
    #[test]
    fn sampling_waits_a_full_minute_from_read_completion_and_reuses_the_outcome() {
        let mut sample = SummarySampling::default();
        assert!(sample.due(0));
        sample.finished(19_000);
        sample.record(summary(19_000));
        assert!(
            !sample.due(60_000),
            "start time must not anchor the cadence"
        );
        assert!(!sample.due(78_999));
        assert!(sample.due(79_000));
        assert_eq!(sample.latest().unwrap().0, 1);
        assert_eq!(sample.latest().unwrap().1.observed_at, 19_000);
        sample.finished(99_000);
        sample.record(summary(99_000));
        assert!(!sample.due(158_999));
        assert!(sample.due(159_000));
        assert_eq!(sample.latest().unwrap().0, 2);
    }
    fn failure(at: u64) -> Prepared {
        Prepared::for_peer(
            &Summary::unavailable("Studio", at).unwrap(),
            &authority(PermittedAgents::All),
        )
        .unwrap()
    }
    fn published_available() -> (Schedule, Prepared) {
        let good = prepared(1);
        let mut schedule = Schedule::default();
        schedule.observe_sample(good.clone(), None, 1, 0);
        schedule.published(good.fingerprint, 5_000);
        (schedule, good)
    }
    #[test]
    fn a_single_failed_read_keeps_published_content_and_its_original_observation_time() {
        let (mut schedule, good) = published_available();
        let retained = schedule.observe_sample(failure(65_000), Some(good.clone()), 2, 65_000);
        assert_eq!(retained.bytes, good.bytes);
        assert_eq!(Summary::read(&retained.bytes).unwrap().observed_at, 1);
        assert!(schedule.due(70_000).is_none());
        // Cached failures on publication ticks do not establish another failed read,
        // even if the heartbeat becomes due while no new read has completed.
        schedule.observe_sample(failure(65_000), Some(good), 2, 306_000);
        assert!(schedule.due(311_000).is_none());
    }
    #[test]
    fn persistent_failed_reads_publish_unavailable_at_heartbeat_then_recover() {
        let (mut schedule, good) = published_available();
        schedule.observe_sample(failure(65_000), Some(good.clone()), 2, 65_000);
        schedule.observe_sample(failure(125_000), Some(good.clone()), 3, 125_000);
        assert!(schedule.due(130_000).is_none());
        // Two actual failed reads already establish the streak. A cached tick
        // at the deadline can release it, without counting a new failed read.
        schedule.observe_sample(failure(125_000), Some(good), 3, 305_000);
        assert!(schedule.due(309_999).is_none());
        let unavailable = schedule.due(310_000).unwrap().clone();
        assert!(
            Summary::read(&unavailable.bytes)
                .unwrap()
                .squads()
                .is_none()
        );
        schedule.published(unavailable.fingerprint, 310_000);
        let recovered = prepared(365_000);
        schedule.observe_sample(recovered.clone(), Some(unavailable), 5, 365_000);
        assert!(schedule.due(369_999).is_none());
        assert_eq!(schedule.due(370_000).unwrap().bytes, recovered.bytes);
    }
    #[test]
    fn one_success_clears_the_failure_streak_including_at_an_overdue_heartbeat() {
        let (mut schedule, good) = published_available();
        schedule.observe_sample(failure(65_000), Some(good.clone()), 2, 65_000);
        schedule.observe_sample(prepared(125_000), Some(good.clone()), 3, 125_000);
        schedule.observe_sample(failure(310_000), Some(good.clone()), 4, 310_000);
        assert!(
            schedule.due(315_000).is_none(),
            "first new failure cannot flip an overdue heartbeat"
        );
        schedule.observe_sample(failure(370_000), Some(good), 5, 370_000);
        assert!(
            Summary::read(&schedule.due(375_000).unwrap().bytes)
                .unwrap()
                .squads()
                .is_none()
        );
    }
    #[test]
    fn uncertain_attempts_and_restart_do_not_renew_the_failure_heartbeat() {
        let (mut live, good) = published_available();
        live.unconfirmed(65_000);
        live.observe_sample(failure(65_000), Some(good.clone()), 2, 65_000);
        live.observe_sample(failure(125_000), Some(good.clone()), 3, 125_000);
        let due = live.observe_sample(failure(125_000), Some(good.clone()), 3, 305_000);
        assert!(Summary::read(&due.bytes).unwrap().squads().is_none());
        let mut restored = Schedule::restore(None, Some(0), 0, 0);
        restored.restore_publication_age(300_000, 0);
        restored.observe_sample(failure(0), Some(good.clone()), 1, 0);
        assert_eq!(restored.failure, SummaryFailure::Holding);
        let due = restored.observe_sample(failure(60_000), Some(good), 2, 60_000);
        assert!(Summary::read(&due.bytes).unwrap().squads().is_none());
    }
}
