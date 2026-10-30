//! Another program's terminal screen: bytes and a size in, a frame of cells out. The program's
//! escape sequences are parsed by `vt100` (the workspace pin, the parser that produced a built-in
//! host's snapshot), so a snapshot and the output after it read the same way. Nothing here does
//! I/O or knows the stream that carries the bytes.
//!
//! The model keeps no scrollback: the screen is what the program has drawn. A cell's colours and
//! attributes are `tmt_cli_style::foreign`'s, the program's own, painted at the terminal's depth by
//! `foreign::style`; they are not TMT's design tokens.

use tmt_cli_style::foreign::{Attr, Shade};
use vt100::Color as Paint;

mod keys;

pub use keys::{is_leave, key_bytes, paste_bytes, wheel_bytes};

/// A colour as the program set it, or none for the terminal's own default.
fn shade(color: Paint) -> Option<Shade> {
    match color {
        Paint::Default => None,
        Paint::Idx(index) => Some(Shade::Indexed(index)),
        Paint::Rgb(r, g, b) => Some(Shade::Rgb(r, g, b)),
    }
}

/// What a cell holds, so a painter can place a wide character across two columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Occupant {
    #[default]
    Narrow,
    /// The first of two columns.
    Wide,
    /// The second column of a wide character; it draws nothing of its own.
    Continuation,
}

/// One column of one row. A blank cell has empty text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub attr: Attr,
    pub occupant: Occupant,
}

/// Which mouse events the program asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum MouseMode {
    #[default]
    Off,
    /// Presses only (the X10 mode): no wheel.
    Press,
    /// Presses and releases, which is what reports the wheel.
    PressRelease,
    ButtonMotion,
    AnyMotion,
}

/// How a mouse report is spelled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseEncoding {
    #[default]
    Default,
    Utf8,
    Sgr,
}

/// The input modes the program has set, which decide what typed keys and the wheel send.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub alternate_screen: bool,
    pub mouse: MouseMode,
    pub mouse_encoding: MouseEncoding,
}

/// What a painter needs of the screen at one moment. Rows are `cols` cells each, top first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
    /// Where the cursor is, `(column, row)`, when the program shows it.
    pub cursor: Option<(u16, u16)>,
    /// Where the cursor is, shown or hidden. A program that hides its cursor may still park it
    /// where it takes text, which is where an input method draws what is not yet committed.
    pub caret: Option<(u16, u16)>,
    pub modes: Modes,
}

impl Frame {
    pub fn cell(&self, column: u16, row: u16) -> Option<&Cell> {
        if column >= self.cols || row >= self.rows {
            return None;
        }
        self.cells
            .get(usize::from(row) * usize::from(self.cols) + usize::from(column))
    }

    /// The text of one row with trailing blanks cut, for tests and plain-text fallbacks.
    pub fn row_text(&self, row: u16) -> String {
        let mut text = String::new();
        for column in 0..self.cols {
            match self.cell(column, row) {
                Some(cell) if cell.occupant == Occupant::Continuation => {}
                Some(cell) if cell.text.is_empty() => text.push(' '),
                Some(cell) => text.push_str(&cell.text),
                None => break,
            }
        }
        text.trim_end().to_owned()
    }
}

/// The screen of one terminal.
pub struct Screen {
    parser: vt100::Parser,
}

impl Screen {
    /// An empty screen of `cols` by `rows`; each is at least one.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows.max(1), cols.max(1), 0),
        }
    }

    /// The screen a snapshot describes: `bytes` are the escape sequences that turn an empty
    /// terminal of this size into it.
    pub fn from_snapshot(cols: u16, rows: u16, bytes: &[u8]) -> Self {
        let mut screen = Self::new(cols, rows);
        screen.feed(bytes);
        screen
    }

    /// Output the program produced, applied on top of what is shown.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// The size in force changed: later output is laid out at this size.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
    }

    /// `(columns, rows)`.
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (cols, rows)
    }

    pub fn modes(&self) -> Modes {
        let screen = self.parser.screen();
        Modes {
            application_cursor: screen.application_cursor(),
            application_keypad: screen.application_keypad(),
            bracketed_paste: screen.bracketed_paste(),
            alternate_screen: screen.alternate_screen(),
            mouse: match screen.mouse_protocol_mode() {
                vt100::MouseProtocolMode::None => MouseMode::Off,
                vt100::MouseProtocolMode::Press => MouseMode::Press,
                vt100::MouseProtocolMode::PressRelease => MouseMode::PressRelease,
                vt100::MouseProtocolMode::ButtonMotion => MouseMode::ButtonMotion,
                vt100::MouseProtocolMode::AnyMotion => MouseMode::AnyMotion,
            },
            mouse_encoding: match screen.mouse_protocol_encoding() {
                vt100::MouseProtocolEncoding::Default => MouseEncoding::Default,
                vt100::MouseProtocolEncoding::Utf8 => MouseEncoding::Utf8,
                vt100::MouseProtocolEncoding::Sgr => MouseEncoding::Sgr,
            },
        }
    }

    /// A copy of the screen to paint, so painting never touches the parser.
    pub fn frame(&self) -> Frame {
        let screen = self.parser.screen();
        let (cols, rows) = self.size();
        let mut cells = Vec::with_capacity(usize::from(cols) * usize::from(rows));
        for row in 0..rows {
            for column in 0..cols {
                cells.push(
                    screen
                        .cell(row, column)
                        .map_or_else(Cell::default, |cell| Cell {
                            text: cell.contents().to_owned(),
                            attr: Attr {
                                fg: shade(cell.fgcolor()),
                                bg: shade(cell.bgcolor()),
                                bold: cell.bold(),
                                dim: cell.dim(),
                                italic: cell.italic(),
                                underline: cell.underline(),
                                reverse: cell.inverse(),
                            },
                            occupant: if cell.is_wide() {
                                Occupant::Wide
                            } else if cell.is_wide_continuation() {
                                Occupant::Continuation
                            } else {
                                Occupant::Narrow
                            },
                        }),
                );
            }
        }
        let (cursor_row, cursor_column) = screen.cursor_position();
        let caret =
            (cursor_row < rows && cursor_column < cols).then_some((cursor_column, cursor_row));
        Frame {
            cols,
            rows,
            cells,
            cursor: caret.filter(|_| !screen.hide_cursor()),
            caret,
            modes: self.modes(),
        }
    }
}

#[cfg(test)]
mod tests;
