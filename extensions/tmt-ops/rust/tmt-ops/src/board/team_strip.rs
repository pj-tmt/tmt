//! TeamStrip: observed members once each, with the observed lead first.
use crate::{board::state_cell, look::Look};
use ratatui::text::{Line, Span};
use std::collections::BTreeSet;
use tmt_cli_style::Role;

pub(in crate::board) struct Member<'a> {
    pub id: &'a str,
    pub state: Option<&'a str>,
    pub unread: super::unread::Count,
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
    let mut spans = Vec::new();
    let mut used = 0;
    for (index, member) in ordered.iter().enumerate() {
        let entry = state_cell::unread(member.unread, look)
            .unwrap_or_else(|| Line::from(state_cell::mark(member.state, look)));
        let gap = usize::from(!spans.is_empty());
        let tail = if index + 1 < ordered.len() { 2 } else { 0 };
        if used + gap + entry.width() + tail > width {
            if used + gap < width {
                if gap > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled("…", look.role(Role::Muted)));
            }
            break;
        }
        if gap > 0 {
            spans.push(Span::raw(" "));
        }
        used += gap + entry.width();
        spans.extend(entry.spans);
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
                unread: Default::default(),
            },
            Member {
                id: "lead",
                state: Some("waiting"),
                unread: Default::default(),
            },
            Member {
                id: "worker",
                state: Some("working"),
                unread: Default::default(),
            },
            Member {
                id: "idle",
                state: Some("idle"),
                unread: Default::default(),
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

#[cfg(test)]
mod unread_tests {
    use super::*;
    #[test]
    fn unread_labels_are_whole_and_members_still_follow_the_lead_once() {
        let members = [
            Member {
                id: "worker",
                state: Some("working"),
                unread: Default::default(),
            },
            Member {
                id: "lead",
                state: Some("waiting"),
                unread: super::super::unread::Count {
                    total: 2,
                    more: true,
                },
            },
            Member {
                id: "lead",
                state: Some("waiting"),
                unread: super::super::unread::Count {
                    total: 2,
                    more: true,
                },
            },
        ];
        let look = Look::default();
        let full = line(&members, Some("lead"), 15, look);
        assert_eq!(full.to_string(), "● 2+ unread ●");
        assert_eq!(full.spans[0].style, look.role(Role::Unread));
        for width in 0..15 {
            let fitted = line(&members, Some("lead"), width, look);
            assert!(fitted.width() <= width);
            assert!(!fitted.to_string().contains("unre") || fitted.to_string().contains("unread"));
        }
    }
}
