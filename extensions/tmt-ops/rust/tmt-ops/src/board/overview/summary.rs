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
    pub working: usize,
    pub needs_you: usize,
    pub unknown_state: usize,
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
    for (id, member) in &roster.members {
        if visible.as_ref().is_some_and(|ids| !ids.contains(id)) {
            continue;
        }
        if member.state.as_deref() == Some("working") {
            summary.working += 1;
        }
        if super::sort::needs_attention(member.state.as_deref()) {
            summary.needs_you += 1;
        }
        if member.state.is_none() {
            summary.unknown_state += 1;
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
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::workspace::OpsMember;

    fn project(snapshot: &OpsWorkspaceSnapshot, labels: &Supplied) -> Summary {
        super::project(snapshot, labels, &Default::default())
    }

    #[test]
    fn digest_intersects_the_roster_and_stale_counts_are_unknown() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tmt-digest/contracts/vectors/status-v1.json"
        )))
        .unwrap();
        let document = &vectors["status"][0]["response"];
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
        let vectors: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tmt-digest/contracts/vectors/status-v1.json"
        )))
        .unwrap();
        let mut document = vectors["status"][0]["response"].clone();
        let rows: Vec<_> = document["members"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| {
                let (_, avoided) = tokens
                    .iter()
                    .find(|(id, _)| Some(*id) == row["identityId"].as_str())?;
                let mut row = row.clone();
                row["savings"] = serde_json::json!({
                    "method": "context-input-avoided-v1", "window": window,
                    "fromMs": 0, "throughMs": 1000,
                    "estimatedInputTokensAvoided": avoided, "complete": true, "gap": null
                });
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
        let summary = super::project(&snapshot, &labels, &scoped);
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
        for (id, state) in [
            ("working", Some("working")),
            ("waiting", Some("waiting")),
            ("unknown", None),
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
                summary.unknown_state
            ),
            (3, 1, 1, 1)
        );
        assert!(!summary.complete);
        assert_eq!(
            (
                summary.off,
                summary.held,
                summary.unknown_mode,
                summary.unknown_held
            ),
            (0, 0, 3, 3)
        );
    }
}
