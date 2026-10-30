//! Painting the Terminal tab: its chips, the rule above it, the program's screen and the line
//! under the screen that says what the stream is doing.
use super::tab::{Caret, Condition, View};
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

/// The chips: the two views, and Zoom while the terminal is the one shown.
fn options(showing: bool, zoomed: bool) -> Vec<(&'static str, &'static str, Choice, bool)> {
    let mut options = vec![
        ("chat", "Chat", Choice::TerminalView(false), !showing),
        ("terminal", "Terminal", Choice::TerminalView(true), showing),
    ];
    if showing {
        options.push(("zoom", "⤢ Zoom", Choice::TerminalZoom, zoomed));
    }
    options
}

/// Paints `Chat   t  Terminal  ⤢ Zoom` on `row`: the key that switches the views is a keycap right
/// before the label it opens, as on the tab labels, and that chip's hit covers both.
pub(in crate::board) fn chips(buffer: &mut Buffer, app: &App, row: Rect) {
    let action = crate::action::OpsUiAction::Terminal;
    let cap = keycap::key(app, action, "t").map(|key| keycap::control_span(app, &key, action));
    let cap_width = cap.as_ref().map_or(0, Span::width) as u16;
    let options = options(app.terminal_showing(), app.terminal_zoomed());
    let width = cap_width
        + options
            .iter()
            .map(|(_, label, _, _)| label.width() as u16 + 3)
            .sum::<u16>();
    if row.height == 0 || row.width < width + 2 {
        return;
    }
    let mut x = row.x + 1;
    for option in &options {
        let from = x;
        if option.0 == "terminal"
            && let Some(cap) = cap.clone()
        {
            strip::paint_left(
                buffer,
                Rect::new(x, row.y, cap_width, 1),
                Line::from(vec![cap]),
            );
            x += cap_width;
        }
        let chip = option.1.width() as u16 + 3;
        paint_chips(
            buffer,
            app,
            Rect::new(x, row.y, chip, 1),
            std::slice::from_ref(option),
            from,
        );
        x += chip;
    }
}

/// Paints `options` in `area`; each hit reaches left to `from`, so a keycap before a chip is part
/// of it.
fn paint_chips(
    buffer: &mut Buffer,
    app: &App,
    area: Rect,
    options: &[(&'static str, &'static str, Choice, bool)],
    from: u16,
) {
    let groups = [chip_group::ChipGroup {
        label: String::new(),
        choices: options
            .iter()
            .map(|(id, label, _, selected)| chip_group::Choice {
                id: (*id).into(),
                actionable: true,
                text: format!(" {label} "),
                gap: " ".into(),
                selected: *selected,
                disabled: false,
                cursor: false,
            })
            .collect(),
    }];
    for hit in chip_group::paint(buffer, area, &groups, app, None) {
        if let Some((_, _, choice, _)) = options.iter().find(|(id, _, _, _)| *id == hit.id) {
            let left = from.min(hit.area.x);
            let area = Rect::new(left, hit.area.y, hit.area.right() - left, hit.area.height);
            app.workspace_strip_hits
                .borrow_mut()
                .push((area, choice.clone()));
        }
    }
}

/// The terminal filling the board: a row with the Zoom chip, then the screen and its line.
pub(in crate::board) fn zoomed(frame: &mut ratatui::Frame, app: &App) {
    let area = frame.area();
    if area.height < 2 {
        return;
    }
    let width = "⤢ Zoom".width() as u16 + 3;
    if area.width > width {
        paint_chips(
            frame.buffer_mut(),
            app,
            Rect::new(area.x + 1, area.y, width, 1),
            &[("zoom", "⤢ Zoom", Choice::TerminalZoom, true)],
            area.x + 1,
        );
    }
    render(
        frame.buffer_mut(),
        app,
        Rect::new(area.x, area.y + 1, area.width, area.height - 1),
        app.terminal_view(),
        &app.terminal_member_name(),
    );
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
    let typing = app.terminal.typing();
    // The last row is the stream's own line; the screen has the rest.
    let (screen_area, note_area) = if area.height > 1 {
        (
            Rect::new(area.x, area.y, area.width, area.height - 1),
            Some(Rect::new(area.x, area.bottom() - 1, area.width, 1)),
        )
    } else {
        (area, None)
    };
    app.terminal.drew(screen_area);
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
            let caret = (live && typing)
                .then(|| caret(screen_area, screen))
                .flatten();
            app.terminal.place_caret(caret);
            if live && caret.is_none() {
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
        let text = note(
            &view.condition,
            view.screen.is_some(),
            typing,
            app.terminal_zoomed(),
            name,
        );
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
fn note(condition: &Condition, screen: bool, typing: bool, zoomed: bool, name: &str) -> String {
    match condition {
        Condition::Live if typing && zoomed => {
            format!(" Typing into {name}'s terminal · Ctrl-\\ back")
        }
        Condition::Live if typing => {
            format!(" Typing into {name}'s terminal · Ctrl-\\ back to the list")
        }
        Condition::Live if zoomed => {
            format!(" Live view of {name}'s terminal · ⏎ type into it · Esc back")
        }
        Condition::Live => format!(" Live view of {name}'s terminal · ⏎ type into it"),
        Condition::Connecting { attempt } if *attempt > 1 || screen => {
            format!(" Reconnecting to {name}'s terminal… (try {attempt})")
        }
        Condition::Connecting { .. } => String::new(),
        Condition::Exited(Exit::Code(code)) => {
            format!(" The program ended (exit {code}). ⏎ attaches again.")
        }
        Condition::Exited(Exit::Signal(signal)) => {
            format!(" The program ended (signal {signal}). ⏎ attaches again.")
        }
        Condition::Evicted(Eviction::TakenOver) => {
            " Another viewer took over this terminal. ⏎ attaches again.".into()
        }
        Condition::Evicted(Eviction::TooSlow) => {
            " This view fell behind and was dropped. ⏎ attaches again.".into()
        }
        Condition::Refused(refusal) => format!(
            " The terminal would not attach this view ({}). ⏎ tries again.",
            match refusal {
                Refusal::Version => "different protocol version",
                Refusal::Handshake => "bad handshake",
                Refusal::Owner => "not the owner",
                Refusal::Full => "too many viewers",
            }
        ),
        Condition::Lost(reason) => {
            format!(" The terminal could not be reached: {reason}. ⏎ tries again.")
        }
    }
}

/// While typing, the real cursor goes where the program's is, shown or hidden as the program has
/// it, when that cell is inside `area`; it then stands in for the reversed cell.
pub(super) fn caret(area: Rect, screen: &Screenful) -> Option<Caret> {
    let (column, row) = screen.caret?;
    (column < area.width && row < area.height).then(|| Caret {
        at: ratatui::layout::Position::new(area.x + column, area.y + row),
        shown: screen.cursor.is_some(),
    })
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
