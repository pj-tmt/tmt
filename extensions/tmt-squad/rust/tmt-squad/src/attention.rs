//! A squad's attention: how many members wait on the user and how many are
//! blocked, from its status document. The board colors each tab by it and
//! `ls --json` reports it, so color is never the only carrier (#507).

use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Attention {
    /// Members that owe the user a decision (`pending`, ◆) or sent a request
    /// that waits on the user.
    pub waiting: usize,
    /// Members in the `blocked` state.
    pub blocked: usize,
}

impl Attention {
    /// The lead and every section row, each member counted once.
    pub fn of(document: &Value) -> Self {
        let mut rows: Vec<&Value> = Vec::new();
        if document["squad"]["lead"].is_object() {
            rows.push(&document["squad"]["lead"]);
        }
        rows.extend(
            document["sections"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|section| section["rows"].as_array().into_iter().flatten()),
        );
        let (mut waiting, mut blocked) = (BTreeSet::new(), BTreeSet::new());
        for row in rows {
            let Some(id) = row["id"].as_str().or_else(|| row["name"].as_str()) else {
                continue;
            };
            let asks = row["waitingOnYou"]
                .as_array()
                .is_some_and(|requests| !requests.is_empty());
            if row["pending"].is_string() || asks {
                waiting.insert(id);
            }
            if row["state"] == "blocked" {
                blocked.insert(id);
            }
        }
        Self {
            waiting: waiting.len(),
            blocked: blocked.len(),
        }
    }

    /// `waiting` over `blocked` over `normal`: what the tab most needs.
    pub fn state(self) -> &'static str {
        if self.waiting > 0 {
            "waiting"
        } else if self.blocked > 0 {
            "blocked"
        } else {
            "normal"
        }
    }

    pub fn document(self) -> Value {
        json!({"state": self.state(), "waiting": self.waiting, "blocked": self.blocked})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_wins_over_blocked_and_each_member_counts_once() {
        let document = json!({
            "squad": {"lead": {"id": "L", "name": "sol", "pending": "approve", "state": null}},
            "sections": [
                {"rows": [
                    {"id": "A", "name": "ann", "pending": null, "state": "blocked", "waitingOnYou": []},
                    {"id": "B", "name": "bob", "pending": null, "state": "working",
                     "waitingOnYou": [{"requestId": "q1"}]},
                ]},
                // A member in a second user section is the same person.
                {"rows": [{"id": "A", "name": "ann", "pending": null, "state": "blocked"}]},
            ],
        });
        let attention = Attention::of(&document);
        assert_eq!(
            attention,
            Attention {
                waiting: 2,
                blocked: 1
            }
        );
        assert_eq!(
            attention.document(),
            json!({"state": "waiting", "waiting": 2, "blocked": 1})
        );
        assert_eq!(
            Attention {
                waiting: 0,
                blocked: 3
            }
            .state(),
            "blocked"
        );
        assert_eq!(Attention::of(&json!({"sections": []})).state(), "normal");
        assert_eq!(Attention::of(&json!({})), Attention::default());
    }
}
