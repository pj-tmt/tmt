//! The bytes a key press, a paste or a wheel turn sends to the program whose screen is shown,
//! spelled the way xterm does under the input modes the program has set.

use super::{Modes, MouseEncoding, MouseMode};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Presses sent for one wheel notch when the program has no mouse mode and is on the alternate
/// screen (the alternate-scroll convention).
const WHEEL_ARROWS: usize = 3;
/// The largest coordinate the default (one byte per value) mouse report can name.
const DEFAULT_MOUSE_LIMIT: u16 = 223;

/// Whether this press leaves typing: Ctrl-\. A terminal without keyboard enhancement delivers
/// the byte `0x1c` as Ctrl-4, so both spellings count. It never reaches the program.
pub fn is_leave(key: &KeyEvent) -> bool {
    key.kind != KeyEventKind::Release
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char('\\' | '4'))
}

/// The bytes for one key press, or `None` for a release or a key the terminal has no bytes for.
/// Shift on a character is already in the character, so only Ctrl and Alt change it.
pub fn key_bytes(key: &KeyEvent, modes: Modes) -> Option<Vec<u8>> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // xterm's modifier parameter: 1 plus shift 1, alt 2, ctrl 4.
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let plain = modifier == 1;
    let cursor = |letter: char| -> Vec<u8> {
        if !plain {
            format!("\x1b[1;{modifier}{letter}").into_bytes()
        } else if modes.application_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |number: u8| -> Vec<u8> {
        if plain {
            format!("\x1b[{number}~").into_bytes()
        } else {
            format!("\x1b[{number};{modifier}~").into_bytes()
        }
    };
    let escaped = |mut bytes: Vec<u8>| -> Vec<u8> {
        if alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                control(c).map(|byte| escaped(vec![byte]))
            } else {
                let mut buffer = [0; 4];
                Some(escaped(c.encode_utf8(&mut buffer).as_bytes().to_vec()))
            }
        }
        KeyCode::Enter => Some(escaped(vec![b'\r'])),
        KeyCode::Tab => Some(escaped(vec![b'\t'])),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Backspace => Some(escaped(vec![if ctrl { 0x08 } else { 0x7f }])),
        KeyCode::Esc => Some(escaped(vec![0x1b])),
        KeyCode::Up => Some(cursor('A')),
        KeyCode::Down => Some(cursor('B')),
        KeyCode::Right => Some(cursor('C')),
        KeyCode::Left => Some(cursor('D')),
        KeyCode::Home => Some(cursor('H')),
        KeyCode::End => Some(cursor('F')),
        KeyCode::Insert => Some(tilde(2)),
        KeyCode::Delete => Some(tilde(3)),
        KeyCode::PageUp => Some(tilde(5)),
        KeyCode::PageDown => Some(tilde(6)),
        KeyCode::F(n @ 1..=4) => {
            let letter = char::from(b'P' + n - 1);
            Some(if plain {
                format!("\x1bO{letter}").into_bytes()
            } else {
                format!("\x1b[1;{modifier}{letter}").into_bytes()
            })
        }
        KeyCode::F(n @ 5..=12) => Some(tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n) - 5])),
        _ => None,
    }
}

/// The control byte for Ctrl plus this character.
fn control(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        ' ' | '@' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '/' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Pasted text as the program should receive it. Under bracketed paste it is wrapped, and the
/// closing marker is cut from the text so a paste cannot end itself early. Otherwise line
/// breaks become the carriage returns Enter sends.
pub fn paste_bytes(text: &str, modes: Modes) -> Vec<u8> {
    if modes.bracketed_paste {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(text.replace("\x1b[201~", "").as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// What one wheel notch sends, with the pointer at `column`, `row` of the screen (from zero).
/// A program with a mouse mode that reports the wheel gets a mouse report. Otherwise, on the
/// alternate screen only, arrow keys; on the normal screen nothing, as there is no scrollback
/// here to move.
pub fn wheel_bytes(up: bool, column: u16, row: u16, modes: Modes) -> Option<Vec<u8>> {
    if modes.mouse >= MouseMode::PressRelease {
        let button = if up { 64 } else { 65 };
        let (x, y) = (column.saturating_add(1), row.saturating_add(1));
        return match modes.mouse_encoding {
            MouseEncoding::Sgr => Some(format!("\x1b[<{button};{x};{y}M").into_bytes()),
            MouseEncoding::Utf8 => {
                let mut out = b"\x1b[M".to_vec();
                for value in [32 + button, 32 + x, 32 + y] {
                    let mut buffer = [0; 4];
                    out.extend_from_slice(
                        char::from_u32(u32::from(value))?
                            .encode_utf8(&mut buffer)
                            .as_bytes(),
                    );
                }
                Some(out)
            }
            MouseEncoding::Default => {
                if x > DEFAULT_MOUSE_LIMIT || y > DEFAULT_MOUSE_LIMIT {
                    return None;
                }
                Some(vec![
                    0x1b,
                    b'[',
                    b'M',
                    32 + button as u8,
                    32 + x as u8,
                    32 + y as u8,
                ])
            }
        };
    }
    if modes.mouse == MouseMode::Off && modes.alternate_screen {
        let arrow = KeyEvent::new(
            if up { KeyCode::Up } else { KeyCode::Down },
            KeyModifiers::NONE,
        );
        return key_bytes(&arrow, modes).map(|bytes| bytes.repeat(WHEEL_ARROWS));
    }
    None
}
