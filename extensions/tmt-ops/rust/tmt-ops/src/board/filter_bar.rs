//! Shared list mode chips, idle search field.
//! Hosts supply options and own selection, filtering and command admission.
use crate::{board::chip_group, look::Look};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    options: &[(&str, &str)],
    selected: usize,
    query: &str,
    look: Look,
) -> Vec<chip_group::Hit> {
    let groups = [chip_group::ChipGroup {
        label: " ".into(),
        choices: options
            .iter()
            .enumerate()
            .map(|(index, (id, label))| chip_group::Choice {
                id: (*id).into(),
                actionable: true,
                text: format!(" {label} "),
                gap: " ".into(),
                selected: index == selected,
                disabled: false,
                cursor: false,
            })
            .collect(),
    }];
    let chips = 1 + options
        .iter()
        .map(|(_, label)| unicode_width::UnicodeWidthStr::width(*label) + 3)
        .sum::<usize>();
    let hits = chip_group::paint(
        buffer,
        Rect {
            width: area.width.min(chips as u16),
            ..area
        },
        &groups,
        look,
    );
    let x = area.x.saturating_add(chips as u16 + 1);
    let available = area.width.saturating_sub(chips as u16 + 1);
    let search_width = available.min(34);
    if search_width > 0 {
        let search = Rect::new(x, area.y, search_width, area.height.min(1));
        strip::paint_left(
            buffer,
            search,
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    "/ ",
                    look.role(Role::Text)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                Span::styled(
                    crate::board::view::fit(
                        &if query.is_empty() {
                            "Search".into()
                        } else {
                            crate::board::notes::sanitize(query)
                        },
                        usize::from(search_width.saturating_sub(3)),
                    ),
                    look.role(Role::Muted),
                ),
            ])
            .style(tmt_cli_style::theme::screen::chip(
                &look.theme,
                tmt_cli_style::theme::screen::ChipKind::Secondary,
                tmt_cli_style::theme::screen::ChipState::Idle,
                look.depth,
            )),
        );
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_hosts_share_fitted_chips_and_idle_search_without_hidden_hits() {
        for options in [
            [("squads", "Squads"), ("agents", "Agents")],
            [("all", "All"), ("pinned", "Pinned")],
        ] {
            for width in [1, 8, 20, 60, 103, 148] {
                let area = Rect::new(2, 3, width, 1);
                let mut buffer = Buffer::empty(Rect::new(0, 0, width + 4, 5));
                let hits = paint(
                    &mut buffer,
                    area,
                    &options,
                    1,
                    "",
                    Look::new(tmt_cli_style::Theme::default()),
                );
                for hit in hits {
                    assert_eq!(hit.area.intersection(area), hit.area);
                    assert!(options.iter().any(|(id, _)| *id == hit.id));
                }
                if width >= 60 {
                    let text: String = (area.x..area.right())
                        .map(|x| buffer[(x, area.y)].symbol())
                        .collect();
                    assert!(text.contains("/ Search"));
                    assert!(text.starts_with(' '));
                }
                assert_eq!(
                    buffer[(0, 0)].symbol(),
                    " ",
                    "no paint outside caller geometry"
                );
            }
        }
    }
}
