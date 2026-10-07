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
        let mut chars = line.chars().peekable();
        let mut previous: Option<char> = None;
        while let Some(character) = chars.next() {
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
            {
                return Some(format!("state mark {character:?} needs a leading space"));
            }
            if states.contains(&character) && chars.peek().copied() != Some(' ') {
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
fn retired_occurrence_prefixes_do_not_relax_semantic_mark_spacing() {
    for bad in ["◆>row", ">◆ row", "│>◆ row", " >◆ row", "label>◆ row"] {
        assert!(glyph_error(bad).is_some(), "negative control {bad:?}");
    }
    for good in ["◆ row", " ◆ row", "  ◆ row", " │ ◆ row"] {
        assert_eq!(glyph_error(good), None, "positive control {good:?}");
    }
}
