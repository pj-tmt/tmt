//! Compact chart labels; exact integer totals remain in bucket Details.
pub(super) fn tokens(value: u128) -> String {
    let (scale, suffix) = if value >= 1_000_000_000 {
        (1_000_000_000, "B")
    } else if value >= 1_000_000 {
        (1_000_000, "M")
    } else if value >= 1_000 {
        (1_000, "k")
    } else {
        return value.to_string();
    };
    // Round half up using integers: source precision never depends on f64.
    let whole = value / scale;
    let tenths = ((value % scale) * 10 + scale / 2) / scale;
    let rounded = whole * 10 + tenths;
    format!("{}.{:01}{suffix}", rounded / 10, rounded % 10)
}

/// Compact the observed total in the existing owner's summary; exact Details stay unchanged.
pub(super) fn range_label(observed: &crate::board::rate::series::Series, minutes: u8) -> String {
    let label = super::project::range_label(observed, minutes);
    let total = observed
        .buckets
        .iter()
        .filter_map(|bucket| bucket.tokens)
        .try_fold(0u128, u128::checked_add);
    total.map_or(label.clone(), |total| {
        label.replacen(
            &format!("observed {total} tok"),
            &format!("observed {} tok", tokens(total)),
            1,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
