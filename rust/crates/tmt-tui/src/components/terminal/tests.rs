use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tmt_cli_style::foreign::{Attr, Shade};

fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn bytes(code: KeyCode, modifiers: KeyModifiers, modes: Modes) -> Option<Vec<u8>> {
    key_bytes(&press(code, modifiers), modes)
}

#[test]
fn a_snapshot_then_output_reads_as_the_screen_with_its_attributes() {
    let mut screen = Screen::from_snapshot(20, 4, b"\x1b[1;31mred\x1b[0m plain\r\nsecond");
    screen.feed(b"\x1b[3;1Hthird");
    let frame = screen.frame();
    assert_eq!((frame.cols, frame.rows), (20, 4));
    assert_eq!(frame.row_text(0), "red plain");
    assert_eq!(frame.row_text(1), "second");
    assert_eq!(frame.row_text(2), "third");
    let red = frame.cell(0, 0).unwrap();
    assert!(red.attr.bold);
    assert_eq!(red.attr.fg, Some(Shade::Indexed(1)));
    assert_eq!(frame.cell(4, 0).unwrap().attr, Attr::default());
    assert_eq!(frame.cursor, Some((5, 2)), "after `third`");
}

#[test]
fn colours_come_through_as_palette_and_true_colour() {
    let screen = Screen::from_snapshot(8, 1, b"\x1b[38;5;200ma\x1b[48;2;1;2;3mb");
    let frame = screen.frame();
    assert_eq!(frame.cell(0, 0).unwrap().attr.fg, Some(Shade::Indexed(200)));
    assert_eq!(frame.cell(1, 0).unwrap().attr.bg, Some(Shade::Rgb(1, 2, 3)));
}

#[test]
fn the_cursor_is_hidden_when_the_program_hides_it() {
    let mut screen = Screen::from_snapshot(8, 2, b"x");
    assert!(screen.frame().cursor.is_some());
    screen.feed(b"\x1b[?25l");
    assert_eq!(screen.frame().cursor, None);
}

#[test]
fn the_caret_stays_where_a_hidden_cursor_is_parked() {
    let mut screen = Screen::from_snapshot(8, 2, b"x");
    assert_eq!(screen.frame().caret, Some((1, 0)));
    screen.feed(b"\x1b[?25l\x1b[2;4H");
    let frame = screen.frame();
    assert_eq!(frame.cursor, None, "hidden");
    assert_eq!(frame.caret, Some((3, 1)), "but still placed");
    screen.feed(b"\x1b[?25h");
    assert_eq!(screen.frame().cursor, Some((3, 1)));
}

#[test]
fn a_wide_character_takes_two_columns() {
    let screen = Screen::from_snapshot(8, 1, "界a".as_bytes());
    let frame = screen.frame();
    assert_eq!(frame.cell(0, 0).unwrap().occupant, Occupant::Wide);
    assert_eq!(frame.cell(1, 0).unwrap().occupant, Occupant::Continuation);
    assert_eq!(frame.cell(2, 0).unwrap().text, "a");
    assert_eq!(frame.row_text(0), "界a");
}

#[test]
fn a_resize_lays_later_output_out_at_the_new_size() {
    let mut screen = Screen::from_snapshot(10, 2, b"abc");
    screen.resize(4, 3);
    assert_eq!(screen.size(), (4, 3));
    screen.feed(b"\r\nlongest");
    let frame = screen.frame();
    assert_eq!((frame.cols, frame.rows), (4, 3));
    assert_eq!(frame.cells.len(), 12);
    assert_eq!(frame.row_text(0), "abc");
    assert_eq!(frame.row_text(1), "long", "wrapped at the new width");
    assert_eq!(frame.row_text(2), "est");
}

#[test]
fn a_screen_never_has_a_zero_dimension() {
    let screen = Screen::new(0, 0);
    assert_eq!(screen.size(), (1, 1));
    assert_eq!(screen.frame().cells.len(), 1);
}

#[test]
fn the_input_modes_the_program_set_are_published() {
    let screen = Screen::from_snapshot(
        8,
        2,
        b"\x1b[?1049h\x1b[?1h\x1b=\x1b[?2004h\x1b[?1002h\x1b[?1006h",
    );
    assert_eq!(
        screen.modes(),
        Modes {
            application_cursor: true,
            application_keypad: true,
            bracketed_paste: true,
            alternate_screen: true,
            mouse: MouseMode::ButtonMotion,
            mouse_encoding: MouseEncoding::Sgr,
        }
    );
    assert_eq!(Screen::new(8, 2).modes(), Modes::default());
}

#[test]
fn a_program_on_the_alternate_screen_shows_only_that_screen() {
    let mut screen = Screen::from_snapshot(10, 2, b"normal");
    screen.feed(b"\x1b[?1049h\x1b[Halt");
    assert_eq!(screen.frame().row_text(0), "alt");
    screen.feed(b"\x1b[?1049l");
    assert_eq!(screen.frame().row_text(0), "normal");
}

