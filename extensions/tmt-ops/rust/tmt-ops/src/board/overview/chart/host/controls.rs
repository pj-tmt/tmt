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
            Selection::EACH.as_slice(),
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
                    text: format!(" {value:>2}m "),
                    gap: " ".into(),
                    selected,
                    disabled: if prefix == "each" {
                        *value > selection.range_minutes
                    } else {
                        *value < selection.each_minutes
                    },
                    cursor: match prefix {
                        "each" => focus == Some(Focus::Each) && Selection::EACH[draft] == *value,
                        _ => focus == Some(Focus::Range) && Selection::RANGE[draft] == *value,
                    },
                }
            })
            .collect(),
    })
    .collect();
    if inline || height(width) == 1 {
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
pub(super) fn height(width: u16) -> u16 {
    if width >= 54 { 1 } else { 2 }
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
    let width = 45;
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
        &groups(selection, cursor.0, cursor.1, true, width),
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
