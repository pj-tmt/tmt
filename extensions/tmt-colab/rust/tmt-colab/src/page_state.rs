//! Manual page state and derived progress for its own direct work items.
use crate::work::Line;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Validate an explicit state change before any publication is prepared.
pub(crate) fn validate_action(action: &str, body: &serde_json::Value) -> crate::closed::Result<()> {
    crate::closed::require(action == "page.state")?;
    crate::closed::keys(body, &["state"], &[])?;
    crate::closed::require(crate::decoder::valid_page_state(&body["state"]))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Progress {
    pub done: u32,
    pub in_progress: u32,
    pub todo: u32,
    pub unknown: u32,
    pub total: u32,
}

impl Progress {
    /// A cached projection is untrusted and must still satisfy the page's task bound.
    pub fn valid(&self) -> bool {
        self.total as usize <= crate::work::TASK_LIMIT
            && self
                .done
                .checked_add(self.in_progress)
                .and_then(|n| n.checked_add(self.todo))
                .and_then(|n| n.checked_add(self.unknown))
                == Some(self.total)
    }
}

/// Count direct task/child items; inaccessible children are unknown.
pub fn progress(lines: &[Line], child_state: &dyn Fn(&str) -> Option<String>) -> Progress {
    let mut result = Progress::default();
    let mut children = BTreeSet::new();
    for line in lines {
        let state = if let Some(child) = &line.child {
            if !children.insert(child) {
                continue;
            }
            // A child supplies only the three page states. Missing/stale data is unknown.
            child_state(child)
                .filter(|state| matches!(state.as_str(), "todo" | "in-progress" | "done"))
        } else {
            Some(line.state.clone())
        };
        result.total += 1;
        match state.as_deref() {
            Some("done") => result.done += 1,
            Some("in-progress" | "blocked" | "in-review") => result.in_progress += 1,
            Some("todo") => result.todo += 1,
            _ => result.unknown += 1,
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(id: &str, state: &str, child: Option<&str>) -> Line {
        Line {
            id: id.into(),
            title: id.into(),
            state: state.into(),
            signoff: false,
            assignee: None,
            child: child.map(str::to_owned),
        }
    }
    #[test]
    fn progress_counts_each_child_once_and_reports_unknown() {
        let lines = [
            line("T-1", "done", None),
            line("T-2", "blocked", None),
            line("T-3", "todo", None),
            line("T-4", "todo", Some("child")),
            line("T-5", "todo", Some("child")),
            line("T-6", "done", Some("missing")),
        ];
        let actual = progress(&lines, &|id| (id == "child").then(|| "done".to_owned()));
        assert_eq!(
            actual,
            Progress {
                done: 2,
                in_progress: 1,
                todo: 1,
                unknown: 1,
                total: 5
            }
        );
        assert_eq!(progress(&[], &|_| None), Progress::default());
    }
}
