//! The board's glyph guard, shared by the tests of every surface that paints
//! authored labels (home, cron, footer). Test-only.

use serde_json::Value;

/// These are structural typography, not decorative/state symbols. Each
/// exception stays explicit so adding a new decoration cannot widen the guard.
const STRUCTURAL_GLYPHS: &[(&str, &str)] = &[
    (
        "─│┌┐└┘├┤┬┴┼",
        "single-line rules and square overlay borders",
    ),
    ("·", "separates adjacent labels without implying a state"),
    ("…", "shared text fitter's truncation indicator"),
    ("–", "unavailable evidence; distinct from measured zero"),
    (
        "↑↓←→",
        "navigation keys, scroll direction and annotation targets",
    ),
];

pub(crate) fn registered_marks() -> std::collections::BTreeSet<char> {
    let tokens: Value =
        serde_json::from_str(include_str!("../../../../../../design/tokens/tokens.json")).unwrap();
    tokens["mark"]
        .as_object()
        .unwrap()
        .keys()
        .flat_map(|key| key.chars().filter(|c| !c.is_whitespace()))
        .collect()
}

/// Call only with board-owned labels or ASCII fixture inputs. Names, tasks and
/// notebooks are dynamic text and deliberately do not go through this guard.
pub(crate) fn glyph_error(text: &str) -> Option<String> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let marks = registered_marks();
    let states = tmt_cli_style::mark::Mark::ALL
        .into_iter()
        .flat_map(|mark| mark.symbol().chars())
        .chain(['◐', '✎'])
        .collect::<std::collections::BTreeSet<_>>();
    for line in text.lines() {
        // Selection occupies one existing prefix blank. Recognize only the three
        // admitted local row prefixes; surrounding canvas/pane text is not a prefix.
        let prefix = line.chars().take(4).collect::<Vec<_>>();
        let grid_cue = prefix.len() >= 2
            && states.contains(&prefix[0])
            && prefix[1] == '>'
            && prefix.get(2) != Some(&'>');
        let mut chars = line.chars().enumerate().peekable();
        let mut previous: Option<char> = None;
        while let Some((column, character)) = chars.next() {
            let structural = STRUCTURAL_GLYPHS.iter().any(|(glyphs, reason)| {
                assert!(!reason.is_empty());
                glyphs.contains(character)
            });
            let ambiguous = character.width() != character.width_cjk();
            // Presentation sequences catch text-default emoji such as the stopwatch;
            // wide glyphs also reject default emoji in these fixed English labels.
            let emoji = matches!(character, '\u{fe0f}' | '\u{20e3}')
                || !character.is_ascii()
                    && (character.width() == Some(2)
                        || format!("{character}\u{fe0f}").width() > character.width().unwrap_or(0));
            if (emoji || ambiguous) && !structural && !marks.contains(&character) {
                return Some(format!("unregistered decorative glyph {character:?}"));
            }
            if states.contains(&character)
                && !character.is_ascii()
                && previous.is_some_and(|previous| !previous.is_whitespace())
                && !((column == 1 && prefix.first() == Some(&'>'))
                    || (column == 2 && prefix.starts_with(&['│', '>'])))
            {
                return Some(format!("state mark {character:?} needs a leading space"));
            }
            if states.contains(&character)
                && chars.peek().map(|(_, c)| *c) != Some(' ')
                && !(column == 0 && grid_cue)
            {
                return Some(format!("state mark {character:?} needs a trailing space"));
            }
            previous = Some(character);
        }
    }
    None
}

#[test]
fn glyph_guard_rejects_unregistered_ambiguous_and_emoji_decorations_and_unspaced_states() {
    for bad in [
        "① squads",
        "② needs you",
        "③ squads",
        "⑤ cron",
        "⏱ cron",
        "😀 cron",
        "1️⃣ cron",
        "◐0",
        "◆✗",
        "↻lead",
        "tab✗ 1",
    ] {
        assert!(glyph_error(bad).is_some(), "negative control {bad:?}");
    }
    for mark in tmt_cli_style::mark::Mark::ALL {
        assert_eq!(glyph_error(&format!("{} state", mark.symbol())), None);
    }
    for good in [
        "── squads · 0 ──",
        "↑ move ↓ · next… –",
        "◆ 0 ✗ 1 ◐ 2 ● 3 ○ 4",
        "2 other",
    ] {
        assert_eq!(glyph_error(good), None, "positive control {good:?}");
    }
}

#[test]
fn selection_prefix_exceptions_are_local_single_cell_and_keep_other_guards() {
    use unicode_width::UnicodeWidthStr;
    for (before, after) in [
        ("◆ row", "◆>row"),
        (" ◆ row", ">◆ row"),
        ("│ ◆ row", "│>◆ row"),
    ] {
        assert_eq!(before.width(), after.width());
        assert_eq!(glyph_error(after), None);
    }
    assert_eq!(glyph_error("◆>row\n>◆ row\n│>◆ row"), None);
    for bad in [
        " ◆>row",
        "label>◆ row",
        "◆>>row",
        ">>◆ row",
        "│>>◆ row",
        ">◆row",
        "│>◆row",
        "◆>row ✗bad",
        "◆>row\nlabel>◆ row",
        ">◆ row\n◆✗",
        "│>◆ row\n😀 decoration",
    ] {
        assert!(glyph_error(bad).is_some(), "negative control {bad:?}");
    }
}
