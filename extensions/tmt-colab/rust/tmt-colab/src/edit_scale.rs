//! How much of a page's source an edit replaces (#5029). The measure is taken on the span the
//! write publishes (`content_diff`, which the isolated decoder child uses to build its updates),
//! so the editing hint and the `--replace` gate read the number the write really spends.

/// A source smaller than this is rewritten without comment: replacing a short page whole is the
/// normal way to edit it.
pub const LARGE_BASE_BYTES: usize = 2 * 1024;

/// The bytes both sources share at their start and at their end, as `(start, end)`: what an edit
/// from `old` to `source` leaves untouched. Everything between is replaced.
pub fn content_diff(old: &str, source: &str) -> (usize, usize) {
    let start = old
        .chars()
        .zip(source.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let end = old[start..]
        .chars()
        .rev()
        .zip(source[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    (start, end)
}

/// How much of the existing source one edit replaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditScale {
    /// UTF-8 bytes of the source before the edit.
    pub base_bytes: usize,
    /// Bytes of that source the edit removes or overwrites.
    pub replaced_bytes: usize,
}

impl EditScale {
    pub const NONE: Self = Self {
        base_bytes: 0,
        replaced_bytes: 0,
    };

    pub fn of(old: &str, source: &str) -> Self {
        let (start, end) = content_diff(old, source);
        Self {
            base_bytes: old.len(),
            replaced_bytes: old.len() - start - end,
        }
    }

    /// An edit that replaces more than half of a source of at least [`LARGE_BASE_BYTES`].
    pub fn is_large_rewrite(self) -> bool {
        self.base_bytes >= LARGE_BASE_BYTES && self.replaced_bytes * 2 > self.base_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(bytes: usize) -> String {
        "a".repeat(bytes)
    }

    #[test]
    fn an_append_or_a_one_line_change_replaces_little_of_the_page() {
        let old = page(4096);
        let appended = format!("{old}{}", page(8192));
        assert_eq!(EditScale::of(&old, &appended).replaced_bytes, 0);
        let edited = format!("{}b{}", page(2000), page(2095));
        let scale = EditScale::of(&old, &edited);
        assert_eq!(scale.replaced_bytes, 1);
        assert!(!scale.is_large_rewrite());
    }

    #[test]
    fn the_line_is_more_than_half_of_a_source_of_at_least_two_kib() {
        let old = page(4096);
        // Exactly half is not more than half; one byte over is.
        let half = format!("{}{}", "b".repeat(2048), page(2048));
        assert_eq!(EditScale::of(&old, &half).replaced_bytes, 2048);
        assert!(!EditScale::of(&old, &half).is_large_rewrite());
        let over = format!("{}{}", "b".repeat(2049), page(2047));
        assert!(EditScale::of(&old, &over).is_large_rewrite());
        // A full replacement of a page just under 2 KiB is exempt; at 2 KiB it counts.
        let small = page(LARGE_BASE_BYTES - 1);
        assert!(!EditScale::of(&small, &"b".repeat(100)).is_large_rewrite());
        let edge = page(LARGE_BASE_BYTES);
        assert!(EditScale::of(&edge, &"b".repeat(100)).is_large_rewrite());
    }

    #[test]
    fn the_span_counts_utf8_bytes_not_characters() {
        // Emoji and CJK are 4 and 3 bytes; the replaced span is whole characters.
        let old = format!("{}🐈漢字{}", page(10), page(10));
        let edited = format!("{}🐕漢字{}", page(10), page(10));
        let (start, end) = content_diff(&old, &edited);
        assert_eq!((start, end), (10, 6 + 10));
        assert_eq!(EditScale::of(&old, &edited).replaced_bytes, 4);
        assert!(old.is_char_boundary(start) && old.is_char_boundary(old.len() - end));
    }

    #[test]
    fn an_empty_page_and_an_identical_page_replace_nothing() {
        assert_eq!(EditScale::of("", "<p>new</p>"), EditScale::NONE);
        let old = page(4096);
        assert_eq!(EditScale::of(&old, &old).replaced_bytes, 0);
        assert!(!EditScale::NONE.is_large_rewrite());
    }
}
