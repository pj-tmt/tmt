//! Anchored page patches: evaluated against the exact source the page folds to, then written
//! through the ordinary whole-source preparation, which commits the minimal text diff. A patch
//! carries the text it expects (an exact find, the hash of a line range, or the old lines of a
//! unified diff), so it applies when that text is unchanged however the rest of the page moved and
//! fails, writing nothing, when it is missing, ambiguous or changed.
use super::lines::{line_number, line_range};
use tmt_colab_model::crypto;

/// What an anchored patch refuses, with the code and message the CLI prints.
#[derive(Debug, PartialEq, Eq)]
pub enum PatchFault {
    /// The patch text itself is malformed (`COLAB_INPUT_INVALID`).
    Invalid(String),
    /// The addressed text is missing, ambiguous or changed (`COLAB_PATCH_CONFLICT`).
    Conflict(String),
}
impl PatchFault {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "COLAB_INPUT_INVALID",
            Self::Conflict(_) => "COLAB_PATCH_CONFLICT",
        }
    }
}
impl std::fmt::Display for PatchFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(why) => write!(f, "{why} Nothing was written."),
            Self::Conflict(why) => write!(
                f,
                "{why} Read the page again and patch against what it holds now. Nothing was written."
            ),
        }
    }
}
impl std::error::Error for PatchFault {}
fn invalid(why: impl Into<String>) -> PatchFault {
    PatchFault::Invalid(why.into())
}
fn conflict(why: impl Into<String>) -> PatchFault {
    PatchFault::Conflict(why.into())
}

/// One anchored change to a page's source.
#[derive(Debug, PartialEq, Eq)]
pub enum Patch {
    /// `find` must occur exactly once, wherever it now is; `replace` takes its place.
    Replace { find: String, replace: String },
    /// Lines `start..=end` (1-based) become `text`, only while they still hash to `sha256`.
    Lines {
        start: usize,
        end: usize,
        sha256: [u8; 32],
        text: String,
    },
    /// Unified diff hunks, each anchored by its context and removed lines.
    Diff(Vec<Hunk>),
}
/// One unified-diff hunk: the exact text it replaces and the text it leaves.
#[derive(Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The first line the hunk names in the old source (1-based); a tie-break between occurrences.
    pub old_start: usize,
    pub old: String,
    pub new: String,
}

impl Patch {
    /// The source after the patch, or why it does not apply. Pure: nothing is read or written.
    pub fn apply(&self, source: &str) -> Result<String, PatchFault> {
        match self {
            Self::Replace { find, replace } => replace_once(source, find, replace),
            Self::Lines {
                start,
                end,
                sha256,
                text,
            } => replace_lines(source, *start, *end, sha256, text),
            Self::Diff(hunks) => apply_hunks(source, hunks),
        }
    }
}

