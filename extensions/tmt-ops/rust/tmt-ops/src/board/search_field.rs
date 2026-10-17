//! Shared in-place search presentation and keys; App retains the only query.
use crate::look::Look;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::Role;
use tmt_tui::components::{Editor, strip};

// The disposable view cache admits the same bounded single-line query.
pub(super) const MAX_BYTES: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Input {
    Changed,
    Previous,
    Next,
}

pub(super) fn key(query: &mut String, editing: &mut bool, key: KeyEvent) -> Option<Input> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return editing.then_some(Input::Changed);
    }
    if !*editing && (query.is_empty() || key.code != KeyCode::Esc) {
        return None;
    }
    match key.code {
        KeyCode::Esc => {
            query.clear();
            *editing = false;
        }
        KeyCode::Enter => *editing = false,
        KeyCode::Backspace if query.is_empty() => *editing = false,
        KeyCode::Backspace => {
            query.pop();
        }
        KeyCode::Up => return Some(Input::Previous),
        KeyCode::Down => return Some(Input::Next),
        KeyCode::Char(character)
            if !character.is_control() && query.len() + character.len_utf8() <= MAX_BYTES =>
        {
            query.push(character);
        }
        _ => return Some(Input::Changed),
    }
    Some(Input::Changed)
}

/// Paste remains a single-line query; reject the whole edit at the cache bound.
pub(super) fn paste(query: &mut String, text: &str) -> bool {
    let text: String = crate::board::notes::sanitize(text)
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if query.len().saturating_add(text.len()) > MAX_BYTES {
        return false;
    }
    query.push_str(&text);
    true
}

pub(super) fn paint(
    buffer: &mut Buffer,
    area: Rect,
    query: &str,
    editing: bool,
    key: Option<&str>,
    look: Look,
) {
    let value = if query.is_empty() && !editing {
        "Search"
    } else {
        query
    };
    let value = crate::board::notes::sanitize(value);
    let cap = key
        .map(|key| super::keycap::span(key, false, look))
        .filter(|cap| cap.width() + 2 <= usize::from(area.width));
    let prefix = 1 + cap.as_ref().map_or(0, |cap| cap.width() + 1);
    let available = usize::from(area.width).saturating_sub(prefix);
    let text = if editing && available > 0 {
        let available = available.saturating_sub(1); // Caret.
        let editor = Editor::from(value.as_str());
        let layout = editor.layout(u16::MAX);
        let cells = &layout.lines[0].cells;
        let clipped = cells.iter().map(|cell| cell.width).sum::<usize>() > available;
        let limit = available.saturating_sub(usize::from(clipped));
        let mut width = 0;
        let mut tail: Vec<_> = cells
            .iter()
            .rev()
            .take_while(|cell| {
                width += cell.width;
                width <= limit
            })
            .map(|cell| cell.text.as_str())
            .collect();
        tail.reverse();
        format!(
            "{}{}▏",
            if clipped && available > 0 { "…" } else { "" },
            tail.concat()
        )
    } else {
        crate::board::view::fit(&value, available)
    };
    let mut spans = vec![Span::raw(" ")];
    if let Some(cap) = cap {
        spans.push(cap);
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(text.clone()));
    let padding = usize::from(area.width)
        .saturating_sub(prefix + unicode_width::UnicodeWidthStr::width(text.as_str()));
    spans.push(Span::raw(" ".repeat(padding)));
    strip::paint_left(
        buffer,
        area,
        Line::from(spans).style(look.role(Role::Text).patch(look.role(Role::Track))),
    );
}

/// Highlight original characters, including Unicode case-fold expansions, with
/// weight only. No lowercased text or control bytes reach the display.
fn highlight(text: &str, query: &str, normal: ratatui::style::Style) -> Line<'static> {
    let text = crate::board::notes::sanitize(text);
    let needle = query.to_lowercase();
    if needle.is_empty() {
        return Line::from(Span::styled(text, normal));
    }
    let mut spans = Vec::new();
    let mut plain = 0;
    let mut consumed = 0;
    for (start, _) in text.char_indices() {
        if start < consumed || !text[start..].to_lowercase().starts_with(&needle) {
            continue;
        }
        if plain < start {
            spans.push(Span::styled(text[plain..start].to_owned(), normal));
        }
        let mut folded = String::new();
        let mut end = start;
        for (offset, character) in text[start..].char_indices() {
            folded.extend(character.to_lowercase());
            end = start + offset + character.len_utf8();
            if folded.len() >= needle.len() {
                break;
            }
        }
        spans.push(Span::styled(
            text[start..end].to_owned(),
            normal.add_modifier(ratatui::style::Modifier::BOLD),
        ));
        consumed = end;
        plain = end;
    }
    if plain < text.len() {
        spans.push(Span::styled(text[plain..].to_owned(), normal));
    }
    Line::from(spans)
}

