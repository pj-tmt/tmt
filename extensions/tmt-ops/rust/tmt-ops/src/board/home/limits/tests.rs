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
        .collect()
}

fn painted(standings: Option<&[Standing]>, width: u16) -> Option<String> {
    limits_in(
        &mut Kept::default(),
        standings,
        NOW,
        width,
        crate::look::Look::default(),
    )
    .map(|line| text(&line))
}

#[test]
fn from_lg_both_windows_say_left_and_the_flag_is_in_words() {
    // Claude's burn empties its week (44% at 2.1%/h is 21 h; 99 h remain).
    assert_eq!(
        painted(Some(&mock()), 140).unwrap(),
        "Claude  7d 44% left · 4d03h · ~2.1%/h · runs out before reset   5h 97% left · 3h14m  │  Codex  7d 78% left · 2d10h · ~0.4%/h   5h –"
    );
}

#[test]
fn at_md_only_the_week_shows_and_the_warning_mark_leads_the_flag() {
    let expected =
        "Claude  7d 44% · 4d03h · ~2.1%/h ! before reset  │  Codex  7d 78% · 2d10h · ~0.4%/h";
    for width in [100, 139] {
        assert_eq!(painted(Some(&mock()), width).unwrap(), expected, "{width}");
    }
}

#[test]
fn nothing_is_painted_below_md() {
    assert_eq!(painted(Some(&mock()), 99), None);
}

#[test]
fn a_rate_appears_only_with_enough_samples_and_the_flag_only_when_it_runs_out() {
    let mut standings = mock();
    standings[0].burn = None;
    standings[1].burn = Some(0.0);
    standings[1].latest.as_mut().unwrap().weekly.left = 78.0;
    assert_eq!(
        painted(Some(&standings), 100).unwrap(),
        "Claude  7d 44% · 4d03h  │  Codex  7d 78% · 2d10h · ~0.0%/h"
    );
}

#[test]
fn a_provider_without_a_current_reading_says_so_and_never_shows_a_number() {
    let stale = reading(3 * HOUR + 5 * MINUTE, window(44.0, 100 * HOUR), None);
    let standings = vec![
        standing(Provider::named("claude"), Some(stale), Some(2.1)),
        standing(Provider::named("codex"), None, None),
    ];
    for width in [100, 140] {
        let row = painted(Some(&standings), width).unwrap();
        assert_eq!(
            row, "Claude  no reading for 3h  │  Codex  no reading yet",
            "{width}"
        );
        assert!(!row.contains("44"), "the old number is never quoted");
    }
}

#[test]
fn a_week_that_already_reset_is_not_quoted_even_when_the_sample_is_fresh() {
    let reset = reading(10 * MINUTE, window(44.0, 0), None);
    let standings = vec![standing(Provider::named("claude"), Some(reset), None)];
    assert_eq!(
        painted(Some(&standings), 100).unwrap(),
        "Claude  no reading for 10m"
    );
}

#[test]
fn before_the_first_sample_a_placeholder_holds_the_row() {
    assert_eq!(painted(None, 100).unwrap(), PENDING);
    assert_eq!(painted(None, 140).unwrap(), PENDING);
}

#[test]
fn a_board_with_no_provider_shows_no_row() {
    assert_eq!(painted(Some(&[]), 140), None);
}

#[test]
fn reset_and_age_use_the_footers_whole_units() {
    for (ms, expected) in [
        (0, "0m"),
        (59_999, "0m"),
        (45 * MINUTE, "45m"),
        (60 * MINUTE, "1h00m"),
        (194 * MINUTE, "3h14m"),
        (24 * HOUR, "1d00h"),
        (5940 * MINUTE, "4d03h"),
    ] {
        assert_eq!(reset(ms), expected, "{ms}");
    }
    for (ms, expected) in [
        (0, "0m"),
        (59 * MINUTE, "59m"),
        (HOUR, "1h"),
        (23 * HOUR + 59 * MINUTE, "23h"),
        (48 * HOUR, "2d"),
    ] {
        assert_eq!(age(ms), expected, "{ms}");
    }
}

#[test]
fn percent_is_rounded_and_a_sliver_is_never_shown_as_zero() {
    for (value, expected) in [
        (0.0, "0%"),
        (0.4, "<1%"),
        (1.0, "1%"),
        (44.0, "44%"),
        (97.4, "97%"),
        (100.0, "100%"),
    ] {
        assert_eq!(percent(value), expected, "{value}");
    }
}

#[test]
fn the_warning_and_the_labels_take_their_roles() {
    let look = crate::look::Look::default();
    let line = limits_in(&mut Kept::default(), Some(&mock()), NOW, 140, look).unwrap();
    let style_of = |needle: &str| {
        line.spans
            .iter()
            .find(|span| span.content.contains(needle))
            .unwrap_or_else(|| panic!("no span with {needle:?}"))
            .style
            .fg
    };
    assert_eq!(
        style_of("runs out before reset"),
        look.role(Role::Waiting).fg
    );
    assert_eq!(style_of("Claude"), look.role(Role::Text).fg);
    assert_ne!(look.role(Role::Waiting).fg, look.role(Role::Text).fg);
}
