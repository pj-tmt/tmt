//! Pages that follow pages (contract: Work). A task may follow another page, and its state is then
//! that page's stage. One page's value cannot see another's, so this is read over what the work index
//! holds for the whole space: how a page's derived values resolve through the pages its tasks follow,
//! and the rules `task.link` is admitted by (one parent, no loop, an available page; any depth). These are the
//! write owner's rules, never a rule of the stored value, so a race of two writers can leave pages
//! the rules would refuse; readers then resolve them the same way every time and never loop.
use crate::{work, work_index::Found};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Pages followed through at most this many levels; a deeper page shows its tasks' own states.
pub const DEPTH: usize = 8;
/// Assignees a list row names; the rest are only counted.
pub const ROW_WORKING: usize = 3;

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
    /// Existing conflicting links must be repaired before adding more.
    InvalidHierarchy,
}
impl Refusal {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Itself | Self::Loop => "cycle",
            Self::Unavailable => "unavailable",
            Self::HasParent { .. } => "second-parent",
            Self::InvalidHierarchy => "invalid-hierarchy",
        }
    }
}

/// What the work index holds for the pages of the space: a page absent from it is unavailable, a
/// page present without work is in discussion.
/// The most code points of a parent task's title a list row carries (`bootstrap.ts` holds the
/// same number).
pub const PARENT_TITLE_POINTS: usize = 32;

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
        if !work.has_work {
            return None;
        }
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

    /// The explicit page state and direct-item progress, only from current known entries.
    pub fn page_state(&self, page: &str) -> Option<&str> {
        self.0.get(page)?.as_ref().map(|work| work.state.as_str())
    }

    pub fn progress(&self, page: &str) -> Option<crate::page_state::Progress> {
        let work = self.0.get(page)?.as_ref()?;
        if work.lines.is_empty() {
            return Some(work.own_progress.clone());
        }
        Some(crate::page_state::progress(&work.lines, &|child| {
            self.page_state(child).map(str::to_owned)
        }))
    }

    /// The task that follows each followed page: the lowest page ID, then the lowest task number,
    /// when several do (two writers raced). `child -> (page, task)`.
    pub fn parents(&self) -> BTreeMap<String, (String, String)> {
        let mut found: BTreeMap<String, (String, String)> = BTreeMap::new();
        // Pages ascend, so the first page to claim a child keeps it; a later task of that same
        // page only replaces it with a lower number.
        for (page, work) in self.0 {
            for line in work.iter().flat_map(|work| &work.lines) {
                let Some(child) = &line.child else { continue };
                match found.get_mut(child) {
                    None => {
                        found.insert(child.clone(), (page.clone(), line.id.clone()));
                    }
                    Some(held)
                        if held.0 == *page
                            && work::task_number(&line.id) < work::task_number(&held.1) =>
                    {
                        held.1 = line.id.clone();
                    }
                    Some(_) => {}
                }
            }
        }
        found
    }

    /// The title of task `task` of `page`, clipped to `PARENT_TITLE_POINTS` code points (a longer
    /// one ends in `…`), when the page's tasks are kept (a page whose tasks follow other pages:
    /// every parent is one). The list reply is bounded for 1,000 rows, so the clip is part of it.
    pub fn task_title(&self, page: &str, task: &str) -> Option<String> {
        let work = self.0.get(page)?.as_ref()?;
        let title = &work.lines.iter().find(|line| line.id == task)?.title;
        Some(if title.chars().count() > PARENT_TITLE_POINTS {
            let head: String = title.chars().take(PARENT_TITLE_POINTS - 1).collect();
            format!("{}…", head.trim_end())
        } else {
            title.clone()
        })
    }

    /// The task that follows `child`, if any (see `parents`). `(page, task)`.
    pub fn parent(&self, child: &str) -> Option<(String, String)> {
        self.parents().remove(child)
    }

    /// The list row's `work` for `page`: the derived values with every task that follows a page
    /// showing that page's stage, naming at most `ROW_WORKING` assignees (kind and name only) and
    /// counting the rest in `more`. `None` for a page without work or outside the index.
    pub fn row(&self, page: &str) -> Option<Value> {
        let derived = self.derived(page)?;
        let working = derived["working"].as_array()?;
        let mut row = json!({
            "stage": derived["stage"],
            "done": derived["done"],
            "total": derived["total"],
            "signoff": derived["signoff"],
            "working": working
                .iter()
                .take(ROW_WORKING)
                .map(|who| json!({"kind": who["kind"], "name": who["name"]}))
                .collect::<Vec<_>>(),
        });
        if working.len() > ROW_WORKING {
            row["more"] = json!(working.len() - ROW_WORKING);
        }
        Some(row)
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

    /// Diagnostics cover the connected hierarchy, including every competing parent claim.
    /// They never alter stored links or the deterministic parent/reader projection.
    pub fn warnings(&self, page: &str) -> Vec<&'static str> {
        let mut children: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut parents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (parent, work) in self.0 {
            for child in work
                .iter()
                .flat_map(|work| &work.lines)
                .filter_map(|line| line.child.as_deref())
            {
                children.entry(parent).or_default().insert(child);
                parents.entry(child).or_default().push(parent);
            }
        }
        let mut component = BTreeSet::new();
        let mut open = vec![page];
        while let Some(next) = open.pop() {
            if !component.insert(next) {
                continue;
            }
            open.extend(children.get(next).into_iter().flatten().copied());
            open.extend(parents.get(next).into_iter().flatten().copied());
        }
        let mut warnings = Vec::new();
        if component
            .iter()
            .any(|id| parents.get(id).is_some_and(|claims| claims.len() > 1))
        {
            warnings.push("subpage-multiple-parents");
        }
        // Topological removal detects a cycle without recursive reads or a depth assumption.
        let mut incoming: BTreeMap<&str, usize> = component
            .iter()
            .map(|id| {
                let count = parents
                    .get(id)
                    .map_or(0, |claims| claims.iter().collect::<BTreeSet<_>>().len());
                (*id, count)
            })
            .collect();
        let mut ready: Vec<_> = incoming
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(*id))
            .collect();
        let mut removed = 0;
        while let Some(parent) = ready.pop() {
            removed += 1;
            for child in children.get(parent).into_iter().flatten() {
                let count = incoming.get_mut(child).expect("child is in its component");
                *count -= 1;
                if *count == 0 {
                    ready.push(child);
                }
            }
        }
        if removed != component.len() {
            warnings.push("subpage-cycle");
        }
        warnings
    }

    /// A conversion creates a new child, so check the source page before creation.
    pub fn conversion_refusal(&self, page: &str) -> Option<Refusal> {
        (!self.warnings(page).is_empty()).then_some(Refusal::InvalidHierarchy)
    }

    /// Why task `task` of `page` may not follow `child`, if it may not.
    pub fn refusal(&self, page: &str, task: &str, child: &str) -> Option<Refusal> {
        if child == page {
            return Some(Refusal::Itself);
        }
        if !self.0.contains_key(child) {
            return Some(Refusal::Unavailable);
        }
        if let Some((parent, parent_task)) = self.0.iter().find_map(|(parent, work)| {
            work.iter()
                .flat_map(|work| &work.lines)
                .find(|line| {
                    line.child.as_deref() == Some(child)
                        && (parent.as_str(), line.id.as_str()) != (page, task)
                })
                .map(|line| (parent.clone(), line.id.clone()))
        }) {
            return Some(Refusal::HasParent {
                page: parent,
                task: parent_task,
            });
        }
        if self.descendants(child).contains(page) {
            return Some(Refusal::Loop);
        }
        if !self.warnings(page).is_empty() || !self.warnings(child).is_empty() {
            return Some(Refusal::InvalidHierarchy);
        }
        None
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
            title: format!("Title of {id}"),
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
            state: "todo".into(),
            has_work: true,
            own_progress: crate::page_state::progress(&lines, &|_| None),
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
    fn manual_progress_is_one_level_deduplicated_and_unknown_when_child_is_stale() {
        let mut child = page(vec![line("T-1", "todo", Some("a"))]).unwrap();
        child.state = "done".into();
        let found = found(vec![
            (
                "a",
                page(vec![
                    line("T-1", "todo", Some("b")),
                    line("T-2", "todo", Some("b")),
                    line("T-3", "done", Some("missing")),
                    line("T-4", "in-review", None),
                    line("T-5", "todo", None),
                ]),
            ),
            ("b", Some(child)),
        ]);
        let graph = Graph(&found);
        assert_eq!(
            serde_json::to_value(graph.progress("a")).unwrap(),
            json!({"done":1,"inProgress":1,"todo":1,"unknown":1,"total":4})
        );
        assert_eq!(graph.page_state("a"), Some("todo"));
        assert!(graph.progress("missing").is_none());
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
    fn a_task_of_a_kept_page_names_its_title_and_other_tasks_do_not() {
        let found = found(vec![
            (
                "b",
                page(vec![
                    line("T-1", "todo", Some("x")),
                    line("T-2", "todo", None),
                ]),
            ),
            ("c", page(vec![line("T-1", "todo", None)])),
        ]);
        let graph = Graph(&found);
        assert_eq!(
            graph.task_title("b", "T-1").as_deref(),
            Some("Title of T-1")
        );
        assert_eq!(graph.task_title("b", "T-9"), None);
        // A page whose tasks follow nothing keeps no lines, so it names none.
        assert_eq!(graph.task_title("c", "T-1"), None);
        assert_eq!(graph.task_title("zz", "T-1"), None);
    }

    #[test]
    fn a_long_title_is_clipped_to_the_bound_and_ends_in_an_ellipsis() {
        let mut long = line("T-1", "todo", Some("x"));
        long.title = "é".repeat(PARENT_TITLE_POINTS + 5);
        let clipped = found(vec![("b", page(vec![long]))]);
        let title = Graph(&clipped).task_title("b", "T-1").unwrap();
        assert_eq!(title.chars().count(), PARENT_TITLE_POINTS);
        assert!(title.ends_with('…'));
        let mut exact = line("T-1", "todo", Some("x"));
        exact.title = "é".repeat(PARENT_TITLE_POINTS);
        let whole = found(vec![("b", page(vec![exact]))]);
        assert_eq!(
            Graph(&whole).task_title("b", "T-1").unwrap(),
            "é".repeat(PARENT_TITLE_POINTS)
        );
    }

    #[test]
    fn the_lowest_task_number_of_the_first_page_is_the_parent_whatever_the_task_order() {
        // b lists T-5 before T-2, and both follow x: the lower number wins, not the first listed.
        let found = found(vec![
            (
                "b",
                page(vec![
                    line("T-5", "todo", Some("x")),
                    line("T-2", "todo", Some("x")),
                    line("T-3", "todo", Some("y")),
                ]),
            ),
            ("x", None),
            ("y", None),
        ]);
        let graph = Graph(&found);
        assert_eq!(graph.parent("x"), Some(("b".to_owned(), "T-2".to_owned())));
        assert_eq!(
            graph.parents(),
            BTreeMap::from([
                ("x".to_owned(), ("b".to_owned(), "T-2".to_owned())),
                ("y".to_owned(), ("b".to_owned(), "T-3".to_owned())),
            ])
        );
    }

    fn working(name: &str) -> Line {
        Line {
            assignee: Some(
                json!({"kind":"agent","name":name,"id":"20000000-0000-4000-8000-000000000001"}),
            ),
            ..line("T-1", "in-progress", None)
        }
    }

    #[test]
    fn a_row_names_three_assignees_without_their_ids_and_counts_the_rest() {
        let tasks: Vec<Line> = ["a", "b", "c", "d", "e"]
            .iter()
            .enumerate()
            .map(|(n, name)| Line {
                id: format!("T-{}", n + 1),
                ..working(name)
            })
            .collect();
        let found = found(vec![
            ("p", page(tasks)),
            ("empty", None),
            ("one", page(vec![working("solo")])),
        ]);
        let graph = Graph(&found);
        assert_eq!(
            graph.row("p").unwrap(),
            json!({"stage":"in-progress","done":0,"total":5,"signoff":0,
                   "working":[{"kind":"agent","name":"a"},{"kind":"agent","name":"b"},
                              {"kind":"agent","name":"c"}],
                   "more":2})
        );
        let one = graph.row("one").unwrap();
        assert_eq!(one["working"], json!([{"kind":"agent","name":"solo"}]));
        assert!(one.get("more").is_none());
        // No work, and a page outside the index, have no row.
        assert_eq!(graph.row("empty"), None);
        assert_eq!(graph.row("gone"), None);
    }

    #[test]
    fn a_row_shows_the_stage_of_a_followed_page_and_no_per_task_states() {
        let found = found(vec![
            ("a", page(vec![line("T-1", "todo", Some("b"))])),
            ("b", page(vec![line("T-1", "done", None)])),
        ]);
        let row = Graph(&found).row("a").unwrap();
        assert_eq!(row["stage"], "done");
        assert_eq!(row["done"], 1);
        assert!(row.get("states").is_none());
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
        // The same task is not a second parent, so its retry is admitted.
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
    #[test]
    fn links_nest_without_a_depth_limit_but_never_loop_or_take_a_second_parent() {
        // a > b > c, and d is free.
        let found = found(vec![
            ("a", page(vec![line("T-1", "todo", Some("b"))])),
            ("b", page(vec![line("T-1", "todo", Some("c"))])),
            ("c", None),
            ("d", None),
        ]);
        let graph = Graph(&found);
        for id in ["a", "b", "c", "d"] {
            assert!(graph.warnings(id).is_empty(), "{id}");
        }
        // A retry of the same link, a sibling, a great-grandchild and a page with children below
        // a free page are all admitted.
        assert_eq!(graph.refusal("a", "T-1", "b"), None);
        assert_eq!(graph.refusal("a", "T-2", "d"), None);
        assert_eq!(graph.refusal("c", "T-1", "d"), None);
        assert_eq!(graph.refusal("d", "T-1", "a"), None);
        // A loop through any depth and a second parent stay refused.
        assert_eq!(graph.refusal("c", "T-1", "a"), Some(Refusal::Loop));
        assert_eq!(graph.refusal("b", "T-2", "a"), Some(Refusal::Loop));
        assert_eq!(
            graph.refusal("d", "T-1", "c"),
            Some(Refusal::HasParent {
                page: "b".into(),
                task: "T-1".into()
            })
        );
        // A sub-page may make a new sub-page of its own.
        assert_eq!(graph.conversion_refusal("a"), None);
        assert_eq!(graph.conversion_refusal("c"), None);
    }

    #[test]
    fn legacy_diagnostics_keep_every_claim_and_do_not_freeze_unrelated_components() {
        for (edges, warning) in [
            (vec![("a", "c"), ("b", "c")], "subpage-multiple-parents"),
            (vec![("a", "b"), ("b", "a")], "subpage-cycle"),
        ] {
            let mut found = found(vec![
                ("a", None),
                ("b", None),
                ("c", None),
                ("d", None),
                ("e", None),
            ]);
            for (parent, child) in edges {
                found.insert(parent.into(), page(vec![line("T-1", "todo", Some(child))]));
            }
            let graph = Graph(&found);
            for id in ["a", "b"] {
                assert!(graph.warnings(id).contains(&warning));
            }
            assert_eq!(
                graph.refusal("a", "T-2", "d"),
                Some(Refusal::InvalidHierarchy)
            );
            assert_eq!(graph.refusal("d", "T-1", "e"), None);
            assert_eq!(
                graph.conversion_refusal("a"),
                Some(Refusal::InvalidHierarchy)
            );
            assert!(graph.warnings("d").is_empty());
            // Readers still resolve old links, including a cycle, and never change their inputs.
            let before = found.clone();
            graph.derived("a");
            graph.parent("b");
            assert_eq!(found, before);
        }
        let found = found(vec![
            (
                "a",
                page(vec![
                    line("T-1", "todo", Some("b")),
                    line("T-2", "todo", Some("b")),
                ]),
            ),
            ("b", None),
        ]);
        assert!(
            Graph(&found)
                .warnings("b")
                .contains(&"subpage-multiple-parents")
        );
    }
}
