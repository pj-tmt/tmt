use super::*;

const OBSERVED: u64 = 1_791_600_000_000;
const MINUTE: u64 = 60_000;

/// Ben's statusline, as the pane shows it.
const FOOTER: &str = "Sonnet 5.5  ctx 89%   5h 97.0% (3h14m)  7d 56.0% (4d03h)   10-10 13:00";

fn reading(text: &str) -> Option<Reading> {
    read(text, OBSERVED)
}

#[test]
fn the_footer_reads_as_percent_left_and_reset_times_from_the_observation() {
    let reading = reading(FOOTER).unwrap();
    assert_eq!(reading.observed_at_ms, OBSERVED);
    assert_eq!(reading.weekly.left, 56.0);
    // 4d03h = 4 * 1440 + 3 * 60 = 5940 minutes.
    assert_eq!(reading.weekly.resets_at_ms, OBSERVED + 5940 * MINUTE);
    let short = reading.short.unwrap();
    assert_eq!(short.left, 97.0);
    assert_eq!(short.resets_at_ms, OBSERVED + 194 * MINUTE);
}

#[test]
fn the_bottom_most_footer_wins_over_scrollback_and_the_windows_may_swap() {
    let capture = format!(
        "older 5h 10.0% (1h00m)  7d 5.0% (1d00h)\n\n> prompt\n{}\n\n",
        "7d 56.0% (4d03h)  5h 97.0% (3h14m)"
    );
    let reading = reading(&capture).unwrap();
    assert_eq!(reading.weekly.left, 56.0);
    assert_eq!(reading.short.unwrap().left, 97.0);
}

#[test]
fn percent_is_a_plain_decimal_between_zero_and_a_hundred() {
    for (word, expected) in [
        ("0%", Some(0.0)),
        ("100%", Some(100.0)),
        ("97.5%", Some(97.5)),
    ] {
        assert_eq!(percent(word), expected, "{word}");
    }
    for word in [
        "", "%", "-1%", "+5%", "100.1%", "1e2%", ".5%", "5.%", "1.2.3%", "5",
    ] {
        assert_eq!(percent(word), None, "{word:?}");
    }
}

#[test]
fn resets_are_whole_units_in_descending_order() {
    for (word, expected) in [
        ("(45m)", 45),
        ("(1d)", 1440),
        ("(3h14m)", 194),
        ("(4d03h)", 5940),
        ("(1d02h03m)", 1563),
        ("(0m)", 0),
    ] {
        assert_eq!(minutes(word), Some(expected), "{word}");
    }
    for word in [
        "", "()", "(3)", "(h)", "(14m3h)", "(3h3h)", "(3x)", "3h14m", "(3h14m", "(12345m)",
        "(-3m)", "(3.5h)",
    ] {
        assert_eq!(minutes(word), None, "{word:?}");
    }
}

#[test]
fn anything_but_the_exact_footer_is_no_reading() {
    let cases = [
        ("no footer at all", "just some text\n> prompt"),
        ("no weekly window", "5h 97.0% (3h14m)"),
        ("no short window", "7d 56.0% (4d03h)"),
        ("missing reset", "5h 97.0%  7d 56.0% (4d03h)"),
        ("percent above 100", "5h 101.0% (3h14m)  7d 56.0% (4d03h)"),
        (
            "short reset beyond five hours",
            "5h 97.0% (5h01m)  7d 56.0% (4d03h)",
        ),
        (
            "weekly reset beyond seven days",
            "5h 97.0% (3h14m)  7d 56.0% (7d00h1m)",
        ),
        (
            "repeated window",
            "5h 97.0% (3h14m)  5h 90.0% (3h00m)  7d 56.0% (4d03h)",
        ),
        ("garbled reset", "5h 97.0% (3x14)  7d 56.0% (4d03h)"),
        (
            "count instead of a percent",
            "5h 97 (3h14m)  7d 56.0% (4d03h)",
        ),
    ];
    for (name, capture) in cases {
        assert_eq!(reading(capture), None, "{name}");
    }
}

#[test]
fn a_reset_exactly_at_the_window_length_is_accepted() {
    let reading = reading("5h 0.0% (5h00m)  7d 100% (7d00h)").unwrap();
    assert_eq!(reading.weekly.left, 100.0);
    assert_eq!(reading.short.unwrap().left, 0.0);
}
