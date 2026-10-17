//! Static-source counterpart of renderer collect/headingSelector/quoteRange. The shared pending
//! vectors exercise both owners; runtime-authored DOM changes remain cosmetic browser claims.
use std::{collections::BTreeMap, time::Instant};

use super::{
    Report,
    tree::{Id, Tree},
};
use crate::export::conversations::Thread;

#[derive(Default)]
pub(crate) struct SummaryAnchors {
    entries: BTreeMap<String, Vec<String>>,
}
impl SummaryAnchors {
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }
    pub(crate) fn threads(&self, id: &str) -> &[String] {
        self.entries.get(id).map_or(&[], Vec::as_slice)
    }
}

fn whitespace(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

fn contains(tree: &Tree, element: Id, node: Option<Id>) -> bool {
    node.is_some_and(|node| {
        element == node || tree.ancestors(node).any(|ancestor| ancestor == element)
    })
}

/// A missing or ambiguous quote cannot associate a record with an authored block. Exceeding
/// the renderer's deterministic text/candidate bounds fails this summary, never returns zero.
pub(super) fn project(
    tree: &Tree,
    report: &Report,
    threads: &[Thread],
) -> Result<SummaryAnchors, &'static str> {
    project_until(
        tree,
        report,
        threads,
        Instant::now() + crate::limits::SUMMARY_PROJECTION_TIMEOUT,
    )
}

fn project_until(
    tree: &Tree,
    report: &Report,
    threads: &[Thread],
    deadline: Instant,
) -> Result<SummaryAnchors, &'static str> {
    let check = || {
        if Instant::now() >= deadline {
            Err("summary projection deadline")
        } else {
            Ok(())
        }
    };
    check()?;
    // Proposals and ordinary comments need no source quote association. Keep their summaries
    // available even when unrelated page text exceeds the renderer's quote-matching budget.
    if report.questions.is_empty() && report.reviews.is_empty() && report.readings.is_empty() {
        return Ok(SummaryAnchors::default());
    }
    let mut points = Vec::<(char, Option<Id>)>::new();
    let mut white = false;
    let text_nodes = tree.text_nodes();
    if text_nodes.len() > 20_000 {
        return Err("summary text budget");
    }
    for (node, text) in text_nodes {
        check()?;
        let parents: Vec<_> = node
            .into_iter()
            .flat_map(|node| std::iter::once(node).chain(tree.ancestors(node)))
            .collect();
        if parents.iter().any(|id| tree.excluded(*id)) {
            continue;
        }
        // resolveQuestions hides each admitted question's choice children before collect().
        let choice_hidden = parents.iter().any(|id| {
            tree.attr(*id, "data-choice").is_some()
                && tree.attr(*id, "data-question").is_none()
                && tree.parent_element(*id).is_some_and(|parent| {
                    tree.name(parent) == "tmt-question"
                        && report
                            .questions
                            .iter()
                            .any(|q| Some(q.id.as_str()) == tree.attr(parent, "data-id"))
                })
        });
        if choice_hidden {
            continue;
        }
        for (index, c) in text.chars().enumerate() {
            if index % 128 == 0 {
                check()?;
            }
            if whitespace(c) {
                if !white && !points.is_empty() {
                    points.push((' ', node));
                }
                white = true;
            } else {
                points.push((c, node));
                white = false;
            }
            if points.len() > 256 * 1024 {
                return Err("summary text budget");
            }
        }
    }
    if points.last().is_some_and(|(c, _)| *c == ' ') {
        points.pop();
    }
    let text: String = points.iter().map(|(c, _)| c).collect();
    let mut matches = Vec::new();
    for thread in threads
        .iter()
        .filter(|thread| !thread.deleted && thread.proposal.is_none())
    {
        check()?;
        let Some(selector) = &thread.anchor else {
            continue;
        };
        let mut selected = Vec::new();
        let mut candidates = 0;
        let mut offset = 0;
        while let Some(found) = text[offset..].find(&selector.exact) {
            let start = offset + found;
            candidates += 1;
            check()?;
            if candidates > 64 {
                return Err("summary quote budget");
            }
            let end = start + selector.exact.len();
            if text[..start].ends_with(&selector.prefix)
                && text[end..].starts_with(&selector.suffix)
            {
                let first = text[..start].chars().count();
                let last = first + selector.exact.chars().count();
                if first < last && last <= points.len() {
                    selected.push((points[first].1, points[last - 1].1));
                }
            }
            let Some(next) = text[start..].chars().next() else {
                break;
            };
            offset = start + next.len_utf8();
            if offset > text.len() {
                break;
            }
        }
        if selected.len() == 1 {
            matches.push((thread, selected[0]));
        }
    }
    let mut anchors = SummaryAnchors::default();
    let blocks = report
        .questions
        .iter()
        .map(|q| (&q.id, &q.title, "tmt-question", "data-question"))
        .chain(
            report
                .reviews
                .iter()
                .map(|r| (&r.id, &r.title, "tmt-review", "data-title")),
        )
        .chain(
            report
                .readings
                .iter()
                .map(|r| (&r.id, &r.title, "tmt-read", "data-title")),
        );
    let elements = tree.elements();
    for (id, title, tag, heading_attr) in blocks {
        check()?;
        let holders: Vec<_> = elements
            .iter()
            .copied()
            .filter(|node| {
                tree.name(*node) == tag && tree.attr(*node, "data-id") == Some(id.as_str())
            })
            .collect();
        if holders.len() != 1 {
            continue;
        }
        let element = holders[0];
        let headings: Vec<_> = tree
            .child_elements(element)
            .filter(|node| tree.attr(*node, heading_attr).is_some())
            .collect();
        if headings.len() != 1 {
            continue;
        }
        let heading = headings[0];
        let Some(mut first) = points
            .iter()
            .position(|(_, node)| contains(tree, heading, *node))
        else {
            continue;
        };
        let mut last = first;
        while last < points.len() && contains(tree, heading, points[last].1) {
            last += 1;
        }
        while first < last && points[first].0 == ' ' {
            first += 1;
        }
        while first < last && points[last - 1].0 == ' ' {
            last -= 1;
        }
        if points[first..last]
            .iter()
            .map(|(c, _)| c)
            .collect::<String>()
            != *title
        {
            continue;
        }
        let associated = matches
            .iter()
            .filter(|(thread, (a, b))| {
                let target = if tag == "tmt-question" {
                    element
                } else {
                    heading
                };
                contains(tree, target, *a)
                    && contains(tree, target, *b)
                    && (tag == "tmt-question"
                        || thread
                            .anchor
                            .as_ref()
                            .is_some_and(|quote| quote.exact == *title))
            })
            .map(|(thread, _)| format!("{}:{}", thread.writer, thread.id))
            .collect();
        anchors.entries.insert(id.clone(), associated);
    }
    check()?;
    Ok(anchors)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_expired_projection_cannot_return_a_false_zero() {
        let tree = Tree::parse("plain text").ok().unwrap();
        let report = Report::default();
        assert!(project_until(&tree, &report, &[], Instant::now()).is_err());
        assert!(project(&tree, &report, &[]).is_ok());
    }
}
