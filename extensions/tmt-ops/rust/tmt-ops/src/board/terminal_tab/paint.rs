//! Painting the Terminal tab: its chips, the rule above it, the program's screen and the line
//! under the screen that says what the stream is doing.
use super::tab::{Condition, View};
use crate::{
    board::{
        app::{App, Choice},
        chip_group, keycap,
    },
    look::Look,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
};
use tmt_cli_style::{Role, foreign};
use tmt_pty_protocol::{Eviction, Exit, Refusal};
use tmt_tui::components::{
    strip,
    terminal::{Frame as Screenful, Occupant},
};
use unicode_width::UnicodeWidthStr;

const OPTIONS: [(&str, &str, bool); 2] = [("chat", "Chat", false), ("terminal", "Terminal", true)];

/// Paints `Chat  t  Terminal` on `row`: the key that switches the views is a keycap right before
/// the label it opens, as on the tab labels, and the chip's hit covers both.
pub(in crate::board) fn chips(buffer: &mut Buffer, app: &App, row: Rect) {
    let action = crate::action::OpsUiAction::Terminal;
    let cap = keycap::key(app, action, "t").map(|key| keycap::control_span(app, &key, action));
    let cap_width = cap.as_ref().map_or(0, Span::width) as u16;
    let width = cap_width
        + OPTIONS
            .iter()
            .map(|(_, label, _)| label.width() as u16 + 3)
            .sum::<u16>();
    if row.height == 0 || row.width < width + 2 {
        return;
    }
    let showing = app.terminal_showing();
    let mut x = row.x + 1;
    for (id, label, terminal) in OPTIONS {
        let from = x;
        if terminal && let Some(cap) = cap.clone() {
            strip::paint_left(
                buffer,
                Rect::new(x, row.y, cap_width, 1),
                Line::from(vec![cap]),
            );
            x += cap_width;
        }
        let chip = label.width() as u16 + 3;
        let groups = [chip_group::ChipGroup {
            label: String::new(),
            choices: vec![chip_group::Choice {
                id: id.into(),
                actionable: true,
                text: format!(" {label} "),
                gap: " ".into(),
                selected: terminal == showing,
                disabled: false,
                cursor: false,
            }],
        }];
        for hit in chip_group::paint(buffer, Rect::new(x, row.y, chip, 1), &groups, app, None) {
            let area = Rect::new(from, hit.area.y, hit.area.right() - from, hit.area.height);
            app.workspace_strip_hits
                .borrow_mut()
                .push((area, Choice::TerminalView(terminal)));
        }
        x += chip;
    }
}

/// The rule's title: what is shown and, once there is a screen, its size.
pub(in crate::board) fn title(view: Option<&View>, width: usize) -> String {
    let Some(screen) = view.and_then(|view| view.screen.as_deref()) else {
        return "Terminal".to_owned();
    };
    format!(
        "Terminal {}×{}{}",
        screen.cols,
        screen.rows,
        if usize::from(screen.cols) > width {
            " · clipped"
        } else {
            ""
        }
    )
}

/// Paints the terminal of `name` into `area`, in place of the chat's exchanges.
pub(in crate::board) fn render(
    buffer: &mut Buffer,
    app: &App,
    area: Rect,
    view: Option<&View>,
    name: &str,
) {
    if area.is_empty() {
        return;
    }
    let look = app.look();
    // The last row is the stream's own line; the screen has the rest.
    let (screen_area, note_area) = if area.height > 1 {
        (
            Rect::new(area.x, area.y, area.width, area.height - 1),
            Some(Rect::new(area.x, area.bottom() - 1, area.width, 1)),
        )
    } else {
        (area, None)
    };
    let Some(view) = view else {
        say(
            buffer,
            area,
            &format!("Looking for {name}'s terminal…"),
            look,
        );
        return;
    };
    match &view.screen {
        Some(screen) => {
            let live = view.condition == Condition::Live;
            cells(buffer, screen_area, screen, !live, look);
            if live {
                cursor(buffer, screen_area, screen);
            }
        }
        None if matches!(view.condition, Condition::Connecting { .. }) => {
            say(
                buffer,
                screen_area,
                &format!("Connecting to {name}'s terminal…"),
                look,
            );
        }
        None => {}
    }
    if let Some(note_area) = note_area {
        let text = note(&view.condition, view.screen.is_some(), name);
        strip::paint_left(
            buffer,
            note_area,
            Line::styled(
                crate::board::view::fit(&text, usize::from(note_area.width.saturating_sub(1))),
                look.role(Role::Muted),
            ),
        );
    }
}

