//! The `tmt-questions` group rules, mirroring the group rules in the app's `questions.ts`.
//!
//! A group that fails a structural rule is skipped whole: its questions and any group inside it
//! are silent, and the one finding names the first failed rule and how many questions it took
//! with it. A group that passes evaluates its direct questions, each reporting its own findings.

use std::collections::{HashMap, HashSet};

use super::{
    question::{has_control, is_uuid, over, question_text},
    tree::{Id, Tree},
};
use crate::limits::{GROUP_LABEL_POINTS, GROUP_QUESTIONS, QUESTION_TITLE_POINTS};

const GROUP: &str = "tmt-questions";
const QUESTION: &str = "tmt-question";
const DUPLICATE: &str = "group-id-duplicate";

/// What the structural rules read about one group.
pub(super) struct GroupCandidate {
    id: String,
    holders: usize,
    nested: bool,
    titles: usize,
    title: String,
    questions: usize,
}

pub(super) type GroupRule = (&'static str, fn(&GroupCandidate) -> bool);

/// The structural rules in report order; a group reports the first that fails. The title rules
/// wait for exactly one title so one defect is not reported twice.
pub(super) const RULES: [GroupRule; 9] = [
    ("group-id-invalid", |c| !is_uuid(&c.id)),
    (DUPLICATE, |c| is_uuid(&c.id) && c.holders > 1),
    ("group-nested", |c| c.nested),
    ("group-title-missing", |c| c.titles == 0),
    ("group-title-multiple", |c| c.titles > 1),
    ("group-title-empty", |c| c.titles == 1 && c.title.is_empty()),
    ("group-title-too-long", |c| {
        c.titles == 1 && over(&c.title, QUESTION_TITLE_POINTS)
    }),
    ("group-title-control", |c| {
        c.titles == 1 && has_control(&c.title)
    }),
    ("group-questions-too-many", |c| {
        c.questions > GROUP_QUESTIONS
    }),
];

/// Warnings about a question's `data-label`; its report label then falls back to `Q<n>`.
pub(super) const LABEL_CODES: [&str; 2] = ["group-label-too-long", "group-label-control"];

/// Warnings about `data-form`: an unknown value on a group or a lone question, and any value on a
/// group member (the group decides). All admitted forms use compact lines.
pub(super) const FORM_CODES: [&str; 3] = [
    "group-form-invalid",
    "question-form-invalid",
    "question-form-member",
];

/// What a group's or lone question's `data-form` asks for. The one value is `chip`, exact and
/// lowercase; any other present value is a warning.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Form {
    Block,
    Chip,
    Invalid,
}

pub(super) fn form(raw: Option<&str>) -> Form {
    match raw {
        None => Form::Block,
        Some("chip") => Form::Chip,
        Some(_) => Form::Invalid,
    }
}

fn failure(candidate: &GroupCandidate, duplicates: bool) -> Option<&'static str> {
    RULES
        .iter()
        .find(|(code, fails)| (duplicates || *code != DUPLICATE) && fails(candidate))
        .map(|(code, _)| *code)
}

/// A question's authored report label, and warnings that make it fall back to `Q<n>`: an
/// absent or empty label falls back silently.
pub(super) fn label(raw: Option<&str>) -> (Option<String>, Vec<&'static str>) {
    let text = question_text(raw.unwrap_or(""));
    if text.is_empty() {
        return (None, Vec::new());
    }
    let mut warnings = Vec::new();
    if over(&text, GROUP_LABEL_POINTS) {
        warnings.push(LABEL_CODES[0]);
    }
    if has_control(&text) {
        warnings.push(LABEL_CODES[1]);
    }
    let authored = warnings.is_empty().then_some(text);
    (authored, warnings)
}

/// Which groups the browser skips, and which question IDs a page's namespace holds.
///
/// IDs are one namespace for lone questions, groups, review sections and the direct questions of a
/// group. Every group and every review section (while the page is within the section cap) holds its
/// own ID wherever it sits, skipped or not. The direct questions of a group that
/// passes the structural rules other than a duplicate ID hold theirs, and a lone question holds
/// its own. That set is fixed before duplicates are judged, so a duplicate failure retracts
/// nothing: the questions of a group skipped for a duplicate still hold their IDs against other
/// holders.
pub(super) struct Survey {
    /// The first failed rule of each skipped group; a group inside another skipped group is not
    /// listed, its subtree is silent.
    failure: HashMap<Id, &'static str>,
    /// The questions each skipped group takes with it.
    skipped: HashMap<Id, usize>,
    /// Groups whose direct questions hold their IDs.
    open: HashSet<Id>,
    holders: HashMap<String, usize>,
    /// The title of each group the survey judged.
    titles: HashMap<Id, String>,
}

