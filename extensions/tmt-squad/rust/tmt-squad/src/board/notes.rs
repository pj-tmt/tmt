//! The lead's notebook, shown as inert text. Notes are agent-written, so every
//! terminal escape and control character is removed before display; this is
//! the plain rendering that Markdown rendering (#391) builds on.

use unicode_width::UnicodeWidthChar;

const ESC: char = '\u{1b}';
const BEL: char = '\u{7}';

/// Removes escape sequences (CSI, OSC, DCS, SOS, PM, APC and two-byte ESC
/// forms, including their 8-bit C1 introducers) and all other control
/// characters. Newlines are kept; tabs become four spaces; CR is dropped.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        let introducer = match character {
            ESC => match chars.next() {
                Some('[') => Some('['),
                Some(']') => Some(']'),
                Some(kind @ ('P' | 'X' | '^' | '_')) => Some(kind),
                // Two-byte sequence (for example ESC 7): drop both.
                _ => None,
            },
            '\u{9b}' => Some('['),
            '\u{9d}' => Some(']'),
            '\u{90}' => Some('P'),
            '\u{98}' => Some('X'),
            '\u{9e}' => Some('^'),
            '\u{9f}' => Some('_'),
            '\n' => {
                out.push('\n');
                continue;
            }
            '\t' => {
                out.push_str("    ");
                continue;
            }
            other if other.is_control() => continue,
            other => {
                out.push(other);
                continue;
            }
        };
        match introducer {
            // CSI ends at a final byte in @..~.
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            // String sequences end at BEL (OSC) or ST (ESC \ or U+9C).
            Some(_) => {
                while let Some(next) = chars.next() {
                    if next == BEL || next == '\u{9c}' {
                        break;
                    }
                    if next == ESC {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            None => {}
        }
    }
    out
}

/// Wraps sanitized text to `width` display cells. Breaks at spaces where
/// possible; a longer word is split between characters, never inside one.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split_inclusive(' ') {
            let body = word.trim_end_matches(' ');
            let spaces = word.len() - body.len();
            let cells: usize = body.chars().map(|c| c.width().unwrap_or(0)).sum();
            if used > 0 && used + cells > width {
                lines.push(line.trim_end().to_owned());
                line.clear();
                used = 0;
            }
            for character in body.chars() {
                let cells = character.width().unwrap_or(0);
                if used + cells > width {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(character);
                used += cells;
            }
            for _ in 0..spaces {
                if used < width {
                    line.push(' ');
                    used += 1;
                }
            }
        }
        lines.push(line.trim_end().to_owned());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_controls_cannot_reach_the_terminal() {
        let hostile = concat!(
            "\u{1b}[31mred\u{1b}[0m ",
            "\u{1b}[2J\u{1b}[H",
            "\u{1b}]52;c;cGF3bmVk\u{7}",
            "\u{1b}]0;title\u{1b}\\",
            "\u{1b}Pdevice\u{1b}\\",
            "\u{9b}31mc1\u{9c}",
            "\u{9d}8;;https://x\u{9c}",
            "\u{1b}7saved\u{1b}8",
            "bell\u{7}\u{0}\r\n",
            "tab\there",
        );
        let clean = sanitize(hostile);
        assert_eq!(clean, "red c1savedbell\ntab    here");
        assert!(!clean.chars().any(|c| c.is_control() && c != '\n'));
        assert_eq!(
            sanitize("## Now\n- 安装指南: one page"),
            "## Now\n- 安装指南: one page"
        );
        // An unterminated sequence consumes the rest rather than leaking it.
        assert_eq!(sanitize("ok\u{1b}]52;c;unterminated"), "ok");
    }

    #[test]
    fn wrapping_counts_display_cells_and_never_splits_a_wide_character() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(
            wrap("整理安装指南文件", 5),
            ["整理", "安装", "指南", "文件"]
        );
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 10), ["a", "", "b"]);
        for line in wrap("mixed 中文 and English words with 宽字符 inside", 9) {
            let cells: usize = line.chars().map(|c| c.width().unwrap_or(0)).sum();
            assert!(cells <= 9, "{line:?}");
        }
    }
}
