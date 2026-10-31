//! Offline check of page markup: which author-written `tmt-*` blocks the browser ignores, and
//! why, before anyone opens the page.
//!
//! The rules are the ones `readQuestions` applies in the browser (`inspectQuestions` in the app's
//! `questions.ts`), over the same tree: both run the shared vectors in
//! `contracts/vectors/page-markup-v1.json`, so they cannot drift. The code table lives in the
//! colab-v1 contract (page markup check). Markup validation is pure: it reads no files or state
//! and uses no clock. Owner-summary quote projection is separately bounded by a deadline.

mod anchors;
mod group;
mod hints;
mod image;
pub(crate) use anchors::SummaryAnchors;
mod question;
mod reading;
mod review;
mod tree;
pub(crate) use tree::start_tags;
mod ui;

pub use hints::{Hint, uses_design};
pub(crate) use image::repair_references;

/// The existing markup UUID grammar, also used by editor-owned registration.
pub fn valid_question_id(value: &str) -> bool {
    question::is_uuid(value)
}

use crate::decoder::BASELINE_BYTES;

/// What a finding means for the block it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Publication is refused regardless of `--strict`; rendering alone does not admit it.
    Blocking,
    /// The browser drops the block today.
    Ignored,
    /// The block is admitted, but something is off.
    Warning,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Blocking => "blocking",
            Severity::Ignored => "ignored",
            Severity::Warning => "warning",
        }
    }
}

/// One thing wrong with one element, with a stable reason code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// A command repair when the finding depends on the current page's registered metadata.
    pub fix: Option<String>,
    /// Lowercase tag name, and the 1-based index among elements with that tag in document order.
    pub element: String,
    pub ordinal: usize,
    /// The authored `data-id`, when there is one.
    pub id: Option<String>,
    /// The line of the end of the element's start tag.
    pub line: u64,
    pub severity: Severity,
    pub code: &'static str,
    /// For a `tmt-questions` group the browser skips: every question inside it, which the skip
    /// silences along with its findings.
    pub questions_skipped: Option<usize>,
}

/// Plain choice metadata; drafts and recorded answers still contain only the label.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct QuestionChoice {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A question the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub choices: Vec<QuestionChoice>,
    pub other: bool,
    pub agent: String,
    /// The group it belongs to; `None` for a lone question.
    pub group: Option<String>,
    /// The group report label (authored `data-label`, else `Q<n>` by admitted order); `None` for
    /// a lone question.
    pub label: Option<String>,
}

/// Every authored question block, including blocks ignored by markup admission. Registry checks
/// reuse the same parsed tree and never lose a mismatch just because another shape rule failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionBlock {
    pub id: String,
    pub agent: String,
    pub line: u64,
    pub ordinal: usize,
}

/// What the page shows as one set of controls: a `tmt-questions` group, or a lone question as an
/// internal group (its own ID, no title).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionGroup {
    pub id: String,
    pub title: Option<String>,
    /// Question IDs, in document order.
    pub questions: Vec<String>,
    /// The author opted into the compact inline form (`data-form="chip"`); otherwise the block.
    pub chip: bool,
    /// Standalone questions share a logical group across separate placements.
    pub split: bool,
}

/// A review section the page shows: its heading is the thread anchor, and `questions` are the
/// admitted questions inside it, in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub questions: Vec<String>,
}

/// An inert reading passage; its ordinary thread records own the read state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub why: String,
}

/// A `colab-attachment:` reference in the right form, at its first use on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// What follows the scheme: 8 lowercase hex digits or a full ID.
    pub token: String,
    ordinal: usize,
    line: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub question_sources: Vec<QuestionBlock>,
    /// The questions the page shows.
    pub questions: Vec<Question>,
    /// The groups the page shows, in document order, lone questions as internal groups.
    pub groups: Vec<QuestionGroup>,
    /// The review sections the page shows, in document order.
    pub reviews: Vec<Review>,
    /// Reading passages, in document order.
    pub readings: Vec<Reading>,
    /// Authored reading blocks, admitted or not.
    pub reading_blocks: usize,
    /// How many `tmt-question` blocks the source holds, shown or not.
    pub question_blocks: usize,
    /// How many `tmt-review` blocks the source holds, shown or not.
    pub review_blocks: usize,
    pub findings: Vec<Finding>,
    /// Advice that blocks nothing (see `hints`).
    pub hints: Vec<Hint>,
    /// Each distinct well-formed reference once, in document order.
    pub references: Vec<Reference>,
}

impl Report {
    /// Page-backed comparison is explicit: the ordinary offline checker reads no registry.
    pub fn check_registry(
        &mut self,
        meta: &serde_json::Value,
        page: &str,
        identities: Option<&crate::question_registry::Identities>,
    ) {
        let findings = crate::question_registry::findings(self, meta, page, identities);
        self.findings.extend(findings);
        self.sort();
    }
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
                fix: None,
                element: "img".to_owned(),
                ordinal: reference.ordinal,
                id: None,
                line: reference.line,
                severity: Severity::Warning,
                code,
                questions_skipped: None,
            });
        }
        self.sort();
    }
}

