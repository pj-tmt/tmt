//! The `tmt-question` rules, mirroring `QUESTION_RULES` in the app's `questions.ts`.

use std::collections::HashMap;

use super::{
    Finding, Question, QuestionGroup, Report, Severity, group, reading, review,
    tree::{Id, Tree},
};
use crate::limits::{
    GROUP_QUESTIONS, QUESTION_AGENT_POINTS, QUESTION_CHOICE_POINTS, QUESTION_CHOICES,
    QUESTION_PAGE, QUESTION_TITLE_POINTS, READING_PAGE, REVIEW_PAGE,
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
    /// Whether this question holds its ID in the shared namespace (see [`group::Survey`]). A
    /// wrapped question, or one inside a skipped group, holds none, so it is never reported as
    /// `question-id-duplicate`.
    holder: bool,
    holders: usize,
    split_id: Option<String>,
    split_conflict: bool,
    split_count: usize,
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
pub(super) const RULES: [Rule; 22] = [
    ("question-id-invalid", |c| !is_uuid(&c.id)),
    ("question-id-duplicate", |c| {
        c.holder && is_uuid(&c.id) && c.holders > 1
    }),
    ("question-nested", |c| c.nested),
    ("question-group-child-invalid", |c| c.wrapped),
    ("question-group-id-invalid", |c| {
        c.split_id.as_ref().is_some_and(|id| !is_uuid(id))
    }),
    ("question-group-conflict", |c| {
        c.split_id.is_some() && c.split_conflict
    }),
    ("question-group-limit", |c| c.split_count > GROUP_QUESTIONS),
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

/// One finding for `id`, which must be one of the page's elements in `ordinals`.
pub(super) fn make_finding(
    tree: &Tree,
    ordinals: &HashMap<Id, usize>,
    id: Id,
    severity: Severity,
    code: &'static str,
    questions_skipped: Option<usize>,
) -> Finding {
    Finding {
        element: tree.name(id).to_owned(),
        ordinal: ordinals[&id],
        id: tree.attr(id, "data-id").map(str::to_owned),
        line: tree.line(id),
        severity,
        code,
        questions_skipped,
    }
}

pub(super) fn check(tree: &Tree, elements: &[(Id, usize)], report: &mut Report) {
    let ordinals: HashMap<Id, usize> = elements.iter().copied().collect();
    let elements: Vec<Id> = elements.iter().map(|(id, _)| *id).collect();
    let finding = |id: Id, severity, code, questions_skipped| {
        make_finding(tree, &ordinals, id, severity, code, questions_skipped)
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
    let all_reviews = named("tmt-review");
    report.question_blocks = questions.len();
    report.review_blocks = all_reviews.len();
    let all_readings = named("tmt-read");
    report.reading_blocks = all_readings.len();
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
    // Past the section cap the page supplies no sections at all, and none of them holds an ID.
    let reviews = if all_reviews.len() > REVIEW_PAGE {
        report.findings.push(finding(
            all_reviews[REVIEW_PAGE],
            Severity::Ignored,
            "review-page-limit",
            None,
        ));
        Vec::new()
    } else {
        all_reviews
    };
    let readings = if all_readings.len() > READING_PAGE {
        report.findings.push(finding(
            all_readings[READING_PAGE],
            Severity::Ignored,
            "reading-page-limit",
            None,
        ));
        Vec::new()
    } else {
        all_readings
    };
    let mut identity_holders = reviews.clone();
    identity_holders.extend(&readings);
    // A logical group holds one ID independently of its raw members' question validity.
    let split_eligible = |id| {
        tree.closest(id, &["tmt-questions", "tmt-question", "tmt-read"])
            .is_none()
    };
    let mut split_groups: HashMap<String, usize> = HashMap::new();
    for id in &questions {
        if let Some(group) = tree.attr(*id, "data-group").filter(|group| is_uuid(group))
            && split_eligible(*id)
        {
            *split_groups.entry(group.to_owned()).or_insert(0) += 1;
        }
    }
    let survey = group::Survey::new(tree, &groups, &questions, &identity_holders, &split_groups);
    for group in &groups {
        if let Some((code, skipped)) = survey.skipped(*group) {
            report
                .findings
                .push(finding(*group, Severity::Ignored, code, Some(skipped)));
        }
    }

    // The group of each admitted question, in document order.
    let mut admitted: Vec<Option<Id>> = Vec::new();
    // The admitted questions themselves, for the sections that hold them.
    let mut admitted_nodes: Vec<(Id, String)> = Vec::new();
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
        let split_id = tree.attr(id, "data-group").map(str::to_owned);
        let candidate = Candidate {
            split_conflict: !split_eligible(id)
                || split_id
                    .as_ref()
                    .is_some_and(|group| is_uuid(group) && survey.holders(group) != 1),
            split_count: split_id
                .as_ref()
                .and_then(|group| split_groups.get(group))
                .copied()
                .unwrap_or(0),
            split_id: split_id.clone(),
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
            nested: tree.has_ancestor(id, "tmt-question") || tree.has_ancestor(id, "tmt-read"),
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
        let label = group.map(|group| {
            let number = direct.entry(group).or_insert(0);
            *number += 1;
            let number = *number;
            let (authored, warnings) = group::label(tree.attr(id, "data-label"));
            report.findings.extend(
                warnings
                    .iter()
                    .map(|code| finding(id, Severity::Warning, code, None)),
            );
            // The group decides the form; a member's own attribute is ignored.
            if tree.attr(id, "data-form").is_some() {
                report
                    .findings
                    .push(finding(id, Severity::Warning, group::FORM_CODES[2], None));
            }
            authored.unwrap_or_else(|| format!("Q{number}"))
        });
        admitted.push(group);
        admitted_nodes.push((id, candidate.id.clone()));
        report.questions.push(Question {
            id: candidate.id,
            kind: candidate.kind,
            title: candidate.title,
            choices: candidate.choices,
            other: candidate.other,
            agent: candidate.agent,
            group: group
                .map(|g| tree.attr(g, "data-id").unwrap_or("").to_owned())
                .or(split_id),
            label,
        });
    }

    // Groups the page shows, and a lone question as the internal group of itself.
    let mut shown: HashMap<Id, usize> = HashMap::new();
    let mut shown_split: HashMap<String, usize> = HashMap::new();
    for (index, (group, admitted_question)) in admitted.iter().zip(&report.questions).enumerate() {
        match group {
            None => {
                if let Some(group) = &admitted_question.group {
                    let index = *shown_split.entry(group.clone()).or_insert_with(|| {
                        report.groups.push(QuestionGroup {
                            id: group.clone(),
                            title: None,
                            questions: Vec::new(),
                            chip: false,
                            split: true,
                        });
                        report.groups.len() - 1
                    });
                    report.groups[index]
                        .questions
                        .push(admitted_question.id.clone());
                    continue;
                }
                let node = admitted_nodes[index].0;
                let form = group::form(tree.attr(node, "data-form"));
                if form == group::Form::Invalid {
                    report.findings.push(finding(
                        node,
                        Severity::Warning,
                        group::FORM_CODES[1],
                        None,
                    ));
                }
                report.groups.push(QuestionGroup {
                    id: admitted_question.id.clone(),
                    title: None,
                    questions: vec![admitted_question.id.clone()],
                    chip: form == group::Form::Chip,
                    split: false,
                });
            }
            Some(group) => {
                let index = *shown.entry(*group).or_insert_with(|| {
                    let form = group::form(tree.attr(*group, "data-form"));
                    if form == group::Form::Invalid {
                        report.findings.push(finding(
                            *group,
                            Severity::Warning,
                            group::FORM_CODES[0],
                            None,
                        ));
                    }
                    report.groups.push(QuestionGroup {
                        id: tree.attr(*group, "data-id").unwrap_or("").to_owned(),
                        title: Some(survey.title(*group).to_owned()),
                        questions: Vec::new(),
                        chip: form == group::Form::Chip,
                        split: false,
                    });
                    report.groups.len() - 1
                });
                report.groups[index]
                    .questions
                    .push(admitted_question.id.clone());
            }
        }
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
    review::check(tree, &ordinals, &survey, &reviews, &admitted_nodes, report);
    reading::check(tree, &ordinals, &survey, &readings, report);
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
