//! The work-in-progress limit: the write owner's admission rule over the whole space, never a rule of
//! the stored value (`work.rs`), so no published state is ever invalid for it. An agent holds a task
//! while it is assigned to the agent and is `todo` or `in-progress`; the limit bounds how many an
//! agent holds across every non-archived page. Only a transition that takes a task into the agent's
//! hands is checked: holding more than the limit after a lowered limit or a remote write is never an
//! error, and a transition that does not add a held task is always admitted.
use serde_json::Value;
use std::collections::BTreeMap;

/// The limit of an agent with no setting of its own and no space default.
pub const DEFAULT_LIMIT: u32 = 1;
/// Larger limits are not a limit.
pub const MAX_LIMIT: u32 = 1000;

/// A held task: which agent, on which page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub agent: String,
    pub page: String,
    pub task: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    /// The space default; `None` is `DEFAULT_LIMIT`.
    pub default: Option<u32>,
    pub agents: BTreeMap<String, u32>,
}
impl Limits {
    pub fn of(&self, agent: &str) -> u32 {
        self.agents
            .get(agent)
            .copied()
            .or(self.default)
            .unwrap_or(DEFAULT_LIMIT)
    }
}

/// The tasks an agent holds in one page's work, in task order.
pub fn held(page: &str, work: &Value) -> Vec<Held> {
    let (Some(order), Some(tasks)) = (work["order"].as_array(), work["tasks"].as_object()) else {
        return Vec::new();
    };
    order
        .iter()
        .filter_map(|id| {
            let id = id.as_str()?;
            let task = tasks.get(id)?;
            let assignee = &task["assignee"];
            let in_hand = matches!(task["state"].as_str(), Some("todo" | "in-progress"));
            (in_hand && assignee["kind"] == "agent").then(|| Held {
                agent: assignee["name"].as_str().unwrap_or_default().to_owned(),
                page: page.to_owned(),
                task: id.to_owned(),
            })
        })
        .collect()
}

/// An agent that would hold more than its limit: every task it would hold, the new one included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub agent: String,
    pub limit: u32,
    pub held: Vec<Held>,
}

/// Admits a transition of one page's work, or names the agent it would put over its limit.
/// `elsewhere` is what every other page holds; `before` and `after` are this page's work.
pub fn admit(
    page: &str,
    before: &Value,
    after: &Value,
    elsewhere: &[Held],
    limits: &Limits,
) -> Option<Refusal> {
    let was = held(page, before);
    let now = held(page, after);
    let mut agents: Vec<&str> = Vec::new();
    for new in now.iter().filter(|task| !was.contains(task)) {
        if !agents.contains(&new.agent.as_str()) {
            agents.push(&new.agent);
        }
    }
    agents.into_iter().find_map(|agent| {
        let all: Vec<Held> = elsewhere
            .iter()
            .chain(&now)
            .filter(|task| task.agent == agent)
            .cloned()
            .collect();
        let limit = limits.of(agent);
        (all.len() > usize::try_from(limit).unwrap_or(usize::MAX)).then(|| Refusal {
            agent: agent.to_owned(),
            limit,
            held: all,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn work(tasks: &[(&str, &str, Option<&str>)]) -> Value {
        let mut map = serde_json::Map::new();
        let mut order = Vec::new();
        for (id, state, agent) in tasks {
            let mut task = json!({"title":"t","state":state});
            if *state == "blocked" {
                task["reason"] = json!("waiting");
            }
            if let Some(name) = agent {
                task["assignee"] = json!({"kind":"agent","name":name});
            }
            map.insert((*id).into(), task);
            order.push(json!(id));
        }
        json!({"version":1,"next":9,"tasks":map,"order":order})
    }

    #[test]
    fn a_task_is_held_when_assigned_to_an_agent_and_todo_or_in_progress() {
        let page = work(&[
            ("T-1", "todo", Some("ada")),
            ("T-2", "in-progress", Some("ada")),
            ("T-3", "blocked", Some("ada")),
            ("T-4", "in-review", Some("ada")),
            ("T-5", "done", Some("ada")),
            ("T-6", "todo", None),
        ]);
        let tasks: Vec<_> = held("p", &page).into_iter().map(|h| h.task).collect();
        assert_eq!(tasks, ["T-1", "T-2"]);
        let mut person = work(&[("T-1", "todo", None)]);
        person["tasks"]["T-1"]["assignee"] = json!({"kind":"person","name":"ben"});
        assert!(held("p", &person).is_empty());
    }

    #[test]
    fn a_transition_that_adds_a_held_task_is_checked_against_the_whole_space() {
        let limits = Limits::default();
        let before = work(&[("T-1", "done", Some("ada"))]);
        let after = work(&[("T-1", "in-progress", Some("ada"))]);
        assert_eq!(admit("p", &before, &after, &[], &limits), None);
        let elsewhere = [Held {
            agent: "ada".into(),
            page: "q".into(),
            task: "T-7".into(),
        }];
        let refusal = admit("p", &before, &after, &elsewhere, &limits).unwrap();
        assert_eq!((refusal.agent.as_str(), refusal.limit), ("ada", 1));
        assert_eq!(refusal.held.len(), 2);
        // Another agent's tasks do not count.
        let other = [Held {
            agent: "bo".into(),
            page: "q".into(),
            task: "T-7".into(),
        }];
        assert_eq!(admit("p", &before, &after, &other, &limits), None);
    }

    #[test]
    fn a_transition_that_adds_nothing_is_admitted_even_over_the_limit() {
        let over = [
            Held {
                agent: "ada".into(),
                page: "q".into(),
                task: "T-1".into(),
            },
            Held {
                agent: "ada".into(),
                page: "q".into(),
                task: "T-2".into(),
            },
        ];
        let before = work(&[("T-1", "in-progress", Some("ada"))]);
        let finished = work(&[("T-1", "done", Some("ada"))]);
        let retitled = work(&[("T-1", "in-progress", Some("ada"))]);
        let limits = Limits::default();
        assert_eq!(admit("p", &before, &finished, &over, &limits), None);
        assert_eq!(admit("p", &before, &retitled, &over, &limits), None);
    }

    #[test]
    fn a_limit_is_per_agent_then_the_space_default_then_one() {
        let mut limits = Limits::default();
        assert_eq!(limits.of("ada"), 1);
        limits.default = Some(3);
        limits.agents.insert("bo".into(), 2);
        assert_eq!((limits.of("ada"), limits.of("bo")), (3, 2));
        let before = work(&[]);
        let after = work(&[
            ("T-1", "todo", Some("ada")),
            ("T-2", "todo", Some("ada")),
            ("T-3", "todo", Some("ada")),
        ]);
        assert_eq!(admit("p", &before, &after, &[], &limits), None);
        limits.default = Some(2);
        assert_eq!(
            admit("p", &before, &after, &[], &limits).unwrap().agent,
            "ada"
        );
    }
}
