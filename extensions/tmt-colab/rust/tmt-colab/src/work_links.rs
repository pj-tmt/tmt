//! Pages under pages (contract: Sub-pages). A page lists its direct sub-pages; one page's value
//! cannot see another's, so this is read over what the work index holds for the whole space: each
//! page's parent, its progress, and the rules `subpage.attach` is admitted by (one parent, no loop,
//! an available page; any depth). These are the write owner's rules, never a rule of the stored
//! value, so a race of two writers can leave pages the rules would refuse; readers then resolve
//! them the same way every time and never loop.
use crate::work_index::Found;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Why a page may not become a sub-page of another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The page itself.
    Itself,
    /// Not a page of this space, deleted or unreadable.
    Unavailable,
    /// Already a sub-page of the page named.
    HasParent { page: String },
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

/// What the work index holds for the pages of the space: a page absent from it is unavailable.
pub struct Graph<'a>(pub &'a Found);

impl Graph<'_> {
    fn children(&self, page: &str) -> &[String] {
        match self.0.get(page) {
            Some(Some(work)) => &work.children,
            _ => &[],
        }
    }

    /// Whether the page still holds task-era work the serve has not migrated (`work-not-migrated`).
    pub fn legacy(&self, page: &str) -> bool {
        matches!(self.0.get(page), Some(Some(work)) if work.legacy)
    }

    /// The explicit page state, only from current known entries.
    pub fn page_state(&self, page: &str) -> Option<&str> {
        self.0.get(page)?.as_ref().map(|work| work.state.as_str())
    }

    /// The page's direct sub-pages counted by their own manual state; a sub-page outside the index
    /// is unknown.
    pub fn progress(&self, page: &str) -> Option<crate::page_state::Progress> {
        let work = self.0.get(page)?.as_ref()?;
        Some(crate::page_state::progress(&work.children, &|child| {
            self.page_state(child).map(str::to_owned)
        }))
    }

    /// The list row's `work` for a page with sub-pages: its stage and direct progress, the same
    /// count the page's work panel shows. `None` for a page without sub-pages or outside the index.
    pub fn row(&self, page: &str) -> Option<Value> {
        let progress = self.progress(page).filter(|progress| progress.total > 0)?;
        Some(json!({"stage":progress.stage(),"done":progress.done,"total":progress.total}))
    }

    /// The parent of each listed page and the page's position in that parent's list: the lowest
    /// page ID when several list it (two writers raced). `child -> (parent, index)`.
    pub fn parents(&self) -> BTreeMap<String, (String, usize)> {
        let mut found = BTreeMap::new();
        // Pages ascend, so the first page to list a child keeps it.
        for (page, work) in self.0 {
            for (index, child) in work
                .iter()
                .flat_map(|work| work.children.iter().enumerate())
            {
                found
                    .entry(child.clone())
                    .or_insert_with(|| (page.clone(), index));
            }
        }
        found
    }

    /// The parent of `child`, if any (see `parents`).
    pub fn parent(&self, child: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(_, work)| {
                work.iter()
                    .any(|work| work.children.iter().any(|listed| listed == child))
            })
            .map(|(page, _)| page.clone())
    }

    /// The pages below `page`, at every level.
    fn descendants(&self, page: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut open = vec![page.to_owned()];
        while let Some(next) = open.pop() {
            for child in self.children(&next) {
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
            for child in work.iter().flat_map(|work| &work.children) {
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
            .map(|id| (*id, parents.get(id).map_or(0, Vec::len)))
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

    /// Creating a sub-page adds a new child, so the source page is checked before the page is made.
    pub fn creation_refusal(&self, page: &str) -> Option<Refusal> {
        (!self.warnings(page).is_empty()).then_some(Refusal::InvalidHierarchy)
    }

    /// Why `child` may not become a sub-page of `page`, if it may not. A page already under
    /// `page` is not a second parent, so its retry is admitted.
    pub fn refusal(&self, page: &str, child: &str) -> Option<Refusal> {
        if child == page {
            return Some(Refusal::Itself);
        }
        if !self.0.contains_key(child) {
            return Some(Refusal::Unavailable);
        }
        if let Some(parent) = self.parent(child).filter(|parent| parent != page) {
            return Some(Refusal::HasParent { page: parent });
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
    use crate::work_index::PageWork;

    fn page(state: &str, children: &[&str]) -> Option<PageWork> {
        Some(PageWork {
            state: state.into(),
            children: children.iter().map(|id| (*id).to_owned()).collect(),
            legacy: false,
        })
    }
    fn found(pages: Vec<(&str, Option<PageWork>)>) -> Found {
        pages
            .into_iter()
            .map(|(id, work)| (id.to_owned(), work))
            .collect()
    }

    #[test]
    fn progress_and_the_row_count_direct_sub_pages_by_their_own_state() {
        let found = found(vec![
            ("a", page("todo", &["b", "c", "gone"])),
            ("b", page("done", &["d"])),
            ("c", page("in-progress", &[])),
            ("d", page("todo", &[])),
        ]);
        let graph = Graph(&found);
        let progress = graph.progress("a").unwrap();
        assert_eq!(
            (
                progress.done,
                progress.in_progress,
                progress.unknown,
                progress.total
            ),
            (1, 1, 1, 3)
        );
        assert_eq!(
            graph.row("a"),
            Some(json!({"stage":"in-progress","done":1,"total":3}))
        );
        // Only direct sub-pages count: b's own state is done, but its sub-page d is not.
        assert_eq!(
            graph.row("b"),
            Some(json!({"stage":"in-progress","done":0,"total":1}))
        );
        assert_eq!(graph.row("c"), None);
        assert_eq!(graph.progress("gone"), None);
    }

    #[test]
    fn the_first_page_by_id_is_the_parent_when_two_list_one() {
        let found = found(vec![
            ("b", page("todo", &["c"])),
            ("a", page("todo", &["c"])),
            ("c", page("todo", &[])),
        ]);
        let graph = Graph(&found);
        assert_eq!(graph.parent("c").as_deref(), Some("a"));
        assert_eq!(
            graph.parents(),
            BTreeMap::from([("c".into(), ("a".into(), 0))])
        );
        assert_eq!(graph.parent("a"), None);
    }

    #[test]
    fn links_nest_at_any_depth_but_never_loop_or_take_a_second_parent() {
        // a > b > c, and d is free.
        let found = found(vec![
            ("a", page("todo", &["b"])),
            ("b", page("todo", &["c"])),
            ("c", page("todo", &[])),
            ("d", page("todo", &[])),
        ]);
        let graph = Graph(&found);
        for id in ["a", "b", "c", "d"] {
            assert!(graph.warnings(id).is_empty(), "{id}");
        }
        assert_eq!(graph.refusal("a", "a"), Some(Refusal::Itself));
        assert_eq!(graph.refusal("a", "gone"), Some(Refusal::Unavailable));
        // A retry of the same link, a sibling, a great-grandchild and a page with children below
        // a free page are all admitted.
        assert_eq!(graph.refusal("a", "b"), None);
        assert_eq!(graph.refusal("a", "d"), None);
        assert_eq!(graph.refusal("c", "d"), None);
        assert_eq!(graph.refusal("d", "a"), None);
        // A loop through any depth and a second parent stay refused.
        assert_eq!(graph.refusal("c", "a"), Some(Refusal::Loop));
        assert_eq!(graph.refusal("b", "a"), Some(Refusal::Loop));
        assert_eq!(
            graph.refusal("d", "c"),
            Some(Refusal::HasParent { page: "b".into() })
        );
        assert_eq!(graph.creation_refusal("a"), None);
        assert_eq!(graph.creation_refusal("c"), None);
    }

    #[test]
    fn stored_conflicts_are_diagnosed_and_freeze_only_their_own_component() {
        for (edges, warning) in [
            (vec![("a", "c"), ("b", "c")], "subpage-multiple-parents"),
            (vec![("a", "b"), ("b", "a")], "subpage-cycle"),
        ] {
            let mut found = found(
                ["a", "b", "c", "d", "e"]
                    .map(|id| (id, page("todo", &[])))
                    .to_vec(),
            );
            for (parent, child) in edges {
                let work = found.get_mut(parent).unwrap().as_mut().unwrap();
                work.children.push(child.into());
            }
            let graph = Graph(&found);
            for id in ["a", "b"] {
                assert!(graph.warnings(id).contains(&warning));
            }
            assert_eq!(graph.refusal("a", "d"), Some(Refusal::InvalidHierarchy));
            assert_eq!(graph.refusal("d", "e"), None);
            assert_eq!(graph.creation_refusal("a"), Some(Refusal::InvalidHierarchy));
            assert!(graph.warnings("d").is_empty());
            // Readers still resolve stored links, including a cycle, and never change their inputs.
            let before = found.clone();
            graph.progress("a");
            graph.parent("b");
            assert_eq!(found, before);
        }
    }
}