impl Survey {
    pub(super) fn new(
        tree: &Tree,
        groups: &[Id],
        questions: &[Id],
        reviews: &[Id],
        split: &HashMap<String, usize>,
    ) -> Survey {
        let mut candidates: Vec<(Id, GroupCandidate)> = groups
            .iter()
            .map(|group| {
                let titles: Vec<Id> = tree
                    .child_elements(*group)
                    .filter(|child| {
                        tree.name(*child) == "p" && tree.attr(*child, "data-title").is_some()
                    })
                    .collect();
                let candidate = GroupCandidate {
                    id: tree.attr(*group, "data-id").unwrap_or("").to_owned(),
                    holders: 0,
                    nested: tree
                        .closest(*group, &[GROUP, QUESTION, "tmt-read"])
                        .is_some(),
                    titles: titles.len(),
                    title: titles
                        .first()
                        .map_or_else(String::new, |title| question_text(&tree.text(*title))),
                    questions: tree
                        .child_elements(*group)
                        .filter(|child| tree.name(*child) == QUESTION)
                        .count(),
                };
                (*group, candidate)
            })
            .collect();

        // Pass 1: every group holds its own ID wherever it sits; the structural rules other than a
        // duplicate ID fix whose direct questions hold theirs.
        let mut open = HashSet::new();
        let mut holders: HashMap<String, usize> = split.keys().map(|id| (id.clone(), 1)).collect();
        for (group, candidate) in &candidates {
            *holders.entry(candidate.id.clone()).or_insert(0) += 1;
            if failure(candidate, false).is_none() {
                open.insert(*group);
            }
        }
        // A review section holds its ID wherever it sits, as a group does.
        for review in reviews {
            *holders
                .entry(tree.attr(*review, "data-id").unwrap_or("").to_owned())
                .or_insert(0) += 1;
        }
        let survey = Survey {
            failure: HashMap::new(),
            skipped: HashMap::new(),
            open,
            holders,
            titles: HashMap::new(),
        };
        let counted = |question: &&Id| survey.holds(tree, **question);
        let mut holders = survey.holders.clone();
        for question in questions.iter().filter(counted) {
            *holders
                .entry(tree.attr(*question, "data-id").unwrap_or("").to_owned())
                .or_insert(0) += 1;
        }

        // Pass 2: the first failed rule of every group that is not already silent.
        let mut failures: HashMap<Id, &'static str> = HashMap::new();
        for (group, candidate) in &mut candidates {
            if tree
                .ancestors(*group)
                .any(|ancestor| failures.contains_key(&ancestor))
            {
                continue;
            }
            candidate.holders = holders.get(&candidate.id).copied().unwrap_or(0);
            if let Some(code) = failure(candidate, true) {
                failures.insert(*group, code);
            }
        }
        let mut skipped: HashMap<Id, usize> = HashMap::new();
        for question in questions {
            if let Some(group) = tree
                .ancestors(*question)
                .find(|ancestor| failures.contains_key(ancestor))
            {
                *skipped.entry(group).or_insert(0) += 1;
            }
        }
        Survey {
            failure: failures,
            skipped,
            open: survey.open,
            holders,
            titles: candidates
                .into_iter()
                .map(|(group, candidate)| (group, candidate.title))
                .collect(),
        }
    }

    /// Whether this question holds its ID in the shared namespace: a lone question, or a direct
    /// question of a group that passes the structural rules. A question wrapped in another
    /// element, or inside a skipped group, holds nothing.
    pub(super) fn holds(&self, tree: &Tree, question: Id) -> bool {
        if !tree.has_ancestor(question, GROUP) {
            return true;
        }
        tree.parent_element(question)
            .is_some_and(|parent| self.open.contains(&parent))
    }

    pub(super) fn holders(&self, id: &str) -> usize {
        self.holders.get(id).copied().unwrap_or(0)
    }

    /// A skipped group's first failed rule and how many questions it took with it.
    pub(super) fn skipped(&self, group: Id) -> Option<(&'static str, usize)> {
        self.failure
            .get(&group)
            .map(|code| (*code, self.skipped.get(&group).copied().unwrap_or(0)))
    }

    /// Whether a skipped group above this element silences it.
    pub(super) fn silenced(&self, tree: &Tree, element: Id) -> bool {
        tree.ancestors(element)
            .any(|ancestor| self.failure.contains_key(&ancestor))
    }

    pub(super) fn title(&self, group: Id) -> &str {
        self.titles.get(&group).map_or("", String::as_str)
    }
}
