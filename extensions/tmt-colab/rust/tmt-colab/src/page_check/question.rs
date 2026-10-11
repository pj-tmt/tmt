//! The `tmt-question` rules, mirroring `QUESTION_RULES` in the app's `questions.ts`.

use std::collections::HashMap;

use super::{
    Finding, Question, QuestionGroup, Report, Severity, group,
    tree::{Id, Tree},
};
use crate::limits::{
    QUESTION_AGENT_POINTS, QUESTION_CHOICE_POINTS, QUESTION_CHOICES, QUESTION_PAGE,
    QUESTION_TITLE_POINTS,
};

const KINDS: [&str; 4] = ["single", "multiple", "short", "long"];

/// What the rules read about one question block.
pub(super) struct Candidate {
    id: String,
    kind: String,
    agent: String,
    other: bool,
    headings: usize,
    title: String,
    choices: Vec<String>,
    nested: bool,
    /// The nearest `tmt-question`/`tmt-questions` ancestor is a group that is not the parent.
    wrapped: bool,
    /// Whether this question holds its ID in the shared namespace (see [`group::Survey`]).
    holder: bool,
    holders: usize,
}

pub(super) type Rule = (&'static str, fn(&Candidate) -> bool);

fn single_or_multiple(c: &Candidate) -> bool {
    matches!(c.kind.as_str(), "single" | "multiple")
}
fn valid_kind(c: &Candidate) -> bool {
    KINDS.contains(&c.kind.as_str())
}

/// The ordered rule table; a question is admitted exactly when no rule fails. Title rules wait for
/// exactly one heading and the choice-shape rules for a valid kind, so one defect is not reported
/// twice.
pub(super) const RULES: [Rule; 19] = [
    ("question-id-invalid", |c| !is_uuid(&c.id)),
    ("question-id-duplicate", |c| {
        c.holder && is_uuid(&c.id) && c.holders > 1
    }),
    ("question-nested", |c| c.nested),
    ("question-group-child-invalid", |c| c.wrapped),
    ("question-heading-missing", |c| c.headings == 0),
    ("question-heading-multiple", |c| c.headings > 1),
    ("question-title-empty", |c| {
        c.headings == 1 && c.title.is_empty()
    }),
    ("question-title-too-long", |c| {
        c.headings == 1 && over(&c.title, QUESTION_TITLE_POINTS)
    }),
    ("question-title-control", |c| {
        c.headings == 1 && has_control(&c.title)
    }),
    ("question-kind-invalid", |c| !valid_kind(c)),
    ("question-choices-too-many", |c| {
        c.choices.len() > QUESTION_CHOICES
    }),
    ("question-choice-empty", |c| {
        c.choices.iter().any(String::is_empty)
    }),
    ("question-choice-too-long", |c| {
        c.choices.iter().any(|x| over(x, QUESTION_CHOICE_POINTS))
    }),
    ("question-choice-control", |c| {
        c.choices.iter().any(|x| has_control(x))
    }),
    ("question-choice-duplicate", |c| has_duplicate(&c.choices)),
    ("question-choices-missing", |c| {
        valid_kind(c) && single_or_multiple(c) && c.choices.is_empty()
    }),
    ("question-choices-unexpected", |c| {
        valid_kind(c) && !single_or_multiple(c) && !c.choices.is_empty()
    }),
    ("question-other-unexpected", |c| {
        valid_kind(c) && !single_or_multiple(c) && c.other
    }),
    ("question-agent-invalid", |c| {
        over(&c.agent, QUESTION_AGENT_POINTS) || has_control(&c.agent)
    }),
];

/// A canonical lowercase UUID, as `questions.ts` accepts: any version and variant.
pub(super) fn is_uuid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, length)| {
            group.len() == length
                && group
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

/// JavaScript's `\s`: Unicode `Zs`, the line terminators, tab, vertical tab, form feed and the
/// byte order mark. Not Rust's `char::is_whitespace`, which also takes U+0085 and not U+FEFF.
fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `questionText`: every whitespace run becomes one space, then the ends are trimmed.
pub(super) fn question_text(value: &str) -> String {
    let mut text = String::new();
    let mut pending = false;
    for c in value.chars() {
        if is_space(c) {
            pending = !text.is_empty();
        } else {
            if pending {
                text.push(' ');
                pending = false;
            }
            text.push(c);
        }
    }
    text
}

pub(super) fn has_control(value: &str) -> bool {
    value.chars().any(|c| u32::from(c) < 32 || c == '\u{7f}')
}

pub(super) fn over(value: &str, points: usize) -> bool {
    value.chars().count() > points
}

fn has_duplicate(values: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    values.iter().any(|value| !seen.insert(value.as_str()))
}

pub(super) fn check(tree: &Tree, elements: &[(Id, usize)], report: &mut Report) {
    let ordinals: HashMap<Id, usize> = elements.iter().copied().collect();
    let elements: Vec<Id> = elements.iter().map(|(id, _)| *id).collect();
    let finding = |id: Id, severity, code, questions_skipped| Finding {
        element: tree.name(id).to_owned(),
        ordinal: ordinals[&id],
        id: tree.attr(id, "data-id").map(str::to_owned),
        line: tree.line(id),
        severity,
        code,
        questions_skipped,
    };

    let named = |name: &str| -> Vec<Id> {
        elements
            .iter()
            .copied()
            .filter(|id| tree.name(*id) == name)
            .collect()
    };
    let questions = named("tmt-question");
    let groups = named("tmt-questions");
    report.question_blocks = questions.len();
    if questions.len() > QUESTION_PAGE {
        // The browser drops the whole page's questions; one finding names the first over the cap.
        report.findings.push(finding(
            questions[QUESTION_PAGE],
            Severity::Ignored,
            "question-page-limit",
            None,
        ));
        return;
    }
    let survey = group::Survey::new(tree, &groups, &questions);
    for group in &groups {
        if let Some((code, skipped)) = survey.skipped(*group) {
            report
                .findings
                .push(finding(*group, Severity::Ignored, code, Some(skipped)));
        }
    }

    // The admitted questions in document order, each with its group when it has one.
    let mut admitted: Vec<(Id, Option<Id>)> = Vec::new();
    let mut direct: HashMap<Id, usize> = HashMap::new();
    for id in questions {
        if survey.silenced(tree, id) {
            continue;
        }
        let headings: Vec<Id> = tree
            .child_elements(id)
            .filter(|child| tree.attr(*child, "data-question").is_some())
            .collect();
        let id_value = tree.attr(id, "data-id").unwrap_or("");
        let container = tree.closest(id, &["tmt-question", "tmt-questions"]);
        let candidate = Candidate {
            id: id_value.to_owned(),
            kind: tree.attr(id, "data-kind").unwrap_or("").to_owned(),
            agent: tree.attr(id, "data-agent").unwrap_or("").to_owned(),
            other: tree.attr(id, "data-other").is_some(),
            headings: headings.len(),
            title: headings
                .first()
                .map_or_else(String::new, |h| question_text(&tree.text(*h))),
            choices: tree
                .child_elements(id)
                .filter(|child| tree.attr(*child, "data-choice").is_some())
                .map(|child| question_text(&tree.text(child)))
                .collect(),
            nested: tree.has_ancestor(id, "tmt-question"),
            wrapped: container.is_some_and(|c| {
                tree.name(c) == "tmt-questions" && tree.parent_element(id) != Some(c)
            }),
            holder: survey.holds(tree, id),
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
                .map(|code| finding(id, Severity::Ignored, code, None)),
        );
        if !failed.is_empty() {
            continue;
        }
        // Admitted. With a group ancestor it is a direct question of a group that passed.
        let group = tree
            .has_ancestor(id, "tmt-questions")
            .then(|| tree.parent_element(id))
            .flatten();
        let (label, number) = match group {
            None => (None, 0),
            Some(group) => {
                let number = direct.entry(group).or_insert(0);
                *number += 1;
                let (authored, warnings) = group::label(tree.attr(id, "data-label"));
                report.findings.extend(
                    warnings
                        .iter()
                        .map(|code| finding(id, Severity::Warning, code, None)),
                );
                (
                    Some(authored.unwrap_or_else(|| format!("Q{number}"))),
                    *number,
                )
            }
        };
        debug_assert!(group.is_some() == (number > 0));
        admitted.push((id, group));
        report.questions.push(Question {
            id: candidate.id,
            kind: candidate.kind,
            title: candidate.title,
            choices: candidate.choices,
            other: candidate.other,
            agent: candidate.agent,
            group: group.map(|g| tree.attr(g, "data-id").unwrap_or("").to_owned()),
            label,
        });
    }

    // Groups the page shows, and a lone question as the internal group of itself.
    let mut shown: HashMap<Id, usize> = HashMap::new();
    for ((question, group), admitted_question) in admitted.iter().zip(&report.questions) {
        match group {
            None => report.groups.push(QuestionGroup {
                id: admitted_question.id.clone(),
                title: None,
                questions: vec![admitted_question.id.clone()],
            }),
            Some(group) => {
                let index = *shown.entry(*group).or_insert_with(|| {
                    report.groups.push(QuestionGroup {
                        id: tree.attr(*group, "data-id").unwrap_or("").to_owned(),
                        title: Some(survey.title(*group).to_owned()),
                        questions: Vec::new(),
                    });
                    report.groups.len() - 1
                });
                report.groups[index]
                    .questions
                    .push(admitted_question.id.clone());
            }
        }
        let _ = question;
    }
    // A group that passed but shows nothing: wrapped, invalid or absent questions only.
    for group in &groups {
        if survey.skipped(*group).is_none()
            && !survey.silenced(tree, *group)
            && !shown.contains_key(group)
        {
            report
                .findings
                .push(finding(*group, Severity::Ignored, "group-empty", None));
        }
    }
    // A marker outside a question is plain content to the browser: nothing is ignored, nothing
    // shows. Inside a skipped group the whole subtree is silent.
    for id in elements {
        let marker =
            tree.attr(id, "data-question").is_some() || tree.attr(id, "data-choice").is_some();
        let in_question = tree
            .parent_element(id)
            .is_some_and(|parent| tree.name(parent) == "tmt-question");
        if marker && !in_question && !survey.silenced(tree, id) {
            report.findings.push(finding(
                id,
                Severity::Warning,
                "question-marker-misplaced",
                None,
            ));
        }
    }
}