pub(super) fn highlight_line(mut line: Line<'static>, query: &str) -> Line<'static> {
    line.spans = line
        .spans
        .into_iter()
        .flat_map(|span| highlight(&span.content, query, span.style).spans)
        .collect();
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(query: &mut String, editing: &mut bool, code: KeyCode) -> Option<Input> {
        key(query, editing, KeyEvent::new(code, KeyModifiers::NONE))
    }
    #[test]
    fn typing_moves_without_committing_enter_retains_and_escape_clears_both_states() {
        let mut query = String::from("night");
        let mut editing = true;
        assert_eq!(
            press(&mut query, &mut editing, KeyCode::Down),
            Some(Input::Next)
        );
        assert_eq!(
            press(&mut query, &mut editing, KeyCode::Up),
            Some(Input::Previous)
        );
        assert!(editing);
        press(&mut query, &mut editing, KeyCode::Enter);
        assert!(!editing);
        assert_eq!(query, "night");
        assert_eq!(press(&mut query, &mut editing, KeyCode::Char('j')), None);
        press(&mut query, &mut editing, KeyCode::Esc);
        assert!(query.is_empty() && !editing);
        editing = true;
        press(&mut query, &mut editing, KeyCode::Char('x'));
        press(&mut query, &mut editing, KeyCode::Esc);
        assert!(query.is_empty() && !editing);
        editing = true;
        press(&mut query, &mut editing, KeyCode::Backspace);
        assert!(!editing);
    }
    #[test]
    fn editing_consumes_modified_shortcuts_without_mutating_the_query() {
        let mut query = String::from("worker");
        let mut editing = true;
        assert_eq!(
            key(
                &mut query,
                &mut editing,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)
            ),
            Some(Input::Changed)
        );
        assert_eq!(query, "worker");
        assert!(editing);
        editing = false;
        assert_eq!(
            key(
                &mut query,
                &mut editing,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)
            ),
            None
        );
    }
    #[test]
    fn narrow_editing_keeps_the_caret_and_unicode_tail_visible_without_changing_query() {
        let query = "long prefix · 工作台 e\u{301}";
        let area = Rect::new(0, 0, 12, 1);
        let mut buffer = Buffer::empty(area);
        paint(
            &mut buffer,
            area,
            query,
            true,
            Some("/"),
            Look::new(tmt_cli_style::Theme::default()),
        );
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.starts_with("  /  …"), "{text}");
        assert!(text.contains("e\u{301}▏"), "{text}");
        assert!(!text.contains('�'));
        assert_eq!(text.matches('▏').count(), 1);
    }
    #[test]
    fn paste_normalizes_lines_and_rejects_a_whole_oversize_edit() {
        let mut query = String::from("night");
        assert!(paste(&mut query, "\nworker"));
        assert_eq!(query, "night worker");
        assert!(!paste(&mut query, &"x".repeat(MAX_BYTES)));
        assert_eq!(query, "night worker");
    }
    #[test]
    fn an_empty_query_preserves_every_existing_span_style() {
        use ratatui::style::{Color, Modifier, Style};
        let original = Line::from(vec![
            Span::styled(
                "worker",
                Style::default()
                    .fg(Color::Rgb(176, 176, 176))
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled(" waiting", Style::default().fg(Color::Rgb(10, 20, 30))),
        ])
        .style(Style::default().bg(Color::Rgb(40, 50, 60)));
        assert_eq!(highlight_line(original.clone(), ""), original);
    }
    #[test]
    fn matched_characters_keep_original_text_and_only_add_weight() {
        let look = Look::new(tmt_cli_style::Theme::default());
        for (text, query, expected) in [
            ("Night-night", "NIGHT", 2),
            ("İnfra", "i", 1),
            ("worker", "missing", 0),
        ] {
            let line = highlight(text, query, look.role(Role::Text));
            assert_eq!(line.to_string(), text);
            assert_eq!(
                line.spans
                    .iter()
                    .filter(|span| span
                        .style
                        .add_modifier
                        .contains(ratatui::style::Modifier::BOLD))
                    .count(),
                expected
            );
            assert!(
                line.spans
                    .iter()
                    .all(|span| span.style.fg == look.role(Role::Text).fg)
            );
        }
    }
}