/// Byte offsets of every occurrence of `needle`, overlapping ones included.
fn occurrences(source: &str, needle: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = source[from..].find(needle) {
        found.push(from + at);
        // Resume one character after the match start so an overlapping match is not missed.
        from += at + source[from + at..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

fn replace_once(source: &str, find: &str, replace: &str) -> Result<String, PatchFault> {
    if find.is_empty() {
        return Err(invalid("The text to find is empty."));
    }
    let found = occurrences(source, find);
    match found.as_slice() {
        [] => Err(conflict("The text to find is not on the page.")),
        [at] => Ok(format!(
            "{}{replace}{}",
            &source[..*at],
            &source[at + find.len()..]
        )),
        many => Err(conflict(format!(
            "The text to find is on the page {} times (first at line {}); add enough around it to match once.",
            many.len(),
            line_number(source, many[0])
        ))),
    }
}

fn replace_lines(
    source: &str,
    start: usize,
    end: usize,
    sha256: &[u8; 32],
    text: &str,
) -> Result<String, PatchFault> {
    if start == 0 || end < start {
        return Err(invalid(
            "A line range is START:END with 1 <= START <= END (1-based).",
        ));
    }
    let Some(range) = line_range(source, start, end) else {
        return Err(conflict(format!(
            "Lines {start}:{end} are past the end of the page."
        )));
    };
    if crypto::digest(source[range.clone()].as_bytes()) != *sha256 {
        return Err(conflict(format!(
            "Lines {start}:{end} changed since they were read."
        )));
    }
    Ok(format!(
        "{}{text}{}",
        &source[..range.start],
        &source[range.end..]
    ))
}

/// Where one hunk's old text sits in `source`: at the start of a line, once, or once at the line
/// the hunk states when it occurs more than once.
fn place_hunk(source: &str, hunk: &Hunk, number: usize) -> Result<usize, PatchFault> {
    let at_line_start = |at: &usize| *at == 0 || source.as_bytes()[*at - 1] == b'\n';
    let found: Vec<usize> = occurrences(source, &hunk.old)
        .into_iter()
        .filter(at_line_start)
        .collect();
    match found.as_slice() {
        [] => Err(conflict(format!(
            "Hunk {number} (old line {}) does not match the page: its context and removed lines are not there as written.",
            hunk.old_start
        ))),
        [at] => Ok(*at),
        many => {
            // Several occurrences: only the one at the stated line is the one meant.
            let at_stated: Vec<usize> = many
                .iter()
                .copied()
                .filter(|at| line_number(source, *at) == hunk.old_start)
                .collect();
            match at_stated.as_slice() {
                [at] => Ok(*at),
                _ => Err(conflict(format!(
                    "Hunk {number} (old line {}) matches the page {} times; add context lines until it matches once.",
                    hunk.old_start,
                    many.len()
                ))),
            }
        }
    }
}

/// The source after every hunk applies, or why they do not: all of them apply or none does.
pub fn apply_hunks(source: &str, hunks: &[Hunk]) -> Result<String, PatchFault> {
    if hunks.is_empty() {
        return Err(invalid("The diff has no hunks."));
    }
    // Every hunk is placed against the original source; the placements must not overlap.
    let mut placed: Vec<(usize, usize, &Hunk, usize)> = Vec::new();
    for (index, hunk) in hunks.iter().enumerate() {
        let number = index + 1;
        let chosen = place_hunk(source, hunk, number)?;
        placed.push((chosen, chosen + hunk.old.len(), hunk, number));
    }
    placed.sort_by_key(|(from, ..)| *from);
    if let Some(pair) = placed.windows(2).find(|pair| pair[0].1 > pair[1].0) {
        return Err(conflict(format!(
            "Hunks {} and {} overlap on the page.",
            pair[0].3, pair[1].3
        )));
    }
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;
    for (from, to, hunk, _) in placed {
        out.push_str(&source[cursor..from]);
        out.push_str(&hunk.new);
        cursor = to;
    }
    out.push_str(&source[cursor..]);
    Ok(out)
}

/// The hunks (0-based, in the order given) that cannot be placed on `source`: their old text is
/// missing or ambiguous, or the later of two that overlap. Empty when the whole diff applies.
pub fn conflicting_hunks(source: &str, hunks: &[Hunk]) -> Vec<usize> {
    let mut conflicting = Vec::new();
    let mut placed: Vec<(usize, usize, usize)> = Vec::new();
    for (index, hunk) in hunks.iter().enumerate() {
        match place_hunk(source, hunk, index + 1) {
            Ok(at) => placed.push((at, at + hunk.old.len(), index)),
            Err(_) => conflicting.push(index),
        }
    }
    placed.sort();
    conflicting.extend(
        placed
            .windows(2)
            .filter(|pair| pair[0].1 > pair[1].0)
            .map(|pair| pair[1].2),
    );
    conflicting.sort_unstable();
    conflicting.dedup();
    conflicting
}

/// Parses a unified diff. File headers and anything before the first hunk are ignored; each
/// hunk is read by the counts of its `@@` header, so a removed line that starts with `--` is not
/// mistaken for a header. Context is exact: there is no fuzz and no whitespace tolerance.
pub fn parse_diff(text: &str) -> Result<Vec<Hunk>, PatchFault> {
    let mut lines = text.split('\n').peekable();
    // A final LF ends the last line; it does not start an empty one.
    let mut rest: Vec<&str> = Vec::new();
    while let Some(line) = lines.next() {
        if line.is_empty() && lines.peek().is_none() {
            break;
        }
        rest.push(line);
    }
    let mut hunks = Vec::new();
    let mut at = 0;
    while at < rest.len() {
        let Some(header) = hunk_header(rest[at]) else {
            // Only a file header or a blank line may sit between hunks: stray text after a hunk is
            // a hunk that ended early or a diff for something else.
            let line = rest[at];
            let header_line = ["diff ", "index ", "--- ", "+++ "]
                .iter()
                .any(|prefix| line.starts_with(prefix));
            if !hunks.is_empty() && !line.is_empty() && !header_line {
                return Err(invalid(format!(
                    "Unexpected text after hunk {}: {line:?}.",
                    hunks.len()
                )));
            }
            at += 1;
            continue;
        };
        let number = hunks.len() + 1;
        at += 1;
        let (mut old_left, mut new_left) = (header.old_count, header.new_count);
        let (mut old, mut new) = (String::new(), String::new());
        // Whether the last line read on each side may still lose its LF (`\ No newline`).
        let mut last: Option<char> = None;
        while old_left > 0 || new_left > 0 {
            let Some(line) = rest.get(at) else {
                return Err(invalid(format!(
                    "Hunk {number} ends before its header says."
                )));
            };
            at += 1;
            let (kind, content) = match line.chars().next() {
                Some(kind @ (' ' | '-' | '+')) => (kind, &line[1..]),
                // An empty line stands for an empty context line some tools strip the space from.
                None => (' ', ""),
                Some('\\') => {
                    match last {
                        Some('-') => strip_lf(&mut old),
                        Some('+') => strip_lf(&mut new),
                        Some(' ') => {
                            strip_lf(&mut old);
                            strip_lf(&mut new);
                        }
                        _ => return Err(invalid(format!("Hunk {number} has a stray marker."))),
                    }
                    continue;
                }
                Some(_) => {
                    return Err(invalid(format!(
                        "Hunk {number} has a line that is not context, removed or added."
                    )));
                }
            };
            match kind {
                ' ' => {
                    if old_left == 0 || new_left == 0 {
                        return Err(invalid(format!(
                            "Hunk {number} has more lines than its header says."
                        )));
                    }
                    old_left -= 1;
                    new_left -= 1;
                    old.push_str(content);
                    old.push('\n');
                    new.push_str(content);
                    new.push('\n');
                }
                '-' => {
                    if old_left == 0 {
                        return Err(invalid(format!(
                            "Hunk {number} has more lines than its header says."
                        )));
                    }
                    old_left -= 1;
                    old.push_str(content);
                    old.push('\n');
                }
                _ => {
                    if new_left == 0 {
                        return Err(invalid(format!(
                            "Hunk {number} has more lines than its header says."
                        )));
                    }
                    new_left -= 1;
                    new.push_str(content);
                    new.push('\n');
                }
            }
            last = Some(kind);
        }
        // A marker may follow the hunk's final line.
        while let Some(line) = rest.get(at).filter(|line| line.starts_with('\\')) {
            let _ = line;
            match last {
                Some('-') => strip_lf(&mut old),
                Some('+') => strip_lf(&mut new),
                Some(' ') => {
                    strip_lf(&mut old);
                    strip_lf(&mut new);
                }
                _ => {}
            }
            at += 1;
        }
        if old.is_empty() {
            return Err(invalid(format!(
                "Hunk {number} has no context or removed line to anchor it; add context lines."
            )));
        }
        hunks.push(Hunk {
            old_start: header.old_start,
            old,
            new,
        });
    }
    if hunks.is_empty() {
        return Err(invalid("The diff has no hunks."));
    }
    Ok(hunks)
}
fn strip_lf(text: &mut String) {
    if text.ends_with('\n') {
        text.pop();
    }
}
struct HunkHeader {
    old_start: usize,
    old_count: usize,
    new_count: usize,
}
/// `@@ -start[,count] +start[,count] @@ [section]`; an omitted count is 1.
fn hunk_header(line: &str) -> Option<HunkHeader> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let range = |text: &str| -> Option<(usize, usize)> {
        match text.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((text.parse().ok()?, 1)),
        }
    };
    let (old_start, old_count) = range(old)?;
    let (_, new_count) = range(new)?;
    Some(HunkHeader {
        old_start,
        old_count,
        new_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(text: &str) -> [u8; 32] {
        crypto::digest(text.as_bytes())
    }

    #[test]
    fn find_replace_needs_exactly_one_match_even_an_overlapping_one() {
        let patch = |find: &str, replace: &str| Patch::Replace {
            find: find.into(),
            replace: replace.into(),
        };
        assert_eq!(
            patch("beta", "BETA").apply("alpha beta gamma").unwrap(),
            "alpha BETA gamma"
        );
        assert_eq!(patch("beta", "").apply("a beta b").unwrap(), "a  b");
        let missing = patch("delta", "x").apply("alpha").unwrap_err();
        assert_eq!(missing.code(), "COLAB_PATCH_CONFLICT");
        let twice = patch("a", "x").apply("a\nb\na").unwrap_err();
        assert!(
            twice.to_string().contains("2 times (first at line 1)"),
            "{twice}"
        );
        // `aa` occurs twice in `aaa`, overlapping: still ambiguous.
        assert_eq!(
            patch("aa", "x").apply("aaa").unwrap_err().code(),
            "COLAB_PATCH_CONFLICT"
        );
        assert_eq!(
            patch("", "x").apply("a").unwrap_err().code(),
            "COLAB_INPUT_INVALID"
        );
        // Multi-byte text is matched and replaced on character boundaries.
        assert_eq!(
            patch("é", "e").apply("café é").unwrap_err().code(),
            "COLAB_PATCH_CONFLICT"
        );
        assert_eq!(patch("🐈", "cat").apply("a 🐈 b").unwrap(), "a cat b");
    }

    #[test]
    fn a_line_range_applies_only_while_its_hash_holds() {
        let source = "one\ntwo\nthree\nfour\n";
        let patch = |sha256, text: &str| Patch::Lines {
            start: 2,
            end: 3,
            sha256,
            text: text.into(),
        };
        let held = hash("two\nthree\n");
        assert_eq!(
            patch(held, "2\n3\n").apply(source).unwrap(),
            "one\n2\n3\nfour\n"
        );
        // A concurrent edit above moves the lines but not their text: the range is anchored by hash
        // at the stated numbers, so the caller re-reads and re-addresses; here the numbers now name
        // other lines and the patch refuses.
        let shifted = "zero\none\ntwo\nthree\nfour\n";
        assert_eq!(
            patch(held, "x\n").apply(shifted).unwrap_err().code(),
            "COLAB_PATCH_CONFLICT"
        );
        let past = Patch::Lines {
            start: 4,
            end: 9,
            sha256: held,
            text: String::new(),
        };
        assert!(
            past.apply(source)
                .unwrap_err()
                .to_string()
                .contains("past the end")
        );
        let backwards = Patch::Lines {
            start: 3,
            end: 2,
            sha256: held,
            text: String::new(),
        };
        assert_eq!(
            backwards.apply(source).unwrap_err().code(),
            "COLAB_INPUT_INVALID"
        );
        // Deleting the range is an empty replacement; the last line may lack its LF.
        let last = Patch::Lines {
            start: 2,
            end: 2,
            sha256: hash("b"),
            text: "B".into(),
        };
        assert_eq!(last.apply("a\nb").unwrap(), "a\nB");
    }

    #[test]
    fn the_conflicting_hunks_are_the_ones_that_cannot_be_placed_or_overlap_an_earlier_one() {
        let hunk = |old_start: usize, old: &str, new: &str| Hunk {
            old_start,
            old: old.into(),
            new: new.into(),
        };
        let source = "a\nb\nc\nd\ne\n";
        let fits = [hunk(1, "a\nb\n", "A\nb\n"), hunk(4, "d\ne\n", "d\nE\n")];
        assert_eq!(conflicting_hunks(source, &fits), Vec::<usize>::new());
        assert!(apply_hunks(source, &fits).is_ok());
        // The second hunk's text is gone, the third overlaps the first (it is the later of the pair),
        // and indices come back in the order the hunks were given.
        let broken = [
            hunk(1, "a\nb\n", "A\nb\n"),
            hunk(3, "x\ny\n", "x\n"),
            hunk(2, "b\nc\n", "b\n"),
        ];
        assert_eq!(conflicting_hunks(source, &broken), vec![1, 2]);
        assert_eq!(
            apply_hunks(source, &broken).unwrap_err().code(),
            "COLAB_PATCH_CONFLICT"
        );
        // Text that occurs twice is placed by its stated line, else it conflicts.
        let twice = "x\ny\nx\ny\n";
        assert_eq!(
            conflicting_hunks(twice, &[hunk(3, "x\ny\n", "z\n")]),
            Vec::<usize>::new()
        );
        assert_eq!(
            conflicting_hunks(twice, &[hunk(2, "x\ny\n", "z\n")]),
            vec![0]
        );
    }

    const DIFF: &str = "--- a/page.html\n+++ b/page.html\n@@ -2,3 +2,3 @@ section\n beta\n-gamma\n+GAMMA\n delta\n";

    #[test]
    fn a_unified_diff_replaces_only_what_its_context_anchors() {
        let hunks = parse_diff(DIFF).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old, "beta\ngamma\ndelta\n");
        assert_eq!(hunks[0].new, "beta\nGAMMA\ndelta\n");
        let source = "alpha\nbeta\ngamma\ndelta\nepsilon\n";
        let patch = Patch::Diff(hunks);
        assert_eq!(
            patch.apply(source).unwrap(),
            "alpha\nbeta\nGAMMA\ndelta\nepsilon\n"
        );
        // Lines were inserted above meanwhile: the context still finds its place.
        assert_eq!(
            patch
                .apply("new\nalpha\nbeta\ngamma\ndelta\nepsilon\n")
                .unwrap(),
            "new\nalpha\nbeta\nGAMMA\ndelta\nepsilon\n"
        );
        // The removed line changed: no fuzz, nothing applies.
        let changed = patch.apply("alpha\nbeta\nGamma\ndelta\n").unwrap_err();
        assert_eq!(changed.code(), "COLAB_PATCH_CONFLICT");
        assert!(
            changed.to_string().contains("Hunk 1 (old line 2)"),
            "{changed}"
        );
    }

    #[test]
    fn several_occurrences_resolve_only_by_the_stated_line() {
        let source = "x\ndup\ny\nz\ndup\ny\n";
        let diff = |start: usize| {
            parse_diff(&format!("@@ -{start},2 +{start},2 @@\n-dup\n+DUP\n y\n")).unwrap()
        };
        assert_eq!(
            Patch::Diff(diff(5)).apply(source).unwrap(),
            "x\ndup\ny\nz\nDUP\ny\n"
        );
        assert_eq!(
            Patch::Diff(diff(2)).apply(source).unwrap(),
            "x\nDUP\ny\nz\ndup\ny\n"
        );
        let wrong = Patch::Diff(diff(3)).apply(source).unwrap_err();
        assert!(
            wrong.to_string().contains("matches the page 2 times"),
            "{wrong}"
        );
    }

    #[test]
    fn a_multi_hunk_diff_applies_all_or_none_and_refuses_overlap() {
        let source = "a\nb\nc\nd\ne\nf\ng\n";
        let ok = parse_diff("@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -6,2 +6,2 @@\n f\n-g\n+G\n").unwrap();
        assert_eq!(
            Patch::Diff(ok).apply(source).unwrap(),
            "A\nb\nc\nd\ne\nf\nG\n"
        );
        // The second hunk no longer matches: the first must not apply either.
        let bad = parse_diff("@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -6,2 +6,2 @@\n f\n-x\n+X\n").unwrap();
        let error = Patch::Diff(bad).apply(source).unwrap_err();
        assert!(error.to_string().contains("Hunk 2"), "{error}");
        let overlap =
            parse_diff("@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -2,2 +2,2 @@\n-b\n+B\n c\n").unwrap();
        let error = Patch::Diff(overlap).apply(source).unwrap_err();
        assert!(
            error.to_string().contains("Hunks 1 and 2 overlap"),
            "{error}"
        );
    }

    #[test]
    fn diff_parsing_follows_the_header_counts_and_refuses_what_it_cannot_anchor() {
        // A removed line that looks like a file header is content, not a header.
        let hunks =
            parse_diff("@@ -1,2 +1,2 @@\n--- not a header\n+++ not either\n ctx\n").unwrap();
        assert_eq!(hunks[0].old, "-- not a header\nctx\n");
        assert_eq!(hunks[0].new, "++ not either\nctx\n");
        // An omitted count is 1; the section heading after the header is ignored.
        let single = parse_diff("@@ -4 +4 @@ fn x()\n-old\n+new\n").unwrap();
        assert_eq!((single[0].old_start, single[0].old.as_str()), (4, "old\n"));
        // No newline at the end of the file: the marker removes the LF from that side only.
        let eof = parse_diff("@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n").unwrap();
        assert_eq!((eof[0].old.as_str(), eof[0].new.as_str()), ("old", "new\n"));
        assert_eq!(Patch::Diff(eof).apply("old").unwrap(), "new\n");
        // CR is content, so a CRLF source matches a diff written with CRs.
        let crlf = parse_diff("@@ -1 +1 @@\n-a\r\n+b\r\n").unwrap();
        assert_eq!(Patch::Diff(crlf).apply("a\r\nc\r\n").unwrap(), "b\r\nc\r\n");
        for (name, text) in [
            ("no hunk", "--- a\n+++ b\n"),
            ("short hunk", "@@ -1,3 +1,3 @@\n a\n"),
            ("long hunk", "@@ -1 +1 @@\n a\n b\n"),
            ("unknown line", "@@ -1 +1 @@\n?a\n"),
            ("insert only", "@@ -3,0 +4 @@\n+x\n"),
        ] {
            assert_eq!(
                parse_diff(text).unwrap_err().code(),
                "COLAB_INPUT_INVALID",
                "{name}"
            );
        }
    }
}
