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
    ("━", "focused base-pane heavy top rule"),
    (
        "▁▂▃▄▅▆▇█▒░■",
        "usage height fractions, provider segments and legend keys",
    ),
    ("↩", "Recent exchanges reply direction"),
    ("▶", "NO_COLOR receiving-pane marker"),
    ("·", "separates adjacent labels without implying a state"),
    ("…", "shared text fitter's truncation indicator"),
    ("≈", "estimated list-price savings in the Digest matrix"),
    ("–", "unavailable evidence; distinct from measured zero"),
    (
        "—",
        "unavailable fixed-table evidence; same unknown meaning as en dash",
    ),
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
        "▶\u{fe0f} Agents",
        "■\u{fe0f} Claude",
        "↩\u{fe0f} reply",
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
        "Est. ctx avoided 24h: —",
        "◆ 0 ✗ 1 ◐ 2 ● 3 ◌ 4",
        "2 other",
        "▁▂▃▄▅▆▇█▒░ ■ Claude · ↩ reply",
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

#[test]
fn real_pane_titles_register_heavy_no_color_and_ascii_focus_cues() {
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    use tmt_cli_style::{Depth, Theme};
    for (depth, ascii) in [
        (Depth::TrueColor, false),
        (Depth::None, false),
        (Depth::None, true),
    ] {
        let mut app = crate::board::app::workspace::tests::capture_case(
            "agents",
            crate::look::Look {
                theme: Theme::default(),
                depth,
            },
        );
        app.cron_glyphs = crate::board::overview::cron_counts::GlyphSet::from_ascii(ascii);
        assert_eq!(app.look().depth, depth);
        let mut terminal = Terminal::new(TestBackend::new(60, 6)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::pane_chrome::paint(
                    frame,
                    &app,
                    Rect::new(0, 0, 60, 6),
                    crate::config::Pane::Rows,
                    "Agents",
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let title = (0..60).map(|x| buffer[(x, 0)].symbol()).collect::<String>();
        assert!(title.starts_with("━━ "), "{title}");
        if depth == Depth::None {
            assert!(
                title.contains(if ascii { "> Agents" } else { "▶ Agents" }),
                "{title}"
            );
        }
        assert_eq!(glyph_error(&title), None, "{title}");
    }
}

#[test]
fn unavailable_table_cell_uses_the_registered_em_dash_without_broadening_decorations() {
    use ratatui::{Terminal, backend::TestBackend};
    let app = crate::board::app::workspace::tests::board();
    let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    let row = terminal
        .backend()
        .buffer()
        .content
        .chunks(60)
        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
        .find(|row| row.contains("▸ Common pool"))
        .unwrap();
    assert!(row.contains('—'));
    assert_eq!(glyph_error(&row), None);
    assert!(glyph_error("—\u{fe0f}").is_some());
    assert!(glyph_error("―").is_some());
}

#[test]
fn digest_matrix_estimate_caption_registers_only_the_approximation_operator() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = crate::board::app::workspace::tests::board();
    app.shell.tab = crate::board::shell::OpsTab::Overview;
    let mut terminal = Terminal::new(TestBackend::new(160, 51)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(&mut app, frame.area());
            crate::board::view::render(frame, &app);
        })
        .unwrap();
    let row = terminal
        .backend()
        .buffer()
        .content
        .chunks(160)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .find(|row| row.contains("≈ $ avoided"))
        .expect("actual matrix estimate caption");
    assert!(row.contains("≈ $ avoided"), "{row}");
    assert_eq!(glyph_error(&row), None);
    assert!(glyph_error("≈\u{fe0f}").is_some());
    assert!(glyph_error("≒").is_some());
}
