//! Manual page state and the progress derived from a page's direct sub-pages.
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
    /// A cached projection is untrusted and must still satisfy the sub-page bound.
    pub fn valid(&self) -> bool {
        self.total as usize <= crate::work::SUBPAGES
            && self
                .done
                .checked_add(self.in_progress)
                .and_then(|n| n.checked_add(self.todo))
                .and_then(|n| n.checked_add(self.unknown))
                == Some(self.total)
    }
    /// The page list's stage: `discussion` without sub-pages, `done` when every one is done,
    /// `in-progress` otherwise (an unknown sub-page is not done).
    pub fn stage(&self) -> &'static str {
        match (self.total, self.done) {
            (0, _) => "discussion",
            (total, done) if total == done => "done",
            _ => "in-progress",
        }
    }
}

/// Count direct sub-pages by their manual state, each once; an inaccessible one is unknown.
pub fn progress(children: &[String], child_state: &dyn Fn(&str) -> Option<String>) -> Progress {
    let mut result = Progress::default();
    let mut seen = BTreeSet::new();
    for child in children {
        if !seen.insert(child) {
            continue;
        }
        result.total += 1;
        // A sub-page supplies only the three page states. Missing or stale data is unknown.
        match child_state(child).as_deref() {
            Some("done") => result.done += 1,
            Some("in-progress") => result.in_progress += 1,
            Some("todo") => result.todo += 1,
            _ => result.unknown += 1,
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_counts_each_sub_page_once_and_reports_unknown() {
        let children: Vec<String> = ["a", "b", "c", "a", "missing", "odd"]
            .map(str::to_owned)
            .to_vec();
        let actual = progress(&children, &|id| match id {
            "a" => Some("done".to_owned()),
            "b" => Some("in-progress".to_owned()),
            "c" => Some("todo".to_owned()),
            "odd" => Some("blocked".to_owned()),
            _ => None,
        });
        assert_eq!(
            actual,
            Progress {
                done: 1,
                in_progress: 1,
                todo: 1,
                unknown: 2,
                total: 5
            }
        );
        assert_eq!(actual.stage(), "in-progress");
        assert_eq!(progress(&[], &|_| None), Progress::default());
        assert_eq!(Progress::default().stage(), "discussion");
        let done = progress(&children[..1], &|_| Some("done".to_owned()));
        assert_eq!(done.stage(), "done");
    }
}
