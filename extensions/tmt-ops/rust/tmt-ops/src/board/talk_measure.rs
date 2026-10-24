//! The right pane's Talk/Measure control, painted at the start of its top rule, after the divider.
use crate::board::{
    app::{App, Choice},
    chip_group, keycap,
    shell::RightTab,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

/// The dash and space before the control.
const LEAD: u16 = 2;

const OPTIONS: [(&str, &str, RightTab); 2] = [
    ("talk", "Talk", RightTab::Talk),
    ("measure", "Measure", RightTab::Measure),
];

/// Paints the control on `rule` and returns what is left of the rule for its own title.
/// Narrow rules keep the whole title and show no control.
pub(in crate::board) fn paint(buffer: &mut Buffer, app: &App, rule: Rect) -> Rect {
    let look = app.look();
    let key = keycap::key(app, crate::action::OpsUiAction::TalkMeasure, "M");
    let cap = key
        .as_deref()
        .map(|key| keycap::control_span(app, key, crate::action::OpsUiAction::TalkMeasure));
    let enabled = app.workspace_strip_enabled();
    let cap_width = cap.as_ref().map_or(0, Span::width);
    let chips = OPTIONS
        .iter()
        .map(|(_, label, _)| label.width() + 3)
        .sum::<usize>();
    let width = (cap_width + chips) as u16;
    // The control leads the rule, right after the divider: `┬─ M  Talk  Measure ── worker ──`.
    // Keep room for a short title after it.
    if rule.height == 0 || rule.width < width + LEAD + 14 {
        return rule;
    }
    strip::paint_left(
        buffer,
        Rect::new(rule.x, rule.y, LEAD, 1),
        Line::styled("─ ", look.role(Role::Dim)),
    );
    let area = Rect::new(rule.x + LEAD, rule.y, width, 1);
    if let Some(cap) = cap {
        strip::paint_left(buffer, area, Line::from(vec![cap]));
    }
    let groups = [chip_group::ChipGroup {
        label: String::new(),
        choices: OPTIONS
            .iter()
            .map(|(id, label, tab)| chip_group::Choice {
                id: (*id).into(),
                actionable: true,
                text: format!(" {label} "),
                gap: " ".into(),
                selected: *tab == app.right_tab,
                disabled: !enabled,
                cursor: false,
            })
            .collect(),
    }];
    let rect = Rect::new(area.x + cap_width as u16, area.y, chips as u16, 1);
    for hit in chip_group::paint(buffer, rect, &groups, app, None) {
        if let Some((_, _, tab)) = OPTIONS.iter().find(|(id, _, _)| *id == hit.id) {
            app.workspace_strip_hits
                .borrow_mut()
                .push((hit.area, Choice::RightTab(*tab)));
        }
    }
    let used = LEAD + width;
    Rect::new(rule.x + used, rule.y, rule.width - used, 1)
}
