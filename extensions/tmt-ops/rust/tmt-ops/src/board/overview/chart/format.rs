//! Compact chart labels; exact integer totals remain in bucket Details.
pub(super) use crate::source::compact_tokens as tokens;

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
            (176_815_646, "176.8M"),
            (1_250_000_000, "1.3B"),
        ] {
            assert_eq!(tokens(value), expected);
        }
    }
}
