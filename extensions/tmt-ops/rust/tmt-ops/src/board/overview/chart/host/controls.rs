//! Two compact dropdown triggers over the existing chip and menu surfaces.
use super::super::model::{Each, Selection};
use super::{Focus, Hit, Target};
use crate::board::{
    app::{App, Overlay},
    chip_group::{self as chips, ChipGroup, Choice},
};
use ratatui::{buffer::Buffer, layout::Rect};

fn target(id: &str) -> Option<Target> {
    match id {
        "each" => Some(Target::OpenEach),
        "range" => Some(Target::OpenRange),
        _ => {
            crate::board::limits::QuotaWindow::parse(id.strip_prefix("quota-")?).map(Target::Quota)
        }
    }
}
pub(super) fn header(
    buffer: &mut Buffer,
    area: Rect,
    selection: Selection,
    title: &str,
    app: &App,
    focus: Option<Focus>,
    each: Each,
) -> Vec<Hit> {
    use ratatui::text::{Line, Span};
    use tmt_cli_style::Role;
    use tmt_tui::components::strip;
    let interval = super::super::format::interval(selection.each_minutes);
    let each = match each {
        Each::Auto => format!("Auto ({interval})"),
        Each::Fixed(_) => interval,
    };
    let group = ChipGroup {
        label: String::new(),
        choices: [
            (
                "range",
                format!(
                    " Range {} ▾ ",
                    super::super::format::range(selection.range_minutes)
                ),
                Focus::Range,
            ),
            ("each", format!(" Each {each} ▾ "), Focus::Each),
        ]
        .into_iter()
        .map(|(id, text, field)| Choice {
            id: id.into(),
            text,
            gap: if id == "range" { "  " } else { " " }.into(),
            selected: false,
            disabled: false,
            cursor: focus == Some(field),
            actionable: true,
        })
        .collect(),
    };
    use unicode_width::UnicodeWidthStr;
    let width = group
        .choices
        .iter()
        .map(|c| c.text.width() + c.gap.width())
        .sum::<usize>() as u16;
    let look = app.look();
    if title.width() + usize::from(width) + 1 > usize::from(area.width) {
        let area = Rect {
            height: u16::from(area.height > 0),
            ..area
        };
        strip::paint_left(
            buffer,
            area,
            Line::styled(
                crate::board::view::fit(title, usize::from(area.width)),
                look.role(Role::Muted),
            ),
        );
        return if area.is_empty() {
            Vec::new()
        } else {
            vec![Hit {
                area,
                target: Target::Controls,
            }]
        };
    }
    let controls = Rect::new(
        area.right().saturating_sub(width.min(area.width)),
        area.y,
        width.min(area.width),
        u16::from(area.height > 0),
    );
    let title_area = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(width + 1),
        u16::from(area.height > 0),
    );
    strip::paint_left(
        buffer,
        title_area,
        if let Some(rest) = title.strip_prefix("Usage") {
            Line::from(vec![
                Span::styled(
                    "Usage",
                    look.role(Role::Text)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                Span::styled(rest.to_owned(), look.role(Role::Muted)),
            ])
        } else {
            Line::styled(
                crate::board::view::fit(title, usize::from(title_area.width)),
                look.role(Role::Muted),
            )
        },
    );
    chips::paint_with_idle(
        buffer,
        controls,
        &[group],
        app,
        app.usage_controls.then_some(Overlay::UsageControls),
        crate::board::input_field::style(look, false),
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

/// Uses the same padded, clipped choice geometry as Each and Range.
pub(super) fn quota(
    buffer: &mut Buffer,
    area: Rect,
    selected: crate::board::limits::QuotaWindow,
    focus: Option<Focus>,
    draft: usize,
    app: &App,
) -> Vec<Hit> {
    use crate::board::limits::QuotaWindow;
    // Neither a clipped choice nor an invisible keyboard draft is admitted.
    if area.width < 15 || area.height == 0 {
        return Vec::new();
    }
    let group = ChipGroup {
        label: "quota ".into(),
        choices: QuotaWindow::ALL
            .iter()
            .enumerate()
            .map(|(index, window)| Choice {
                id: format!("quota-{}", window.key()),
                actionable: true,
                text: format!(" {} ", window.key()),
                gap: " ".into(),
                selected: *window == selected,
                disabled: false,
                cursor: focus == Some(Focus::Quota) && draft == index,
            })
            .collect(),
    };
    chips::paint(
        buffer,
        area,
        &[group],
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
