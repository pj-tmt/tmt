//! Each/Range controller over shared ChipGroup; the chart owns effects.
use super::super::model::Selection;
use super::{Focus, Hit, Target};
use crate::{
    board::app::{App, Overlay},
    board::chip_group::{self as chips, ChipGroup, Choice},
};
use ratatui::{buffer::Buffer, layout::Rect};
fn target(id: &str) -> Option<Target> {
    let (prefix, value) = id.split_once('-')?;
    let value = value.parse().ok()?;
    match prefix {
        "each" => Some(Target::Each(value)),
        "range" => Some(Target::Range(value)),
        _ => None,
    }
}
fn groups(
    selection: Selection,
    focus: Option<Focus>,
    draft: usize,
    inline: bool,
    width: u16,
) -> Vec<ChipGroup> {
    let mut groups: Vec<_> = [
        (
            "Each  ",
            "each",
            Selection::each_choices(selection.range_minutes),
            selection.each_minutes,
        ),
        (
            "Range ",
            "range",
            Selection::RANGE.as_slice(),
            selection.range_minutes,
        ),
    ]
    .into_iter()
    .map(|(label, prefix, values, selected)| ChipGroup {
        label: if inline { String::new() } else { label.into() },
        choices: values
            .iter()
            .map(|value| {
                let selected = *value == selected;
                Choice {
                    id: format!("{prefix}-{value}"),
                    actionable: true,
                    text: format!(
                        " {} ",
                        if prefix == "range" {
                            super::super::format::range(*value)
                        } else if selection.range_minutes == 60 {
                            format!("{value:>2}m")
                        } else {
                            super::super::format::interval(*value)
                        }
                    ),
                    gap: " ".into(),
                    selected,
                    disabled: false,
                    cursor: match prefix {
                        "each" => {
                            focus == Some(Focus::Each)
                                && Selection::each_choices(selection.range_minutes)[draft] == *value
                        }
                        _ => focus == Some(Focus::Range) && Selection::RANGE[draft] == *value,
                    },
                }
            })
            .collect(),
    })
    .collect();
    if inline || joined_width(&groups) <= width {
        let mut second = groups.pop().unwrap();
        groups[0].choices.push(Choice {
            id: "separator".into(),
            actionable: false,
            text: if inline {
                " · ".into()
            } else {
                "Range ".into()
            },
            gap: String::new(),
            selected: false,
            disabled: false,
            cursor: false,
        });
        groups[0].choices.append(&mut second.choices);
    }
    groups
}
fn joined_width(groups: &[ChipGroup]) -> u16 {
    groups
        .iter()
        .map(|group| {
            group.label.len()
                + group
                    .choices
                    .iter()
                    .map(|choice| choice.text.len() + choice.gap.len())
                    .sum::<usize>()
        })
        .sum::<usize>() as u16
}
pub(super) fn height(width: u16, selection: Selection) -> u16 {
    groups(selection, None, 0, false, width).len() as u16
}
pub(super) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    selection: Selection,
    focus: Option<Focus>,
    draft: usize,
    app: &App,
) -> Vec<Hit> {
    chips::paint(
        buffer,
        area,
        &groups(selection, focus, draft, false, area.width),
        app,
        app.usage_controls.then_some(Overlay::UsageControls),
    )
    .into_iter()
    .filter_map(|hit| {
        Some(Hit {
            area: hit.area,
            target: target(&hit.id)?,
        })
    })
    .collect()
}
/// Both shipped selector groups share the heading. The fixed right-hand track
/// is independent of title length and the provider spark width.
pub(super) fn header(
    buffer: &mut Buffer,
    area: Rect,
    selection: Selection,
    title: &str,
    app: &App,
    cursor: (Option<Focus>, usize),
    interactive: bool,
) -> Vec<Hit> {
    use ratatui::text::{Line, Span};
    use tmt_cli_style::Role;
    use tmt_tui::components::strip;
    let look = app.look();
    // Match the approved roomy chip spacing when the header can afford it.
    if area.width < 80 {
        strip::paint_left(
            buffer,
            Rect { height: 1, ..area },
            Line::styled(
                crate::board::view::fit(title, usize::from(area.width)),
                look.role(Role::Muted),
            ),
        );
        return if area.is_empty() {
            Vec::new()
        } else {
            vec![Hit {
                area: Rect { height: 1, ..area },
                target: Target::Controls,
            }]
        };
    }
    let options = groups(selection, cursor.0, cursor.1, true, area.width);
    let width = joined_width(&options);
    let controls = Rect::new(area.right() - width, area.y, width, 1);
    let title_area = Rect::new(area.x, area.y, area.width.saturating_sub(width + 1), 1);
    let title = crate::board::view::fit(title, usize::from(title_area.width));
    let spans = if let Some(rest) = title.strip_prefix("Usage") {
        vec![
            Span::styled(
                "Usage",
                look.role(Role::Text)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(rest.to_owned(), look.role(Role::Muted)),
        ]
    } else {
        vec![Span::styled(title, look.role(Role::Muted))]
    };
    strip::paint_left(buffer, title_area, Line::from(spans));
    let mut hits: Vec<_> = chips::paint(
        buffer,
        controls,
        &options,
        app,
        app.usage_controls.then_some(Overlay::UsageControls),
    )
    .into_iter()
    .filter_map(|hit| {
        Some(Hit {
            area: hit.area,
            target: target(&hit.id)?,
        })
    })
    .collect();
    if !interactive {
        for hit in &mut hits {
            hit.target = Target::Controls;
        }
    }
    hits
}
