//! Shared list mode chips, idle search field.
//! Hosts supply options and own selection, filtering and command admission.
use crate::board::{app::App, chip_group};
use ratatui::{buffer::Buffer, layout::Rect};

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    options: &[(&str, &str)],
    selected: usize,
    query: &str,
    app: &App,
    toggle: crate::action::OpsUiAction,
) -> Vec<chip_group::Hit> {
    let look = app.look();
    let area = Rect {
        height: area.height.min(1),
        ..area
    };
    if area.is_empty() {
        return Vec::new();
    }
    let key = super::keycap::key(app, toggle, "f");
    let prefix = key
        .as_deref()
        .map(|key| super::keycap::span(key, false, look));
    let prefix_width = prefix.as_ref().map_or(0, |span| span.width() + 1);
    let prefix_width = if prefix_width <= usize::from(area.width) {
        prefix_width
    } else {
        0
    };
    if prefix_width > 0 {
        tmt_tui::components::strip::paint_left(
            buffer,
            Rect {
                width: prefix_width as u16,
                ..area
            },
            ratatui::text::Line::from(prefix.into_iter().collect::<Vec<_>>()),
        );
    }
    let area = Rect::new(
        area.x + prefix_width as u16,
        area.y,
        area.width - prefix_width as u16,
        area.height,
    );
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
    let mut hits = chip_group::paint(
        buffer,
        Rect {
            width: area.width.min(chips as u16),
            ..area
        },
        &groups,
        app,
        None,
    );
    let x = area.x.saturating_add(chips as u16 + 1);
    let available = area.width.saturating_sub(chips as u16 + 1);
    let search_width = available.min(34);
    if search_width > 0 {
        let search = Rect::new(x, area.y, search_width, area.height.min(1));
        let search_key = super::keycap::key(app, crate::action::OpsUiAction::Search, "/");
        super::search_field::paint(
            buffer,
            search,
            query,
            app.searching,
            search_key.as_deref(),
            look,
        );
        if search.width >= 3 {
            hits.push(chip_group::Hit {
                area: search,
                id: "search".into(),
            });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_uses_one_fixed_filled_slot_in_every_editing_state() {
        use ratatui::style::Color;
        use tmt_cli_style::{Base, Depth, Role, Theme};
        for options in [
            [("squads", "Squads"), ("agents", "Agents")],
            [("all", "All"), ("pinned", "Pinned")],
        ] {
            for width in [103, 148] {
                for (base, depth) in [
                    (Base::parse("tmt").unwrap(), Depth::TrueColor),
                    (Base::parse("tmt-light").unwrap(), Depth::TrueColor),
                    (Base::parse("tmt").unwrap(), Depth::None),
                ] {
                    let mut geometry = None;
                    for (query, editing) in
                        [("", false), ("agent-00001", true), ("agent-00001", false)]
                    {
                        let mut app = App::default();
                        app.initial_look = Some(crate::look::Look {
                            theme: Theme::new(base),
                            depth,
                        });
                        app.searching = editing;
                        let area = Rect::new(0, 0, width, 1);
                        let mut buffer = Buffer::empty(area);
                        buffer.set_style(area, ratatui::style::Style::default().bg(Color::Magenta));
                        let hits = paint(
                            &mut buffer,
                            area,
                            &options,
                            0,
                            query,
                            &app,
                            crate::action::OpsUiAction::OverviewView,
                        );
                        let slot = hits.iter().find(|hit| hit.id == "search").unwrap().area;
                        assert_eq!(slot.width, 34);
                        assert_eq!(*geometry.get_or_insert(slot), slot);
                        let style = app
                            .look()
                            .role(Role::Text)
                            .patch(app.look().role(Role::Track));
                        for x in slot.x..slot.right() {
                            let cell = &buffer[(x, 0)];
                            if (slot.x + 1..slot.x + 4).contains(&x) {
                                let cap = tmt_cli_style::theme::screen::keycap(
                                    &app.look().theme,
                                    depth,
                                    false,
                                );
                                assert_eq!(cell.bg, cap.bg.unwrap_or(Color::Reset));
                                assert_eq!(cell.fg, cap.fg.unwrap_or(Color::Reset));
                                assert_eq!(cell.modifier, cap.add_modifier);
                                continue;
                            }
                            assert_eq!(
                                cell.bg,
                                style.bg.unwrap_or(Color::Reset),
                                "{query:?}, editing={editing}, x={x}"
                            );
                            assert_eq!(cell.fg, style.fg.unwrap_or(Color::Reset));
                            assert_eq!(cell.modifier, style.add_modifier);
                        }
                        assert_eq!(buffer[(slot.right(), 0)].bg, Color::Magenta);
                    }
                }
            }
        }
    }

    #[test]
    fn both_hosts_share_fitted_chips_and_idle_search_without_hidden_hits() {
        for options in [
            [("squads", "Squads"), ("agents", "Agents")],
            [("all", "All"), ("pinned", "Pinned")],
        ] {
            for width in [1, 8, 20, 60, 103, 148] {
                let area = Rect::new(2, 3, width, 1);
                let mut buffer = Buffer::empty(Rect::new(0, 0, width + 4, 5));
                let mut app = App::default();
                app.initial_look = Some(crate::look::Look::new(tmt_cli_style::Theme::default()));
                let hits = paint(
                    &mut buffer,
                    area,
                    &options,
                    1,
                    "",
                    &app,
                    crate::action::OpsUiAction::OverviewView,
                );
                for hit in hits {
                    assert_eq!(hit.area.intersection(area), hit.area);
                    assert!(hit.id == "search" || options.iter().any(|(id, _)| *id == hit.id));
                }
                if width >= 60 {
                    let text: String = (area.x..area.right())
                        .map(|x| buffer[(x, area.y)].symbol())
                        .collect();
                    assert!(text.contains("/  Search"));
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
