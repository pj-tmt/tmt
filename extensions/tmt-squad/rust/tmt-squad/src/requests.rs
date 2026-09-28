//! Request state read from core: the user's open annotations in the squad
//! room and the requests members are waiting on the user for. Everything is
//! derived per load from bounded `requests.list` pages; nothing is stored and
//! nothing here acknowledges.

use crate::{
    core::{Core, SquadError},
    squad::Squad,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Items per `requests.list` page, and the most pages one view reads.
pub const PAGE: usize = 50;
pub const PAGES: usize = 4;

/// The newest requests matching a filter, and whether older ones exist.
pub struct Window {
    pub items: Vec<Value>,
    pub complete: bool,
}

/// Follows `nextBefore` for at most [`PAGES`] pages.
pub fn window(
    mut fetch: impl FnMut(Value) -> Result<Value, SquadError>,
    filter: &Value,
) -> Result<Window, SquadError> {
    let mut items = Vec::new();
    let mut before = Value::Null;
    for _ in 0..PAGES {
        let mut input = filter.clone();
        input["limit"] = json!(PAGE);
        if !before.is_null() {
            input["before"] = before;
        }
        let page = fetch(input)?;
        items.extend(page["items"].as_array().into_iter().flatten().cloned());
        before = page["nextBefore"].clone();
        if before.is_null() {
            return Ok(Window {
                items,
                complete: true,
            });
        }
    }
    Ok(Window {
        items,
        complete: false,
    })
}

/// The tag that ties an annotation to a squad row.
pub fn tag(squad: &str, row: &str) -> String {
    format!("[{squad} · {row}] ")
}

fn open_request(item: &Value) -> bool {
    item["kind"] == "request" && item["final"]["status"] == "not_submitted"
}

fn sender(item: &Value) -> Option<&str> {
    item["sender"]["identityId"].as_str()
}

/// Per row name, the user's newest open annotation: `{requestId, to, text}`.
fn annotations(
    room: &Window,
    me: &str,
    squad: &str,
    rows: &[&str],
    names: &BTreeMap<String, String>,
) -> BTreeMap<String, Value> {
    let mut found = BTreeMap::new();
    for item in room
        .items
        .iter()
        .filter(|item| open_request(item) && sender(item) == Some(me))
    {
        let preview = item["preview"].as_str().unwrap_or_default();
        for row in rows {
            let Some(text) = preview.strip_prefix(&tag(squad, row)) else {
                continue;
            };
            let to = item["recipientId"].as_str().unwrap_or_default();
            found.entry((*row).to_owned()).or_insert_with(|| {
                json!({
                    "requestId": item["requestId"],
                    "to": names.get(to).map_or(to, String::as_str),
                    "text": text,
                })
            });
        }
    }
    found
}

/// Per sender identity ID, open requests to the user, newest first.
fn waiting(inbox: &Window) -> BTreeMap<String, Vec<Value>> {
    let mut found: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for item in inbox.items.iter().filter(|item| open_request(item)) {
        if let Some(sender) = sender(item) {
            found.entry(sender.to_owned()).or_default().push(json!({
                "requestId": item["requestId"],
                "preview": item["preview"],
                "preparedAtMs": item["preparedAtMs"],
            }));
        }
    }
    found
}

/// Visits the lead and every section row; a member can appear in several
/// user sections, and each copy gets the same values.
fn each_row(document: &mut Value, mut visit: impl FnMut(&mut Value)) {
    let lead = &mut document["squad"]["lead"];
    if lead.is_object() {
        visit(lead);
    }
    for section in document["sections"].as_array_mut().into_iter().flatten() {
        for row in section["rows"].as_array_mut().into_iter().flatten() {
            visit(row);
        }
    }
}

/// Adds `annotation` and `waitingOnYou` to every row of a status document,
/// and `olderRequestsNotShown` when a window was cut off.
pub fn apply(document: &mut Value, squad: &str, me: &str, room: &Window, inbox: &Window) {
    let mut names = BTreeMap::new();
    each_row(document, |row| {
        if let (Some(id), Some(name)) = (row["id"].as_str(), row["name"].as_str()) {
            names.insert(id.to_owned(), name.to_owned());
        }
    });
    let row_names: Vec<String> = names.values().cloned().collect();
    let rows: Vec<&str> = row_names.iter().map(String::as_str).collect();
    let annotations = annotations(room, me, squad, &rows, &names);
    let waiting = waiting(inbox);
    each_row(document, |row| {
        let name = row["name"].as_str().unwrap_or_default().to_owned();
        let id = row["id"].as_str().unwrap_or_default().to_owned();
        row["annotation"] = annotations.get(&name).cloned().unwrap_or(Value::Null);
        row["waitingOnYou"] = json!(waiting.get(&id).cloned().unwrap_or_default());
    });
    document["olderRequestsNotShown"] = json!(!(room.complete && inbox.complete));
}

/// The saved identity that is the user, as `(id, name)`.
pub fn me(core: &Core, name: &str) -> Result<(String, String), SquadError> {
    let shown = core.json(&["identity", "show", name])?;
    match (
        shown["identity"]["id"].as_str(),
        shown["identity"]["name"].as_str(),
    ) {
        (Some(id), Some(name)) => Ok((id.to_owned(), name.to_owned())),
        _ => Err(SquadError::new(
            "SQUAD_CORE_UNAVAILABLE",
            "tmt identity show returned no identity.",
        )),
    }
}

/// Reads both windows and applies them; without `me` rows get empty values.
pub fn overlay(
    core: &Core,
    squad: &Squad,
    me: Option<&str>,
    document: &mut Value,
) -> Result<(), SquadError> {
    let Some(me) = me else {
        let empty = Window {
            items: Vec::new(),
            complete: true,
        };
        apply(document, &squad.name, "", &empty, &empty);
        return Ok(());
    };
    let (me_id, _) = self::me(core, me)?;
    let fetch = |input| core.api("requests.list", input);
    let room = window(fetch, &json!({"roomId": squad.room_id}))?;
    let inbox = window(fetch, &json!({"recipientId": me_id}))?;
    apply(document, &squad.name, &me_id, &room, &inbox);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, from: &str, to: &str, preview: &str, done: bool) -> Value {
        json!({
            "requestId": id, "kind": "request", "recipientId": to,
            "sender": {"kind": "explicit", "identityId": from},
            "final": {"status": if done { "submitted" } else { "not_submitted" }},
            "preview": preview, "preparedAtMs": 1,
        })
    }

    #[test]
    fn the_pager_reads_at_most_the_bounded_window() {
        assert_eq!((PAGE, PAGES), (50, 4), "200 requests at most");
        let mut calls = Vec::new();
        let endless = window(
            |input| {
                calls.push(input.clone());
                Ok(json!({"items": [{"requestId": "r"}], "nextBefore": format!("c{}", calls.len())}))
            },
            &json!({"roomId": "room"}),
        )
        .unwrap();
        assert!(!endless.complete);
        assert_eq!(calls.len(), PAGES);
        assert_eq!(calls[0], json!({"roomId": "room", "limit": 50}));
        assert_eq!(calls[1]["before"], "c1", "the cursor is passed unchanged");

        let mut calls = 0;
        let short = window(
            |_| {
                calls += 1;
                Ok(json!({"items": [], "nextBefore": null}))
            },
            &json!({}),
        )
        .unwrap();
        assert!(short.complete);
        assert_eq!(calls, 1);
    }

    #[test]
    fn open_annotations_and_requests_to_the_user_attach_to_rows() {
        let mut document = json!({
            "squad": {"name": "product", "lead": {"id": "L", "name": "sol"}},
            "sections": [{"title": null, "rows": [
                {"id": "A", "name": "auth-fix"}, {"id": "D", "name": "docs"}
            ]}]
        });
        let room = Window {
            items: vec![
                item("r3", "ME", "L", "[product · auth-fix] newest note", false),
                item("r2", "ME", "A", "[product · auth-fix] older note", false),
                item("r1", "ME", "L", "[product · docs] answered", true),
                item("r0", "X", "L", "[product · docs] someone else", false),
                item("r9", "ME", "D", "plain talk, untagged", false),
            ],
            complete: true,
        };
        let inbox = Window {
            items: vec![
                item("q2", "A", "ME", "approve the plan?", false),
                item("q1", "A", "ME", "answered already", true),
            ],
            complete: false,
        };
        apply(&mut document, "product", "ME", &room, &inbox);
        let rows = &document["sections"][0]["rows"];
        assert_eq!(
            rows[0]["annotation"],
            json!({"requestId": "r3", "to": "sol", "text": "newest note"})
        );
        assert_eq!(rows[1]["annotation"], Value::Null, "answered or not mine");
        assert_eq!(rows[0]["waitingOnYou"][0]["requestId"], "q2");
        assert_eq!(rows[0]["waitingOnYou"].as_array().unwrap().len(), 1);
        assert_eq!(rows[1]["waitingOnYou"], json!([]));
        assert_eq!(document["squad"]["lead"]["annotation"], Value::Null);
        assert_eq!(document["olderRequestsNotShown"], true);
    }
}
