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

/// One chip: its id, its word, what a click chooses, whether it is the selected one, and the
/// key that does the same, shown as a keycap before the word as on every tab-header control.
type Chip = (
    &'static str,
    &'static str,
    Choice,
    bool,
    Option<Span<'static>>,
);

/// The chips: the two views, and Zoom while the terminal is the one shown. `j` sits on the view
/// it switches to: Terminal from the chat, Chat from the terminal (a bound `terminal` action
/// keeps its own key on Chat). `z` zooms.
fn options(app: &App) -> Vec<Chip> {
    use crate::action::{OpsUiAction, Verb};
    let (showing, zoomed) = (app.terminal_showing(), app.terminal_zoomed());
    let ui = |action: OpsUiAction, preferred: &str| {
        keycap::key(app, action, preferred).map(|key| keycap::control_span(app, &key, action))
    };
    let jump =
        super::super::view::footer::effective_key(app, &app.workspace_bindings(), Verb::Jump, "j")
            .map(|key| {
                keycap::span(
                    super::super::view::footer::key_label(&key),
                    false,
                    app.look(),
                )
            });
    let mut options = vec![
        (
            "chat",
            "Chat",
            Choice::TerminalView(false),
            !showing,
            ui(OpsUiAction::Terminal, "t").or_else(|| jump.clone().filter(|_| showing)),
        ),
        (
            "terminal",
            "Terminal",
            Choice::TerminalView(true),
            showing,
            jump.filter(|_| !showing),
        ),
    ];
    if showing {
        options.push((
            "zoom",
            "Zoom",
            Choice::TerminalZoom,
            zoomed,
            ui(OpsUiAction::TerminalZoom, "z"),
        ));
    }
    options
}

/// Paints `Chat  j Terminal  z Zoom` on `row`: each key is a keycap right before the word it
/// acts on, and that chip's hit covers both.
pub(in crate::board) fn chips(buffer: &mut Buffer, app: &App, row: Rect) {
    let options = options(app);
    let width = options.iter().map(chip_width).sum::<u16>();
    if row.height == 0 || row.width < width + 2 {
        return;
    }
    let mut x = row.x + 1;
    for option in &options {
        paint_chip(
            buffer,
            app,
            Rect::new(x, row.y, chip_width(option), 1),
            option,
        );
        x += chip_width(option);
    }
}

fn chip_width(option: &Chip) -> u16 {
    option.4.as_ref().map_or(0, Span::width) as u16 + option.1.width() as u16 + 3
}

/// Paints one chip in `area`, its keycap first; the hit covers the keycap too.
fn paint_chip(buffer: &mut Buffer, app: &App, area: Rect, option: &Chip) {
    let cap = option.4.as_ref().map_or(0, Span::width) as u16;
    if let Some(span) = option.4.clone() {
        strip::paint_left(
            buffer,
            Rect::new(area.x, area.y, cap, 1),
            Line::from(vec![span]),
        );
    }
    let (id, label, choice, selected, _) = option;
    paint_chips(
        buffer,
        app,
        Rect::new(area.x + cap, area.y, area.width - cap, 1),
        &[(*id, *label, choice.clone(), *selected)],
        area.x,
    );
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
    if let Some(zoom) = options(app).into_iter().find(|option| option.0 == "zoom") {
        let width = chip_width(&zoom);
        if area.width > width {
            paint_chip(
                frame.buffer_mut(),
                app,
                Rect::new(area.x + 1, area.y, width, 1),
                &zoom,
            );
        }
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
            selection(buffer, screen_area, screen, app);
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
            app.terminal.modes().mouse != tmt_tui::components::terminal::MouseMode::Off,
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
/// `mouse`: the program being typed into asked for the mouse, so a drag is its own.
fn note(
    condition: &Condition,
    screen: bool,
    typing: bool,
    mouse: bool,
    zoomed: bool,
    name: &str,
) -> String {
    match condition {
        Condition::Live if typing && mouse && zoomed => {
            format!(" Typing into {name}'s terminal · bypass-drag selects · Ctrl-\\ back")
        }
        Condition::Live if typing && mouse => {
            format!(
                " Typing into {name}'s terminal · bypass-drag selects · Ctrl-\\ back to the list"
            )
        }
        Condition::Live if typing && zoomed => {
            format!(" Typing into {name}'s terminal · Ctrl-\\ back")
        }
        Condition::Live if typing => {
            format!(" Typing into {name}'s terminal · Ctrl-\\ back to the list")
        }
        Condition::Live if zoomed => {
            format!(" Live view of {name}'s terminal · ⏎ type into it · Esc back")
        }
        Condition::Live => {
            format!(" Live view of {name}'s terminal · ⏎ type into it")
        }
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

/// A drag's selection shows as reversed cells, like the terminal's own selection.
fn selection(buffer: &mut Buffer, area: Rect, screen: &Screenful, app: &App) {
    for row in 0..screen.rows.min(area.height) {
        for column in 0..screen.cols.min(area.width) {
            if app.terminal.cell_selected(column, row) {
                buffer[(area.x + column, area.y + row)]
                    .modifier
                    .toggle(Modifier::REVERSED);
            }
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