/// The source is longer than a page may be.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceTooLarge;

/// One source and its [`check`], kept together so a later step of the same write reuses the
/// report only for that exact source.
pub struct Checked<'a> {
    source: &'a str,
    report: Report,
}
impl<'a> Checked<'a> {
    /// # Errors
    /// [`SourceTooLarge`], as [`check`].
    pub fn new(source: &'a str) -> Result<Self, SourceTooLarge> {
        Ok(Self {
            source,
            report: check(source)?,
        })
    }
    pub fn source(&self) -> &'a str {
        self.source
    }
    pub fn report(&self) -> &Report {
        &self.report
    }
    /// The report, when it is the check of `source`.
    pub fn of(&self, source: &str) -> Option<&Report> {
        (self.source == source).then_some(&self.report)
    }
}

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
                    fix: None,
                    element: deep.name,
                    ordinal: deep.ordinal,
                    id: deep.id,
                    line: deep.line,
                    severity: Severity::Warning,
                    code: "page-too-deep",
                    questions_skipped: None,
                }],
                ..Report::default()
            });
        }
    };
    let mut report = inspect(&tree);
    report.hints = hints::of(source, &tree);
    Ok(report)
}

/// A source nested past the depth where a browser's parse and ours stop agreeing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooDeep;

/// An HTML widget requests the bundled runtime without a script or marker. Otherwise this is
/// the first runtime meta's content, parsed the way a browser finds it; comments, escaped text
/// and foreign-namespace elements do not request registration. A depth refusal remains closed.
pub(crate) fn runtime_requirement(source: &str) -> Result<Option<String>, TooDeep> {
    let tree = tree::Tree::parse(source).map_err(|_| TooDeep)?;
    if tree
        .elements()
        .iter()
        .any(|&id| tree.html_element(id) && ui::element_name(tree.name(id)))
    {
        return Ok(Some("react".to_owned()));
    }
    Ok(tree
        .elements()
        .into_iter()
        .find(|&id| tree.name(id) == "meta" && tree.attr(id, "name") == Some("tmt-colab-runtime"))
        .map(|id| tree.attr(id, "content").unwrap_or_default().to_owned()))
}

/// One static source parse for the owner summary's markup and normal quote associations.
pub(crate) fn summary(
    source: &str,
    threads: &[crate::export::conversations::Thread],
) -> Result<(Report, SummaryAnchors), &'static str> {
    if source.len() > BASELINE_BYTES {
        return Err("summary source budget");
    }
    let tree = tree::Tree::parse(source).map_err(|_| "summary source depth")?;
    let report = inspect(&tree);
    let anchors = anchors::project(&tree, &report, threads)?;
    Ok((report, anchors))
}

fn inspect(tree: &tree::Tree) -> Report {
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
    let mut report = Report {
        question_sources: ordinals
            .iter()
            .filter(|(id, _)| tree.name(*id) == "tmt-question")
            .map(|(id, ordinal)| QuestionBlock {
                id: tree.attr(*id, "data-id").unwrap_or_default().to_owned(),
                agent: tree.attr(*id, "data-agent").unwrap_or_default().to_owned(),
                line: tree.line(*id),
                ordinal: *ordinal,
            })
            .collect(),
        ..Report::default()
    };
    question::check(tree, &ordinals, &mut report);
    let images: Vec<(tree::Id, usize)> = ordinals
        .iter()
        .copied()
        .filter(|(id, _)| tree.name(*id) == "img")
        .collect();
    image::check(tree, &images, &mut report);
    ui::check(tree, &ordinals, &mut report);
    report.sort();
    report
}

/// Every reason code the checker can report, for the vector coverage test.
#[must_use]
pub fn codes() -> Vec<&'static str> {
    let mut codes = vec![
        "page-too-deep",
        "question-page-limit",
        "question-marker-misplaced",
        "question-choice-description-invalid",
    ];
    codes.extend(question::RULES.iter().map(|(code, _)| *code));
    codes.extend(group::RULES.iter().map(|(code, _)| *code));
    codes.extend(group::LABEL_CODES);
    codes.extend(group::FORM_CODES);
    codes.push("reading-page-limit");
    codes.extend(reading::RULES.iter().map(|(code, _)| *code));
    codes.push("review-page-limit");
    codes.extend(review::RULES.iter().map(|(code, _)| *code));
    // Not a rule of the group itself: a group that passed and shows nothing.
    codes.push("group-empty");
    codes.extend([
        "image-data-oversized",
        "image-data-total",
        "attachment-ref-invalid",
        "attachment-ref-too-many",
        "attachment-ref-unresolved",
        "attachment-ref-ambiguous",
        "unknown-ui-class",
        "unknown-icon",
        "unknown-ui-element",
        "unknown-ui-attribute",
        "invalid-ui-attribute",
        "invalid-ui-structure",
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
