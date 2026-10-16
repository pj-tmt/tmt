//! One neutral spark glyph per exact bucket; integer values stay in Details.
pub fn glyph(tokens: Option<u64>, scale: u64) -> &'static str {
    let Some(tokens) = tokens else { return "·" };
    if tokens == 0 {
        return " ";
    };
    let level = (u128::from(tokens) * 8)
        .div_ceil(u128::from(scale.max(1)))
        .clamp(1, 8) as usize;
    [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"][level]
}
pub fn scale(values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.flatten().max()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_eight_levels_zero_and_unknown_remain_distinct() {
        assert_eq!(glyph(None, 8), "·");
        assert_eq!(glyph(Some(0), 8), " ");
        for (value, expected) in (1..=8).zip(["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"])
        {
            assert_eq!(glyph(Some(value), 8), expected);
        }
        assert_eq!(glyph(Some(1), u64::MAX), "▁");
        assert_eq!(glyph(Some(u64::MAX), u64::MAX), "█");
        assert_eq!(scale([None].into_iter()), None);
        assert_eq!(scale([None, Some(0)].into_iter()), Some(0));
    }
}
