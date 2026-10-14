//! Counts intersect the held roster and typed observations; missing is not zero.
use crate::{
    board::workspace::{OpsObservation, observe::OpsWorkspaceSnapshot},
    labels::{Supplied, digest::Mode},
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
}

pub(super) fn project(snapshot: &OpsWorkspaceSnapshot, labels: &Supplied) -> Summary {
    let roster = &snapshot.roster;
    let mut summary = Summary {
        total: roster.members.len(),
        complete: roster.observation == OpsObservation::Complete
            && roster.membership_complete
            && roster
                .squads
                .iter()
                .all(|squad| squad.observation == OpsObservation::Complete),
        ..Default::default()
    };
    let digest = labels
        .digest()
        .filter(|(_, stale)| !stale)
        .map(|(status, _)| status);
    summary.digest_available = digest.is_some();
    for (id, member) in &roster.members {
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
    pub fn digest_line(&self) -> String {
        if !self.digest_available {
            return "Digest status unavailable".into();
        }
        let held = if self.unknown_held == self.total && self.total > 0 {
            "held unavailable".into()
        } else if self.unknown_held > 0 {
            format!("≥{} observed held", self.held)
        } else {
            format!("{} held", self.held)
        };
        format!(
            "Digest · {held} · {} Auto · {} timed · {} Off{}",
            self.auto,
            self.timed,
            self.off,
            if self.unknown_mode > 0 {
                format!(" · {} mode unknown", self.unknown_mode)
            } else {
                String::new()
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::workspace::OpsMember;

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
        assert!(summary.digest_line().contains("0 held"));
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
        assert_eq!(stale.digest_line(), "Digest status unavailable");
        assert_eq!(
            project(&OpsWorkspaceSnapshot::default(), &Supplied::default()).digest_line(),
            "Digest status unavailable"
        );
        assert!(
            project(&OpsWorkspaceSnapshot::default(), &Supplied::default())
                .agents_line(false)
                .contains("totals unknown")
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
