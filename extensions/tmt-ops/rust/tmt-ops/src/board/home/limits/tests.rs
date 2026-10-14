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
fn expanded_windows_keep_remaining_reset_and_measured_weekly_pace() {
    for width in [160, 140, 100, 80, 60] {
        let lines = limits_in(
            &mut Kept::default(),
            Some(&mock()),
            NOW,
            width,
            crate::look::Look::default(),
        );
        assert_eq!(lines.len(), 3);
        let contents = lines.iter().map(text).collect::<Vec<_>>();
        assert!(contents[0].contains("(7d) 44% left"));
        assert!(contents[0].contains("reset 4d03h"));
        assert!(contents[0].contains("pace ~2.1%/h"));
        assert!(contents[1].contains("(5h) 97% left"));
        assert!(contents[1].contains("reset 3h14m"));
        assert!(
            !contents[1].contains("pace"),
            "short-window pace is not reported"
        );
        assert!(contents[2].contains("(7d) 78% left"));
        assert!(
            contents[2].contains("reset 2d10h"),
            "healthy account still labels reset"
        );
        assert!(!contents[2].contains("!"));
        assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        assert!(lines.iter().all(|line| {
            line.spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::REVERSED))
        }));
    }
}

#[test]
fn expanded_unknown_and_zero_have_distinct_cells_without_inventing_a_window() {
    let missing = rows(
        Some(&[standing(Provider::named("codex"), None, None)]),
        NOW,
        80,
    );
    let missing_text = missing.to_string();
    assert!(missing_text.contains("no reading yet"));
    assert!(!missing_text.contains("0%"));
    let zero = rows(
        Some(&[standing(
            Provider::named("codex"),
            Some(reading(MINUTE, window(0.0, HOUR), None)),
            None,
        )]),
        NOW,
        80,
    );
    let rows = zero.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["pieces"][0]["text"], "");
    assert!(
        rows[0]["pieces"][1]["text"]
            .as_str()
            .unwrap()
            .contains("0% left")
    );
    assert!(
        rows[0]["pieces"][2]["text"]
            .as_str()
            .unwrap()
            .contains("reset 1h00m")
    );
}

#[test]
fn expanded_clock_projection_matches_current_visible_rows() {
    let standings = mock();
    let rows = rows(Some(&standings), NOW, 80);
    let mark: Value = serde_json::from_str(&clock_marks(Some(&standings), NOW, 80, 2)).unwrap();
    assert_eq!(mark.as_array().unwrap(), &rows.as_array().unwrap()[..2]);
    assert_ne!(
        clock_marks(Some(&standings), NOW, 80, 2),
        clock_marks(Some(&standings), NOW + MINUTE, 80, 2)
    );
}

#[test]
fn meters_follow_the_approved_widths_without_clipping_critical_values() {
    for (width, count) in [(160, 3), (100, 3), (80, 3), (60, 3)] {
        let lines = painted(Some(&mock()), width);
        assert_eq!(lines.len(), count, "{width}");
        let content = lines.join("\n");
        for value in [
            "Claude",
            "(7d) 44% left",
            "Codex",
            "(7d) 78% left",
            "!",
            "~20h57m",
        ] {
            assert!(content.contains(value), "{width}: {content}");
        }
        assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        assert!(content.contains("~2.1%/h"));
        assert!(content.contains("(5h) 97% left"));
        assert!(content.contains("reset 4d03h"));
        assert!(!content.contains("empties"));
        assert!(content.contains(" left"));
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
        for width in [160, 140, 100, 80, 60] {
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
    assert!(content.contains("pace not known"));
    assert!(content.contains("no use measured"));
    assert!(content.contains("5h"));
    assert!(!content.contains("%/h"));
    assert!(!content.contains("│"));
    assert!(!content.contains("empties") && !content.contains("!"));
}

#[test]
fn pending_and_no_providers_have_explicit_geometry() {
    for width in [160, 140, 100, 80, 60] {
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
                look.role(Role::Working)
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
            44.0 => 10,
            _ => 24,
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
    for width in [160, 140, 100, 80, 60] {
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
fn arrows_fit_only_beside_pace_and_use_their_semantic_roles() {
    for (trend, glyph, role) in [
        (Trend::Up, "↗", Role::Waiting),
        (Trend::Same, "→", Role::Muted),
        (Trend::Down, "↘", Role::Working),
    ] {
        let mut standings = mock();
        standings.truncate(1);
        standings[0].trend = Some(trend);
        for width in [160, 140, 100, 80, 60] {
            let look = crate::look::Look::default();
            let lines = limits_in(&mut Kept::default(), Some(&standings), NOW, width, look);
            assert!(text(&lines[0]).contains("~2.1%/h"));
            assert!(text(&lines[0]).ends_with(glyph));
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

#[test]
fn remaining_bands_do_not_follow_forecast_urgency() {
    for (left, expected) in [
        (0.0, Role::Blocked),
        (14.99, Role::Blocked),
        (15.0, Role::Waiting),
        (40.0, Role::Waiting),
        (40.01, Role::Working),
        (100.0, Role::Working),
    ] {
        for burn in [None, Some(100.0)] {
            let sample = standing(
                Provider::named("claude"),
                Some(reading(MINUTE, window(left, 100 * HOUR), None)),
                burn,
            );
            let Outlook::Known(figures) = sample.outlook(NOW) else {
                panic!("fresh reading")
            };
            assert_eq!(state(&figures), expected, "{left}, {burn:?}");
        }
    }
    for (left, burn, glyph, warning_role) in [
        (89.0, 10.0, "✗", Role::Blocked),
        (14.0, 0.5, "!", Role::Waiting),
    ] {
        let sample = standing(
            Provider::named("claude"),
            Some(reading(MINUTE, window(left, 100 * HOUR), None)),
            Some(burn),
        );
        let parts = meter(&sample, NOW, 60);
        let warning = parts.iter().find(|p| p["id"] == "warning").unwrap();
        assert!(warning["text"].as_str().unwrap().contains(glyph));
        assert_eq!(warning["role"], warning_role.name());
    }
}

#[test]
fn forecast_tail_requires_depletion_strictly_before_reset_at_every_width() {
    for (burn, forecasts) in [
        (None, false),
        (Some(0.0), false),
        (Some(1.0), false),
        (Some(2.0), false),
        (Some(2.1), true),
    ] {
        let mut sample = standing(
            Provider::named("claude"),
            Some(reading(MINUTE, window(40.0, 20 * HOUR), None)),
            burn,
        );
        sample.trend = Some(Trend::Up);
        for width in [160, 140, 100, 80, 60] {
            let parts = meter(&sample, NOW, width);
            let warning = parts.iter().find(|p| p["id"] == "warning");
            assert_eq!(warning.is_some(), forecasts, "{burn:?}, {width}");
            let content = painted(Some(&[sample.clone()]), width).join("\n");
            assert!(content.contains("40% left"));
            assert!(content.contains("reset 20h00m"));
            assert_eq!(
                content.contains("! ~") || content.contains("✗ ~"),
                forecasts
            );
        }
    }
}
