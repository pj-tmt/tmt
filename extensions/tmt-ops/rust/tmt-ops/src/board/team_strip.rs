//! M5 TeamStrip: observed members once each, with the observed lead first.
use crate::{board::m6_state_cell, look::Look};
use ratatui::text::{Line, Span};
use std::collections::BTreeSet;
use tmt_cli_style::Role;

pub(in crate::board) struct Member<'a> {
    pub id: &'a str,
    pub state: Option<&'a str>,
}
pub(in crate::board) fn line(
    members: &[Member<'_>],
    lead: Option<&str>,
    width: usize,
    look: Look,
) -> Line<'static> {
    let first = lead.and_then(|id| members.iter().find(|member| member.id == id));
    let mut seen = BTreeSet::new();
    let ordered: Vec<_> = first
        .into_iter()
        .chain(members.iter())
        .filter(|member| seen.insert(member.id))
        .collect();
    let capacity = width.div_ceil(2);
    let overflow = ordered.len() > capacity;
    let count = if overflow {
        capacity.saturating_sub(1)
    } else {
        ordered.len()
    };
    let mut spans = Vec::new();
    for member in ordered.into_iter().take(count) {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(m6_state_cell::mark(member.state, look));
    }
    if overflow && width > 0 {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled("…", look.role(Role::Muted)));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lead_is_first_without_inventing_or_repeating_members() {
        let look = Look::new(tmt_cli_style::Theme::default());
        let members = [
            Member {
                id: "worker",
                state: Some("working"),
            },
            Member {
                id: "lead",
                state: Some("waiting"),
            },
            Member {
                id: "worker",
                state: Some("working"),
            },
            Member {
                id: "idle",
                state: Some("idle"),
            },
        ];
        assert_eq!(line(&members, Some("lead"), 11, look).to_string(), "◆ ● ◌");
        assert_eq!(
            line(&members, Some("missing"), 11, look).to_string(),
            "● ◆ ◌"
        );
        for width in 0..8 {
            assert!(line(&members, Some("lead"), width, look).width() <= width);
        }
        assert_eq!(line(&members, Some("lead"), 3, look).to_string(), "◆ …");
    }
}
