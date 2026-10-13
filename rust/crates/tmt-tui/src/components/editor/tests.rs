use super::*;
fn key(editor: &mut Editor, code: KeyCode, modifiers: KeyModifiers) {
    assert!(editor.key(KeyEvent::new(code, modifiers), 8, 1024));
}
#[test]
fn middle_edits_and_selection_never_split_graphemes() {
    let mut editor = Editor::from("Ae\u{301}👩‍💻界Z");
    key(&mut editor, KeyCode::Left, KeyModifiers::NONE);
    key(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    assert_eq!(&editor.as_str()[editor.selection()], "界");
    assert!(editor.insert("中文", 1024));
    assert_eq!(editor.as_str(), "Ae\u{301}👩‍💻中文Z");
    key(&mut editor, KeyCode::Backspace, KeyModifiers::NONE);
    key(&mut editor, KeyCode::Backspace, KeyModifiers::NONE);
    key(&mut editor, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(editor.as_str(), "Ae\u{301}Z");
    key(&mut editor, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(editor.as_str(), "AZ");
}
#[test]
fn combining_insert_and_delete_keep_a_valid_boundary() {
    let mut editor = Editor::from("ab");
    key(&mut editor, KeyCode::Left, KeyModifiers::NONE);
    editor.insert("\u{301}", 1024);
    assert_eq!(editor.cursor(), "a\u{301}".len());
    key(&mut editor, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(editor.as_str(), "b");
}
#[test]
fn paste_overflow_preserves_text_selection_and_cursor() {
    let mut editor = Editor::from("abc");
    key(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    let before = editor.clone();
    assert!(!editor.insert("123456", 5));
    assert_eq!(editor, before);
    assert!(editor.insert("界", 5));
    assert_eq!(editor.as_str(), "ab界");
}
#[test]
fn newline_wrap_spaces_and_pointer_share_source_geometry() {
    let mut editor = Editor::from("ab 界\n\nx ");
    let layout = editor.layout(4);
    assert_eq!(layout.lines.len(), 4);
    assert_eq!(
        layout.lines[0]
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>(),
        "ab "
    );
    assert_eq!(layout.lines[1].cells[0].text, "界");
    editor.place(4, 1, 0, false);
    assert_eq!(editor.cursor(), 3);
    editor.place(4, 1, 1, true);
    assert_eq!(&editor.as_str()[editor.selection()], "界");
}

#[test]
fn terminal_paste_line_endings_keep_exact_source_and_share_visual_newlines() {
    for source in ["a\nb", "a\rb", "a\r\nb"] {
        let mut editor = Editor::from(source);
        let layout = editor.layout(20);
        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.lines[0].cells[0].text, "a");
        assert_eq!(layout.lines[1].cells[0].text, "b");
        editor.place(20, 1, 0, false);
        assert_eq!(&editor.as_str()[editor.cursor()..], "b");
        assert_eq!(editor.as_str(), source);
    }
}
#[test]
fn vertical_movement_keeps_column_across_short_lines_and_resize() {
    let mut editor = Editor::from("abcdef\nx\n123456");
    editor.place(20, 0, 5, false);
    assert!(editor.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 20, 1024));
    assert_eq!(editor.cursor(), 8);
    assert!(editor.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 20, 1024));
    assert_eq!(editor.cursor(), 14);
    let before = editor.clone();
    for width in [1, 2, 5, 80] {
        let frame = editor.layout(width);
        let top = frame.reveal(99, 2);
        assert!(frame.cursor.0 >= top && frame.cursor.0 < top + 2);
    }
    assert_eq!(editor, before);
}
#[test]
fn controls_and_bidi_are_inert_but_raw_payload_is_preserved() {
    let raw = "a\x1b]52;c;abc\x07\r\x08\u{202e}z";
    let editor = Editor::from(raw);
    let shown: String = editor
        .layout(100)
        .lines
        .iter()
        .flat_map(|l| &l.cells)
        .map(|c| c.text.as_str())
        .collect();
    assert!(!shown.contains('\x1b'));
    assert!(!shown.contains('\x07'));
    assert!(!shown.contains('\u{202e}'));
    assert_eq!(editor.as_str(), raw);
}
#[test]
fn select_all_replace_and_application_keys_remain_separate() {
    let mut editor = Editor::from("a\nb");
    key(&mut editor, KeyCode::Char('a'), KeyModifiers::CONTROL);
    editor.insert("hello", 1024);
    assert_eq!(editor.as_str(), "hello");
    for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Tab] {
        assert!(!editor.key(KeyEvent::new(code, KeyModifiers::NONE), 8, 1024));
    }
}

#[test]
fn folded_paste_keeps_exact_source_but_bounds_collapsed_geometry() {
    for bytes in [1024, 10 * 1024, 100 * 1024] {
        let source = format!("  \n{}\n\x1b[31m", "x".repeat(bytes));
        let mut editor = Editor::from("before ");
        assert!(editor.insert_folded(&source, "[paste]".into(), 1024 * 1024));
        assert_eq!(editor.as_str(), format!("before {source}"));
        assert_eq!(editor.folds().len(), 1);
        assert_eq!(editor.layout(80).lines.len(), 1);
        assert!(editor.toggle_fold());
        assert!(editor.layout(80).lines.len() > 10);
        assert!(editor.toggle_fold());
        editor.key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            80,
            1024 * 1024,
        );
        assert_eq!(editor.as_str(), "before ");
        assert!(editor.folds().is_empty());
    }
}

#[test]
fn folded_ranges_follow_insertions_and_do_not_swallow_adjacent_text() {
    let mut editor = Editor::from("a");
    assert!(editor.insert_folded("long\nsource", "[paste]".into(), 100));
    editor.insert("z", 100);
    editor.key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL), 40, 100);
    editor.insert("start", 100);
    assert_eq!(editor.folds()[0].source, 6..17);
    editor.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), 40, 100);
    editor.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE), 40, 100);
    assert_eq!(editor.as_str(), "startaz");
    assert!(editor.folds().is_empty());
}

#[test]
fn isolated_combining_marks_have_a_cell_and_folds_never_split_joined_graphemes() {
    let editor = Editor::from("\u{301}");
    assert_eq!(editor.layout(20).cursor, (0, 1));
    let mut editor = Editor::from("e");
    editor.insert_folded("\u{301}", "[source]".into(), 100);
    assert!(editor.folds().is_empty());
    editor.key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        20,
        100,
    );
    assert!(editor.is_empty());
    editor.insert_folded("e", "[source]".into(), 100);
    editor.insert("\u{301}", 100);
    assert!(editor.folds().is_empty());
    assert_eq!(editor.as_str(), "e\u{301}");
}

#[test]
fn geometry_is_reused_for_paint_and_caret_but_invalidated_by_edits_and_resize() {
    let mut editor = Editor::from("a long editable line");
    let first = editor.layout(8);
    let unpainted = editor.clone();
    let second = editor.layout(8);
    assert!(std::rc::Rc::ptr_eq(&first.lines, &second.lines));
    editor.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), 8, 100);
    let moved = editor.layout(8);
    assert!(std::rc::Rc::ptr_eq(&first.lines, &moved.lines));
    assert_ne!(first.cursor, moved.cursor);
    let mut painted = unpainted.clone();
    painted.layout(10);
    assert_eq!(painted, unpainted, "geometry is not a draft revision");
    painted.insert("!", 100);
    assert!(!std::rc::Rc::ptr_eq(&first.lines, &painted.layout(8).lines));
    assert!(!std::rc::Rc::ptr_eq(&first.lines, &editor.layout(10).lines));
}
