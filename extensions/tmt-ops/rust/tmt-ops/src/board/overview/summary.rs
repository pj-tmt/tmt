//! Counts intersect the held roster and typed observations; missing is not zero.
use crate::{
    board::workspace::{OpsObservation, observe::OpsWorkspaceSnapshot},
    config::{DigestWindow, SavingsUnit},
    labels::{
        Supplied,
        digest::{Mode, SavingsSummary},
    },
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::board) struct Summary {
    pub total: usize,
    pub complete: bool,
    pub digest_available: bool,
    /// Agents core observes as working, each identity once.
    pub working: usize,
    pub needs_you: usize,
    /// Agents whose activity core could not observe.
    pub unknown_activity: usize,
    pub auto: usize,
    pub timed: usize,
    pub off: usize,
    pub unknown_mode: usize,
    pub held: u64,
    pub unknown_held: usize,
    /// The shared estimate over the same members as the counts above.
    pub savings: SavingsSummary,
    pub window: DigestWindow,
    pub unit: SavingsUnit,
}

pub(super) fn project(
    snapshot: &OpsWorkspaceSnapshot,
    labels: &Supplied,
    visibility: &crate::board::workspace::OpsVisibility,
    activity_stale: bool,
) -> Summary {
    let roster = &snapshot.roster;
    // Visibility affects this view's counts, never the acquired global roster
    // or shell attention. Search and the needs-you filter do not affect totals.
    let visible = visibility.member_ids(roster, &snapshot.preferences);
    let mut summary = Summary {
        total: visible
            .as_ref()
            .map_or(roster.members.len(), |ids| ids.len()),
        complete: roster.observation == OpsObservation::Complete
            && roster.membership_complete
            && roster
                .squads
                .iter()
                .filter(|squad| visibility.shows_squad(&squad.room_id, &snapshot.preferences))
                .all(|squad| squad.observation == OpsObservation::Complete),
        window: snapshot.preferences.digest_window,
        unit: snapshot.preferences.savings_unit,
        ..Default::default()
    };
    let digest = labels
        .digest()
        .filter(|(_, stale)| !stale)
        .map(|(status, _)| status);
    summary.digest_available = digest.is_some();
    summary.savings = crate::labels::digest::summarize_savings(
        labels.digest(),
        roster
            .members
            .keys()
            .filter(|id| visible.as_ref().is_none_or(|ids| ids.contains(id)))
            .map(String::as_str),
        summary.window,
    );
    let mut activity = crate::board::activity::Tally::default();
    for (id, member) in &roster.members {
        if visible.as_ref().is_some_and(|ids| !ids.contains(id)) {
            continue;
        }
        // Agent activity is core's observation; the task state stays a human fact.
        activity.add(member.activity, activity_stale);
        if super::sort::needs_attention(member.state.as_deref()) {
            summary.needs_you += 1;
        }
        if let Some(status) = digest.and_then(|status| status.members.get(id)) {
            match status.mode {
                Mode::Auto => summary.auto += 1,
                Mode::Interval => summary.timed += 1,
                Mode::Off => summary.off += 1,
            }
            if let Some(held) = status.held_count {
                summary.held = summary.held.saturating_add(held);
            } else {
                summary.unknown_held += 1;
            }
        } else {
            summary.unknown_mode += 1;
            summary.unknown_held += 1;
        }
    }
    summary.working = activity.working;
    summary.unknown_activity = activity.unknown;
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::workspace::OpsMember;

    fn project(snapshot: &OpsWorkspaceSnapshot, labels: &Supplied) -> Summary {
        super::project(snapshot, labels, &Default::default(), false)
    }

    #[test]
    fn digest_intersects_the_roster_and_stale_counts_are_unknown() {
        let document = &crate::labels::tests::status_document();
        let id = document["members"][0]["identityId"].as_str().unwrap();
        let mut snapshot = OpsWorkspaceSnapshot::default();
        snapshot.roster.members.insert(
            id.into(),
            OpsMember {
                id: id.into(),
                name: "observed agent".into(),
                state: Some("working".into()),
                waiting: false,
                presence: None,
                usage: None,
                activity: Default::default(),
            },
        );
        snapshot.roster.observation = OpsObservation::Complete;
        snapshot.roster.membership_complete = true;
        let current = Supplied::from_document("digest", document, false);
        let summary = project(&snapshot, &current);
        assert_eq!(
            (
                summary.auto,
                summary.timed,
                summary.off,
                summary.unknown_mode
            ),
            (1, 0, 0, 0)
        );
        assert_eq!(
            summary.held, 0,
            "other identities' held items do not contribute"
        );
        assert!(summary.complete);
        assert!(summary.digest_available);
        assert_eq!(summary.unknown_held, 0);
        let stale = project(
            &snapshot,
            &Supplied::from_document("digest", document, true),
        );
        assert_eq!(
            (
                stale.auto,
                stale.off,
                stale.unknown_mode,
                stale.unknown_held
            ),
            (0, 0, 1, 1)
        );
        assert!(!stale.digest_available);
        let unavailable = project(&OpsWorkspaceSnapshot::default(), &Supplied::default());
        assert!(!unavailable.digest_available && !unavailable.complete);
    }

    const FIRST: &str = "10000000-0000-4000-8000-000000000001";
    const SECOND: &str = "10000000-0000-4000-8000-000000000002";
    const THIRD: &str = "10000000-0000-4000-8000-000000000003";

    fn savings_document(window: &str, tokens: &[(&str, u64)]) -> serde_json::Value {
        let mut document = crate::labels::tests::status_document();
        let rows: Vec<_> = document["members"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| {
                let (_, avoided) = tokens
                    .iter()
                    .find(|(id, _)| Some(*id) == row["identityId"].as_str())?;
                let mut row = row.clone();
                row["savings"] = crate::labels::tests::savings(window, 0, 1000, *avoided);
                Some(row)
            })
            .collect();
        document["members"] = rows.into();
        document
    }

    fn roster(ids: &[&str]) -> OpsWorkspaceSnapshot {
        let mut snapshot = OpsWorkspaceSnapshot::default();
        for id in ids {
            snapshot.roster.members.insert(
                (*id).into(),
                OpsMember {
                    id: (*id).into(),
                    name: (*id).into(),
                    state: None,
                    waiting: false,
                    presence: None,
                    usage: None,
                    activity: Default::default(),
                },
            );
        }
        snapshot.roster.observation = OpsObservation::Complete;
        snapshot.roster.membership_complete = true;
        snapshot
    }

    #[test]
    fn the_estimate_sums_the_roster_for_the_selected_window_and_names_coverage() {
        let labels = Supplied::from_document(
            "digest",
            &savings_document("24h", &[(FIRST, 18_200), (SECOND, 1_800)]),
            false,
        );
        let mut snapshot = roster(&[FIRST, SECOND, THIRD]);
        let summary = project(&snapshot, &labels);
        assert_eq!(
            (
                summary.savings.tokens,
                summary.savings.included,
                summary.savings.total
            ),
            (Some(20_000), 2, 3)
        );
        snapshot.preferences.digest_window = DigestWindow::Week;
        let summary = project(&snapshot, &labels);
        assert_eq!(
            (
                summary.savings.tokens,
                summary.savings.included,
                summary.savings.total
            ),
            (None, 0, 3)
        );
        snapshot.preferences.digest_window = DigestWindow::Day;
        let scoped = crate::board::workspace::OpsVisibility {
            scope: vec!["no-such-room".into()],
            reveal_hidden: false,
        };
        let summary = super::project(&snapshot, &labels, &scoped, false);
        assert_eq!((summary.savings.tokens, summary.savings.total), (None, 0));
    }
    #[test]
    fn unavailable_estimates_stay_unknown_and_current_values_keep_every_digit() {
        let document = savings_document("24h", &[(FIRST, 18_200)]);
        let snapshot = roster(&[FIRST]);
        let stale = project(
            &snapshot,
            &Supplied::from_document("digest", &document, true),
        );
        assert!(!stale.digest_available);
        assert_eq!(stale.savings.tokens, None);
        let current = project(
            &snapshot,
            &Supplied::from_document("digest", &document, false),
        );
        assert_eq!(current.savings.tokens, Some(18_200));
        assert_eq!((current.savings.included, current.savings.total), (1, 1));
    }

    #[test]
    fn unknown_modes_and_partial_counts_do_not_become_off_or_complete() {
        let mut snapshot = OpsWorkspaceSnapshot::default();
        use crate::board::activity::{ObservedActivity, Phase};
        for (id, state, phase) in [
            ("working", Some("working"), Phase::Working),
            ("waiting", Some("waiting"), Phase::Idle),
            ("unknown", None, Phase::Unknown),
            ("ended", Some("working"), Phase::Ended),
        ] {
            snapshot.roster.members.insert(
                id.into(),
                OpsMember {
                    id: id.into(),
                    name: id.into(),
                    state: state.map(str::to_owned),
                    waiting: false,
                    presence: None,
                    usage: None,
                    activity: ObservedActivity {
                        phase,
                        ..Default::default()
                    },
                },
            );
        }
        snapshot.roster.observation = OpsObservation::Partial;
        let summary = project(&snapshot, &Supplied::default());
        assert_eq!(
            (
                summary.total,
                summary.working,
                summary.needs_you,
                summary.unknown_activity
            ),
            (4, 1, 1, 1)
        );
        assert!(!summary.complete);
        assert_eq!(
            (
                summary.off,
                summary.held,
                summary.unknown_mode,
                summary.unknown_held
            ),
            (0, 0, 4, 4)
        );
    }

    /// Builds the workspace the way the board does: global core `ls` rows
    /// (which carry no squad state) joined with one squad's roster document.
    fn acquired(listed: serde_json::Value, rows: Vec<serde_json::Value>) -> OpsWorkspaceSnapshot {
        let root = std::env::temp_dir().join(format!(
            "ops-summary-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("ops.toml"), "").unwrap();
        let config = crate::config::Config::read(root.join("ops.toml")).unwrap();
        let core = crate::core::Core::at(root.join("core"));
        let squad = crate::squad::Squad {
            name: "a".into(),
            room_id: "room-a".into(),
        };
        let mut acquired = crate::tab_view::roster_documents(&core, &config, &[], None, None);
        acquired.include(
            &config,
            "a",
            serde_json::json!({"squad": {"name": "a", "lead": null}, "sections": [{"rows": rows}]}),
        );
        let snapshot = OpsWorkspaceSnapshot::from_acquired(&[&squad], &acquired, &Ok(listed));
        std::fs::remove_dir_all(&root).unwrap();
        snapshot
    }

    #[test]
    fn working_counts_agents_whose_core_activity_is_working() {
        let task = |id: &str, state: &str| {
            serde_json::json!({"id": id, "name": id, "state": state, "pending": null,
                "fields": {"state": state}, "waitingOnYou": []})
        };
        let listed = |id: &str, phase: &str| {
            serde_json::json!({"id": id, "name": id, "presence": "active",
                "session": {"activity": {"state": phase, "sinceMs": 1, "lastActivityMs": 1,
                "providers": {}}}})
        };
        let snapshot = acquired(
            serde_json::json!({"identities": [listed("busy", "working"), listed("quiet", "idle")]}),
            vec![task("busy", "working"), task("quiet", "working")],
        );
        // Only the agent core observes as working counts; a task named
        // "working" on an idle agent does not.
        assert_eq!(project(&snapshot, &Supplied::default()).working, 1);
    }
}
