//! The page source as numbered lines: the one definition the ranged read (`page read --lines`) and
//! the anchored patch (`page patch --lines`) share (contract: Ranged read and anchored patch).
use tmt_colab_model::crypto;

/// Lines are split on LF only; a CR stays content, and a final LF ends the last line instead of
/// starting an empty one. An empty source is one empty line. The byte range of lines `start..=end`
/// runs from the first byte of `start` through the LF that ends `end` (or to the end of the source
/// when `end` has none).
pub fn line_range(source: &str, start: usize, end: usize) -> Option<std::ops::Range<usize>> {
    if start == 0 || end < start {
        return None;
    }
    if source.is_empty() {
        return (start == 1 && end == 1).then_some(0..0);
    }
    let mut from = None;
    let mut line = 1;
    let mut offset = 0;
    while offset < source.len() {
        if line == start {
            from = Some(offset);
        }
        let next = source[offset..]
            .find('\n')
            .map_or(source.len(), |at| offset + at + 1);
        if line == end {
            return from.map(|from| from..next);
        }
        offset = next;
        line += 1;
    }
    None
}
/// SHA-256 of the exact bytes `line_range` names.
pub fn range_sha256(source: &str, start: usize, end: usize) -> Option<[u8; 32]> {
    let range = line_range(source, start, end)?;
    Some(crypto::digest(source[range].as_bytes()))
}

/// How many lines the source has: an empty source is one empty line, and a final LF ends the last
/// line instead of starting another.
pub fn line_count(source: &str) -> usize {
    if source.is_empty() {
        return 1;
    }
    source.bytes().filter(|b| *b == b'\n').count() + usize::from(!source.ends_with('\n'))
}

/// The 1-based line holding the byte `offset`. Any byte offset names a line, inside a multi-byte
/// character or at the source's end included (an LF is one byte and never part of a longer
/// character); an offset past the end counts as the end.
pub fn line_number(source: &str, offset: usize) -> usize {
    let bytes = source.as_bytes();
    bytes[..offset.min(bytes.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(text: &str) -> [u8; 32] {
        crypto::digest(text.as_bytes())
    }

    #[test]
    fn lines_split_on_lf_only_and_a_final_lf_adds_no_line() {
        let source = "one\r\ntwo\nthree\n";
        assert_eq!(line_range(source, 1, 1), Some(0..5));
        assert_eq!(&source[line_range(source, 2, 3).unwrap()], "two\nthree\n");
        assert_eq!(line_range(source, 4, 4), None);
        assert_eq!(line_range(source, 3, 2), None);
        assert_eq!(line_range(source, 0, 1), None);
        // The last line without an LF ends the range at the end of the source.
        assert_eq!(&"a\nb"[line_range("a\nb", 2, 2).unwrap()], "b");
        // An empty source is one empty line: 1:1 is its (empty) bytes, nothing past it exists.
        assert_eq!(line_range("", 1, 1), Some(0..0));
        assert_eq!(line_range("", 1, 2), None);
        assert_eq!(line_range("", 2, 2), None);
        assert_eq!(range_sha256("", 1, 1), Some(hash("")));
        // A source that is only an LF is one line; its trailing LF starts no second one.
        assert_eq!(line_range("\n", 1, 1), Some(0..1));
        assert_eq!(line_range("\n", 2, 2), None);
        // Each hash is of the exact bytes, including the LF that ends the range.
        assert_eq!(range_sha256(source, 2, 2), Some(hash("two\n")));
    }

    #[test]
    fn line_count_and_number_follow_the_same_records() {
        for (source, count) in [
            ("", 1),
            ("\n", 1),
            ("a", 1),
            ("a\n", 1),
            ("a\nb", 2),
            ("a\nb\n", 2),
            ("a\n\n", 2),
            ("a\r\nb\n", 2),
        ] {
            assert_eq!(line_count(source), count, "{source:?}");
            // Every line in 1..=count has a range, and none past it does.
            for line in 1..=count {
                assert!(
                    line_range(source, line, line).is_some(),
                    "{source:?} line {line}"
                );
            }
            assert_eq!(line_range(source, count + 1, count + 1), None, "{source:?}");
            assert_eq!(
                line_range(source, 1, count).map(|r| r.end),
                Some(source.len())
            );
        }
        assert_eq!(line_number("a\nb\nc", 0), 1);
        assert_eq!(line_number("a\nb\nc", 2), 2);
        assert_eq!(line_number("a\nb\nc", 5), 3);
    }

    #[test]
    fn a_line_number_is_defined_for_every_byte_offset_even_inside_a_character() {
        // "é" is two bytes, "🐈" four; the LF after each ends line 1 and 2.
        let source = "é🐈\n🐈é\nx";
        let first = source.find('\n').unwrap();
        for offset in 0..=first {
            assert_eq!(line_number(source, offset), 1, "offset {offset}");
        }
        assert_eq!(line_number(source, first + 1), 2);
        let second = source.rfind('\n').unwrap();
        for offset in first + 1..=second {
            assert_eq!(line_number(source, offset), 2, "offset {offset}");
        }
        assert_eq!(line_number(source, second + 1), 3);
        assert_eq!(line_number(source, source.len()), 3);
        // The end of a match that ends a line is that line's offset one past its last byte.
        let end = source.find("🐈é").unwrap() + "🐈é".len();
        assert_eq!(line_number(source, end), 2);
        // Past the end counts as the end, and an empty source is line 1.
        assert_eq!(line_number(source, usize::MAX), 3);
        assert_eq!(line_number("", 0), 1);
    }
}
