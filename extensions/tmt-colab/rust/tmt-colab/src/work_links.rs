//! Pages that follow pages (contract: Work). A task may follow another page, and its state is then
//! that page's stage. One page's value cannot see another's, so this is read over what the work index
//! holds for the whole space: how a page's derived values resolve through the pages its tasks follow,
//! and the rules `task.link` is admitted by (one parent, no loop, a page that exists). These are the
//! write owner's rules, never a rule of the stored value, so a race of two writers can leave pages
//! the rules would refuse; readers then resolve them the same way every time and never loop.
use crate::{work, work_index::Found};
use serde_json::Value;
use std::collections::BTreeSet;

/// Pages followed through at most this many levels; a deeper page shows its tasks' own states.
pub const DEPTH: usize = 8;

/// Why a task may not follow a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The page itself.
    Itself,
    /// Not a page of this space, deleted or unreadable.
    Unavailable,
    /// Already followed by a task of the page named.
    HasParent { page: String, task: String },
    /// An ancestor of the page, so the link would close a loop.
    Loop,
}

/// What the work index holds for the pages of the space: a page absent from it is unavailable, a
/// page present without work is in discussion.
pub struct Graph<'a>(pub &'a Found);

impl Graph<'_> {
    /// The stage `page` shows, with its tasks that follow a page showing that page's stage. `trail`
    /// is the pages being resolved: one already on it, or one past `DEPTH`, answers nothing, so its
    /// task shows its own state.
    fn stage(&self, page: &str, trail: &mut Vec<String>) -> Option<String> {
        let work = match self.0.get(page)? {
            None => return Some("discussion".to_owned()),
            Some(work) => work,
        };
        if work.lines.is_empty() {
            return work.derived["stage"].as_str().map(str::to_owned);
        }
        trail.push(page.to_owned());
        let derived = work::derive(&work.lines, &|child| {
            if trail.len() >= DEPTH || trail.iter().any(|seen| seen == child) {
                return None;
            }
            self.stage(child, &mut trail.clone())
        });
        Some(derived.stage.to_owned())
    }

    /// The derived values of `page`, every task that follows a page showing that page's stage.
    /// `None` for a page with no work or one that is not in the index.
    pub fn derived(&self, page: &str) -> Option<Value> {
        let work = self.0.get(page)?.as_ref()?;
        if work.lines.is_empty() {
            return Some(work.derived.clone());
        }
        let trail = vec![page.to_owned()];
        let derived = work::derive(&work.lines, &|child| {
            if trail.len() >= DEPTH || trail.contains(&child.to_owned()) {
                return None;
            }
            self.stage(child, &mut trail.clone())
        });
        Some(derived.to_value())
    }

    /// The task that follows `child`: the lowest page ID, then the lowest task number, when several
    /// do (two writers raced). `(page, task)`.
    pub fn parent(&self, child: &str) -> Option<(String, String)> {
        self.0.iter().find_map(|(page, work)| {
            let line = work
                .as_ref()?
                .lines
                .iter()
                .find(|line| line.child.as_deref() == Some(child))?;
            Some((page.clone(), line.id.clone()))
        })
    }

    /// The pages `page` follows, through its tasks, and the pages those follow, and so on.
    fn descendants(&self, page: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut open = vec![page.to_owned()];
        while let Some(next) = open.pop() {
            let Some(Some(work)) = self.0.get(&next) else {
                continue;
            };
            for child in work.lines.iter().filter_map(|line| line.child.as_ref()) {
                if found.insert(child.clone()) {
                    open.push(child.clone());
                }
            }
        }
        found
    }

    /// Why task `task` of `page` may not follow `child`, if it may not.
    pub fn refusal(&self, page: &str, task: &str, child: &str) -> Option<Refusal> {
        if child == page {
            return Some(Refusal::Itself);
        }
        if !self.0.contains_key(child) {
            return Some(Refusal::Unavailable);
        }
        if let Some((parent, parent_task)) = self.parent(child)
            && (parent.as_str(), parent_task.as_str()) != (page, task)
        {
            return Some(Refusal::HasParent {
                page: parent,
                task: parent_task,
            });
        }
        self.descendants(child)
            .contains(page)
            .then_some(Refusal::Loop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work::Line;
    use crate::work_index::PageWork;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn line(id: &str, state: &str, child: Option<&str>) -> Line {
        Line {
            id: id.into(),
            state: state.into(),
            signoff: false,
            assignee: None,
            child: child.map(str::to_owned),
        }
    }
    /// A page whose tasks are `lines`, its stored derived values computed from them as they are.
    fn page(lines: Vec<Line>) -> Option<PageWork> {
        let derived = work::derive(&lines, &|_| None).to_value();
        let linked = lines.iter().any(|line| line.child.is_some());
        Some(PageWork {
            derived,
            held: vec![],
            lines: if linked { lines } else { vec![] },
        })
    }
    fn found(pages: Vec<(&str, Option<PageWork>)>) -> Found {
        pages
            .into_iter()
            .map(|(id, work)| (id.to_owned(), work))
            .collect::<BTreeMap<_, _>>()
    }

    #[test]
    fn a_task_shows_the_stage_of_its_page_through_every_level() {
        // a: T-1 follows b; b: T-1 follows c; c has one task in progress.
        let found = found(vec![
            ("a", page(vec![line("T-1", "todo", Some("b"))])),
            ("b", page(vec![line("T-1", "todo", Some("c"))])),
            ("c", page(vec![line("T-1", "in-progress", None)])),
        ]);
        let graph = Graph(&found);
        assert_eq!(graph.derived("a").unwrap()["stage"], "in-progress");
        assert_eq!(
            graph.derived("a").unwrap()["states"],
            json!({"T-1":"in-progress"})
        );
        assert_eq!(graph.derived("c").unwrap()["stage"], "in-progress");
    }

    #[test]
    fn done_pages_finish_the_pages_above_them() {
        let found = found(vec![
            ("a", page(vec![line("T-1", "todo", Some("b"))])),
            ("b", page(vec![line("T-1", "done", None)])),
        ]);
        let derived = Graph(&found).derived("a").unwrap();
        assert_eq!(derived["stage"], "done");
        assert_eq!(derived["states"], json!({"T-1":"done"}));
    }

    #[test]
    fn a_page_without_work_is_in_discussion_and_a_missing_page_leaves_the_stored_state() {
        let found = found(vec![
            (
                "a",
                page(vec![
                    line("T-1", "done", Some("empty")),
                    line("T-2", "done", Some("gone")),
                ]),
            ),
            ("empty", None),
        ]);
        let derived = Graph(&found).derived("a").unwrap();
        // The empty page is in discussion, so its task is todo; the missing one keeps `done`.
        assert_eq!(derived["states"], json!({"T-1":"todo"}));
        assert_eq!(derived["done"], 1);
    }

    #[test]
    fn a_loop_that_got_stored_resolves_to_the_stored_states_and_ends() {
        let found = found(vec![
            ("a", page(vec![line("T-1", "done", Some("b"))])),
            ("b", page(vec![line("T-1", "todo", Some("a"))])),
        ]);
        let graph = Graph(&found);
        // a follows b; b follows a, which is on the trail, so b's task shows its own stored state
        // (todo), which makes b's stage in-progress, which is what a's task shows. The same the
        // other way: a's own state (done) is b's stage seen from b.
        assert_eq!(
            graph.derived("a").unwrap()["states"],
            json!({"T-1":"in-progress"})
        );
        assert_eq!(graph.derived("b").unwrap()["states"], json!({"T-1":"done"}));
    }

    #[test]
    fn a_chain_deeper_than_the_limit_stops_following() {
        let ids: Vec<String> = (0..=DEPTH + 2).map(|n| format!("p{n:02}")).collect();
        let mut pages = Vec::new();
        for (n, id) in ids.iter().enumerate() {
            let lines = match ids.get(n + 1) {
                Some(next) => vec![line("T-1", "todo", Some(next))],
                None => vec![line("T-1", "done", None)],
            };
            pages.push((id.as_str(), page(lines)));
        }
        let found = found(pages);
        // The last page is done, but too many levels away to be followed from the first.
        let top = Graph(&found).derived(&ids[0]).unwrap();
        assert_eq!(top["stage"], "in-progress");
        // From a page `DEPTH - 1` levels above it, it is followed.
        let near = Graph(&found).derived(&ids[ids.len() - DEPTH]).unwrap();
        assert_eq!(near["stage"], "done");
    }

    #[test]
    fn the_first_page_by_id_is_the_parent_when_two_follow_one() {
        let found = found(vec![
            (
                "b",
                page(vec![
                    line("T-1", "todo", None),
                    line("T-2", "todo", Some("x")),
                ]),
            ),
            ("a", page(vec![line("T-4", "todo", Some("x"))])),
            ("x", None),
        ]);
        assert_eq!(
            Graph(&found).parent("x"),
            Some(("a".to_owned(), "T-4".to_owned()))
        );
        assert_eq!(Graph(&found).parent("a"), None);
    }

    #[test]
    fn a_link_is_refused_for_itself_a_missing_page_a_second_parent_and_a_loop() {
        let found = found(vec![
            (
                "a",
                page(vec![
                    line("T-1", "todo", Some("b")),
                    line("T-2", "todo", None),
                ]),
            ),
            ("b", page(vec![line("T-1", "todo", Some("c"))])),
            ("c", None),
            ("d", None),
        ]);
        let graph = Graph(&found);
        assert_eq!(graph.refusal("a", "T-2", "a"), Some(Refusal::Itself));
        assert_eq!(
            graph.refusal("a", "T-2", "gone"),
            Some(Refusal::Unavailable)
        );
        // b is already followed by a's T-1, so another task of the same page is refused too.
        assert_eq!(
            graph.refusal("a", "T-2", "b"),
            Some(Refusal::HasParent {
                page: "a".into(),
                task: "T-1".into()
            })
        );
        assert_eq!(
            graph.refusal("d", "T-1", "b"),
            Some(Refusal::HasParent {
                page: "a".into(),
                task: "T-1".into()
            })
        );
        // The same task linking the same page again is not a second parent.
        assert_eq!(graph.refusal("a", "T-1", "b"), None);
        // c is below a through b, so a page below it may not follow a.
        assert_eq!(graph.refusal("c", "T-1", "a"), Some(Refusal::Loop));
        assert_eq!(graph.refusal("b", "T-2", "a"), Some(Refusal::Loop));
        assert_eq!(
            graph.refusal("d", "T-1", "c"),
            Some(Refusal::HasParent {
                page: "b".into(),
                task: "T-1".into()
            })
        );
        assert_eq!(graph.refusal("a", "T-2", "d"), None);
    }
}