#[test]
fn plain_keys_are_their_utf8_and_alt_prefixes_escape() {
    let none = Modes::default();
    assert_eq!(
        bytes(KeyCode::Char('a'), KeyModifiers::NONE, none),
        Some(b"a".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Char('A'), KeyModifiers::SHIFT, none),
        Some(b"A".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Char('é'), KeyModifiers::NONE, none),
        Some("é".as_bytes().to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Char('x'), KeyModifiers::ALT, none),
        Some(b"\x1bx".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Enter, KeyModifiers::NONE, none),
        Some(b"\r".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Tab, KeyModifiers::NONE, none),
        Some(b"\t".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::BackTab, KeyModifiers::SHIFT, none),
        Some(b"\x1b[Z".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Backspace, KeyModifiers::NONE, none),
        Some(vec![0x7f])
    );
    assert_eq!(
        bytes(KeyCode::Esc, KeyModifiers::NONE, none),
        Some(vec![0x1b])
    );
    assert_eq!(
        bytes(KeyCode::Esc, KeyModifiers::ALT, none),
        Some(vec![0x1b, 0x1b])
    );
}

#[test]
fn control_keys_are_the_c0_bytes() {
    let none = Modes::default();
    let ctrl = KeyModifiers::CONTROL;
    assert_eq!(bytes(KeyCode::Char('c'), ctrl, none), Some(vec![0x03]));
    assert_eq!(bytes(KeyCode::Char('C'), ctrl, none), Some(vec![0x03]));
    assert_eq!(bytes(KeyCode::Char('z'), ctrl, none), Some(vec![0x1a]));
    assert_eq!(bytes(KeyCode::Char(' '), ctrl, none), Some(vec![0x00]));
    assert_eq!(bytes(KeyCode::Char('['), ctrl, none), Some(vec![0x1b]));
    assert_eq!(bytes(KeyCode::Char('\\'), ctrl, none), Some(vec![0x1c]));
    assert_eq!(bytes(KeyCode::Char('4'), ctrl, none), Some(vec![0x1c]));
    assert_eq!(bytes(KeyCode::Char('?'), ctrl, none), Some(vec![0x7f]));
    assert_eq!(
        bytes(KeyCode::Char('1'), ctrl, none),
        None,
        "no byte for Ctrl-1"
    );
    assert_eq!(
        bytes(KeyCode::Char('c'), ctrl | KeyModifiers::ALT, none),
        Some(vec![0x1b, 0x03])
    );
}

#[test]
fn cursor_keys_follow_the_application_cursor_mode_and_carry_modifiers() {
    let normal = Modes::default();
    let application = Modes {
        application_cursor: true,
        ..Modes::default()
    };
    assert_eq!(
        bytes(KeyCode::Up, KeyModifiers::NONE, normal),
        Some(b"\x1b[A".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Up, KeyModifiers::NONE, application),
        Some(b"\x1bOA".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Home, KeyModifiers::NONE, application),
        Some(b"\x1bOH".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::End, KeyModifiers::NONE, normal),
        Some(b"\x1b[F".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Right, KeyModifiers::CONTROL, application),
        Some(b"\x1b[1;5C".to_vec()),
        "a modified cursor key ignores the application mode"
    );
    assert_eq!(
        bytes(
            KeyCode::Left,
            KeyModifiers::SHIFT | KeyModifiers::ALT,
            normal
        ),
        Some(b"\x1b[1;4D".to_vec())
    );
}

#[test]
fn editing_and_function_keys_use_the_xterm_sequences() {
    let none = Modes::default();
    let plain = KeyModifiers::NONE;
    assert_eq!(
        bytes(KeyCode::Insert, plain, none),
        Some(b"\x1b[2~".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::Delete, plain, none),
        Some(b"\x1b[3~".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::PageUp, plain, none),
        Some(b"\x1b[5~".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::PageDown, KeyModifiers::CONTROL, none),
        Some(b"\x1b[6;5~".to_vec())
    );
    assert_eq!(bytes(KeyCode::F(1), plain, none), Some(b"\x1bOP".to_vec()));
    assert_eq!(bytes(KeyCode::F(4), plain, none), Some(b"\x1bOS".to_vec()));
    assert_eq!(
        bytes(KeyCode::F(2), KeyModifiers::SHIFT, none),
        Some(b"\x1b[1;2Q".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::F(5), plain, none),
        Some(b"\x1b[15~".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::F(10), plain, none),
        Some(b"\x1b[21~".to_vec())
    );
    assert_eq!(
        bytes(KeyCode::F(12), KeyModifiers::ALT, none),
        Some(b"\x1b[24;3~".to_vec())
    );
    assert_eq!(bytes(KeyCode::F(13), plain, none), None);
    assert_eq!(bytes(KeyCode::CapsLock, plain, none), None);
}

