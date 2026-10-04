//! Named pane dispatch and existing split/tab chrome.

use super::{
    detail::render_detail, notes::render_notes, replies::render_replies, rows::render_rows,
    tabs::pane_tab,
};
use crate::board::app::App;
use crate::board::app::TitleHit;
use crate::board::notes::sanitize;
use crate::config::{BoardMode, Pane};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::Role;

/// Split mode tiles the configured panes; tabs mode shows the focused pane
/// under a tab bar. The focused pane's border is highlighted.
pub(super) fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    let look = app.look();
    let Some(view) = &app.view else {
        render_rows(frame, app, area);
        return;
    };
    if view.home.is_some() {
        crate::board::home::render(frame, app, area);
        return;
    }
    let board = app.effective_board().expect("loaded view has a board");
    let focused = app.focused_pane();
    let pane_block = |pane: Pane| {
        let title = match pane {
            // The lead's notes nobody updated for a while say how long.
            Pane::Notes => {
                let lead = view.document["squad"]["lead"]["name"]
                    .as_str()
                    .unwrap_or("no lead");
                let mut title = vec![Span::raw(format!(" notes · {lead} "))];
                if let Some(age) =
                    crate::staleness::label(&view.document["squad"]["notesStaleness"])
                {
                    title.push(Span::styled(format!("· {age} "), look.role(Role::Waiting)));
                }
                Line::from(title)
            }
            other => Line::from(format!(" {} ", other.title())),
        };
        let style = if Some(pane) == focused && board.panes.len() > 1 {
            look.role(Role::Accent).add_modifier(Modifier::BOLD)
        } else {
            look.role(Role::Dim)
        };
        Block::new()
            .borders(Borders::ALL)
            .border_style(style)
            .title_style(if Some(pane) == focused && board.panes.len() > 1 {
                look.role(Role::Accent).add_modifier(Modifier::BOLD)
            } else {
                look.role(Role::Muted)
            })
            .title(title)
    };
    let placement = crate::board::composition::layout(
        &mut view.derived.borrow_mut().composition,
        board,
        &app.collapsed_panes(),
        app.focused(),
        area,
    );
    let slots = match placement {
        Ok(slots) => slots,
        Err(error) => {
            frame.render_widget(Paragraph::new(error).style(look.role(Role::Dim)), area);
            return;
        }
    };
    match board.mode {
        BoardMode::Split if board.panes.len() == 1 && app.collapsed_panes().is_empty() => {
            render_pane(frame, app, board.panes[0], slots[0].1)
        }
        BoardMode::Split => render_split(frame, app, &slots, &pane_block),
        BoardMode::Tabs => {
            let focused = app.focused();
            let bar = slots[0].1;
            let rest = slots[1].1;
            let mut spans = Vec::new();
            for pane in &board.panes {
                spans.push(pane_tab(look, pane.title(), *pane == focused));
                spans.push(Span::raw(" "));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), bar);
            let block = pane_block(focused);
            let inner = block.inner(rest);
            frame.render_widget(block, rest);
            render_pane(frame, app, focused, inner);
        }
    }
}

/// Named pane slots retain their rich painter, borders and title hits.
fn render_split(
    frame: &mut Frame,
    app: &App,
    slots: &[(Vec<String>, Rect)],
    pane_block: &dyn Fn(Pane) -> Block<'static>,
) {
    let collapsed = app.collapsed_panes();
    for (id, area) in slots {
        let pane = Pane::parse(id.last().expect("named pane")).expect("validated pane slot");
        let area = *area;
        if area.is_empty() {
            continue;
        }
        let title = Rect { height: 1, ..area };
        app.title_hits
            .borrow_mut()
            .push(TitleHit { pane, area: title });
        if collapsed.contains(&pane) {
            let mut text = format!("▸ {}", pane.title());
            if pane == Pane::Notes
                && let Some(view) = &app.view
            {
                if let Some(lead) = view.document["squad"]["lead"]["name"].as_str() {
                    text.push_str(&format!(" · {}", sanitize(lead)));
                }
                if let Some(age) =
                    crate::staleness::label(&view.document["squad"]["notesStaleness"])
                {
                    text.push_str(&format!(" · {age}"));
                }
            }
            frame.render_widget(
                Paragraph::new(Line::styled(text, app.look().role(Role::Muted))),
                title,
            );
        } else {
            let block = pane_block(pane);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            if !inner.is_empty() {
                render_pane(frame, app, pane, inner);
            }
        }
    }
}

fn render_pane(frame: &mut Frame, app: &App, pane: Pane, area: Rect) {
    match pane {
        Pane::Rows => render_rows(frame, app, area),
        Pane::Notes => render_notes(frame, app, area),
        Pane::Detail => render_detail(frame, app, area),
        Pane::Replies => render_replies(frame, app, area),
    }
}
