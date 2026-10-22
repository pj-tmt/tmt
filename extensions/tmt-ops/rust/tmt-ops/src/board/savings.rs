//! Unit-aware savings captions and the number/money formatters shared by the
//! Agents, Conversation, Overview and Digest surfaces. The data they format is
//! `labels::digest::SavingsSummary`; the Digest tab keeps only its own Details.

/// Formatted amount, source coverage and optional combination failure.
/// Captions, key/value rows and matrices share the same unit evidence.
pub(in crate::board) fn savings_value(
    summary: &crate::labels::digest::SavingsSummary,
    window: crate::config::DigestWindow,
    unit: crate::config::SavingsUnit,
) -> (Option<String>, String, Option<&'static str>) {
    use crate::config::SavingsUnit;
    let coverage = |included: usize| {
        if included < summary.total {
            format!(" ({included} of {})", summary.total)
        } else {
            String::new()
        }
    };
    // (value, coverage, note about values that could not be combined)
    match unit.shown(window) {
        SavingsUnit::Tokens => (
            summary
                .tokens
                .map(|tokens| crate::source::tokens(u128::from(tokens))),
            coverage(summary.included),
            None,
        ),
        SavingsUnit::Percent => {
            let percent = &summary.percent;
            match (percent.value, percent.conflict) {
                (Some(points), _) => (
                    Some(format!("{} of input", percent_text(points))),
                    coverage(percent.included),
                    None,
                ),
                (None, true) => (None, String::new(), Some("intervals differ")),
                (None, false) => (None, coverage(percent.included), None),
            }
        }
        SavingsUnit::Cost => {
            let cost = &summary.cost;
            match (cost.value, cost.conflict) {
                (Some(micros), _) => (
                    Some(format!("{} at list price", usd_text(micros))),
                    coverage(cost.included),
                    None,
                ),
                (None, true) => (None, String::new(), Some("prices differ")),
                (None, false) => (None, coverage(cost.included), None),
            }
        }
    }
}

/// Shared secondary caption for member, squad, Overview and Digest usage
/// surfaces. One label for every unit; only the value changes. Estimate and
/// coverage stay visible even when the longer context label cannot fit; only
/// a combination note ("intervals differ") yields to width, before the segment.
pub(in crate::board) fn savings_caption(
    summary: &crate::labels::digest::SavingsSummary,
    window: crate::config::DigestWindow,
    unit: crate::config::SavingsUnit,
    width: usize,
) -> String {
    use unicode_width::UnicodeWidthStr;
    let (value, coverage, note) = savings_value(summary, window, unit);
    let value = value.unwrap_or_else(|| "—".into());
    let build = |label: &str, note: Option<&str>| {
        format!(
            "{label} {}: {value}{coverage}{}",
            window.label(),
            note.map_or_else(String::new, |note| format!(" ({note})"))
        )
    };
    let full = build("Est. ctx avoided", note);
    if full.width() <= width {
        return full;
    }
    let short = build("Est. avoided", note);
    if short.width() <= width {
        short
    } else {
        build("Est. avoided", None)
    }
}

/// Compact focus variant of the supplied savings caption; omitted unless a
/// value exists and the complete amount, window and coverage fit together.
pub(in crate::board) fn compact_savings_caption(
    summary: &crate::labels::digest::SavingsSummary,
    window: crate::config::DigestWindow,
    unit: crate::config::SavingsUnit,
    width: usize,
) -> Option<String> {
    use unicode_width::UnicodeWidthStr;
    let (value, coverage, note) = savings_value(summary, window, unit);
    let value = value?;
    let value = value
        .strip_suffix(" at list price")
        .or_else(|| value.strip_suffix(" of input"))
        .unwrap_or(&value);
    let value = value.strip_prefix("≈ ").unwrap_or(value);
    let text = format!(
        "≈{}: {value}{coverage}{}",
        window.label(),
        note.map_or_else(String::new, |note| format!(" ({note})"))
    );
    (text.width() <= width).then_some(text)
}

