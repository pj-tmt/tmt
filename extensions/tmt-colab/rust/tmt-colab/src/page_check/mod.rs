//! Offline check of page markup: which author-written `tmt-*` blocks the browser ignores, and
//! why, before anyone opens the page.
//!
//! The rules are the ones `readQuestions` applies in the browser (`inspectQuestions` in the app's
//! `questions.ts`), over the same tree: both run the shared vectors in
//! `contracts/vectors/page-markup-v1.json`, so they cannot drift. The code table lives in the
//! colab-v1 contract (page markup check). This module is pure: no files, no state, no clock.

mod image;
mod question;
mod tree;

pub(crate) use image::repair_references;

use crate::decoder::BASELINE_BYTES;

/// What a finding means for the block it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The browser drops the block today.
    Ignored,
    /// The block is admitted, but something is off.
    Warning,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Ignored => "ignored",
            Severity::Warning => "warning",
        }
    }
}

/// One thing wrong with one element, with a stable reason code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Lowercase tag name, and the 1-based index among elements with that tag in document order.
    pub element: String,
    pub ordinal: usize,
    /// The authored `data-id`, when there is one.
    pub id: Option<String>,
    /// The line of the end of the element's start tag.
    pub line: u64,
    pub severity: Severity,
    pub code: &'static str,
}

/// A question the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub choices: Vec<String>,
    pub other: bool,
    pub agent: String,
}

/// A `colab-attachment:` reference in the right form, at its first use on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// What follows the scheme: 8 lowercase hex digits or a full ID.
    pub token: String,
    ordinal: usize,
    line: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The questions the page shows.
    pub questions: Vec<Question>,
    /// How many `tmt-question` blocks the source holds, shown or not.
    pub question_blocks: usize,
    pub findings: Vec<Finding>,
    /// Each distinct well-formed reference once, in document order.
    pub references: Vec<Reference>,
}

impl Report {
    /// Findings in a stable order: by line, then element, ordinal and code.
    fn sort(&mut self) {
        self.findings.sort_by(|a, b| {
            (a.line, &a.element, a.ordinal, a.code).cmp(&(b.line, &b.element, b.ordinal, b.code))
        });
    }

    /// Report the references no current document attachment can show. `matches[i]` is how many
    /// of the page's current document attachments `references[i]` names: none is unresolved,
    /// more than one is ambiguous, because the renderer shows a reference only for exactly one.
    pub fn resolve_references(&mut self, matches: &[usize]) {
        for (reference, count) in self.references.iter().zip(matches) {
            let code = match count {
                0 => "attachment-ref-unresolved",
                1 => continue,
                _ => "attachment-ref-ambiguous",
            };
            self.findings.push(Finding {
                element: "img".to_owned(),
                ordinal: reference.ordinal,
                id: None,
                line: reference.line,
                severity: Severity::Warning,
                code,
            });
        }
        self.sort();
    }
}

/// The source is longer than a page may be.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceTooLarge;

/// Check one page source.
///
/// # Errors
/// [`SourceTooLarge`] when `source` is over the page source cap; the parse is never attempted.
pub fn check(source: &str) -> Result<Report, SourceTooLarge> {
    if source.len() > BASELINE_BYTES {
        return Err(SourceTooLarge);
    }
    let tree = match tree::Tree::parse(source) {
        Ok(tree) => tree,
        // Nothing past the cap can be judged as the browser would: one finding, no verdicts.
        Err(deep) => {
            return Ok(Report {
                findings: vec![Finding {
                    element: deep.name,
                    ordinal: deep.ordinal,
                    id: deep.id,
                    line: deep.line,
                    severity: Severity::Warning,
                    code: "page-too-deep",
                }],
                ..Report::default()
            });
        }
    };
    let elements = tree.elements();
    // The 1-based index of each element among those with its tag, in document order.
    let mut counts = std::collections::HashMap::new();
    let ordinals: Vec<(tree::Id, usize)> = elements
        .iter()
        .map(|id| {
            let count = counts.entry(tree.name(*id)).or_insert(0);
            *count += 1;
            (*id, *count)
        })
        .collect();
    let mut report = Report::default();
    question::check(&tree, &ordinals, &mut report);
    let images: Vec<(tree::Id, usize)> = ordinals
        .iter()
        .copied()
        .filter(|(id, _)| tree.name(*id) == "img")
        .collect();
    image::check(&tree, &images, &mut report);
    report.sort();
    Ok(report)
}

