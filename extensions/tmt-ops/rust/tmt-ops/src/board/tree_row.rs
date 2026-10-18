//! One dense squad/member row with neutral, full-row selection.
use crate::{
    board::workspace::{OpsTarget, OpsTreeRow, columns::AgentsField},
    look::Look,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use tmt_cli_style::{Role, grid::Align};
use tmt_tui::{components::strip, style::TextFlow};
use unicode_width::UnicodeWidthStr;

pub(in crate::board) struct Row<'a> {
    pub source: &'a OpsTreeRow,
    pub expanded: bool,
    pub last: bool,
    pub pinned: bool,
    pub state: Line<'static>,
    pub state_primary_width: Option<usize>,
    pub tokens: String,
    pub model: String,
    pub timers: Option<u32>,
    pub selected: bool,
    pub query: &'a str,
}
#[derive(Default)]
pub(in crate::board) struct Painted {
    pub state: Option<Rect>,
    pub task: Option<Rect>,
    pub timers: Option<Rect>,
}

fn style(look: Look, selected: bool, value: Style) -> Style {
    if selected {
        look.selection()
            .patch(look.row_span(true, value, false))
            .remove_modifier(Modifier::BOLD)
    } else {
        value
    }
}

pub(in crate::board) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    slots: &[(AgentsField, Rect)],
    row: &Row<'_>,
    look: Look,
) -> Painted {
    let base = style(look, row.selected, look.role(Role::Text));
    for y in area.y..area.bottom() {
        strip::paint_left(
            buffer,
            Rect::new(area.x, y, area.width, 1),
            Line::styled(" ".repeat(usize::from(area.width)), base),
        );
    }
    let mut painted = Painted::default();
    for (field, slot) in slots {
        let cell = Rect::new(slot.x, area.y, slot.width, area.height.min(1)).intersection(area);
        if cell.is_empty() {
            continue;
        }
        let fit = |value: &str, align| {
            tmt_tui::text::fit_line(value, cell.width, TextFlow::Truncate, align)
        };
        let mut line = match field {
            AgentsField::Agent => {
                let branch = if matches!(row.source.target, OpsTarget::Member { .. }) {
                    if row.last { "  └ " } else { "  ├ " }
                } else if row.expanded {
                    "▾ "
                } else {
                    "▸ "
                };
                let name_width = usize::from(cell.width)
                    .saturating_sub(1 + branch.width() + usize::from(row.source.lead));
                // The leading gutter carries the pin without moving name tracks.
                Line::from(vec![
                    Span::styled(if row.pinned { "▪" } else { " " }, look.role(Role::Muted)),
                    Span::styled(branch, look.role(Role::Text)),
                    Span::styled(
                        if row.source.lead { "*" } else { "" },
                        look.role(Role::Muted),
                    ),
                    Span::styled(
                        crate::board::view::fit(
                            &crate::board::notes::single_line(&row.source.title),
                            name_width,
                        ),
                        look.role(Role::Text),
                    ),
                ])
            }
            AgentsField::State => {
                let primary = row
                    .state_primary_width
                    .unwrap_or(row.state.width())
                    .min(usize::from(cell.width)) as u16;
                painted.state = (primary > 0).then_some(Rect::new(cell.x, cell.y, primary, 1));
                let task = row.state.width().min(usize::from(cell.width)) as u16 - primary;
                painted.task = (task > 0).then_some(Rect::new(cell.x + primary, cell.y, task, 1));
                let mut state = row.state.clone();
                if state.width() > usize::from(cell.width) {
                    let prefix = state
                        .spans
                        .iter()
                        .take(state.spans.len().saturating_sub(1))
                        .map(Span::width)
                        .sum::<usize>();
                    if let Some(last) = state.spans.last_mut() {
                        last.content = crate::board::view::fit(
                            &last.content,
                            usize::from(cell.width).saturating_sub(prefix),
                        )
                        .into();
                    }
                }
                state
            }
            AgentsField::Tokens => Line::styled(
                fit(&row.tokens, Align::Right),
                look.role(if row.tokens == "—" {
                    Role::Dim
                } else {
                    Role::Text
                }),
            ),
            AgentsField::Cron => {
                if row.timers.is_some() {
                    painted.timers = Some(cell);
                }
                Line::styled(
                    fit(
                        &row.timers.map_or_else(|| "—".into(), |n| n.to_string()),
                        Align::Center,
                    ),
                    look.role(Role::Muted),
                )
            }
            AgentsField::Model => {
                Line::styled(fit(&row.model, Align::Left), look.role(Role::Muted))
            }
        };
        line.style = style(look, row.selected, line.style);
        for span in &mut line.spans {
            span.style = style(look, row.selected, span.style);
        }
        strip::paint_left(
            buffer,
            cell,
            crate::board::search_field::highlight_line(line, row.query),
        );
    }
    painted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lead_mark_uses_the_name_budget_and_model_text_is_muted_and_ellipsized() {
        use crate::board::workspace::OpsObservation;
        let source = OpsTreeRow {
            target: OpsTarget::Member {
                room_id: None,
                identity_id: "member".into(),
            },
            title: "a-very-long-agent-name".into(),
            depth: 1,
            lead: true,
            state: None,
            presence: None,
            usage: None,
            activity: None,
            count: None,
            observation: OpsObservation::Complete,
        };
        let look = Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::Tmt),
            depth: tmt_cli_style::Depth::TrueColor,
        };
        let area = Rect::new(0, 0, 34, 1);
        let slots = [
            (AgentsField::Agent, Rect::new(0, 0, 25, 1)),
            (AgentsField::Model, Rect::new(26, 0, 8, 1)),
        ];
        for pinned in [false, true] {
            let mut buffer = Buffer::empty(area);
            paint(
                &mut buffer,
                area,
                &slots,
                &Row {
                    source: &source,
                    expanded: false,
                    last: false,
                    pinned,
                    state: Line::default(),
                    state_primary_width: None,
                    tokens: "—".into(),
                    model: "gpt-6.1-sol".into(),
                    timers: None,
                    selected: false,
                    query: "",
                },
                look,
            );
            assert_eq!(buffer[(5, 0)].symbol(), "*");
            assert_eq!(buffer[(5, 0)].fg, look.role(Role::Muted).fg.unwrap());
            assert_eq!(buffer[(24, 0)].symbol(), "…");
            assert_eq!(buffer[(25, 0)].symbol(), " ");
            assert_eq!(buffer[(33, 0)].symbol(), "…");
            assert_eq!(buffer[(26, 0)].fg, look.role(Role::Muted).fg.unwrap());
        }
    }

    #[test]
    fn pins_do_not_move_fields_and_selected_rows_fill_every_cell_without_bold_words() {
        let app = crate::board::app::workspace::tests::board();
        let rows = app.workspace_rows();
        let source = rows
            .iter()
            .find(|row| matches!(row.target, OpsTarget::Member { .. }))
            .unwrap();
        let look = Look {
            theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::Tmt),
            depth: tmt_cli_style::Depth::TrueColor,
        };
        let area = Rect::new(0, 0, 70, 2);
        let slots = [
            (AgentsField::Agent, Rect::new(0, 0, 25, 1)),
            (AgentsField::State, Rect::new(26, 0, 16, 1)),
            (AgentsField::Tokens, Rect::new(43, 0, 7, 1)),
            (AgentsField::Cron, Rect::new(53, 0, 3, 1)),
        ];
        let mut unpinned = None;
        for pinned in [false, true] {
            let mut buffer = Buffer::empty(area);
            let painted = paint(
                &mut buffer,
                area,
                &slots,
                &Row {
                    source,
                    expanded: false,
                    last: true,
                    pinned,
                    state: crate::board::state_cell::unread(
                        crate::board::unread::Count {
                            total: 2,
                            more: false,
                        },
                        look,
                    )
                    .unwrap(),
                    state_primary_width: Some(10),
                    tokens: "12.3K".into(),
                    model: "gpt-6.1-sol".into(),
                    timers: Some(0),
                    selected: true,
                    query: "",
                },
                look,
            );
            look.selected_words_except(&mut buffer, &[painted.state.unwrap()]);
            for cell in &buffer.content {
                assert_eq!(cell.bg, look.selection().bg.unwrap());
                assert!(!cell.modifier.contains(Modifier::BOLD));
            }
            assert_eq!(buffer[(26, 0)].fg, look.role(Role::Unread).fg.unwrap());
            assert_eq!(buffer[(54, 0)].symbol(), "0");
            let fields = buffer.content[26..70].to_vec();
            assert_eq!(*unpinned.get_or_insert(fields.clone()), fields);
            assert_eq!(buffer[(0, 0)].symbol(), if pinned { "▪" } else { " " });
        }
    }
}