/// Whole percent, half up; a nonzero share below one percent never reads as zero.
pub(in crate::board) fn percent_text(basis_points: u16) -> String {
    match basis_points {
        0 => "0%".into(),
        1..=49 => "<1%".into(),
        points => format!("{}%", (u32::from(points) + 50) / 100),
    }
}

/// A list-price estimate to the cent, half up. Never worded as money saved.
pub(in crate::board) fn usd_text(micros: u64) -> String {
    let cents = (u128::from(micros) + 5_000) / 10_000;
    if micros == 0 {
        return "$0.00".into();
    }
    if cents == 0 {
        return "< $0.01".into();
    }
    let dollars = (cents / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in dollars.chars().enumerate() {
        if index > 0 && (dollars.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("≈ ${grouped}.{:02}", cents % 100)
}

/// Dates describe the included members' own collection starts, never a common
/// reconstructed window. The caller supplies its local zone; tests pin one.
pub(in crate::board) fn savings_start(
    summary: &crate::labels::digest::SavingsSummary,
    window: crate::config::DigestWindow,
    zone: &jiff::tz::TimeZone,
    width: usize,
) -> Option<String> {
    if window != crate::config::DigestWindow::AllTime {
        return None;
    }
    let date = |ms| {
        jiff::Timestamp::from_millisecond(i64::try_from(ms).ok()?)
            .ok()
            .map(|time| time.to_zoned(zone.clone()).strftime("%m-%d").to_string())
    };
    let first = date(summary.earliest_from_ms?)?;
    let last = date(summary.latest_from_ms?)?;
    Some(if first == last {
        format!("since Digest began recording on {first}")
    } else {
        let prefix = "since Digest began recording ·";
        let range = format!("members started {first} to {last}");
        let line = format!("{prefix} {range}");
        // Keep the collection range together on narrow usage surfaces.
        if unicode_width::UnicodeWidthStr::width(line.as_str()) > width {
            format!("{prefix}\n{range}")
        } else {
            line
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn savings_captions_keep_estimate_window_and_partial_coverage() {
        use crate::{
            config::{DigestWindow, SavingsUnit},
            labels::digest::SavingsSummary,
        };
        let mut value = SavingsSummary {
            tokens: Some(18_200),
            included: 3,
            total: 5,
            ..Default::default()
        };
        assert_eq!(
            super::savings_caption(&value, DigestWindow::Day, SavingsUnit::Tokens, 100),
            "Est. ctx avoided 1d: 18.2k (3 of 5)"
        );
        assert_eq!(
            super::savings_caption(&value, DigestWindow::Day, SavingsUnit::Tokens, 30),
            "Est. avoided 1d: 18.2k (3 of 5)"
        );
        value.tokens = Some(1_400_000);
        assert_eq!(
            super::savings_caption(&value, DigestWindow::AllTime, SavingsUnit::Tokens, 100),
            "Est. ctx avoided all time: 1.4M (3 of 5)"
        );
        value.tokens = Some(0);
        value.included = 5;
        assert_eq!(
            super::savings_caption(&value, DigestWindow::Week, SavingsUnit::Tokens, 100),
            "Est. ctx avoided 7d: 0"
        );
        value.tokens = None;
        value.included = 0;
        assert_eq!(
            super::savings_caption(&value, DigestWindow::Day, SavingsUnit::Tokens, 100),
            "Est. ctx avoided 1d: — (0 of 5)"
        );
    }

    #[test]
    fn every_unit_keeps_one_label_and_changes_only_the_value() {
        use crate::{
            config::{DigestWindow, SavingsUnit},
            labels::digest::{SavingsSummary, Valuation},
        };
        let mut value = SavingsSummary {
            tokens: Some(18_200),
            included: 3,
            total: 3,
            percent: Valuation {
                value: Some(1_204),
                included: 3,
                conflict: false,
            },
            cost: Valuation {
                value: Some(12_398_600),
                included: 2,
                conflict: false,
            },
            ..Default::default()
        };
        let caption = |value: &SavingsSummary, window, unit, width| {
            super::savings_caption(value, window, unit, width)
        };
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Tokens, 100),
            "Est. ctx avoided 1d: 18.2k"
        );
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Percent, 100),
            "Est. ctx avoided 1d: 12% of input"
        );
        // Each unit has its own coverage.
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Cost, 100),
            "Est. ctx avoided 1d: ≈ $12.40 at list price (2 of 3)"
        );
        // Lifetime has no percent: the saved choice reads as tokens.
        assert_eq!(
            caption(&value, DigestWindow::AllTime, SavingsUnit::Percent, 100),
            "Est. ctx avoided all time: 18.2k"
        );
        // The qualifier that stops cost reading as money saved never narrows away.
        assert!(
            caption(&value, DigestWindow::Day, SavingsUnit::Cost, 20)
                .ends_with("at list price (2 of 3)")
        );
        for (points, shown) in [
            (0, "0%"),
            (1, "<1%"),
            (49, "<1%"),
            (50, "1%"),
            (1_667, "17%"),
            (10_000, "100%"),
        ] {
            value.percent.value = Some(points);
            assert_eq!(
                caption(&value, DigestWindow::Week, SavingsUnit::Percent, 100),
                format!("Est. ctx avoided 7d: {shown} of input")
            );
        }
        for (micros, shown) in [
            (0, "$0.00"),
            (4_999, "< $0.01"),
            (5_000, "≈ $0.01"),
            (12_395_000, "≈ $12.40"),
            (1_234_567_890, "≈ $1,234.57"),
            (9_007_199_254_740_991, "≈ $9,007,199,254.74"),
        ] {
            value.cost.value = Some(micros);
            value.cost.included = 3;
            assert_eq!(
                caption(&value, DigestWindow::Day, SavingsUnit::Cost, 100),
                format!("Est. ctx avoided 1d: {shown} at list price")
            );
        }
        // Values that could not be combined are unavailable, named, and the note
        // yields to width before the segment does.
        value.percent = Valuation {
            value: None,
            included: 2,
            conflict: true,
        };
        value.cost = Valuation {
            value: None,
            included: 2,
            conflict: true,
        };
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Percent, 100),
            "Est. ctx avoided 1d: — (intervals differ)"
        );
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Percent, 37),
            "Est. avoided 1d: — (intervals differ)"
        );
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Percent, 36),
            "Est. avoided 1d: —"
        );
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Cost, 100),
            "Est. ctx avoided 1d: — (prices differ)"
        );
        // Nobody has the valuation: unavailable with its own coverage, not zero.
        value.percent = Valuation::default();
        assert_eq!(
            caption(&value, DigestWindow::Day, SavingsUnit::Percent, 100),
            "Est. ctx avoided 1d: — (0 of 3)"
        );
    }

    #[test]
    fn lifetime_detail_uses_individual_source_starts_in_local_dates() {
        use crate::{config::DigestWindow, labels::digest::SavingsSummary};
        let ms = |value: &str| value.parse::<jiff::Timestamp>().unwrap().as_millisecond() as u64;
        let zone = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
        let mut value = SavingsSummary {
            tokens: Some(1),
            included: 1,
            total: 1,
            earliest_from_ms: Some(ms("2026-10-01T16:00:00Z")),
            latest_from_ms: Some(ms("2026-10-08T16:00:00Z")),
            ..Default::default()
        };
        assert_eq!(
            super::savings_start(&value, DigestWindow::AllTime, &zone, 160).as_deref(),
            Some("since Digest began recording · members started 10-02 to 10-09")
        );
        value.latest_from_ms = value.earliest_from_ms;
        assert_eq!(
            super::savings_start(&value, DigestWindow::AllTime, &zone, 160).as_deref(),
            Some("since Digest began recording on 10-02")
        );
        assert_eq!(
            super::savings_start(&value, DigestWindow::Day, &zone, 160),
            None
        );
        value.earliest_from_ms = None;
        assert_eq!(
            super::savings_start(&value, DigestWindow::AllTime, &zone, 160),
            None
        );
    }
}
