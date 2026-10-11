use super::*;
use crate::board::limits::{Provider, Reading, Window};

const NOW: u64 = 1_791_600_000_000;
const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;

fn window(left: f64, resets_in_ms: u64) -> Window {
    Window {
        left,
        resets_at_ms: NOW + resets_in_ms,
    }
}

fn standing(provider: Provider, reading: Option<Reading>, burn: Option<f64>) -> Standing {
    Standing {
        provider,
        latest: reading,
        burn,
        trend: None,
    }
}

fn reading(observed_ago: u64, weekly: Window, short: Option<Window>) -> Reading {
    Reading {
        observed_at_ms: NOW - observed_ago,
        weekly,
        short,
    }
}

/// The approved mock: Claude runs out before its week resets, Codex has no
/// short window.
fn mock() -> Vec<Standing> {
    vec![
        standing(
            Provider::named("claude"),
            Some(reading(
                MINUTE,
                window(44.0, (4 * 24 + 3) * HOUR),
                Some(window(97.0, 3 * HOUR + 14 * MINUTE)),
            )),
            Some(2.1),
        ),
        standing(
            Provider::named("codex"),
            Some(reading(MINUTE, window(78.0, (2 * 24 + 10) * HOUR), None)),
            Some(0.4),
        ),
    ]
}

fn text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
        .trim_end()
        .into()
}

fn painted(standings: Option<&[Standing]>, width: u16) -> Vec<String> {
    limits_in(
        &mut Kept::default(),
        standings,
        NOW,
        width,
        crate::look::Look::default(),
    )
    .iter()
    .map(text)
    .collect()
}

#[test]
fn meters_follow_the_approved_widths_without_clipping_critical_values() {
    for (width, count) in [(160, 1), (100, 2), (80, 2), (60, 2)] {
        let lines = painted(Some(&mock()), width);
        assert_eq!(lines.len(), count, "{width}");
        let content = lines.join("\n");
        for value in ["Claude 44%", "Codex 78%", "!", "~20h57m"] {
            assert!(content.contains(value), "{width}: {content}");
        }
        assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        assert!(content.contains("~2.1%/h"));
        assert!(!content.contains("5h 97%"));
        assert!(content.contains("│ 4d03h"));
        assert!(!content.contains("empties"));
        assert!(!content.contains(" left"));
        assert!(content.contains("(7d)"));
    }
}

#[test]
fn missing_stale_and_reset_readings_say_so_without_historical_numbers() {
    for observed in [3 * HOUR + 5 * MINUTE, 10 * MINUTE] {
        let expired = observed == 10 * MINUTE;
        let stale = reading(
            observed,
            window(44.0, if expired { 0 } else { 100 * HOUR }),
            None,
        );
        let standings = vec![
            standing(Provider::named("claude"), Some(stale), Some(2.1)),
            standing(Provider::named("codex"), None, None),
        ];
        for width in [160, 100, 80, 60] {
            let content = painted(Some(&standings), width).join("\n");
            assert!(
                content.contains(if expired {
                    "no reading for 10m"
                } else {
                    "no reading for 3h"
                }),
                "{width}: {content}"
            );
            assert!(content.contains("no reading yet"));
            assert!(!content.contains("44%") && !content.contains("2.1%"));
        }
    }
}

#[test]
fn no_rate_and_zero_never_add_negative_pace_text_or_invent_a_countdown() {
    let mut standings = mock();
    standings[0].burn = None;
    standings[1].burn = Some(0.0);
    let content = painted(Some(&standings), 100).join("\n");
    assert!(!content.contains("pace not known"));
    assert!(!content.contains("no use measured"));
    assert!(!content.contains("5h"));
    assert!(!content.contains("%/h"));
    assert!(!content.contains("empties") && !content.contains("!"));
}

#[test]
fn pending_and_no_providers_have_explicit_geometry() {
    for width in [160, 100, 80, 60] {
        assert_eq!(painted(None, width), vec![PENDING]);
        assert!(painted(Some(&[]), width).is_empty());
    }
}

#[test]
fn fill_and_track_keep_their_roles_without_selection_even_in_light_and_no_color() {
    use tmt_cli_style::{Theme, theme::Base};
    for base in [Base::Tmt, Base::TmtLight] {
        for depth in [Depth::TrueColor, Depth::None] {
            let look = crate::look::Look {
                theme: Theme::new(base),
                depth,
            };
            let lines = limits_in(&mut Kept::default(), Some(&mock()), NOW, 80, look);
            let fill = lines[0]
                .spans
                .iter()
                .find(|span| span.style.add_modifier.contains(Modifier::REVERSED))
                .unwrap();
            assert_eq!(
                fill.style.fg.unwrap_or(ratatui::style::Color::Reset),
                look.role(Role::Waiting)
                    .fg
                    .unwrap_or(ratatui::style::Color::Reset)
            );
            let track = lines[0]
                .spans
                .iter()
                .find(|span| {
                    if depth == Depth::None {
                        span.style.add_modifier.contains(Modifier::UNDERLINED)
                    } else {
                        span.style.bg == look.role(Role::Track).bg
                    }
                })
                .unwrap();
            assert_eq!(
                track.style.fg.unwrap_or(ratatui::style::Color::Reset),
                look.role(Role::Text)
                    .fg
                    .unwrap_or(ratatui::style::Color::Reset)
            );
            assert!(!track.style.add_modifier.contains(Modifier::REVERSED));
            assert!(text(&lines[0]).contains("! ~"));
        }
    }
}

