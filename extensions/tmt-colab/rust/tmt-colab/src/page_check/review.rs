//! The `tmt-review` rules, mirroring `REVIEW_RULES` in the app's `questions.ts`.
//!
//! A section is admitted exactly when no rule fails. An ignored section only loses its own
//! controls: the questions inside it stay ordinary questions.

use std::collections::HashMap;

use super::{
    Report, Review, Severity,
    group::Survey,
    question::{has_control, is_uuid, make_finding, over, question_text},
    tree::{Id, Tree},
};
use crate::limits::{QUESTION_AGENT_POINTS, QUESTION_TITLE_POINTS};

/// What the rules read about one section.
pub(super) struct Candidate {
    id: String,
    agent: String,
    headings: usize,
    title: String,
    nested: bool,
    holders: usize,
}

pub(super) type Rule = (&'static str, fn(&Candidate) -> bool);

/// The ordered rule table. The title rules wait for exactly one heading, so one defect is not
/// reported twice.
pub(super) const RULES: [Rule; 9] = [
    ("review-id-invalid", |c| !is_uuid(&c.id)),
    ("review-id-duplicate", |c| is_uuid(&c.id) && c.holders > 1),
    ("review-nested", |c| c.nested),
    ("review-heading-missing", |c| c.headings == 0),
    ("review-heading-multiple", |c| c.headings > 1),
    ("review-title-empty", |c| {
        c.headings == 1 && c.title.is_empty()
    }),
    ("review-title-too-long", |c| {
        c.headings == 1 && over(&c.title, QUESTION_TITLE_POINTS)
    }),
    ("review-title-control", |c| {
        c.headings == 1 && has_control(&c.title)
    }),
    ("review-agent-invalid", |c| {
        over(&c.agent, QUESTION_AGENT_POINTS) || has_control(&c.agent)
    }),
];

/// Judge every section in `reviews` (already empty when the page is past the section cap).
/// `admitted` are the admitted questions, in document order, with their IDs.
pub(super) fn check(
    tree: &Tree,
    ordinals: &HashMap<Id, usize>,
    survey: &Survey,
    reviews: &[Id],
    admitted: &[(Id, String)],
    report: &mut Report,
) {
    for review in reviews {
        let headings: Vec<Id> = tree
            .child_elements(*review)
            .filter(|child| tree.attr(*child, "data-title").is_some())
            .collect();
        let id_value = tree.attr(*review, "data-id").unwrap_or("");
        let candidate = Candidate {
            id: id_value.to_owned(),
            agent: tree.attr(*review, "data-agent").unwrap_or("").to_owned(),
            headings: headings.len(),
            title: headings
                .first()
                .map_or_else(String::new, |h| question_text(&tree.text(*h))),
            nested: tree
                .closest(
                    *review,
                    &["tmt-review", "tmt-question", "tmt-questions", "tmt-read"],
                )
                .is_some(),
            holders: survey.holders(id_value),
        };
        let failed: Vec<&'static str> = RULES
            .iter()
            .filter(|(_, fails)| fails(&candidate))
            .map(|(code, _)| *code)
            .collect();
        report.findings.extend(
            failed
                .iter()
                .map(|code| make_finding(tree, ordinals, *review, Severity::Ignored, code, None)),
        );
        if !failed.is_empty() {
            continue;
        }
        report.reviews.push(Review {
            id: candidate.id,
            title: candidate.title,
            agent: candidate.agent,
            questions: admitted
                .iter()
                .filter(|(question, _)| tree.ancestors(*question).any(|a| a == *review))
                .map(|(_, id)| id.clone())
                .collect(),
        });
    }
}
