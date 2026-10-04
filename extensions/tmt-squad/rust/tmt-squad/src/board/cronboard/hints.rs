//! The scoped job keys as footer, overlay-footer and help wording, from one
//! table so the three never disagree. Whole hints drop by priority.

use unicode_width::UnicodeWidthStr;

/// (key, hint word, help description), highest priority first.
const KEYS: [(&str, &str, &str); 7] = [
    ("⏎", "owner", "go to the job's owner"),
    ("n", "new", "add a job to this squad"),
    ("e", "edit", "edit the message and schedule"),
    ("p", "pause", "pause, or resume a paused job"),
    ("x", "send", "send the message now, once"),
    ("o", "reassign", "give the job to another member"),
    ("d", "delete", "delete the job after a confirmation"),
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
            let first = tail.first().copied().unwrap_or_default();
            return crate::board::view::fit(first, width.min(first.width()));
        }
        parts.pop();
    }
}

/// The base footer while the jobs half has focus.
pub(in crate::board) fn jobs(width: usize) -> String {
    let mut parts: Vec<String> = KEYS
        .iter()
        .map(|(key, word, _)| format!("{key} {word}"))
        .collect();
    parts.extend(["tab members".into(), "c all squads".into()]);
    fit(parts, "  ", &["? more", "q quit"], width)
}

/// The `c` list's inside footer.
pub(in crate::board) fn overlay(width: usize) -> String {
    let mut parts: Vec<String> = vec!["↑↓ choose".into(), "⏎ open squad".into()];
    parts.extend(
        KEYS[1..]
            .iter()
            .map(|(key, word, _)| format!("{key} {word}")),
    );
    fit(parts, " · ", &["Esc close"], width)
}

/// Help rows for the scoped keys.
pub(in crate::board) fn help() -> Vec<(&'static str, &'static str)> {
    KEYS.iter()
        .map(|(key, _, description)| (*key, *description))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_hints_drop_by_priority_and_help_with_quit_stay() {
        assert!(jobs(120).starts_with("⏎ owner  n new  e edit"));
        for width in 20..120 {
            let text = jobs(width);
            assert!(text.width() <= width, "{width}: {text}");
            assert!(text.ends_with("? more  q quit"), "{width}: {text}");
        }
        assert_eq!(jobs(40), "⏎ owner  n new  e edit  ? more  q quit");
        assert_eq!(jobs(14), "? more  q quit");
        assert!(jobs(5).width() <= 5);
        let wide = overlay(100);
        assert!(
            wide.starts_with("↑↓ choose · ⏎ open squad · n new"),
            "{wide}"
        );
        assert!(overlay(30).ends_with("Esc close"));
    }
}