fn say(buffer: &mut Buffer, area: Rect, text: &str, look: Look) {
    strip::paint_left(
        buffer,
        Rect::new(area.x, area.y, area.width, 1),
        Line::styled(
            crate::board::view::fit(text, usize::from(area.width)),
            look.role(Role::Muted),
        ),
    );
}

/// The line under the screen.
fn note(condition: &Condition, screen: bool, name: &str) -> String {
    match condition {
        Condition::Live => format!(" Live view of {name}'s terminal"),
        Condition::Connecting { attempt } if *attempt > 1 || screen => {
            format!(" Reconnecting to {name}'s terminal… (try {attempt})")
        }
        Condition::Connecting { .. } => String::new(),
        Condition::Exited(Exit::Code(code)) => {
            format!(" The program ended (exit {code}). Switch away and back to look again.")
        }
        Condition::Exited(Exit::Signal(signal)) => {
            format!(" The program ended (signal {signal}). Switch away and back to look again.")
        }
        Condition::Evicted(Eviction::TakenOver) => {
            " Another viewer took over this terminal. Switch away and back to look again.".into()
        }
        Condition::Evicted(Eviction::TooSlow) => {
            " This view fell behind and was dropped. Switch away and back to look again.".into()
        }
        Condition::Refused(refusal) => format!(
            " The terminal would not attach this view ({}).",
            match refusal {
                Refusal::Version => "different protocol version",
                Refusal::Handshake => "bad handshake",
                Refusal::Owner => "not the owner",
                Refusal::Full => "too many viewers",
            }
        ),
        Condition::Lost(reason) => format!(" The terminal could not be reached: {reason}."),
    }
}

/// Where the program's cursor is, as one reversed cell (not the terminal's own cursor), when the
/// program shows it and the cell is inside `area`. On a cell the program already reversed it
/// shows as the plain cell.
fn cursor(buffer: &mut Buffer, area: Rect, screen: &Screenful) {
    let Some((column, row)) = screen.cursor else {
        return;
    };
    if column < area.width && row < area.height {
        buffer[(area.x + column, area.y + row)]
            .modifier
            .toggle(Modifier::REVERSED);
    }
}

/// The program's cells in `area`: a bigger screen is cut at the edges, a smaller one leaves the
/// rest blank. A wide character that would straddle the edge is not drawn.
pub(in crate::board) fn cells(
    buffer: &mut Buffer,
    area: Rect,
    screen: &Screenful,
    dim: bool,
    look: Look,
) {
    for row in 0..screen.rows.min(area.height) {
        let y = area.y + row;
        let mut column = 0;
        let limit = screen.cols.min(area.width);
        while column < limit {
            let Some(cell) = screen.cell(column, row) else {
                break;
            };
            let width = match cell.occupant {
                Occupant::Wide => 2,
                Occupant::Narrow | Occupant::Continuation => 1,
            };
            if column + width > limit {
                break;
            }
            // Equal neighbours are drawn as one run.
            let mut text = String::new();
            let mut end = column;
            while end < limit {
                let Some(next) = screen.cell(end, row) else {
                    break;
                };
                if next.attr != cell.attr {
                    break;
                }
                let span = match next.occupant {
                    Occupant::Wide => 2,
                    Occupant::Narrow | Occupant::Continuation => 1,
                };
                if end + span > limit {
                    break;
                }
                if next.occupant != Occupant::Continuation {
                    text.push_str(if next.text.is_empty() {
                        " "
                    } else {
                        &next.text
                    });
                }
                end += span;
            }
            let mut style = foreign::style(cell.attr, look.depth);
            if dim {
                style = style.add_modifier(Modifier::DIM);
            }
            buffer.set_string(area.x + column, y, &text, style);
            column = end.max(column + 1);
        }
    }
}
