//! Scoped Timers modal hints. Whole hints drop by priority.

use unicode_width::UnicodeWidthStr;

/// (key, hint word), highest priority first.
const KEYS: [(&str, &str); 7] = [
    ("n", "new"),
    ("e", "expand"),
    ("E", "edit"),
    ("p", "pause"),
    ("x", "send"),
    ("o", "reassign"),
    ("d", "delete"),
];

fn fit(parts: Vec<String>, separator: &str, tail: &[&str], width: usize) -> String {
    let mut parts = parts;
    loop {
        let text = parts
            .iter()
            .map(String::as_str)
            .chain(tail.iter().copied())
            .collect::<Vec<_>>()
            .join(separator);
        if text.width() <= width {
            return text;
        }
        if parts.is_empty() {
            // Below the always-kept hints themselves: `? more` alone, then clipped.
            let last = tail.last().copied().unwrap_or_default();
            return crate::board::view::fit(last, width.min(last.width()));
        }
        parts.pop();
    }
}

/// The `c` list's inside footer.
pub(in crate::board) fn overlay(width: usize, selected: bool) -> String {
    let mut parts: Vec<String> = vec!["↑↓ choose".into(), "⏎ open squad".into()];
    parts.extend(
        KEYS.iter()
            .filter(|(key, _)| *key != "E" || selected)
            .map(|(key, word)| format!("{key} {word}")),
    );
    fit(parts, "  ", &["Esc close"], width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_hints_drop_by_priority_and_close_stays() {
        assert!(overlay(120, true).starts_with("↑↓ choose  ⏎ open squad  n new"));
        for width in 9..120 {
            let text = overlay(width, true);
            assert!(text.width() <= width, "{width}: {text}");
            assert!(text.ends_with("Esc close"), "{width}: {text}");
        }
        assert!(!overlay(160, false).contains("E edit"));
        assert!(overlay(5, true).width() <= 5);
    }
}
