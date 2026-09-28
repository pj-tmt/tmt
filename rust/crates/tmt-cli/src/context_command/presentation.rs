//! Byte-bounded context output; never truncate serialized JSON or inspect commands.

use serde_json::{Value, json};
use std::io;
use tmt_adapters::storage::IdentityContextSnapshot;

pub(super) const OUTPUT_LIMIT: usize = 4096;
const UNBOUND_HINT: &str = "TMT: this pane has no identity. If the user wants TMT messaging here, they can run: tmt name <name> (-s to save).";

pub(super) fn unbound() -> Value {
    json!({"bound": false, "status": "unbound", "hint": UNBOUND_HINT})
}

pub(super) fn unavailable() -> Value {
    json!({"bound": false, "status": "unavailable"})
}

fn requests(count: u64, identity: &str, incoming: bool) -> Value {
    let identity = identity.replace('\'', "'\\''");
    json!({"count": count, "inspect": format!("tmt x{} --identity '{identity}' --json",
        if incoming { " --incoming" } else { "" })})
}

pub(super) fn document(snapshot: IdentityContextSnapshot, notes: Option<String>) -> Value {
    let identity = snapshot.entry.identity;
    json!({"bound": true, "id": identity.id, "name": identity.name,
        "lifetime": identity.lifetime.as_str(), "role": snapshot.role, "notesPath": notes,
        "originated": requests(snapshot.requests.originated, &identity.id, false),
        "incoming": requests(snapshot.requests.incoming, &identity.id, true),
        "extensions": [], "truncated": snapshot.role_truncated})
}

fn render(document: &Value, json_mode: bool) -> io::Result<String> {
    if json_mode {
        return serde_json::to_string(document)
            .map(|text| text + "\n")
            .map_err(io::Error::other);
    }
    if document["bound"] != true {
        return Ok(if document["status"] == "unbound" {
            format!("{UNBOUND_HINT}\n")
        } else {
            String::new()
        });
    }
    // Quote user text as data, escaping embedded control lines.
    let mut text = format!(
        "TMT identity: {} ({})\n",
        document["name"],
        document["lifetime"].as_str().unwrap_or_default()
    );
    for (key, title) in [("role", "Role"), ("notesPath", "Notes path")] {
        if !document[key].is_null() {
            text.push_str(&format!("{title}: {}\n", document[key]));
        }
    }
    for (key, label) in [("originated", "Originated"), ("incoming", "Incoming")] {
        text.push_str(&format!(
            "{label} X items: {} unacknowledged; {}\n",
            document[key]["count"],
            document[key]["inspect"].as_str().unwrap_or_default()
        ));
    }
    if document["truncated"] == true {
        text.push_str("Context shortened to the output limit.\n");
    }
    Ok(text)
}

pub(super) fn bounded(mut document: Value, json_mode: bool) -> io::Result<String> {
    loop {
        let text = render(&document, json_mode)?;
        if text.len() <= OUTPUT_LIMIT {
            return Ok(text);
        }
        document["truncated"] = true.into();
        if let Some(role) = document["role"].as_str().filter(|value| !value.is_empty()) {
            document["role"] = role
                .chars()
                .take(role.chars().count() / 2)
                .collect::<String>()
                .into();
        } else if !document["notesPath"].is_null() {
            document["notesPath"] = Value::Null;
        } else {
            return Err(io::Error::other(
                "Identity context exceeds its output bound.",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_roles_and_paths_stay_bounded_without_losing_counts_or_commands() {
        let value = json!({"bound": true, "id": "identity", "name": "Agent", "lifetime": "saved",
            "extensions": [], "role": "\0".repeat(500), "notesPath": "p".repeat(8000),
            "truncated": false, "originated": requests(100, "identity", false),
            "incoming": requests(200, "identity", true)});
        for json_mode in [true, false] {
            let output = bounded(value.clone(), json_mode).unwrap();
            assert!(output.len() <= OUTPUT_LIMIT);
            assert!(output.ends_with('\n'));
            assert!(output.contains("tmt x --incoming --identity 'identity' --json"));
            if json_mode {
                let parsed: Value = serde_json::from_str(&output).unwrap();
                assert_eq!(parsed["truncated"], true);
                assert_eq!(parsed["originated"]["count"], 100);
                assert_eq!(parsed["incoming"]["count"], 200);
            }
        }
    }

    #[test]
    fn only_verified_unbound_context_suggests_binding() {
        assert_eq!(
            bounded(unbound(), false).unwrap(),
            format!("{UNBOUND_HINT}\n")
        );
        assert_eq!(bounded(unavailable(), false).unwrap(), "");
        assert_eq!(
            serde_json::from_str::<Value>(&bounded(unavailable(), true).unwrap()).unwrap()["status"],
            "unavailable"
        );
        assert_eq!(
            requests(1, "id'quote", true)["inspect"],
            "tmt x --incoming --identity 'id'\\''quote' --json"
        );
    }
}