#[test]
fn positive_slivers_fill_one_cell_and_zero_fills_none() {
    for percent in [0.0, 0.4, 44.0, 100.0] {
        let mut standings = mock();
        standings.truncate(1);
        standings[0].latest.as_mut().unwrap().weekly.left = percent;
        let lines = limits_in(
            &mut Kept::default(),
            Some(&standings),
            NOW,
            60,
            crate::look::Look::default(),
        );
        let filled: usize = lines[0]
            .spans
            .iter()
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.width())
            .sum();
        let expected = match percent {
            0.0 => 0,
            0.4 => 1,
            44.0 => 8,
            _ => 18,
        };
        assert_eq!(filled, expected, "{percent}");
        let expected_label = if percent == 0.4 {
            "<1%".into()
        } else {
            format!("{percent:.0}%")
        };
        assert!(text(&lines[0]).contains(&expected_label));
    }
}

#[test]
fn labels_shorten_before_percentage_and_reset_and_cache_matches_fresh() {
    let mut standings = mock();
    standings[0].provider = Provider::named("longprovider");
    let mut slot = Kept::default();
    for width in [160, 100, 80, 60] {
        let look = crate::look::Look::default();
        let lines = limits_in(&mut slot, Some(&standings), NOW, width, look);
        assert_eq!(
            lines,
            limits_in(&mut Kept::default(), Some(&standings), NOW, width, look)
        );
        let built = slot.builds;
        assert_eq!(
            lines,
            limits_in(&mut slot, Some(&standings), NOW, width, look)
        );
        assert_eq!(slot.builds, built);
        let content = lines.iter().map(text).collect::<Vec<_>>().join("\n");
        assert!(content.contains("44%"));
        assert!(content.contains("~20h57m"));
        if width >= 80 {
            assert!(content.contains("4d03h"));
        }
    }
}

#[test]
fn reset_and_percent_preserve_the_actual_units() {
    assert_eq!(reset(5940 * MINUTE), "4d03h");
    assert_eq!(reset(194 * MINUTE), "3h14m");
    assert_eq!(reset(45 * MINUTE), "45m");
    assert_eq!(percent(0.4), "<1%");
    assert_eq!(percent(0.0), "0%");
}

#[test]
fn overview_detail_contains_only_the_values_hidden_at_each_width() {
    let standings = mock();
    for width in [160, 100, 80, 60] {
        let hidden = hidden_limits(&standings, NOW, width);
        let detail = hidden
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        assert_eq!(detail.contains("~2.1%/h"), false, "{width}: {detail}");
        assert_eq!(detail.contains("5h 97%"), true, "{width}: {detail}");
        assert_eq!(detail.contains("4d03h"), false, "{width}: {detail}");
        assert!(!hidden.iter().any(|(label, _)| *label == "provider"));
        assert!(
            !detail.contains("44%"),
            "the visible weekly reading is never repeated"
        );
    }
    let mut long = standings[0].clone();
    long.provider = Provider::named("longprovider");
    let hidden = hidden_limits(&[long], NOW, 60);
    assert!(
        hidden
            .iter()
            .any(|(label, text)| *label == "provider" && text == "Longprovider")
    );
}

#[test]
fn arrows_fit_only_beside_pace_and_use_their_semantic_roles() {
    for (trend, glyph, role) in [
        (Trend::Up, "↗", Role::Waiting),
        (Trend::Same, "→", Role::Muted),
        (Trend::Down, "↘", Role::Working),
    ] {
        let mut standings = mock();
        standings.truncate(1);
        standings[0].trend = Some(trend);
        for width in [160, 100, 80, 60] {
            let look = crate::look::Look::default();
            let lines = limits_in(&mut Kept::default(), Some(&standings), NOW, width, look);
            assert!(text(&lines[0]).ends_with(&format!("~2.1%/h {glyph}")));
            let arrow = lines[0]
                .spans
                .iter()
                .find(|span| span.content.contains(glyph))
                .unwrap();
            assert_eq!(arrow.style.fg, look.role(role).fg);
            assert!(
                !arrow
                    .style
                    .add_modifier
                    .intersects(Modifier::REVERSED | Modifier::UNDERLINED)
            );
        }
        for burn in [None, Some(0.0), Some(f64::INFINITY)] {
            standings[0].burn = burn;
            assert!(!painted(Some(&standings), 60).join("\n").contains(glyph));
        }
        standings[0].burn = Some(2.1);
        assert!(
            !painted(Some(&standings), 32).join("\n").contains(glyph),
            "a hidden pace cannot have an arrow"
        );
    }
}
