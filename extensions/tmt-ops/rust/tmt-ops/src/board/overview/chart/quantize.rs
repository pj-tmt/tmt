//! Integer paint quantization; exact token totals stay in the series model.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Empty,
    LowerHalf,
    Full,
    Unknown,
}

/// Bottom-up cells on one shared scale. Tiny positive readings receive a half
/// cell, zero is empty, unavailable is marked at the baseline, never zero-filled.
pub fn cells(tokens: Option<u64>, scale: u64, height: u8) -> Vec<Cell> {
    let mut cells = vec![Cell::Empty; usize::from(height)];
    if height == 0 {
        return cells;
    }
    let Some(tokens) = tokens else {
        cells[0] = Cell::Unknown;
        return cells;
    };
    if tokens == 0 {
        return cells;
    }
    let half_cells = (u128::from(tokens) * u128::from(height) * 2)
        .div_ceil(u128::from(scale.max(1)))
        .clamp(1, u128::from(height) * 2) as usize;
    for (index, cell) in cells.iter_mut().enumerate() {
        *cell = match half_cells.saturating_sub(index * 2) {
            0 => Cell::Empty,
            1 => Cell::LowerHalf,
            _ => Cell::Full,
        };
    }
    cells
}

pub fn scale(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.flatten().max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_tiny_half_peak_and_unknown_are_distinct() {
        assert_eq!(cells(Some(0), 100, 3), vec![Cell::Empty; 3]);
        assert_eq!(
            cells(Some(1), 100, 3),
            vec![Cell::LowerHalf, Cell::Empty, Cell::Empty]
        );
        assert_eq!(
            cells(Some(50), 100, 3),
            vec![Cell::Full, Cell::LowerHalf, Cell::Empty]
        );
        assert_eq!(cells(Some(100), 100, 3), vec![Cell::Full; 3]);
        assert_eq!(
            cells(None, 100, 3),
            vec![Cell::Unknown, Cell::Empty, Cell::Empty]
        );
    }

    #[test]
    fn extremes_do_not_overflow_or_create_unbounded_cells() {
        assert_eq!(scale([None].into_iter()), None);
        assert_eq!(scale([None, Some(0)].into_iter()), Some(0));
        assert_eq!(scale([Some(8), Some(100)].into_iter()), Some(100));
        assert_eq!(cells(Some(u64::MAX), u64::MAX, 24), vec![Cell::Full; 24]);
        assert!(cells(None, 1, 0).is_empty());
    }
}
