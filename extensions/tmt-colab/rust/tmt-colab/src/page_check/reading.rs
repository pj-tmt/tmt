//! The `tmt-read` rules, mirroring `READING_RULES` in the app's `questions.ts`.
//!
//! A passage supplies controls only when its whole metadata and nesting shape is valid.

use std::collections::HashMap;

use super::{
    Reading, Report, Severity,
    group::Survey,
    question::{has_control, is_uuid, make_finding, over, question_text},
    tree::{Id, Tree},
};
use crate::limits::{QUESTION_AGENT_POINTS, QUESTION_TITLE_POINTS};

/// What the rules read about one passage.
pub(super) struct Candidate {
    id: String,
    agent: String,
    why: String,
    headings: usize,
    title: String,
    nested: bool,
    holders: usize,
}

pub(super) type Rule = (&'static str, fn(&Candidate) -> bool);

/// The ordered rule table. The title rules wait for exactly one heading, so one defect is not
/// reported twice.
pub(super) const RULES: [Rule; 10] = [
    ("reading-id-invalid", |c| !is_uuid(&c.id)),
    ("reading-id-duplicate", |c| is_uuid(&c.id) && c.holders > 1),
    ("reading-nested", |c| c.nested),
    ("reading-heading-missing", |c| c.headings == 0),
    ("reading-heading-multiple", |c| c.headings > 1),
    ("reading-title-empty", |c| {
        c.headings == 1 && c.title.is_empty()
    }),
    ("reading-title-too-long", |c| {
        c.headings == 1 && over(&c.title, QUESTION_TITLE_POINTS)
    }),
    ("reading-title-control", |c| {
        c.headings == 1 && has_control(&c.title)
    }),
    ("reading-agent-invalid", |c| {
        question_text(&c.agent).is_empty()
            || over(&c.agent, QUESTION_AGENT_POINTS)
            || has_control(&c.agent)
    }),
    ("reading-why-invalid", |c| {
        question_text(&c.why).is_empty()
            || over(&c.why, QUESTION_TITLE_POINTS)
            || has_control(&c.why)
    }),
];

/// Judge every passage in `readings` (already empty when the page is past the passage cap).
pub(super) fn check(
    tree: &Tree,
    ordinals: &HashMap<Id, usize>,
    survey: &Survey,
    readings: &[Id],
    report: &mut Report,
) {
    for reading in readings {
        let headings: Vec<Id> = tree
            .child_elements(*reading)
            .filter(|child| tree.attr(*child, "data-title").is_some())
            .collect();
        let id_value = tree.attr(*reading, "data-id").unwrap_or("");
        let candidate = Candidate {
            id: id_value.to_owned(),
            agent: tree.attr(*reading, "data-agent").unwrap_or("").to_owned(),
            why: tree.attr(*reading, "data-why").unwrap_or("").to_owned(),
            headings: headings.len(),
            title: headings
                .first()
                .map_or_else(String::new, |h| question_text(&tree.text(*h))),
            nested: tree
                .closest(
                    *reading,
                    &["tmt-read", "tmt-review", "tmt-question", "tmt-questions"],
                )
                .is_some()
                || tree.elements().iter().any(|child| {
                    matches!(
                        tree.name(*child),
                        "tmt-read" | "tmt-review" | "tmt-question" | "tmt-questions"
                    ) && tree.ancestors(*child).any(|ancestor| ancestor == *reading)
                }),
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
                .map(|code| make_finding(tree, ordinals, *reading, Severity::Ignored, code, None)),
        );
        if !failed.is_empty() {
            continue;
        }
        report.readings.push(Reading {
            id: candidate.id,
            title: candidate.title,
            agent: candidate.agent,
            why: candidate.why,
        });
    }
}
