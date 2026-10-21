//! Structured work values and trial actions: an ordinary page's summary and task list. No Yjs,
//! storage, target lookup or authority. The admitted writer supplies attribution, and the
//! work-in-progress limit is the write owner's admission rule over the whole space, never a rule
//! of this value, so no published state is ever invalid for it (contract: Work).
use crate::closed::{
    Invalid, canonical, id, integer, keys, multiline, nonempty, object, plain, require, text,
};
use crate::summary_lines;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use tmt_colab_model::values;

use crate::limits::{
    WORK_ACCEPTANCE, WORK_CRITERION_BYTES, WORK_DETAIL_BYTES, WORK_EXTERNAL_ID_BYTES, WORK_ID_MAX,
    WORK_NAME_BYTES, WORK_REASON_BYTES, WORK_RESULT_BYTES, WORK_TITLE_POINTS,
};
pub use crate::limits::{WORK_BYTES as DOCUMENT_BYTES, WORK_TASKS as TASK_LIMIT};

type Result<T> = crate::closed::Result<T>;

/// The five task states, in the order the page lists them.
pub const STATES: [&str; 5] = ["todo", "in-progress", "blocked", "in-review", "done"];

/// A task ID is `T-` and a decimal number from 1, without a leading zero.
pub fn task_number(value: &str) -> Option<u64> {
    let digits = value.strip_prefix("T-")?;
    if digits.is_empty()
        || digits.len() > 7
        || digits.starts_with('0')
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    digits
        .parse()
        .ok()
        .filter(|number| (1..=WORK_ID_MAX).contains(number))
}
fn task_id(value: &Value) -> Result<&str> {
    let value = text(value)?;
    task_number(value).ok_or(Invalid)?;
    Ok(value)
}
fn title(value: &Value) -> Result<()> {
    let title = plain(value, WORK_TITLE_POINTS * 4)?;
    require(!title.is_empty() && title.chars().count() <= WORK_TITLE_POINTS)
}
fn assignee(value: &Value) -> Result<()> {
    match text(&value["kind"])? {
        "agent" => {
            keys(value, &["kind", "name"], &["id"])?;
            require(crate::decoder::valid_publisher_agent(text(&value["name"])?))?;
            if let Some(id) = value.get("id") {
                values::core_id(text(id)?).map_err(|_| Invalid)?;
            }
            Ok(())
        }
        "person" => {
            keys(value, &["kind", "name"], &[])?;
            nonempty(&value["name"], WORK_NAME_BYTES).map(drop)
        }
        _ => Err(Invalid),
    }
}
fn criterion(value: &Value) -> Result<()> {
    match text(&value["kind"])? {
        "test" => {
            keys(value, &["kind", "name"], &[])?;
            nonempty(&value["name"], WORK_CRITERION_BYTES).map(drop)
        }
        "check" => {
            keys(value, &["kind", "command"], &["result"])?;
            nonempty(&value["command"], WORK_CRITERION_BYTES)?;
            value.get("result").map_or(Ok(()), |result| {
                nonempty(result, WORK_RESULT_BYTES).map(drop)
            })
        }
        "person" => {
            keys(value, &["kind", "who"], &[])?;
            nonempty(&value["who"], WORK_NAME_BYTES).map(drop)
        }
        _ => Err(Invalid),
    }
}
/// The fields a person or agent edits: everything except state, reason and assignee.
const EDITABLE: [&str; 5] = [
    "title",
    "detail",
    "acceptance",
    "needsSignoff",
    "externalId",
];
fn editable(value: &Value) -> Result<()> {
    title(&value["title"])?;
    if let Some(detail) = value.get("detail") {
        multiline(detail, WORK_DETAIL_BYTES)?;
    }
    if let Some(list) = value.get("acceptance") {
        let list = list.as_array().ok_or(Invalid)?;
        require(!list.is_empty() && list.len() <= WORK_ACCEPTANCE)?;
        list.iter().try_for_each(criterion)?;
    }
    if let Some(flag) = value.get("needsSignoff") {
        require(flag == &Value::Bool(true))?;
    }
    if let Some(id) = value.get("externalId") {
        nonempty(id, WORK_EXTERNAL_ID_BYTES)?;
    }
    Ok(())
}
pub fn validate_task(value: &Value) -> Result<()> {
    keys(
        value,
        &["title", "state"],
        &[
            "detail",
            "reason",
            "assignee",
            "acceptance",
            "needsSignoff",
            "externalId",
            "child",
        ],
    )?;
    editable(value)?;
    // A task links to a page or to an external issue, never both: its state has one owner.
    if let Some(child) = value.get("child") {
        id(child)?;
        require(value.get("externalId").is_none())?;
    }
    let state = text(&value["state"])?;
    require(STATES.contains(&state))?;
    // A blocked task says why, and only a blocked task does.
    match value.get("reason") {
        Some(reason) => {
            require(state == "blocked")?;
            nonempty(reason, WORK_REASON_BYTES)?;
        }
        None => require(state != "blocked")?,
    }
    value.get("assignee").map_or(Ok(()), assignee)
}
/// Measures the admitted value, without retained Yjs history or transport framing.
pub fn validate(value: &Value) -> Result<()> {
    keys(value, &["version", "next", "tasks", "order"], &["summary"])?;
    require(integer(&value["version"])? == 1)?;
    let next = integer(&value["next"])?;
    require((1..=WORK_ID_MAX + 1).contains(&next))?;
    let tasks = object(&value["tasks"])?;
    let order = value["order"].as_array().ok_or(Invalid)?;
    require(tasks.len() <= TASK_LIMIT && order.len() == tasks.len())?;
    for (key, task) in tasks {
        // Every task number is below `next`: a removed number is never handed out again.
        require(task_number(key).ok_or(Invalid)? < next)?;
        validate_task(task)?;
    }
    let mut seen = BTreeSet::new();
    for entry in order {
        let key = task_id(entry)?;
        require(tasks.contains_key(key) && seen.insert(key))?;
    }
    if let Some(summary) = value.get("summary") {
        summary_lines::validate(summary)?;
    }
    require(canonical(value)?.len() <= DOCUMENT_BYTES)
}
/// A page that has had no work yet carries no `meta.work`; its first action starts from this.
pub fn empty() -> Value {
    json!({"version":1,"next":1,"tasks":{},"order":[]})
}
/// Produces a detached result or refuses the whole action. Does not mutate the supplied base.
/// `attribution` comes from the write owner, never from the action body. The work-in-progress
/// limit is not checked here: the write owner admits an assignment before it prepares one.
pub fn apply(base: &Value, action: &str, body: &Value, attribution: &Value) -> Result<Value> {
    validate(base)?;
    let mut next = base.clone();
    match action {
        "summary.set" => next["summary"] = summary_lines::set(body, attribution)?,
        "summary.clear" => {
            keys(body, &[], &[])?;
            next.as_object_mut().ok_or(Invalid)?.remove("summary");
        }
        "task.add" => {
            keys(body, &["task"], &[])?;
            let task = &body["task"];
            keys(
                task,
                &["title"],
                &[
                    "detail",
                    "assignee",
                    "acceptance",
                    "needsSignoff",
                    "externalId",
                ],
            )?;
            let number = integer(&base["next"])?;
            require(number <= WORK_ID_MAX)?;
            let key = format!("T-{number}");
            let mut stored = task.clone();
            stored["state"] = json!("todo");
            next["tasks"][&key] = stored;
            next["order"]
                .as_array_mut()
                .ok_or(Invalid)?
                .push(Value::String(key));
            next["next"] = json!(number + 1);
        }
        "task.set" | "task.state" | "task.assign" | "task.link" | "task.unlink" | "task.remove" => {
            let (required, optional): (&[&str], &[&str]) = match action {
                "task.set" => (
                    &["taskId", "title"],
                    &["detail", "acceptance", "needsSignoff", "externalId"],
                ),
                "task.state" => (&["taskId", "state"], &["reason"]),
                "task.assign" => (&["taskId"], &["assignee"]),
                "task.link" => (&["taskId", "pageId"], &[]),
                _ => (&["taskId"], &[]),
            };
            keys(body, required, optional)?;
            let key = task_id(&body["taskId"])?;
            let task = next["tasks"].get_mut(key).ok_or(Invalid)?;
            match action {
                "task.set" => {
                    // The whole editable part is replaced: a field the body omits is cleared.
                    editable(body)?;
                    let fields = task.as_object_mut().ok_or(Invalid)?;
                    for field in EDITABLE {
                        match body.get(field) {
                            Some(value) => fields.insert(field.into(), value.clone()),
                            None => fields.remove(field),
                        };
                    }
                }
                "task.state" => {
                    // A linked task's state follows its page: nothing sets it.
                    require(task.get("child").is_none())?;
                    let fields = task.as_object_mut().ok_or(Invalid)?;
                    fields.insert("state".into(), body["state"].clone());
                    match body.get("reason") {
                        Some(reason) => fields.insert("reason".into(), reason.clone()),
                        None => fields.remove("reason"),
                    };
                }
                "task.assign" => {
                    let fields = task.as_object_mut().ok_or(Invalid)?;
                    match body.get("assignee") {
                        Some(value) => fields.insert("assignee".into(), value.clone()),
                        None => fields.remove("assignee"),
                    };
                }
                "task.link" => {
                    let child = id(&body["pageId"])?;
                    let fields = task.as_object_mut().ok_or(Invalid)?;
                    // Another page replaces nothing: the task is unlinked first.
                    require(fields.get("child").is_none_or(|now| now == child))?;
                    fields.insert("child".into(), json!(child));
                }
                "task.unlink" => {
                    task.as_object_mut().ok_or(Invalid)?.remove("child");
                }
                _ => {
                    next["tasks"].as_object_mut().ok_or(Invalid)?.remove(key);
                    next["order"]
                        .as_array_mut()
                        .ok_or(Invalid)?
                        .retain(|value| value.as_str() != Some(key));
                }
            }
        }
        _ => return Err(Invalid),
    }
    validate(&next)?;
    Ok(next)
}

