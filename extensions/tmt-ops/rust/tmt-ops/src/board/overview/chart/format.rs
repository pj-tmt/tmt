//! Compact chart labels; exact integer totals remain in bucket Details.
pub(super) use crate::source::tokens;

pub(super) fn interval(minutes: u32) -> String {
    if minutes.is_multiple_of(1_440) {
        format!("{}d", minutes / 1_440)
    } else if minutes.is_multiple_of(60) {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

pub(super) fn range(minutes: u32) -> String {
    if minutes == 1_440 {
        "1d".into()
    } else {
        interval(minutes)
    }
}

/// Axis units follow the range, keeping the existing short-range minute ticks.
pub(super) fn axis(minutes: u32) -> [String; 3] {
    let (unit, suffix) = if minutes <= 60 {
        (1, "m")
    } else if minutes <= 1_440 {
        (60, "h")
    } else {
        (1_440, "d")
    };
    let label = |divisor| {
        let whole = minutes / divisor;
        let half = if minutes.is_multiple_of(divisor) {
            ""
        } else {
            ".5"
        };
        format!("−{whole}{half}{suffix}")
    };
    // A day reads 1d, as the range caption does; its middle tick stays in hours.
    let start = if minutes == 1_440 {
        "−1d".to_owned()
    } else {
        label(unit)
    };
    [start, label(2 * unit), "now".into()]
}

/// Percent of the complete observed denominator, without overflowing a large
/// exact source total while formatting a compact scalar label.
pub(super) fn share(amount: u128, total: u128) -> String {
    if total == 0 {
        return "0%".into();
    }
    let percent = amount
        .checked_mul(100)
        .map(|scaled| scaled / total)
        .unwrap_or_else(|| {
            let mut remainder = 0u128;
            let mut percent = 0;
            for _ in 0..100 {
                if remainder >= total - amount {
                    remainder -= total - amount;
                    percent += 1;
                } else {
                    remainder += amount;
                }
            }
            percent
        });
    if amount > 0 && percent == 0 {
        "<1%".into()
    } else {
        format!("{percent}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_and_axis_labels_keep_hours_days_and_exact_half_ticks() {
        for (minutes, caption, left, middle) in [
            (60, "1h", "−60m", "−30m"),
            (360, "6h", "−6h", "−3h"),
            (1_440, "1d", "−1d", "−12h"),
            (10_080, "7d", "−7d", "−3.5d"),
            (43_200, "30d", "−30d", "−15d"),
        ] {
            assert_eq!(range(minutes), caption);
            assert_eq!(axis(minutes), [left, middle, "now"].map(str::to_owned));
        }
        assert_eq!(axis(15), ["−15m", "−7.5m", "now"].map(str::to_owned));
        assert_eq!(interval(1_440), "1d");
        assert_eq!(interval(2_880), "2d");
    }
    #[test]
    fn shares_preserve_the_full_large_denominator_without_overflow() {
        assert_eq!(share(u128::MAX, u128::MAX), "100%");
        assert_eq!(share(u128::MAX / 2, u128::MAX), "49%");
        assert_eq!(share(1, u128::MAX), "<1%");
        assert_eq!(share(0, 0), "0%");
    }
    #[test]
    fn compact_units_preserve_measured_zero_and_the_requested_real_examples() {
        for (value, expected) in [
            (0, "0"),
            (999, "999"),
            (1_000, "1.0k"),
            (23_058_051, "23.1M"),
            (176_815_646, "177M"),
            (1_250_000_000, "1.3B"),
        ] {
            assert_eq!(tokens(value), expected);
        }
    }
}
