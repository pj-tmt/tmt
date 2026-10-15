//! Counts intersect the held roster and typed observations; missing is not zero.
use crate::{
    board::workspace::{OpsObservation, observe::OpsWorkspaceSnapshot},
    config::{DigestWindow, SavingsUnit},
    labels::{
        Supplied,
        digest::{Mode, SavingsSummary},
    },
};
use unicode_width::UnicodeWidthStr;

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

const SEPARATOR: &str = " · ";

impl Summary {
    pub fn agents_line(&self, ascii: bool) -> String {
        let unknown = if self.unknown_state > 0 {
            format!(" · {} state unknown", self.unknown_state)
        } else {
            String::new()
        };
        format!(
            "{}{} {}{} · {} {} working · {} {} {} you{}",
            if self.complete { "" } else { "Observed " },
            self.total,
            if self.total == 1 { "agent" } else { "agents" },
            if self.complete {
                ""
            } else {
                " · totals unknown"
            },
            if ascii { "+" } else { "●" },
            self.working,
            if ascii { "!" } else { "◆" },
            self.needs_you,
            if self.needs_you == 1 { "needs" } else { "need" },
            unknown,
        )
    }
    /// Structural marks carry the existing state roles; counts remain readable as words.
    pub fn agents_paint(
        &self,
        ascii: bool,
        look: crate::look::Look,
    ) -> ratatui::text::Line<'static> {
        use ratatui::text::Span;
        use tmt_cli_style::Role;
        let text = self.agents_line(ascii);
        let marks = if ascii { ['+', '!'] } else { ['●', '◆'] };
        let mut spans = Vec::new();
        let mut from = 0;
        for (at, mark) in text.match_indices(&marks[..]) {
            spans.push(Span::styled(
                text[from..at].to_owned(),
                look.role(Role::Text),
            ));
            let role = if mark == marks[0].to_string() {
                Role::Working
            } else if self.needs_you > 0 {
                Role::Waiting
            } else {
                Role::Muted
            };
            spans.push(Span::styled(mark.to_owned(), look.role(role)));
            from = at + mark.len();
        }
        spans.push(Span::styled(text[from..].to_owned(), look.role(Role::Text)));
        ratatui::text::Line::from(spans)
    }
    /// The measured counts, then the estimate (with its separator) only when it
    /// fits whole: a truncated figure would read as a smaller one.
    pub fn digest_parts(&self, width: usize) -> (String, Option<String>) {
        if !self.digest_available {
            return ("Digest status unavailable".into(), None);
        }
        let held = if self.unknown_held == self.total && self.total > 0 {
            "held unavailable".into()
        } else if self.unknown_held > 0 {
            format!("≥{} observed held", self.held)
        } else {
            format!("{} held", self.held)
        };
        let counts = format!(
            "Digest · {held} · {} Auto · {} timed · {} Off{}",
            self.auto,
            self.timed,
            self.off,
            if self.unknown_mode > 0 {
                format!(" · {} mode unknown", self.unknown_mode)
            } else {
                String::new()
            }
        );
        let room = width.saturating_sub(counts.width() + SEPARATOR.width());
        let savings = crate::board::digest_dashboard::savings_caption(
            &self.savings,
            self.window,
            self.unit,
            room,
        );
        let estimate = (savings.width() <= room).then(|| format!("{SEPARATOR}{savings}"));
        (counts, estimate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::workspace::OpsMember;

    fn project(snapshot: &OpsWorkspaceSnapshot, labels: &Supplied) -> Summary {
        super::project(snapshot, labels, &Default::default())
    }

    impl Summary {
        fn digest_line(&self, width: usize) -> String {
            let (counts, estimate) = self.digest_parts(width);
            counts + &estimate.unwrap_or_default()
        }
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
        assert!(summary.digest_line(80).contains("0 held"));
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
        assert_eq!(stale.digest_line(80), "Digest status unavailable");
        assert_eq!(
            project(&OpsWorkspaceSnapshot::default(), &Supplied::default()).digest_line(80),
            "Digest status unavailable"
        );
        assert!(
            project(&OpsWorkspaceSnapshot::default(), &Supplied::default())
                .agents_line(false)
                .contains("totals unknown")
        );
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
        let line = project(&snapshot, &labels).digest_line(160);
        assert!(
            line.ends_with(" · Est. ctx avoided 24h: 20.0k (2 of 3)"),
            "{line}"
        );
        // The window is the Digest preference; evidence for another window is
        // never relabelled, and never counted as zero.
        snapshot.preferences.digest_window = DigestWindow::Week;
        let line = project(&snapshot, &labels).digest_line(160);
        assert!(
            line.ends_with(" · Est. ctx avoided 7d: — (0 of 3)"),
            "{line}"
        );
        // Scope follows the counts above it: hidden members leave both.
        snapshot.preferences.digest_window = DigestWindow::Day;
        let scoped = crate::board::workspace::OpsVisibility {
            scope: vec!["no-such-room".into()],
            reveal_hidden: false,
        };
        let line = super::project(&snapshot, &labels, &scoped).digest_line(160);
        assert!(line.ends_with(" · Est. ctx avoided 24h: —"), "{line}");
    }

    #[test]
    fn the_estimate_is_absent_when_digest_is_unavailable_and_never_truncated() {
        let current = Supplied::from_document(
            "digest",
            &savings_document("24h", &[(FIRST, 18_200)]),
            false,
        );
        let stale =
            Supplied::from_document("digest", &savings_document("24h", &[(FIRST, 18_200)]), true);
        let snapshot = roster(&[FIRST]);
        assert_eq!(
            project(&snapshot, &stale).digest_line(160),
            "Digest status unavailable"
        );
        let summary = project(&snapshot, &current);
        let counts = summary.digest_line(0);
        assert!(!counts.contains("Est."), "{counts}");
        let width = |text: &str| unicode_width::UnicodeWidthStr::width(text);
        let full = " · Est. ctx avoided 24h: 18.2k";
        let compact = " · Est. avoided 24h: 18.2k";
        assert_eq!(
            summary.digest_line(width(&counts) + width(full)),
            format!("{counts}{full}")
        );
        assert_eq!(
            summary.digest_line(width(&counts) + width(full) - 1),
            format!("{counts}{compact}")
        );
        // Neither form fits: the whole fact goes, not its last digits.
        assert_eq!(
            summary.digest_line(width(&counts) + width(compact) - 1),
            counts
        );
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