/// Where a page stands, computed from its tasks and never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    /// `discussion` with no task, `in-progress` while any is not done, `done` when all are.
    pub stage: &'static str,
    pub done: usize,
    pub total: usize,
    /// The assignees of in-progress tasks, each once, in task order.
    pub working: Vec<Value>,
    /// Flagged tasks waiting in review. The flag is display only until the owner can act on it.
    pub signoff: usize,
    /// What each task that follows a page shows, when that page's stage is known; the others show
    /// their own state.
    pub states: BTreeMap<String, &'static str>,
}
/// What a task shows while its page is at `stage`: nothing yet is `todo`, work under way is
/// `in-progress`, and a finished page is `done`, or `in-review` when the task waits for sign-off.
pub fn linked_state(stage: &str, needs_signoff: bool) -> Option<&'static str> {
    match stage {
        "discussion" => Some("todo"),
        "in-progress" => Some("in-progress"),
        "done" if needs_signoff => Some("in-review"),
        "done" => Some("done"),
        _ => None,
    }
}
pub fn derived(work: &Value) -> Result<Derived> {
    derived_with(work, &|_| None)
}
/// `derived` where `stage_of` answers a linked page's stage. A page it cannot answer for (deleted,
/// unreadable) leaves its task showing its own state.
pub fn derived_with(work: &Value, stage_of: &dyn Fn(&str) -> Option<String>) -> Result<Derived> {
    validate(work)?;
    Ok(derive(&lines(work)?, stage_of))
}
/// One task as the derived values read it: everything a page's stage, progress, workers and sign-offs
/// are computed from, so a page's derived values can be recomputed without its whole value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub id: String,
    pub state: String,
    pub signoff: bool,
    pub assignee: Option<Value>,
    pub child: Option<String>,
}
pub fn lines(work: &Value) -> Result<Vec<Line>> {
    let tasks = object(&work["tasks"])?;
    work["order"]
        .as_array()
        .ok_or(Invalid)?
        .iter()
        .map(|key| {
            let task = &tasks[text(key)?];
            Ok(Line {
                id: text(key)?.to_owned(),
                state: text(&task["state"])?.to_owned(),
                signoff: task.get("needsSignoff").is_some(),
                assignee: task.get("assignee").cloned(),
                child: task["child"].as_str().map(str::to_owned),
            })
        })
        .collect()
}
pub fn derive(lines: &[Line], stage_of: &dyn Fn(&str) -> Option<String>) -> Derived {
    let mut done = 0;
    let mut signoff = 0;
    let mut working: Vec<Value> = Vec::new();
    let mut states = BTreeMap::new();
    for line in lines {
        let followed = line
            .child
            .as_deref()
            .and_then(stage_of)
            .and_then(|stage| linked_state(&stage, line.signoff));
        if let Some(state) = followed {
            states.insert(line.id.clone(), state);
        }
        match followed.unwrap_or(line.state.as_str()) {
            "done" => done += 1,
            "in-progress" => {
                if let Some(assignee) = line.assignee.as_ref().filter(|a| !working.contains(a)) {
                    working.push(assignee.clone());
                }
            }
            "in-review" if line.signoff => signoff += 1,
            _ => {}
        }
    }
    let total = lines.len();
    let stage = match (total, done) {
        (0, _) => "discussion",
        (total, done) if total == done => "done",
        _ => "in-progress",
    };
    Derived {
        stage,
        done,
        total,
        working,
        signoff,
        states,
    }
}
impl Derived {
    pub fn to_value(&self) -> Value {
        let mut value = json!({"stage":self.stage,"done":self.done,"total":self.total,
            "working":self.working,"signoff":self.signoff});
        if !self.states.is_empty() {
            value["states"] = json!(self.states);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(base: &Value, action: &str, body: &Value, attribution: &Value) -> Result<Value> {
        apply(base, action, body, attribution)
    }
    #[test]
    fn shared_work_vectors() {
        let vectors: Value =
            serde_json::from_str(include_str!("../../../contracts/vectors/work-v1.json")).unwrap();
        for case in vectors["cases"].as_array().unwrap() {
            let base = case["document"].clone();
            let result = if let Some(action) = case.get("action") {
                at(
                    &base,
                    action.as_str().unwrap(),
                    &case["body"],
                    &case["attribution"],
                )
            } else {
                validate(&base).map(|()| base.clone())
            };
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}: {result:?}",
                case["name"]
            );
            if let Some(expected) = case.get("expected") {
                assert_eq!(result.as_ref().unwrap(), expected, "{}", case["name"]);
            }
            if case["valid"] == true {
                assert_eq!(
                    String::from_utf8(canonical(result.as_ref().unwrap()).unwrap()).unwrap(),
                    case["canonical"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
                if let Some(derived) = case.get("derived") {
                    // A case may say the stage of each page its tasks follow.
                    let stage = |page: &str| case["stages"][page].as_str().map(str::to_owned);
                    assert_eq!(
                        &derived_with(result.as_ref().unwrap(), &stage)
                            .unwrap()
                            .to_value(),
                        derived,
                        "{}",
                        case["name"]
                    );
                }
            }
            assert_eq!(base, case["document"]);
        }
    }

    #[test]
    fn exact_task_count_and_document_bounds() {
        let mut value = empty();
        for n in 1..=TASK_LIMIT {
            let key = format!("T-{n}");
            value["tasks"][&key] = json!({"title":"t","state":"todo"});
            value["order"].as_array_mut().unwrap().push(json!(key));
        }
        value["next"] = json!(TASK_LIMIT + 1);
        validate(&value).unwrap();
        let mut too_many = value.clone();
        too_many["tasks"]["T-257"] = json!({"title":"t","state":"todo"});
        too_many["order"]
            .as_array_mut()
            .unwrap()
            .push(json!("T-257"));
        too_many["next"] = json!(258);
        assert_eq!(validate(&too_many), Err(Invalid));
        // Fill the details up to exactly the document bound, then one byte over.
        let keys: Vec<String> = value["order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let mut last = String::new();
        for key in keys {
            value["tasks"][&key]["detail"] = json!("");
            let remaining = DOCUMENT_BYTES - canonical(&value).unwrap().len();
            value["tasks"][&key]["detail"] = json!("x".repeat(remaining.min(WORK_DETAIL_BYTES)));
            if remaining <= WORK_DETAIL_BYTES {
                last = key;
                break;
            }
        }
        assert_eq!(canonical(&value).unwrap().len(), DOCUMENT_BYTES);
        validate(&value).unwrap();
        let detail = value["tasks"][&last]["detail"]
            .as_str()
            .unwrap()
            .to_string();
        value["tasks"][&last]["detail"] = json!(format!("{detail}x"));
        assert_eq!(validate(&value), Err(Invalid));
    }

    #[test]
    fn task_numbers_are_exact() {
        for good in ["T-1", "T-10", "T-9999999"] {
            assert!(task_number(good).is_some(), "{good}");
        }
        for bad in [
            "T-0",
            "T-01",
            "T-",
            "t-1",
            "T-+1",
            "T-10000000",
            "T--1",
            "1",
            "T-1 ",
        ] {
            assert!(task_number(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_removed_number_is_never_issued_again() {
        let attribution = json!({"writerId":"10000000-0000-4000-8000-000000000001","timeMs":1});
        let mut value = empty();
        for title in ["a", "b"] {
            value = at(
                &value,
                "task.add",
                &json!({"task":{"title":title}}),
                &attribution,
            )
            .unwrap();
        }
        value = at(
            &value,
            "task.remove",
            &json!({"taskId":"T-2"}),
            &attribution,
        )
        .unwrap();
        value = at(
            &value,
            "task.add",
            &json!({"task":{"title":"c"}}),
            &attribution,
        )
        .unwrap();
        assert_eq!(value["order"], json!(["T-1", "T-3"]));
        assert_eq!(value["next"], 4);
    }
}