#[test]
fn a_key_release_sends_nothing_and_a_repeat_sends_again() {
    let mut key = press(KeyCode::Char('a'), KeyModifiers::NONE);
    key.kind = KeyEventKind::Release;
    assert_eq!(key_bytes(&key, Modes::default()), None);
    key.kind = KeyEventKind::Repeat;
    assert_eq!(key_bytes(&key, Modes::default()), Some(b"a".to_vec()));
}

#[test]
fn ctrl_backslash_leaves_in_either_spelling_and_nothing_else_does() {
    let ctrl = KeyModifiers::CONTROL;
    assert!(is_leave(&press(KeyCode::Char('\\'), ctrl)));
    assert!(
        is_leave(&press(KeyCode::Char('4'), ctrl)),
        "the 0x1c byte arrives as Ctrl-4"
    );
    assert!(!is_leave(&press(KeyCode::Char('\\'), KeyModifiers::NONE)));
    assert!(!is_leave(&press(KeyCode::Char('4'), KeyModifiers::NONE)));
    assert!(!is_leave(&press(
        KeyCode::Char('\\'),
        ctrl | KeyModifiers::ALT
    )));
    assert!(
        !is_leave(&press(KeyCode::Esc, KeyModifiers::NONE)),
        "Esc goes to the program"
    );
    let mut release = press(KeyCode::Char('\\'), ctrl);
    release.kind = KeyEventKind::Release;
    assert!(!is_leave(&release));
}

#[test]
fn a_paste_is_bracketed_when_the_program_asked_and_cannot_close_itself_early() {
    let bracketed = Modes {
        bracketed_paste: true,
        ..Modes::default()
    };
    assert_eq!(
        paste_bytes("a\nb", bracketed),
        b"\x1b[200~a\nb\x1b[201~".to_vec()
    );
    assert_eq!(
        paste_bytes("x\x1b[201~y", bracketed),
        b"\x1b[200~xy\x1b[201~".to_vec(),
        "the closing marker inside the text is cut"
    );
    assert_eq!(
        paste_bytes("a\r\nb\nc", Modes::default()),
        b"a\rb\rc".to_vec(),
        "unbracketed, line breaks are the carriage returns Enter sends"
    );
}

#[test]
fn the_wheel_reports_to_a_program_with_a_mouse_mode_in_its_own_encoding() {
    let sgr = Modes {
        mouse: MouseMode::AnyMotion,
        mouse_encoding: MouseEncoding::Sgr,
        ..Modes::default()
    };
    assert_eq!(
        wheel_bytes(true, 4, 9, sgr),
        Some(b"\x1b[<64;5;10M".to_vec())
    );
    assert_eq!(
        wheel_bytes(false, 0, 0, sgr),
        Some(b"\x1b[<65;1;1M".to_vec())
    );
    let default = Modes {
        mouse: MouseMode::PressRelease,
        ..Modes::default()
    };
    assert_eq!(
        wheel_bytes(true, 4, 9, default),
        Some(vec![0x1b, b'[', b'M', 96, 37, 42])
    );
    assert_eq!(
        wheel_bytes(true, 300, 0, default),
        None,
        "beyond what one byte can name"
    );
    let utf8 = Modes {
        mouse: MouseMode::ButtonMotion,
        mouse_encoding: MouseEncoding::Utf8,
        ..Modes::default()
    };
    let report = wheel_bytes(false, 300, 0, utf8).unwrap();
    assert!(report.starts_with(b"\x1b[M") && std::str::from_utf8(&report[3..]).is_ok());
}

#[test]
fn without_a_mouse_mode_the_wheel_is_arrows_on_the_alternate_screen_and_nothing_elsewhere() {
    let alternate = Modes {
        alternate_screen: true,
        ..Modes::default()
    };
    assert_eq!(
        wheel_bytes(true, 0, 0, alternate),
        Some(b"\x1b[A\x1b[A\x1b[A".to_vec())
    );
    let application = Modes {
        application_cursor: true,
        alternate_screen: true,
        ..Modes::default()
    };
    assert_eq!(
        wheel_bytes(false, 0, 0, application),
        Some(b"\x1bOB\x1bOB\x1bOB".to_vec())
    );
    assert_eq!(
        wheel_bytes(true, 0, 0, Modes::default()),
        None,
        "the normal screen has none"
    );
    let press_only = Modes {
        mouse: MouseMode::Press,
        alternate_screen: true,
        ..Modes::default()
    };
    assert_eq!(
        wheel_bytes(true, 0, 0, press_only),
        None,
        "a program that asked only for presses does not get a wheel, nor arrows"
    );
}
