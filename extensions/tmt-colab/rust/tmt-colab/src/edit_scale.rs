//! The regions of a page's source an edit replaces (#5024), and how much of the source that is
//! (#5029). The isolated decoder child builds one delta per region from [`content_hunks`], and the
//! editing hint and the large-patch warning measure the same regions, so they read the number the
//! write really spends.
use std::collections::HashMap;

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

/// The most changed lines the line diff looks for before it gives up and replaces the whole
/// differing span, as one region. A few hundred scattered edits fit; a rewrite does not.
const MAX_CHANGED_LINES: usize = 400;

/// The most diagonal steps the line diff may take, so a large page cannot cost more than a
/// bounded time in the decoder child and the CLI.
const MAX_DIFF_STEPS: usize = 8_000_000;

/// One replaced region: bytes `start..start + remove` of the old source become `insert`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk<'a> {
    pub start: usize,
    pub remove: usize,
    pub insert: &'a str,
}

/// The regions that turn `old` into `new`, in order and apart from each other, each trimmed to the
/// characters that differ. Text between two regions is untouched, so far-apart edits do not
/// rewrite it. Offsets are UTF-8 bytes on character boundaries. An edit with more than
/// `MAX_CHANGED_LINES` changed lines is one region over the differing span.
pub fn content_hunks<'a>(old: &str, new: &'a str) -> Vec<Hunk<'a>> {
    if old == new {
        return Vec::new();
    }
    let whole = || {
        let (start, end) = content_diff(old, new);
        vec![Hunk {
            start,
            remove: old.len() - start - end,
            insert: &new[start..new.len() - end],
        }]
    };
    let (old_lines, new_lines) = (lines(old), lines(new));
    let mut ids = HashMap::new();
    let mut id = |line: &str| {
        let next = ids.len();
        *ids.entry(line.to_owned()).or_insert(next)
    };
    let a: Vec<usize> = old_lines.iter().map(|l| id(&old[l.clone()])).collect();
    let b: Vec<usize> = new_lines.iter().map(|l| id(&new[l.clone()])).collect();
    let shared_start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let shared_end = a[shared_start..]
        .iter()
        .rev()
        .zip(b[shared_start..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (
        &a[shared_start..a.len() - shared_end],
        &b[shared_start..b.len() - shared_end],
    );
    let Some(regions) = changed_lines(a, b) else {
        return whole();
    };
    let at = |lines: &[std::ops::Range<usize>], index: usize, end: usize| {
        lines.get(index).map_or(end, |line| line.start)
    };
    regions
        .into_iter()
        .map(|(a_from, a_to, b_from, b_to)| {
            let old_from = at(&old_lines, shared_start + a_from, old.len());
            let old_to = at(&old_lines, shared_start + a_to, old.len());
            let new_from = at(&new_lines, shared_start + b_from, new.len());
            let new_to = at(&new_lines, shared_start + b_to, new.len());
            let (old_part, new_part) = (&old[old_from..old_to], &new[new_from..new_to]);
            // Within the changed lines, keep what both versions share at either end.
            let (head, tail) = content_diff(old_part, new_part);
            Hunk {
                start: old_from + head,
                remove: old_part.len() - head - tail,
                insert: &new_part[head..new_part.len() - tail],
            }
        })
        .collect()
}

/// The byte range of each line of `text`, its LF included.
fn lines(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut start = 0;
    text.split_inclusive('\n')
        .map(|line| {
            let range = start..start + line.len();
            start = range.end;
            range
        })
        .collect()
}

/// The shortest edit script from `a` to `b` (Myers), as `(a_from, a_to, b_from, b_to)` runs of
/// changed lines; `None` once it needs more than `MAX_CHANGED_LINES` changed lines or
/// `MAX_DIFF_STEPS` steps.
fn changed_lines(a: &[usize], b: &[usize]) -> Option<Vec<(usize, usize, usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    if n == 0 || m == 0 {
        return Some(vec![(0, a.len(), 0, b.len())]);
    }
    let limit = ((n + m) as usize).min(MAX_CHANGED_LINES);
    let offset = limit as isize + 1;
    let mut v = vec![0isize; 2 * limit + 3];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut steps = 0usize;
    let mut found = None;
    'search: for d in 0..=limit as isize {
        trace.push(v.clone());
        for k in (-d..=d).step_by(2) {
            let index = |k: isize| (offset + k) as usize;
            let mut x = if k == -d || (k != d && v[index(k - 1)] < v[index(k + 1)]) {
                v[index(k + 1)]
            } else {
                v[index(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
                steps += 1;
            }
            steps += 1;
            if steps > MAX_DIFF_STEPS {
                return None;
            }
            v[index(k)] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'search;
            }
        }
    }
    let d_end = found?;
    // Walk back from the end, noting each change; runs of changes between matches are the regions.
    let (mut x, mut y) = (n, m);
    let mut moves = Vec::new();
    for (d, v) in trace.iter().enumerate().take(d_end as usize + 1).rev() {
        let d = d as isize;
        let index = |k: isize| (offset + k) as usize;
        let k = x - y;
        let previous = if k == -d || (k != d && v[index(k - 1)] < v[index(k + 1)]) {
            k + 1
        } else {
            k - 1
        };
        let (previous_x, previous_y) = (v[index(previous)], v[index(previous)] - previous);
        while x > previous_x && y > previous_y {
            x -= 1;
            y -= 1;
            moves.push(None);
        }
        if d > 0 {
            moves.push(Some((previous_x, previous_y, x, y)));
        }
        (x, y) = (previous_x, previous_y);
    }
    moves.reverse();
    let mut regions: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut after_match = true;
    for step in moves {
        match step {
            None => after_match = true,
            Some((from_x, from_y, to_x, to_y)) => {
                let (from_x, from_y, to_x, to_y) = (
                    from_x as usize,
                    from_y as usize,
                    to_x as usize,
                    to_y as usize,
                );
                match regions.last_mut() {
                    Some(last) if !after_match => {
                        last.1 = to_x;
                        last.3 = to_y;
                    }
                    _ => regions.push((from_x, to_x, from_y, to_y)),
                }
                after_match = false;
            }
        }
    }
    Some(regions)
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
        Self {
            base_bytes: old.len(),
            replaced_bytes: content_hunks(old, source).iter().map(|h| h.remove).sum(),
        }
    }

    /// An edit that replaces more than half of the existing source, regardless of page size.
    pub fn is_large_rewrite(self) -> bool {
        self.replaced_bytes > self.base_bytes / 2
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
    fn the_line_is_more_than_half_of_the_source_at_any_size() {
        let old = page(4096);
        // Exactly half is not more than half; one byte over is.
        let half = format!("{}{}", "b".repeat(2048), page(2048));
        assert_eq!(EditScale::of(&old, &half).replaced_bytes, 2048);
        assert!(!EditScale::of(&old, &half).is_large_rewrite());
        let over = format!("{}{}", "b".repeat(2049), page(2047));
        assert!(EditScale::of(&old, &over).is_large_rewrite());
        for bytes in [1, 17, 2047, 2048] {
            assert!(EditScale::of(&page(bytes), "different").is_large_rewrite());
        }
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

    /// `old` with each hunk applied: what a reader of the published regions ends up with.
    fn applied(old: &str, hunks: &[Hunk<'_>]) -> String {
        let mut out = String::new();
        let mut at = 0;
        for hunk in hunks {
            assert!(hunk.start >= at, "regions overlap or are out of order");
            out.push_str(&old[at..hunk.start]);
            out.push_str(hunk.insert);
            at = hunk.start + hunk.remove;
        }
        out.push_str(&old[at..]);
        out
    }

    fn mixed(lines: usize) -> String {
        (0..lines)
            .map(|n| format!("<p>第{n}行 🐈 row {n} 漢字 ✓</p>\n"))
            .collect()
    }

    #[test]
    fn far_apart_edits_are_separate_regions_that_leave_the_text_between_them() {
        let old = mixed(300);
        let new = old
            .replacen("row 5 ", "ROW 🐕 ", 1)
            .replacen("<p>第150行 🐈 row 150 漢字 ✓</p>\n", "", 1)
            .replacen("第290行", "第二九〇行 🚀", 1);
        let hunks = content_hunks(&old, &new);
        assert_eq!(hunks.len(), 3, "{hunks:?}");
        assert_eq!(applied(&old, &hunks), new);
        for hunk in &hunks {
            assert!(old.is_char_boundary(hunk.start));
            assert!(old.is_char_boundary(hunk.start + hunk.remove));
        }
        // Only the differing characters are replaced, not the lines around them.
        let replaced: usize = hunks.iter().map(|h| h.remove).sum();
        assert!(replaced < 100, "{replaced}");
        assert_eq!(EditScale::of(&old, &new).replaced_bytes, replaced);
        assert!(!EditScale::of(&old, &new).is_large_rewrite());
    }

    #[test]
    fn insertions_deletions_and_the_two_ends_are_regions_too() {
        let old = mixed(50);
        for new in [
            format!("<h1>新 🚀</h1>\n{old}"),
            format!("{old}<footer>完 🎉</footer>\n"),
            old.replacen("<p>第10行", "<p>🆕</p>\n<p>第10行", 1),
            old.replacen("<p>第20行 🐈 row 20 漢字 ✓</p>\n", "", 1),
            old.replacen('\n', "\r\n", 3),
            String::new(),
            "x".to_owned(),
        ] {
            let hunks = content_hunks(&old, &new);
            assert_eq!(applied(&old, &hunks), new, "{new:?}");
        }
        assert!(content_hunks(&old, &old).is_empty());
        let hunks = content_hunks("", "<p>new</p>\n");
        assert_eq!(
            hunks,
            [Hunk {
                start: 0,
                remove: 0,
                insert: "<p>new</p>\n"
            }]
        );
    }

    #[test]
    fn a_rewrite_or_a_page_without_lines_is_one_region_over_the_differing_span() {
        // No LF: a single line, so only the characters that differ are replaced.
        let old = format!("{}{}{}", "a".repeat(500), "🐈漢", "z".repeat(500));
        let new = format!("{}{}{}", "a".repeat(500), "🐕字", "z".repeat(500));
        let hunks = content_hunks(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].start, hunks[0].remove), (500, 7));
        assert_eq!(applied(&old, &hunks), new);
        // Every line differs: far more changed lines than the diff follows.
        let old: String = (0..2 * MAX_CHANGED_LINES)
            .map(|n| format!("old {n}\n"))
            .collect();
        let new: String = (0..2 * MAX_CHANGED_LINES)
            .map(|n| format!("new {n}\n"))
            .collect();
        let hunks = content_hunks(&old, &new);
        assert_eq!(hunks.len(), 1);
        assert_eq!(applied(&old, &hunks), new);
        assert!(EditScale::of(&old, &new).replaced_bytes * 2 > old.len());
    }

    #[test]
    fn regions_rebuild_the_new_source_for_many_pseudo_random_edits() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let alphabet = ["a", "b", "é", "漢", "🐈", "\n", "\n", " "];
        for _ in 0..300 {
            let old: String = (0..next(80))
                .map(|_| alphabet[next(alphabet.len())])
                .collect();
            let mut new = old.clone();
            for _ in 0..next(6) {
                let chars: Vec<(usize, char)> = new.char_indices().collect();
                let at = chars.get(next(chars.len().max(1))).map_or(0, |c| c.0);
                match next(3) {
                    0 => new.insert_str(at, alphabet[next(alphabet.len())]),
                    1 if at < new.len() => {
                        new.remove(at);
                    }
                    _ => new.replace_range(
                        at..at + new[at..].chars().next().map_or(0, char::len_utf8),
                        alphabet[next(alphabet.len())],
                    ),
                }
            }
            let hunks = content_hunks(&old, &new);
            assert_eq!(applied(&old, &hunks), new, "{old:?} -> {new:?}");
        }
    }

    #[test]
    fn an_empty_page_and_an_identical_page_replace_nothing() {
        assert_eq!(EditScale::of("", "<p>new</p>"), EditScale::NONE);
        let old = page(4096);
        assert_eq!(EditScale::of(&old, &old).replaced_bytes, 0);
        assert!(!EditScale::NONE.is_large_rewrite());
    }
}
