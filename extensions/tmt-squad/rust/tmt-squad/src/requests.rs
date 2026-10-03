//! Request state read from core: the user's open annotations in the squad
//! room and the finals to the user's squad requests (from bounded
//! `requests.list` pages), and the requests members are waiting on the user
//! for (from `tmt inbox`). Everything is derived per load; nothing is stored
//! and nothing here acknowledges.

use crate::{
    core::{Core, SquadError},
    squad::Squad,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Items per `requests.list` page, and the most pages one view reads.
pub const PAGE: usize = 50;
pub const PAGES: usize = 4;
/// Replies whose bodies one view reads; older ones show only their header.
pub const BODIES: usize = 8;

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

/// A readable, bounded source-line quote fits inside core's 160-character preview.
pub fn note_row(squad: &str, line: usize, text: &str) -> String {
    let mut quote: String = text.chars().take(40).collect();
    loop {
        let row = format!(
            "notes L{} {}",
            line + 1,
            serde_json::to_string(&quote).expect("text quote")
        );
        if tag(squad, &row).chars().count() <= 150 || quote.is_empty() {
            return row;
        }
        quote.pop();
    }
}

fn note_annotations(room: &Window, me: &str, squad: &str, lead: &str) -> Vec<Value> {
    let prefix = format!("[{squad} · notes L");
    room.items
        .iter()
        .filter(|item| {
            open_request(item) && sender(item) == Some(me) && item["recipientId"] == lead
        })
        .filter_map(|item| {
            let preview = item["preview"].as_str()?.strip_prefix(&prefix)?;
            let (line, rest) = preview.split_once(' ')?;
            let line = line.parse::<usize>().ok()?.checked_sub(1)?;
            let mut quote = serde_json::Deserializer::from_str(rest).into_iter::<String>();
            let text = quote.next()?.ok()?;
            rest.get(quote.byte_offset()..)?.strip_prefix("] ")?;
            Some(json!({"requestId": item["requestId"], "line": line, "quote": text}))
        })
        .collect()
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

/// Per sender identity ID, the requests waiting on the user, oldest first,
/// from `tmt inbox`: core decides what still takes an answer.
fn waiting(inbox: &Window) -> BTreeMap<String, Vec<Value>> {
    let mut found: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for item in &inbox.items {
        if let Some(sender) = item["from"]["identityId"].as_str() {
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
    let lead = document["squad"]["lead"]["id"].as_str().unwrap_or_default();
    let notes = note_annotations(room, me, squad, lead);
    if notes.is_empty() {
        if let Some(squad) = document["squad"].as_object_mut() {
            squad.remove("noteAnnotations");
        }
    } else {
        document["squad"]["noteAnnotations"] = json!(notes);
    }
    let waiting = waiting(inbox);
    each_row(document, |row| {
        let name = row["name"].as_str().unwrap_or_default().to_owned();
        let id = row["id"].as_str().unwrap_or_default().to_owned();
        row["annotation"] = annotations.get(&name).cloned().unwrap_or(Value::Null);
        row["waitingOnYou"] = json!(waiting.get(&id).cloned().unwrap_or_default());
    });
    document["olderRequestsNotShown"] = json!(!(room.complete && inbox.complete));
}

/// The squad room's requests as the user sent them, kept for [`replies`].
pub struct Sent {
    me: String,
    room: Window,
}

/// Reads both windows and applies them; without `me` rows get empty values
/// and there is nothing sent. A room window already read for the observation
/// is reused rather than read again.
pub fn overlay(
    core: &Core,
    squad: &Squad,
    me: Option<&crate::me::Me>,
    document: &mut Value,
    room: Option<Window>,
) -> Result<Option<Sent>, SquadError> {
    let Some(me) = me else {
        let empty = Window {
            items: Vec::new(),
            complete: true,
        };
        apply(document, &squad.name, "", &empty, &empty);
        return Ok(None);
    };
    let me_id = me.id.clone();
    let room = match room {
        Some(room) => room,
        None => room_window(core, squad)?,
    };
    let inbox = inbox(core, &me_id)?;
    apply(document, &squad.name, &me_id, &room, &inbox);
    Ok(Some(Sent { me: me_id, room }))
}

/// The same bounded room history for annotations, replies and activity.
pub fn room_window(core: &Core, squad: &Squad) -> Result<Window, SquadError> {
    window(
        |input| core.api("requests.list", input),
        &json!({"roomId": squad.room_id}),
    )
}

/// The requests waiting on the user, from `tmt inbox`.
pub fn inbox(core: &Core, me_id: &str) -> Result<Window, SquadError> {
    let inbox = core.json(&[
        "inbox",
        "--identity",
        me_id,
        "--limit",
        &(PAGE * PAGES).to_string(),
    ])?;
    Ok(Window {
        items: inbox["items"].as_array().cloned().unwrap_or_default(),
        complete: inbox["more"] != true,
    })
}

/// Only what waits on the user, for a squad whose rows are not shown: the
/// same `waitingOnYou` values, without reading the squad room.
pub fn apply_waiting(document: &mut Value, squad: &str, me: &str, inbox: &Window) {
    let room = Window {
        items: Vec::new(),
        complete: true,
    };
    apply(document, squad, me, &room, inbox);
}

/// Finals to the user's requests in the squad room, newest final first:
/// `{requestId, to, prompt, status, submittedAtMs, response}`. `response`
/// is filled by [`bodies`]; listing never reads or acknowledges one.
pub fn replies(sent: &Sent, document: &Value) -> Vec<Value> {
    let mut names = BTreeMap::new();
    let lead = &document["squad"]["lead"];
    let rows = document["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|section| section["rows"].as_array().into_iter().flatten());
    for row in rows.chain([lead]) {
        if let (Some(id), Some(name)) = (row["id"].as_str(), row["name"].as_str()) {
            names.insert(id, name);
        }
    }
    let mut replies: Vec<Value> = sent
        .room
        .items
        .iter()
        .filter(|item| {
            item["kind"] == "request"
                && sender(item) == Some(sent.me.as_str())
                && !matches!(
                    item["final"]["status"].as_str(),
                    None | Some("not_submitted" | "not_required")
                )
        })
        .map(|item| {
            let to = item["recipientId"].as_str().unwrap_or_default();
            json!({
                "requestId": item["requestId"],
                "to": names.get(to).copied().unwrap_or(to),
                "prompt": item["preview"],
                "status": item["final"]["status"],
                "submittedAtMs": item["final"]["submittedAtMs"],
                "response": null,
            })
        })
        .collect();
    replies.sort_by_key(|reply| std::cmp::Reverse(reply["submittedAtMs"].as_u64()));
    replies
}

/// Fills `response` for the newest [`BODIES`] retained finals. A submitted
/// final never changes, so bodies are cached by request ID; the cache keeps
/// only what the current list shows.
pub fn bodies(
    mut show: impl FnMut(&str) -> Result<Value, SquadError>,
    replies: &mut [Value],
    cache: &mut BTreeMap<String, String>,
) -> Result<(), SquadError> {
    let shown: Vec<String> = replies
        .iter()
        .take(BODIES)
        .filter_map(|reply| reply["requestId"].as_str().map(str::to_owned))
        .collect();
    cache.retain(|id, _| shown.contains(id));
    for reply in replies.iter_mut().take(BODIES) {
        if reply["status"] != "retained" {
            continue;
        }
        let id = reply["requestId"].as_str().unwrap_or_default().to_owned();
        if !cache.contains_key(&id) {
            let detail = show(&id)?;
            if let Some(response) = detail["final"]["response"].as_str() {
                cache.insert(id.clone(), response.to_owned());
            }
        }
        if let Some(response) = cache.get(&id) {
            reply["response"] = json!(response);
        }
    }
    Ok(())
}

/// A compact age: 45s, 12m, 3h or 2d.
pub fn age(now_ms: u64, then_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(then_ms) / 1000;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

/// Reads one request's detail through `requests.show`, which acknowledges
/// nothing.
pub fn show_request(core: &Core, id: &str) -> Result<Value, SquadError> {
    core.api("requests.show", json!({"requestId": id}))
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
    fn note_annotations_are_quoted_bounded_and_only_open_mine_to_current_lead() {
        let squad = "abcdefghijklmnopqrstuvwx";
        let text = "\"] malicious ] \\".repeat(100);
        let row = note_row(squad, 12, &text);
        let preview = format!("{}message", tag(squad, &row));
        assert!(tag(squad, &row).chars().count() <= 150);
        let room = Window {
            items: vec![
                item("mine", "ME", "L", &preview, false),
                item("answered", "ME", "L", &preview, true),
                item("other", "OTHER", "L", &preview, false),
                item("foreign", "ME", "OTHER", &preview, false),
                item(
                    "broken",
                    "ME",
                    "L",
                    &format!("[{squad} · notes L3 bad"),
                    false,
                ),
            ],
            complete: true,
        };
        let mut document = json!({"squad": {"lead": {"id": "L", "name": "lead"}}, "sections": []});
        let inbox = Window {
            items: Vec::new(),
            complete: true,
        };
        apply(&mut document, squad, "ME", &room, &inbox);
        let notes = document["squad"]["noteAnnotations"].as_array().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0]["requestId"], "mine");
        assert_eq!(notes[0]["line"], 12);
        assert!(text.starts_with(notes[0]["quote"].as_str().unwrap()));
        let answered = Window {
            items: vec![item("mine", "ME", "L", &preview, true)],
            complete: true,
        };
        apply(&mut document, squad, "ME", &answered, &inbox);
        assert!(document["squad"].get("noteAnnotations").is_none());
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
    fn replies_list_only_my_finals_newest_first_and_bodies_are_bounded_and_cached() {
        let final_item = |id: &str, from: &str, to: &str, at: u64, status: &str| {
            let mut item = item(id, from, to, "[product · auth-fix] note", false);
            item["final"] = json!({"status": status, "submittedAtMs": at});
            item
        };
        let mut items = vec![
            final_item("a", "ME", "L", 10, "retained"),
            final_item("b", "ME", "A", 30, "retained"),
            final_item("c", "X", "L", 40, "retained"),
            final_item("d", "ME", "L", 20, "expired"),
            item("e", "ME", "L", "still open", false),
        ];
        for index in 0..10 {
            items.push(final_item(
                &format!("old{index}"),
                "ME",
                "L",
                index,
                "retained",
            ));
        }
        let sent = Sent {
            me: "ME".into(),
            room: Window {
                items,
                complete: true,
            },
        };
        let document = json!({
            "squad": {"lead": {"id": "L", "name": "sol"}},
            "sections": [{"rows": [{"id": "A", "name": "auth-fix"}]}]
        });
        let mut list = replies(&sent, &document);
        let order: Vec<&str> = list
            .iter()
            .take(3)
            .map(|r| r["requestId"].as_str().unwrap())
            .collect();
        assert_eq!(
            order,
            ["b", "d", "a"],
            "only mine, finals only, newest first"
        );
        assert_eq!(list[0]["to"], "auth-fix");
        assert_eq!(list.len(), 13);

        let reads = std::cell::Cell::new(0);
        let mut cache = BTreeMap::new();
        let show = |id: &str| {
            reads.set(reads.get() + 1);
            Ok(json!({"final": {"response": format!("body of {id}")}}))
        };
        bodies(show, &mut list, &mut cache).unwrap();
        assert_eq!(list[0]["response"], "body of b");
        assert_eq!(
            list[1]["response"],
            Value::Null,
            "an expired final has no body"
        );
        assert!(
            list[BODIES]["response"].is_null(),
            "older ones stay headers"
        );
        assert_eq!(
            reads.get(),
            BODIES - 1,
            "retained ones among the newest eight"
        );
        bodies(show, &mut list, &mut cache).unwrap();
        assert_eq!(reads.get(), BODIES - 1, "cached bodies are not read again");
        assert!(cache.len() <= BODIES);
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
        // `tmt inbox --json` items: only what still waits, oldest first.
        let waiting = |id: &str, from: &str, preview: &str| json!({"requestId": id, "from": {"identityId": from, "name": from}, "preview": preview, "preparedAtMs": 1});
        let inbox = Window {
            items: vec![
                waiting("q1", "A", "which branch?"),
                waiting("q2", "A", "approve the plan?"),
                waiting("q3", "X", "from someone off the board"),
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
        assert_eq!(rows[0]["waitingOnYou"][0]["requestId"], "q1");
        assert_eq!(rows[0]["waitingOnYou"][1]["requestId"], "q2");
        assert_eq!(rows[0]["waitingOnYou"].as_array().unwrap().len(), 2);
        assert_eq!(rows[1]["waitingOnYou"], json!([]));
        assert_eq!(document["squad"]["lead"]["annotation"], Value::Null);
        assert_eq!(document["olderRequestsNotShown"], true);
    }
}
