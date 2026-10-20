//! The right pane's Talk/Measure control, painted at the right end of its top rule.
use crate::board::{
    app::{App, Choice},
    chip_group, keycap,
    shell::RightTab,
};
use ratatui::{buffer::Buffer, layout::Rect, text::Line};
use tmt_tui::components::strip;
use unicode_width::UnicodeWidthStr;

const OPTIONS: [(&str, &str, RightTab); 2] = [
    ("talk", "Talk", RightTab::Talk),
    ("measure", "Measure", RightTab::Measure),
];

/// Paints the control on `rule` and returns what is left of the rule for its own title.
/// Narrow rules keep the whole title and show no control.
pub(in crate::board) fn paint(buffer: &mut Buffer, app: &App, rule: Rect) -> Rect {
    let look = app.look();
    let key = keycap::key(app, crate::action::OpsUiAction::TalkMeasure, "t");
    let cap = key.as_deref().map(|key| keycap::span(key, false, look));
    let cap_width = cap.as_ref().map_or(0, |cap| cap.width() + 1);
    let chips = 1 + OPTIONS
        .iter()
        .map(|(_, label, _)| label.width() + 3)
        .sum::<usize>();
    let width = (cap_width + chips) as u16;
    // Keep room for "── " and a short title before the control.
    if rule.height == 0 || rule.width < width + 14 {
        return rule;
    }
    let area = Rect::new(rule.right() - width, rule.y, width, 1);
    if let Some(cap) = cap {
        strip::paint_left(buffer, area, Line::from(vec![cap]));
    }
    let groups = [chip_group::ChipGroup {
        label: " ".into(),
        choices: OPTIONS
            .iter()
            .map(|(id, label, tab)| chip_group::Choice {
                id: (*id).into(),
                actionable: true,
                text: format!(" {label} "),
                gap: " ".into(),
                selected: *tab == app.right_tab,
                disabled: false,
                cursor: false,
            })
            .collect(),
    }];
    let rect = Rect::new(area.x + cap_width as u16, area.y, chips as u16, 1);
    for hit in chip_group::paint(buffer, rect, &groups, app, None) {
        let choice = Choice::Action(crate::action::Action::parse("talk-measure").unwrap());
        app.workspace_strip_hits
            .borrow_mut()
            .push((hit.area, choice));
    }
    Rect::new(rule.x, rule.y, rule.width - width, 1)
}