/// Every reason code the checker can report, for the vector coverage test.
#[must_use]
pub fn codes() -> Vec<&'static str> {
    let mut codes = vec![
        "page-too-deep",
        "question-page-limit",
        "question-marker-misplaced",
    ];
    codes.extend(question::RULES.iter().map(|(code, _)| *code));
    codes.extend([
        "image-data-oversized",
        "image-data-total",
        "attachment-ref-invalid",
        "attachment-ref-too-many",
        "attachment-ref-unresolved",
        "attachment-ref-ambiguous",
    ]);
    codes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_resolve_only_to_exactly_one_attachment() {
        let source = [
            r#"<img src="colab-attachment:0123abcd">"#,
            r#"<img src="colab-attachment:0123abcd">"#,
            r#"<img src="colab-attachment:89abcdef">"#,
            r#"<img src="colab-attachment:11111111">"#,
        ]
        .join("\n");
        let mut report = check(&source).unwrap();
        // Repeats of one reference are one reference, charged once.
        let tokens: Vec<&str> = report.references.iter().map(|r| r.token.as_str()).collect();
        assert_eq!(tokens, ["0123abcd", "89abcdef", "11111111"]);
        report.resolve_references(&[1, 0, 2]);
        let found: Vec<(usize, u64, &str)> = report
            .findings
            .iter()
            .map(|f| (f.ordinal, f.line, f.code))
            .collect();
        assert_eq!(
            found,
            [
                (3, 3, "attachment-ref-unresolved"),
                (4, 4, "attachment-ref-ambiguous")
            ]
        );
    }

    #[test]
    fn a_source_over_the_page_cap_is_refused_before_parsing() {
        let source = " ".repeat(BASELINE_BYTES + 1);
        assert_eq!(check(&source), Err(SourceTooLarge));
        assert!(check(&source[1..]).is_ok());
    }

    #[test]
    fn nesting_past_the_cap_stops_the_parse_with_one_finding() {
        // 400,000 nested divs fill the 2 MiB cap and cost minutes in an unguarded tree builder.
        let source = "<div>".repeat(400_000);
        assert!(source.len() <= BASELINE_BYTES);
        let started = std::time::Instant::now();
        let report = check(&source).unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        let found: Vec<(&str, usize, &str)> = report
            .findings
            .iter()
            .map(|f| (f.element.as_str(), f.ordinal, f.code))
            .collect();
        assert_eq!(
            found,
            [("div", crate::limits::PAGE_DEPTH + 1, "page-too-deep")]
        );
    }

    /// `<img src="{scheme}{filler}">` whose `src` is exactly `bytes` long. These sizes are built
    /// here, not in the shared vectors: a 32 KiB run of one character in the contract file is a
    /// poor fit for every scanner that reads the repository.
    fn image_of(scheme: &str, filler: char, bytes: usize) -> String {
        let payload = filler.to_string().repeat(bytes - scheme.len());
        format!(r#"<img src="{scheme}{payload}">"#)
    }

    fn found(report: &Report) -> Vec<(usize, u64, &'static str)> {
        report
            .findings
            .iter()
            .map(|f| (f.ordinal, f.line, f.code))
            .collect()
    }

    #[test]
    fn a_data_image_is_reported_only_past_its_own_src_cap() {
        let png = "data:image/png;base64,";
        let source = [
            image_of(png, 'A', crate::limits::DATA_IMAGE_BYTES),
            image_of(png, 'A', crate::limits::DATA_IMAGE_BYTES + 1),
            // The scheme and media type match case-insensitively.
            image_of(
                "DATA:IMAGE/PNG;base64,",
                'A',
                crate::limits::DATA_IMAGE_BYTES + 1,
            ),
            // A data: URL that is not an image is not this rule's business, however large.
            image_of("data:text/plain,", 'a', crate::limits::DATA_IMAGE_BYTES * 2),
            image_of(png, 'A', 100),
        ]
        .join("\n");
        let report = check(&source).unwrap();
        assert_eq!(
            found(&report),
            [
                (2, 2, "image-data-oversized"),
                (3, 3, "image-data-oversized")
            ]
        );
    }

    #[test]
    fn the_page_wide_data_image_total_is_reported_once_on_the_image_that_crosses_it() {
        // Eight images at their own cap fill the page total exactly: that is not past it.
        let full = crate::limits::DATA_IMAGE_BYTES;
        assert_eq!(8 * full, crate::limits::DATA_IMAGES_TOTAL_BYTES);
        let png = "data:image/png;base64,";
        let mut images = vec![image_of(png, 'A', full); 8];
        assert!(found(&check(&images.join("\n")).unwrap()).is_empty());
        // The ninth crosses it, and the tenth is not reported again.
        images.extend([image_of(png, 'A', 100), image_of(png, 'A', 100)]);
        let report = check(&images.join("\n")).unwrap();
        assert_eq!(found(&report), [(9, 9, "image-data-total")]);
    }
}
